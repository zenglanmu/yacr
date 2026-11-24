//! Undo/redo over the drawing database (spec v2.0 §4.7, §16.3).
//!
//! Undo stores explicit before/after patches; it never re-executes a command,
//! so a non-deterministic recomputation can never produce a different result.
//! Undo and redo publish a [`ChangeSet`] like any other commit.

use cad_db::{DbEntity, EntityDisplayState};
use cad_domain::*;
use std::collections::{HashSet, VecDeque};

mod group_patch_bytes;
mod param_bytes;
mod record_bytes;

use param_bytes::{expr_bytes, param_row_bytes};

/// A reversible change to one drawing entity (spec F-EDIT).
///
/// It keeps the full [`DbEntity`] before and after so an undo or redo restores
/// the geometry *and* the identity/layer/space/draw_order exactly, without
/// re-executing the original command. `None` on either side means "did not
/// exist" (a creation or a deletion). A record may apply several of these in one
/// commit (for example a MOVE over a multi-selection).
///
/// A delete also prunes per-entity display state (render attributes and
/// dynamic-visibility membership) that the [`DbEntity`] does not carry, so the
/// patch keeps an optional [`EntityDisplayState`] for each side; the undo/redo
/// applier restores it when that side is re-inserted.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingPatch {
    pub id: EntityId,
    /// `None` means the entity did not exist before (a creation).
    pub before: Option<DbEntity>,
    /// `None` means the entity does not exist after (a deletion).
    pub after: Option<DbEntity>,
    /// Display state to restore when the `before` side is (re-)inserted.
    pub before_state: Option<EntityDisplayState>,
    /// Display state to restore when the `after` side is (re-)inserted.
    pub after_state: Option<EntityDisplayState>,
}

/// Build a drawing patch from the difference between two entity states.
///
/// Display state is not captured; use [`drawing_patch_deleted`] for a deletion
/// that removed render attributes / dynamic-visibility membership.
pub fn drawing_patch(
    id: EntityId,
    before: Option<DbEntity>,
    after: Option<DbEntity>,
) -> DrawingPatch {
    DrawingPatch {
        id,
        before,
        after,
        before_state: None,
        after_state: None,
    }
}

/// A deletion patch that also captures the pre-delete display state, so an undo
/// restores the entity's render attributes and dynamic-visibility membership.
pub fn drawing_patch_deleted(
    id: EntityId,
    before: DbEntity,
    before_state: EntityDisplayState,
) -> DrawingPatch {
    DrawingPatch {
        id,
        before: Some(before),
        after: None,
        before_state: Some(before_state),
        after_state: None,
    }
}

/// Build a reversible layer-table patch from the before/after rows.
///
/// `None` on either side means the row did not exist (a create) / no longer
/// exists (a delete). Handlers that will record layer writes use this instead of
/// spelling the struct out, so the row shape stays in `cad-db`.
pub fn layer_patch(
    id: LayerId,
    before: Option<cad_db::LayerRow>,
    after: Option<cad_db::LayerRow>,
) -> cad_db::LayerPatch {
    cad_db::LayerPatch { id, before, after }
}

/// Build a reversible layout-table patch from the before/after rows.
///
/// `None` on either side means the row did not exist (a create) / no longer
/// exists (a delete). The row carries its plot settings, so the delete cascade
/// (delete the layout ⇒ delete its plot row) round-trips exactly.
pub fn layout_patch(
    id: LayoutId,
    before: Option<cad_db::LayoutRow>,
    after: Option<cad_db::LayoutRow>,
) -> cad_db::LayoutPatch {
    cad_db::LayoutPatch { id, before, after }
}

/// Build a reversible text-style-table patch from the before/after rows.
///
/// `None` on either side means the row did not exist (a create) / no longer
/// exists (a delete).
pub fn style_patch(
    id: StyleId,
    before: Option<cad_db::Style>,
    after: Option<cad_db::Style>,
) -> cad_db::StylePatch {
    cad_db::StylePatch { id, before, after }
}

/// Build a reversible dimension-style-table patch from the before/after rows.
pub fn dim_style_patch(
    id: DimStyleId,
    before: Option<cad_db::DimStyle>,
    after: Option<cad_db::DimStyle>,
) -> cad_db::DimStylePatch {
    cad_db::DimStylePatch { id, before, after }
}

/// Build a reversible multileader-style-table patch from the before/after rows.
pub fn mleader_style_patch(
    id: MLeaderStyleId,
    before: Option<cad_db::MLeaderStyle>,
    after: Option<cad_db::MLeaderStyle>,
) -> cad_db::MLeaderStylePatch {
    cad_db::MLeaderStylePatch { id, before, after }
}

/// Build a reversible linetype-table patch from the before/after rows.
///
/// The linetype table has a rename but no create/delete API in this build, so
/// both sides are normally `Some`; the shape still mirrors the other table
/// patches so a future create/delete cannot need a second type.
pub fn linetype_patch(
    id: LinetypeId,
    before: Option<cad_db::LineType>,
    after: Option<cad_db::LineType>,
) -> cad_db::LinetypePatch {
    cad_db::LinetypePatch { id, before, after }
}

/// A reversible batch of drawing-entity changes.
///
/// The patches carry the full before/after [`DbEntity`]; `transaction` is the
/// id to reuse when the record is undone/redone so the resulting [`ChangeSet`]
/// is traceable to the same logical edit. An empty record (no entity, block,
/// layer or layout patch) is a caller bug and [`History`] rejects it, so a
/// command can never record a silent no-op.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingUndoRecord {
    pub transaction: TransactionId,
    pub label: String,
    pub patches: Vec<DrawingPatch>,
    /// Reversible block-table rows (definition + metadata) committed together
    /// with `patches` as one revision. A future `BLOCK` command records the new
    /// definition here and the member moves/reference insert as entity patches,
    /// so undo/redo restores the whole edit atomically.
    pub block_patches: Vec<cad_db::BlockPatch>,
    /// Reversible layer-table rows (layer + attributes) committed together with
    /// `patches`/`block_patches` as one revision. A layer-only record (create,
    /// rename, delete, attribute write) carries these and no entity patch; a
    /// record that deletes a layer and its entities carries both, and the
    /// database applies the layer deletes last.
    pub layer_patches: Vec<cad_db::LayerPatch>,
    /// Reversible layout-table rows (layout + plot settings) committed together
    /// with the other batches as one revision. A layout-only record (create,
    /// rename, delete, viewport/plot write) carries these and no entity patch; a
    /// record that moves the last entity out of a layout and deletes it carries
    /// both, and the database applies layout upserts first and layout deletes
    /// last.
    pub layout_patches: Vec<cad_db::LayoutPatch>,
    /// Reversible text-style-table rows committed together with the other
    /// batches as one revision. A style-only record (create, rename, delete,
    /// modify) carries these and no entity patch.
    pub style_patches: Vec<cad_db::StylePatch>,
    /// Reversible dimension-style-table rows (same atomic rule). A text-style
    /// rename that retargets `DIMTXSTY` carries the dim-style rows here too, so
    /// the cascade is one revision.
    pub dim_style_patches: Vec<cad_db::DimStylePatch>,
    /// Reversible multileader-style-table rows (same atomic rule).
    pub mleader_style_patches: Vec<cad_db::MLeaderStylePatch>,
    /// Records with an equal merge key may be coalesced (e.g. a drag). The
    /// drawing commands in `cad-app` deliberately pass `None` so each user
    /// action is exactly one undo step.
    pub merge_key: Option<String>,
}

impl DrawingUndoRecord {
    /// An entity-patch-only record: the common shape, with every table batch
    /// empty. Callers that write a table use the struct literal so a new batch
    /// cannot be forgotten silently.
    pub fn entity_only(
        transaction: TransactionId,
        label: impl Into<String>,
        patches: Vec<DrawingPatch>,
    ) -> Self {
        DrawingUndoRecord {
            transaction,
            label: label.into(),
            patches,
            block_patches: Vec::new(),
            layer_patches: Vec::new(),
            layout_patches: Vec::new(),
            style_patches: Vec::new(),
            dim_style_patches: Vec::new(),
            mleader_style_patches: Vec::new(),
            merge_key: None,
        }
    }

    /// Whether every batch of this record is empty.
    fn is_empty(&self) -> bool {
        self.patches.is_empty()
            && self.block_patches.is_empty()
            && self.layer_patches.is_empty()
            && self.layout_patches.is_empty()
            && self.style_patches.is_empty()
            && self.dim_style_patches.is_empty()
            && self.mleader_style_patches.is_empty()
    }

    /// Validate the batch shape and the duplicate/coalescing rules.
    ///
    /// Does **not** require a non-empty batch: a linetype-only step carries its
    /// row beside the record (see [`DrawingHistoryStep`]).
    fn validate_batches(&self) -> CadResult<()> {
        // Block, layer and layout rows are not orthogonal per-key edits like a
        // drag: coalescing them into the previous record could silently drop a
        // definition or its members/attributes. Refuse the combination instead
        // of guessing.
        if self.merge_key.is_some()
            && (!self.block_patches.is_empty()
                || !self.layer_patches.is_empty()
                || !self.layout_patches.is_empty()
                || !self.style_patches.is_empty()
                || !self.dim_style_patches.is_empty()
                || !self.mleader_style_patches.is_empty())
        {
            return Err(CadError::InvalidInput(
                "a drawing undo record with block, layer or layout patches cannot be merged".into(),
            ));
        }
        let mut block_ids = HashSet::new();
        for patch in &self.block_patches {
            if !block_ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate block ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a block patch needs a before or after state".into(),
                ));
            }
        }
        let mut layer_ids = HashSet::new();
        for patch in &self.layer_patches {
            if !layer_ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate layer ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a layer patch needs a before or after state".into(),
                ));
            }
        }
        let mut layout_ids = HashSet::new();
        for patch in &self.layout_patches {
            if !layout_ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate layout ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a layout patch needs a before or after state".into(),
                ));
            }
        }
        let mut style_ids = HashSet::new();
        for patch in &self.style_patches {
            if !style_ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate style ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a style patch needs a before or after state".into(),
                ));
            }
        }
        let mut dim_style_ids = HashSet::new();
        for patch in &self.dim_style_patches {
            if !dim_style_ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate dim style ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a dim style patch needs a before or after state".into(),
                ));
            }
        }
        let mut mleader_style_ids = HashSet::new();
        for patch in &self.mleader_style_patches {
            if !mleader_style_ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate multileader style ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a multileader style patch needs a before or after state".into(),
                ));
            }
        }
        let mut ids = HashSet::new();
        for patch in &self.patches {
            if !ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate entity ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a drawing patch needs a before or after state".into(),
                ));
            }
            for entity in patch.before.iter().chain(patch.after.iter()) {
                if entity.id != patch.id || entity.object.id.0 != patch.id.0 {
                    return Err(CadError::InvalidInput(
                        "drawing patch key does not match snapshot identity".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn approx_bytes(&self) -> usize {
        record_bytes::approx_bytes(self)
    }
}

/// One retained history step: the drawing record plus the batches committed
/// with it.
///
/// Every other named table is a field of [`DrawingUndoRecord`]. The linetype
/// table is the one symbol table with a **rename but no create/delete API**, so
/// exactly one command writes it; carrying its row here keeps the record's
/// exhaustive batch list (which every command's record literal names) unchanged.
/// If a second linetype write ever lands, promote this to a record field.
///
/// A second side batch has since been added the same way: BIM element patches
/// ([`Self::bim_patches`]) are a **replace-only** rewrite of a `bim` block that
/// already exists, written by its own database path. It is still not a
/// [`DrawingUndoRecord`] field, because every command's record literal would
/// have to name it and no command besides the BIM property edit writes one.
///
/// The step is the unit undo and redo move: the record's entity/table rows and
/// the linetype row are applied through the database's combined write path, the
/// BIM patches through [`cad_db::DrawingDatabase::apply_bim_updates`], each as
/// one revision. Parameter rows and binding rows are applied through
/// [`cad_db::DrawingDatabase::apply_parametric_update`], also one revision.
///
/// A step carries at most one kind of write (see `validate`): a BIM step is
/// exclusive, and a parametric step must carry an **empty** `record.patches` as
/// well. ADR-0043 D8's "one command" intent is satisfied by carrying the derived
/// geometry on [`cad_db::ParamBindingPatch`] (`before_geometry`/`after_geometry`),
/// not by duplicating entity rows in the record — a mixed step would be silently
/// dropped by the parametric replay branch, so it is refused instead.
///
/// # Replay contract
///
/// A step with param patches is replayed by calling
/// [`cad_db::DrawingDatabase::apply_parametric_update`] with the direction's
/// before/after sides: redo passes the stored `before`/`after`, undo passes the
/// swapped pair (the `after` row as `before` and vice versa), and the matching
/// `before_geometry`/`after_geometry` travels with each binding patch. The
/// record's entity patches are **not** replayed on that branch (geometry is
/// derived, or restored from the binding patch's geometry) — which is exactly
/// why `validate` requires them to be empty.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingHistoryStep {
    pub record: DrawingUndoRecord,
    pub linetype_patches: Vec<cad_db::LinetypePatch>,
    /// Reversible BIM element rows committed with the record. Replace-only (both
    /// sides always present), so a step carrying these never creates or deletes
    /// a `bim` block — see [`cad_db::BimPatch`].
    pub bim_patches: Vec<cad_db::BimPatch>,
    /// Reversible parameter rows (ADR-0043 D8). Unlike [`Self::bim_patches`],
    /// these do **not** ride with any `DrawingUndoRecord` batch: the derived
    /// geometry travels on [`cad_db::ParamBindingPatch::before_geometry`]/
    /// `after_geometry`, so a parametric step carries no record entity/table
    /// rows. Both halves still go through a single
    /// [`cad_db::DrawingDatabase::apply_parametric_update`] call (one revision).
    pub param_patches: Vec<cad_db::ParamPatch>,
    /// Reversible element slot-binding rows committed with the parameter rows.
    pub param_binding_patches: Vec<cad_db::ParamBindingPatch>,
    /// Reversible persisted-group rows (create/delete/rename/member edits).
    ///
    /// Applied by their own one-revision database path
    /// (`apply_group_changes`), so a step carrying them has an empty record and
    /// no linetype/BIM/parameter batch.
    pub group_patches: Vec<cad_db::GroupPatch>,
}

impl DrawingHistoryStep {
    /// Validate the step as a whole: at least one entity/table row, linetype row,
    /// BIM patch or parameter patch; the record's own batch rules; and no
    /// duplicate ids.
    ///
    /// A step carrying BIM patches must be **that alone**: the record's batches
    /// and the linetype batch are applied by a different database path than
    /// [`cad_db::DrawingDatabase::apply_bim_updates`], so mixing them would make
    /// one undo span two revisions.
    ///
    /// A step carrying parameter patches is the deliberate opposite of a BIM
    /// step. ADR-0043 D8's "one command" intent lives on the binding patches
    /// (`before_geometry`/`after_geometry`), so a parametric step must have an
    /// **empty** `record.patches` and every other record batch empty. A mixed
    /// step would be silently dropped by the parametric replay branch, so it is
    /// refused. A linetype row or BIM patch is refused for the same
    /// one-revision reason, and a merge key cannot coalesce a row.
    fn validate(&self) -> CadResult<()> {
        let parametric = !self.param_patches.is_empty() || !self.param_binding_patches.is_empty();
        if self.record.is_empty()
            && self.linetype_patches.is_empty()
            && self.bim_patches.is_empty()
            && self.group_patches.is_empty()
            && !parametric
        {
            return Err(CadError::InvalidInput(
                "a drawing undo step needs at least one entity, block, layer, layout, style, \
                 linetype, BIM or parameter row"
                    .into(),
            ));
        }
        self.record.validate_batches()?;
        if !self.group_patches.is_empty()
            && (!self.record.is_empty()
                || !self.linetype_patches.is_empty()
                || !self.bim_patches.is_empty()
                || parametric)
        {
            return Err(CadError::InvalidInput(
                "a group undo step cannot also carry entity, table, linetype, BIM or parameter \
                 rows"
                    .into(),
            ));
        }
        let mut group_seqs = HashSet::new();
        for patch in &self.group_patches {
            if !group_seqs.insert(patch.seq) {
                return Err(CadError::InvalidInput(
                    "a group undo step cannot contain duplicate group sequences".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a group patch needs a before or after state".into(),
                ));
            }
        }
        let mut ids = HashSet::new();
        for patch in &self.linetype_patches {
            if !ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate linetype ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a linetype patch needs a before or after state".into(),
                ));
            }
        }
        // A linetype row is not an orthogonal per-key edit: coalescing it into a
        // drag would drop the row. Refuse the combination like the other tables.
        if self.record.merge_key.is_some() && !self.linetype_patches.is_empty() {
            return Err(CadError::InvalidInput(
                "a drawing undo record with linetype patches cannot be merged".into(),
            ));
        }
        let mut bim_ids = HashSet::new();
        for patch in &self.bim_patches {
            if !bim_ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo record cannot contain duplicate BIM element ids".into(),
                ));
            }
        }
        if self.record.merge_key.is_some() && !self.bim_patches.is_empty() {
            return Err(CadError::InvalidInput(
                "a drawing undo record with BIM patches cannot be merged".into(),
            ));
        }
        // BIM patches are applied by their own write path (one revision of its
        // own), so they cannot ride with the record's batches or a linetype row:
        // one undo would then have to be two revisions.
        if !self.bim_patches.is_empty()
            && (!self.record.is_empty() || !self.linetype_patches.is_empty())
        {
            return Err(CadError::InvalidInput(
                "a drawing undo step with BIM patches cannot also carry entity or table rows"
                    .into(),
            ));
        }
        let mut param_seqs = HashSet::new();
        for patch in &self.param_patches {
            if !param_seqs.insert(patch.seq) {
                return Err(CadError::InvalidInput(
                    "a drawing undo step cannot contain duplicate parameter sequences".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a parameter patch needs a before or after row".into(),
                ));
            }
        }
        let mut binding_ids = HashSet::new();
        for patch in &self.param_binding_patches {
            if !binding_ids.insert(patch.id) {
                return Err(CadError::InvalidInput(
                    "a drawing undo step cannot contain duplicate parameter binding ids".into(),
                ));
            }
            if patch.before.is_none() && patch.after.is_none() {
                return Err(CadError::InvalidInput(
                    "a parameter binding patch needs a before or after state".into(),
                ));
            }
        }
        if parametric {
            // A parametric commit is one `apply_parametric_update` revision, so it
            // cannot ride with the BIM write path or a linetype row (each is its
            // own revision). Record rows are refused too: derived geometry travels
            // on the binding patches, and the replay branch never applies record
            // rows.
            if !self.bim_patches.is_empty() || !self.linetype_patches.is_empty() {
                return Err(CadError::InvalidInput(
                    "a drawing undo step with parameter patches cannot also carry BIM or \
                     linetype rows"
                        .into(),
                ));
            }
            // A parametric step must not carry record rows at all: the replay
            // branch never applies them, so a mixed step would be silently
            // dropped. Derived geometry travels on the binding patches instead.
            if !self.record.patches.is_empty()
                || !self.record.block_patches.is_empty()
                || !self.record.layer_patches.is_empty()
                || !self.record.layout_patches.is_empty()
                || !self.record.style_patches.is_empty()
                || !self.record.dim_style_patches.is_empty()
                || !self.record.mleader_style_patches.is_empty()
            {
                return Err(CadError::InvalidInput(
                    "a drawing undo step with parameter patches may not carry record rows; \
                     derived geometry travels on the binding patches"
                        .into(),
                ));
            }
            if self.record.merge_key.is_some() {
                return Err(CadError::InvalidInput(
                    "a drawing undo record with parameter patches cannot be merged".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn approx_bytes(&self) -> usize {
        let mut bytes = self.record.approx_bytes();
        for patch in &self.linetype_patches {
            bytes += std::mem::size_of::<cad_db::LinetypePatch>();
            for row in patch.before.iter().chain(patch.after.iter()) {
                bytes += std::mem::size_of::<cad_db::LineType>() + row.name.len();
                bytes += row.pattern.elements.len() * std::mem::size_of::<f64>();
            }
        }
        for patch in &self.bim_patches {
            bytes += std::mem::size_of::<cad_db::BimPatch>();
            // Both sides are retained, so both are counted: a property edit
            // keeps the pre-edit pset alongside its replacement.
            bytes += bim_element_bytes(&patch.before) + bim_element_bytes(&patch.after);
        }
        for patch in &self.param_patches {
            bytes += std::mem::size_of::<cad_db::ParamPatch>();
            for row in patch.before.iter().chain(patch.after.iter()) {
                bytes += param_row_bytes(row);
            }
        }
        bytes += group_patch_bytes::group_patch_bytes(&self.group_patches);
        for patch in &self.param_binding_patches {
            bytes += std::mem::size_of::<cad_db::ParamBindingPatch>();
            for bindings in patch.before.iter().chain(patch.after.iter()) {
                bytes += std::mem::size_of::<cad_domain::param::ParamBindings>();
                for (slot, expr) in bindings.entries() {
                    bytes += slot.len() + expr_bytes(expr);
                }
            }
        }
        bytes
    }
}

/// Estimated retained bytes of one element's BIM data.
///
/// The fixed-size value alone would under-count badly: nearly all of a `bim`
/// block is strings (source provenance, pset/property names, `Text` values and
/// each `Real`'s IFC type/unit), and it is exactly those that a pset edit grows.
/// Without them a step could hold an arbitrarily large pset and still report a
/// few hundred bytes, bypassing the history budget.
fn bim_element_bytes(element: &cad_db::BimElement) -> usize {
    let mut bytes = std::mem::size_of::<cad_db::BimElement>();
    bytes += element.source.connector.len();
    for text in [
        element.category.as_deref(),
        element.name.as_deref(),
        element.source.document_id.as_deref(),
        element.source.id.as_deref(),
        element.source.native_id.as_deref(),
        element.level.as_deref(),
        element.element_type.as_deref(),
        element.material.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        bytes += text.len();
    }
    for set in &element.psets {
        bytes += std::mem::size_of::<cad_db::BimPropertySet>() + set.name.len();
        for property in &set.properties {
            bytes += std::mem::size_of::<cad_db::BimProperty>() + property.name.len();
            match &property.value {
                // Empty `Text` is legal, so only its length is counted.
                cad_db::BimValue::Text(text) => bytes += text.len(),
                cad_db::BimValue::Real { ifc_type, unit, .. } => {
                    bytes += ifc_type.as_deref().map_or(0, str::len);
                    bytes += unit.as_deref().map_or(0, str::len);
                }
                cad_db::BimValue::Integer(_)
                | cad_db::BimValue::Bool(_)
                | cad_db::BimValue::Null => {}
            }
        }
    }
    bytes
}

/// Undo/redo stacks with an approximate budget on retained undo entries.
///
/// The newest undo entry is retained even if it exceeds the budget. The byte
/// estimate is not a total heap measurement: nested geometry allocations,
/// spare collection capacity, redo entries and a pending drawing step are not
/// fully accounted for.
pub struct History {
    pub memory_budget_bytes: usize,
    undo: VecDeque<DrawingHistoryStep>,
    redo: Vec<DrawingHistoryStep>,
    pending: Option<PendingDrawing>,
    used_bytes: usize,
}

impl Default for History {
    fn default() -> Self {
        History::new(16 * 1024 * 1024)
    }
}

impl History {
    pub fn new(memory_budget_bytes: usize) -> Self {
        History {
            memory_budget_bytes,
            undo: VecDeque::new(),
            redo: Vec::new(),
            pending: None,
            used_bytes: 0,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Estimated bytes of retained undo entries.
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Record a committed drawing-entity change. Clears the redo stack.
    ///
    /// Coalescing keeps the earliest before state and the latest after state.
    /// Patches that return to their original state are removed; if all patches
    /// cancel out, the merged entry is removed without touching earlier entries.
    /// Even a fully canceled merge clears redo, since new edits were committed.
    /// An empty patch list is rejected, so a command can never claim success
    /// while recording nothing.
    pub fn record_drawing(&mut self, record: DrawingUndoRecord) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed drawing change plus the linetype-table rows it wrote.
    ///
    /// [`Self::record_drawing`] is the common shape (no linetype rows). The
    /// linetype batch is carried beside the record (see [`DrawingHistoryStep`]),
    /// is never coalesced, and counts toward "this step records something", so a
    /// linetype-only rename is still one real undo step.
    pub fn record_drawing_with_linetypes(
        &mut self,
        record: DrawingUndoRecord,
        linetype_patches: Vec<cad_db::LinetypePatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            linetype_patches,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed BIM property edit plus its element patches.
    ///
    /// The patches are carried beside the record like the linetype batch (see
    /// [`DrawingHistoryStep`]), are never coalesced, and count toward "this step
    /// records something", so a BIM-only edit is one real undo step. The record
    /// must be empty: the BIM batch is applied by its own database path and a
    /// mixed step would make one undo span two revisions (see
    /// `DrawingHistoryStep::validate`).
    pub fn record_drawing_with_bim(
        &mut self,
        record: DrawingUndoRecord,
        bim_patches: Vec<cad_db::BimPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            bim_patches,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    /// Record a committed parametric edit plus the parameter and binding rows it
    /// wrote (ADR-0043 D8).
    ///
    /// The patches are carried beside the record like the linetype/BIM batches
    /// (see [`DrawingHistoryStep`]), are never coalesced, and count toward "this
    /// step records something". A parametric step carries **no** record entity or
    /// table rows: the derived geometry travels on
    /// [`cad_db::ParamBindingPatch::before_geometry`]/`after_geometry` instead,
    /// and `validate()` refuses any non-empty record batch. `validate()` runs
    /// before the step is recorded, so a refused mix leaves the retained history
    /// untouched.
    pub fn record_drawing_with_params(
        &mut self,
        record: DrawingUndoRecord,
        param_patches: Vec<cad_db::ParamPatch>,
        param_binding_patches: Vec<cad_db::ParamBindingPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            param_patches,
            param_binding_patches,
            Vec::new(),
        )
    }

    /// Record a committed group edit plus the rows it wrote.
    ///
    /// The patches are carried beside the record like the linetype/BIM batches,
    /// count toward "this step records something", and are applied by
    /// [`cad_db::DrawingDatabase::apply_group_changes`] as one revision. The
    /// record must be empty (see `DrawingHistoryStep::validate`).
    pub fn record_drawing_with_groups(
        &mut self,
        record: DrawingUndoRecord,
        group_patches: Vec<cad_db::GroupPatch>,
    ) -> CadResult<()> {
        self.record_drawing_step(
            record,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            group_patches,
        )
    }

    fn record_drawing_step(
        &mut self,
        record: DrawingUndoRecord,
        linetype_patches: Vec<cad_db::LinetypePatch>,
        bim_patches: Vec<cad_db::BimPatch>,
        param_patches: Vec<cad_db::ParamPatch>,
        param_binding_patches: Vec<cad_db::ParamBindingPatch>,
        group_patches: Vec<cad_db::GroupPatch>,
    ) -> CadResult<()> {
        self.ensure_no_pending_drawing()?;
        let mut step = DrawingHistoryStep {
            record,
            linetype_patches,
            bim_patches,
            param_patches,
            param_binding_patches,
            group_patches,
        };
        step.validate()?;
        if let Some(key) = step.record.merge_key.clone() {
            if let Some(last) = self.undo.back_mut() {
                if last.record.merge_key.as_deref() == Some(key.as_str()) {
                    for patch in std::mem::take(&mut step.record.patches) {
                        if let Some(existing) =
                            last.record.patches.iter_mut().find(|p| p.id == patch.id)
                        {
                            existing.after = patch.after;
                        } else {
                            last.record.patches.push(patch);
                        }
                    }
                    last.record
                        .patches
                        .retain(|patch| patch.before != patch.after);
                    if last.record.patches.is_empty() {
                        self.undo.pop_back();
                    }
                    self.recompute_used_bytes();
                    self.redo.clear();
                    self.enforce_budget();
                    return Ok(());
                }
            }
        }
        self.used_bytes += step.approx_bytes();
        self.undo.push_back(step);
        self.redo.clear();
        self.enforce_budget();
        Ok(())
    }

    fn recompute_used_bytes(&mut self) {
        self.used_bytes = self.undo.iter().map(|step| step.approx_bytes()).sum();
    }

    fn enforce_budget(&mut self) {
        while self.used_bytes > self.memory_budget_bytes && self.undo.len() > 1 {
            let Some(removed) = self.undo.pop_front() else {
                break;
            };
            self.used_bytes = self.used_bytes.saturating_sub(removed.approx_bytes());
        }
    }

    /// Discard all history, including any pending drawing step.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
        self.used_bytes = 0;
    }

    /// Undo the most recent drawing step, returning its patches.
    ///
    /// The caller applies the `before` entities through the drawing database's
    /// own validated write path; the step is only moved to the redo stack after
    /// the caller reports success via [`History::finish_drawing`].
    /// This method pops the step and holds it until finished, so a failed
    /// apply cannot silently lose or duplicate the step.
    /// Other history writes are rejected until this step is finished or canceled.
    pub fn begin_drawing_undo(&mut self) -> CadResult<DrawingHistoryStep> {
        self.ensure_no_pending_drawing()?;
        let step = self
            .undo
            .pop_back()
            .ok_or_else(|| CadError::InvalidInput("nothing to undo".into()))?;
        self.used_bytes = self.used_bytes.saturating_sub(step.approx_bytes());
        self.pending = Some(PendingDrawing {
            step: step.clone(),
            direction: UndoDirection::Undo,
        });
        Ok(step)
    }

    /// Redo the most recently undone drawing step, returning its patches.
    pub fn begin_drawing_redo(&mut self) -> CadResult<DrawingHistoryStep> {
        self.ensure_no_pending_drawing()?;
        let step = self
            .redo
            .pop()
            .ok_or_else(|| CadError::InvalidInput("nothing to redo".into()))?;
        self.pending = Some(PendingDrawing {
            step: step.clone(),
            direction: UndoDirection::Redo,
        });
        Ok(step)
    }

    /// Commit the pending drawing undo/redo after the database accepted it.
    pub fn finish_drawing(&mut self) -> CadResult<()> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| CadError::Invariant("no pending drawing undo".into()))?;
        match pending.direction {
            UndoDirection::Undo => self.redo.push(pending.step),
            UndoDirection::Redo => {
                self.used_bytes += pending.step.approx_bytes();
                self.undo.push_back(pending.step);
                self.enforce_budget();
            }
        }
        Ok(())
    }

    /// Put a pending drawing undo/redo back where it came from.
    ///
    /// Called when the database rejected the patch after
    /// [`History::begin_drawing_undo`]/[`History::begin_drawing_redo`] popped it:
    /// the step must not be lost. The byte accounting mirrors `finish_drawing`.
    pub fn cancel_pending_drawing(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        match pending.direction {
            UndoDirection::Undo => {
                self.used_bytes += pending.step.approx_bytes();
                self.undo.push_back(pending.step);
            }
            UndoDirection::Redo => {
                self.redo.push(pending.step);
            }
        }
    }

    fn ensure_no_pending_drawing(&self) -> CadResult<()> {
        if self.pending.is_some() {
            return Err(CadError::InvalidInput(
                "finish or cancel the pending drawing history step first".into(),
            ));
        }
        Ok(())
    }
}

enum UndoDirection {
    Undo,
    Redo,
}

struct PendingDrawing {
    step: DrawingHistoryStep,
    direction: UndoDirection,
}

#[cfg(test)]
mod param_tests;

#[cfg(test)]
mod tests;
