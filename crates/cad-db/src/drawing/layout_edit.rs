//! The sanctioned layout/paper-viewport write paths: create, rename, delete,
//! the viewport write, and the undo/redo apply entry.
//!
//! A layout is one row in [`DrawingDatabase::layouts`] plus its plot configuration
//! in [`DrawingDatabase::plot_settings`] beside it, exactly like a layer is a
//! [`Layer`](crate::tables::Layer) plus its attributes (see
//! `drawing::layer_patch`). Every method here validates **everything** first,
//! then mutates, then raises the revision exactly once and publishes a
//! [`ChangeSet`]: a refusal leaves the layout table, the plot settings and the
//! revision exactly as they were.
//!
//! # Why the change set carries no object changes
//!
//! [`ObjectChange`](crate::ObjectChange) addresses entities; there is no layout
//! object-change kind. A layout edit therefore publishes a `ChangeSet` with an
//! empty `changes` list and a raised revision: the revision is the documented
//! staleness tag every derived consumer already binds to (layout tabs, paper
//! scene identity, plot preview), so a new/renamed/deleted layout invalidates
//! them without inventing a fake entity `Update`.
//!
//! # These writes record no undo step
//!
//! [`LayoutRow`]/[`LayoutPatch`] are the row shapes a caller records
//! before/after a layout edit; this module only offers the validated apply entry
//! ([`DrawingDatabase::apply_layout_changes`]) that a later record/replay layer
//! drives. Recording the patch is that layer's job, not this one.
//!
//! # Naming rules
//!
//! Mirrors the app's `validate_layout_name`: the name is stored **trimmed**, and
//! is refused when empty or whitespace-only, when it contains a line break, when
//! it is longer than 255 characters, or when it contains an AutoCAD-illegal
//! character. Names are unique **case-insensitively** (like AutoCAD symbol-table
//! names, so `Sheet` and `SHEET` may not coexist). Nothing is clamped or silently
//! renamed.
//!
//! # Id allocation
//!
//! Layout ids are allocated monotonically from [`DrawingDatabase::next_layout_id`]
//! (initialised by the builder to `max(existing) + 1`, at least 1): allocation is
//! not rewound by a delete, so a deleted id is never handed out again.
//! `LayoutId(0)` is reserved for the synthetic model row and is never allocated
//! by [`DrawingDatabase::create_layout`].
//!
//! # Tab order
//!
//! `LayoutId` order is tab order. These writes never renumber an existing layout:
//! a create appends (monotonic id) and rename/delete leave the other ids alone.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::*;

use super::layer_patch::LayerRow;
use super::{BlockRow, DrawingDatabase, EntityDisplayState};
use crate::change::ChangeSet;
use crate::entity::DbEntity;
use crate::tables::{Layout, PaperViewport, PlotSettingsRecord};

/// Characters AutoCAD forbids in a symbol-table name (layer, layout, style, …).
///
/// The same set `cad_app::app_layout::validate_layout_name` refuses, so a name
/// accepted by one entry point is accepted by the other.
pub const ILLEGAL_LAYOUT_NAME_CHARS: &[char] =
    &['<', '>', '/', '\\', '"', ':', ';', '?', '*', '|', '=', '`'];

/// A layout name longer than this is refused (AutoCAD's symbol-table limit).
pub const MAX_LAYOUT_NAME_CHARS: usize = 255;

/// One layout table row as a writable unit.
///
/// [`Layout`] stays the row every consumer reads; the plot configuration lives
/// beside it exactly as it does in the database (see
/// [`DrawingDatabase::plot_settings`]), so a restore never invents a page setup.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutRow {
    pub layout: Layout,
    /// The stored plot settings for this layout, `None` when the drawing carried
    /// no PLOTSETTINGS record (resolved later by
    /// [`DrawingDatabase::plot_settings_for`], never fabricated here).
    pub plot: Option<PlotSettingsRecord>,
}

/// A before/after record of one layout row.
///
/// `None` means the row did not exist (`before`) / no longer exists (`after`).
/// The type is public so the record/replay layer does not have to invent a
/// second, divergent row shape. A plot row is part of the row, so the cascade
/// (delete the layout ⇒ delete its plot settings) round-trips exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutPatch {
    pub id: LayoutId,
    pub before: Option<LayoutRow>,
    pub after: Option<LayoutRow>,
}

impl DrawingDatabase {
    /// The next id [`Self::create_layout`] will allocate.
    ///
    /// Allocated from a per-database monotonic counter (initialised by the
    /// builder to `max(existing) + 1`), never from "largest existing id + 1" at
    /// call time: that would reuse the id of a just-deleted layout.
    pub fn next_layout_id(&self) -> LayoutId {
        LayoutId(self.next_layout_id)
    }

    /// Create a new paper layout.
    ///
    /// Returns the allocated [`LayoutId`] plus the [`ChangeSet`] that publishes
    /// the revision bump. The new layout has no viewports and no plot settings
    /// row; its page resolves to the documented default through
    /// [`Self::plot_settings_for`].
    ///
    /// Refusals (nothing written, revision unchanged): an invalid name (see the
    /// module docs) and a name that duplicates an existing layout
    /// case-insensitively.
    pub fn create_layout(
        &mut self,
        name: &str,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<(LayoutId, ChangeSet)> {
        // Phase 1: validate. Nothing below mutates until this passes.
        let name = validated_layout_name(name)?;
        self.reject_duplicate_layout_name(&name, None)?;

        // Phase 2: mutate. `max(1)` keeps `LayoutId(0)` (the reserved synthetic
        // model row) out of the allocator even if the counter were ever 0.
        let id = LayoutId(self.next_layout_id.max(1));
        self.next_layout_id = id.0.saturating_add(1);
        self.layouts.insert(
            id,
            Layout {
                id,
                name,
                viewports: Vec::new(),
            },
        );
        let change_set = self.publish_layout_change(transaction, reason);
        Ok((id, change_set))
    }

    /// Rename an existing layout.
    ///
    /// Same validation as [`Self::create_layout`] for the new name, with
    /// uniqueness excluding the renamed row itself (so a pure case change
    /// `sheet` -> `SHEET` is allowed).
    ///
    /// The plot settings row is keyed by id, so a rename leaves it untouched.
    /// Exactly one revision bump per accepted call.
    ///
    /// Refusals (nothing written, revision unchanged): an unknown layout id and
    /// any invalid/duplicate name.
    pub fn rename_layout(
        &mut self,
        id: LayoutId,
        name: &str,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        // Phase 1: validate.
        self.layouts.get(&id).ok_or_else(|| unknown_layout(id))?;
        let name = validated_layout_name(name)?;
        self.reject_duplicate_layout_name(&name, Some(id))?;

        // Phase 2: mutate.
        if let Some(layout) = self.layouts.get_mut(&id) {
            layout.name = name;
        }
        Ok(self.publish_layout_change(transaction, reason))
    }

    /// Delete a layout and its plot settings row atomically.
    ///
    /// Refused when it would remove a reserved row or leave a dangling entity
    /// reference:
    ///
    /// * `LayoutId(0)` (the reserved synthetic model row);
    /// * an unknown id;
    /// * a layout that any entity still lives in (`space == SpaceId::Paper(id)`).
    ///   The refusal names the referencing count; entities are never
    ///   re-pointed to another layout.
    pub fn delete_layout(
        &mut self,
        id: LayoutId,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        // Phase 1: validate.
        if id == LayoutId(0) {
            return Err(reserved_layout_id());
        }
        let layout = self.layouts.get(&id).ok_or_else(|| unknown_layout(id))?;
        let referencing = paper_entity_count(self, id);
        if referencing > 0 {
            return Err(in_use(id, &layout.name, referencing));
        }

        // Phase 2: mutate. The plot row is keyed by layout id and must not
        // outlive its layout: both removals happen before the one revision bump.
        self.layouts.remove(&id);
        self.plot_settings.remove(&id);
        Ok(self.publish_layout_change(transaction, reason))
    }

    /// Replace a layout's paper viewports.
    ///
    /// The rows are stored as given; the only structural rules enforced here are
    /// that the layout exists and that every viewport is usable geometry: each
    /// clip point and every `model_to_paper` matrix entry must be finite. A
    /// non-finite value would poison every derived paper representation, so it
    /// is refused instead of stored.
    pub fn set_layout_viewports(
        &mut self,
        id: LayoutId,
        viewports: Vec<PaperViewport>,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        // Phase 1: validate.
        if !self.layouts.contains_key(&id) {
            return Err(unknown_layout(id));
        }
        for viewport in &viewports {
            for point in &viewport.clip {
                if !finite_point(point) {
                    return Err(CadError::InvalidInput(format!(
                        "layout.non_finite_viewport: a clip point of layout {} is not finite",
                        id.0
                    )));
                }
            }
            if !finite_matrix(&viewport.model_to_paper.matrix) {
                return Err(CadError::InvalidInput(format!(
                    "layout.non_finite_viewport: the model_to_paper matrix of layout {} is not \
                     finite",
                    id.0
                )));
            }
        }

        // Phase 2: mutate.
        if let Some(layout) = self.layouts.get_mut(&id) {
            layout.viewports = viewports;
        }
        Ok(self.publish_layout_change(transaction, reason))
    }

    /// Apply a layout-only batch as **one** revision and **one** [`ChangeSet`].
    ///
    /// This is the undo/redo apply entry: `changes` maps a layout id to its new
    /// [`LayoutRow`] (`None` = delete). There are no entity changes in this
    /// entry, so the current entity table is the post-batch view.
    ///
    /// Validation, in order (nothing mutates until all of it passes):
    ///
    /// 1. no id appears twice; a row's `layout.id` equals the change key;
    /// 2. names are trimmed, valid and unique case-insensitively across the
    ///    **final** layout set (a name freed by a delete in the same batch is
    ///    available);
    /// 3. `LayoutId(0)` may be updated but never created by a patch;
    /// 4. a row that carries `plot` must keep `plot.layout == id`;
    /// 5. every delete names an existing layout, is not `LayoutId(0)`, and has no
    ///    entity in `SpaceId::Paper(id)`.
    ///
    /// Then upserts land first (upserting the row and storing/removing its plot
    /// settings together), deletes last (layout **and** plot row), and the
    /// revision is raised once.
    pub fn apply_layout_changes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        changes: &[(LayoutId, Option<LayoutRow>)],
    ) -> CadResult<ChangeSet> {
        // Phase 1: validate. There are no entity changes in this entry, so the
        // **current** entity table is the post-batch view.
        validate_layout_batch(self, changes, |id| paper_entity_count(self, id))?;

        // Phase 2: mutate. Upserts first, deletes last; one revision bump.
        for (id, change) in changes {
            if let Some(row) = change {
                self.install_layout_row(*id, row);
            }
        }
        for (id, change) in changes {
            if change.is_none() {
                self.remove_layout_row(*id);
            }
        }
        Ok(self.publish_layout_change(transaction, reason))
    }

    /// [`Self::apply_drawing_changes_with_blocks_display_and_layers`] plus a
    /// layout batch, committed as **one** revision and **one** [`ChangeSet`].
    ///
    /// `layout_changes` maps a layout id to its new [`LayoutRow`] (`None` =
    /// delete); the other batches keep the layer variant's contract. The layout
    /// batch uses exactly the rules of [`Self::apply_layout_changes`], but its
    /// delete safety counts the **post-batch** entity view, so a record that
    /// moves the last paper entity out of a layout and deletes that layout is
    /// accepted in one revision. Layout upserts land before the entity batch, so
    /// an entity may be restored into a layout the same batch restores; layout
    /// deletes land last, once the entity deletes have left.
    ///
    /// The combined path also applies the layer/block/display batches (see
    /// `drawing::layer_patch` and `drawing::block_edit`).
    // The batch shape is the contract: every table is one argument so a caller
    // cannot silently omit a half of an atomic write.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_drawing_changes_with_blocks_display_layers_and_layouts(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        block_changes: &[(BlockId, Option<BlockRow>)],
        entity_changes: Vec<(EntityId, Option<DbEntity>)>,
        display_changes: &[(EntityId, Option<EntityDisplayState>)],
        layer_changes: &[(LayerId, Option<LayerRow>)],
        layout_changes: &[(LayoutId, Option<LayoutRow>)],
    ) -> CadResult<ChangeSet> {
        let display: Vec<(EntityId, EntityDisplayState)> = display_changes
            .iter()
            .filter_map(|(id, state)| state.clone().map(|state| (*id, state)))
            .collect();
        self.apply_drawing_changes_combined(
            reason,
            transaction,
            block_changes,
            entity_changes,
            &display,
            layer_changes,
            layout_changes,
        )
    }

    /// Store one validated layout row and its plot settings, keeping the id
    /// allocator ahead.
    ///
    /// Validation already proved the name is legal and unique in the final set;
    /// it is re-trimmed here because the stored spelling is always trimmed
    /// ([`Self::create_layout`] does the same). A row with no stored page setup
    /// removes any previous plot row, so the table never disagrees with the
    /// record.
    pub(super) fn install_layout_row(&mut self, id: LayoutId, row: &LayoutRow) {
        let name = row.layout.name.trim().to_string();
        self.layouts.insert(
            id,
            Layout {
                id,
                name,
                viewports: row.layout.viewports.clone(),
            },
        );
        match &row.plot {
            Some(plot) => {
                self.plot_settings.insert(id, plot.clone());
            }
            None => {
                self.plot_settings.remove(&id);
            }
        }
        self.next_layout_id = self.next_layout_id.max(id.0.saturating_add(1));
    }

    /// Remove a layout row and its plot settings together.
    ///
    /// The plot row is keyed by layout id and must not outlive its layout (see
    /// [`Self::delete_layout`]); the id allocator is not rewound, so a deleted id
    /// is never handed out again.
    pub(super) fn remove_layout_row(&mut self, id: LayoutId) {
        self.layouts.remove(&id);
        self.plot_settings.remove(&id);
    }

    /// Raise the revision exactly once and build the revision-tagged change set.
    ///
    /// The only mutation step shared by the layout writes: after this the table
    /// change is authoritative and every revision-bound projection is stale by
    /// construction.
    fn publish_layout_change(&mut self, transaction: TransactionId, reason: &str) -> ChangeSet {
        let before = self.revision;
        let after = Revision(before.0 + 1);
        self.revision = after;
        // A layout edit is not a geometry change, but it raises the revision, so
        // the revision-keyed bounds cache is already stale; clearing it makes the
        // invalidation explicit (the same defensive rule the layer writes use).
        self.invalidate_bounds_cache();
        ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            // No entity changed: see the module docs.
            changes: Vec::new(),
        }
    }

    /// Refuse `name` when another layout already uses it case-insensitively.
    ///
    /// `except` is the row being renamed, so a rename may keep its own name
    /// (including a pure case change). The comparison folds with
    /// [`str::to_lowercase`], the same rule the layer table uses; it is a
    /// superset of the app's ASCII-only lookup, so this entry can never accept
    /// two names the resolver would treat as distinct only by accident.
    fn reject_duplicate_layout_name(&self, name: &str, except: Option<LayoutId>) -> CadResult<()> {
        let folded = name.to_lowercase();
        if let Some(existing) = self
            .layouts
            .values()
            .find(|layout| Some(layout.id) != except && layout.name.to_lowercase() == folded)
        {
            return Err(CadError::InvalidInput(format!(
                "layout.duplicate_name: this drawing already has a layout named '{}' (names are \
                 unique case-insensitively)",
                existing.name
            )));
        }
        Ok(())
    }
}

/// Validate the whole layout batch; returns the staged (created/updated) ids.
///
/// `count_in_space` counts the entities that will sit in `SpaceId::Paper(id)`
/// **after** the batch. A caller with entity changes passes the post-batch view
/// (so a layout may be deleted in the same batch that moves/removes its last
/// entity); [`DrawingDatabase::apply_layout_changes`] passes the current table,
/// because it stages no entity rows.
///
/// Rules, all checked before any mutation: no id twice; a row's `layout.id`
/// equals its change key; names are trimmed, legal and unique
/// case-insensitively across the **final** set (a name freed by a delete is
/// available); `LayoutId(0)` may be updated but never created; a row carrying
/// `plot` keeps `plot.layout == id`; every delete names an existing non-reserved
/// layout with no surviving paper entity.
pub(super) fn validate_layout_batch<F>(
    db: &DrawingDatabase,
    changes: &[(LayoutId, Option<LayoutRow>)],
    count_in_space: F,
) -> CadResult<BTreeSet<LayoutId>>
where
    F: Fn(LayoutId) -> usize,
{
    let mut seen = BTreeSet::new();
    let mut staged: BTreeMap<LayoutId, &LayoutRow> = BTreeMap::new();
    let mut deleted: Vec<LayoutId> = Vec::new();
    for (id, change) in changes {
        if !seen.insert(*id) {
            return Err(CadError::Invariant(format!(
                "transaction modifies layout {id:?} twice"
            )));
        }
        match change {
            Some(row) => {
                if row.layout.id != *id {
                    return Err(CadError::Invariant(format!(
                        "change key {id:?} does not match layout id {:?}",
                        row.layout.id
                    )));
                }
                if *id == LayoutId(0) && !db.layouts.contains_key(id) {
                    return Err(reserved_layout_id());
                }
                if let Some(plot) = &row.plot {
                    if plot.layout != *id {
                        return Err(CadError::Invariant(format!(
                            "layout.plot_mismatch: plot settings for layout {:?} are keyed to \
                             layout {:?}",
                            plot.layout, id
                        )));
                    }
                }
                validated_layout_name(&row.layout.name)?;
                staged.insert(*id, row);
            }
            None => deleted.push(*id),
        }
    }

    // Uniqueness is checked against the **final** set: a name freed by a delete
    // in the same batch is available, and a name taken by an upsert is not.
    let mut final_names: BTreeMap<String, (LayoutId, String)> = BTreeMap::new();
    for layout in db
        .layouts
        .values()
        .filter(|layout| !seen.contains(&layout.id))
    {
        final_names.insert(layout.name.to_lowercase(), (layout.id, layout.name.clone()));
    }
    for (id, row) in &staged {
        let name = row.layout.name.trim().to_string();
        if let Some((existing, existing_name)) =
            final_names.insert(name.to_lowercase(), (*id, name.clone()))
        {
            return Err(CadError::InvalidInput(format!(
                "layout.duplicate_name: this drawing already has a layout named \
                 '{existing_name}' ({}) and the batch also stages layout '{name}' ({id:?}); \
                 names are unique case-insensitively",
                existing.0
            )));
        }
    }

    for id in &deleted {
        if *id == LayoutId(0) {
            return Err(reserved_layout_id());
        }
        let layout = db.layouts.get(id).ok_or_else(|| {
            CadError::Invariant(format!(
                "layout.unknown_id: cannot delete layout {id:?}; it does not exist"
            ))
        })?;
        let referencing = count_in_space(*id);
        if referencing > 0 {
            return Err(in_use(*id, &layout.name, referencing));
        }
    }
    Ok(staged.into_keys().collect())
}

/// Trim and validate a layout name, returning the value that is stored.
///
/// Mirrors `cad_app::app_layout::validate_layout_name` exactly so both entry
/// points accept and store the same spelling.
pub(super) fn validated_layout_name(name: &str) -> CadResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CadError::InvalidInput(
            "layout.invalid_name: a layout name must not be empty".into(),
        ));
    }
    if trimmed.contains('\r') || trimmed.contains('\n') {
        return Err(CadError::InvalidInput(
            "layout.invalid_name: a layout name must be a single line".into(),
        ));
    }
    if trimmed.chars().count() > MAX_LAYOUT_NAME_CHARS {
        return Err(CadError::InvalidInput(format!(
            "layout.invalid_name: a layout name is limited to {MAX_LAYOUT_NAME_CHARS} characters"
        )));
    }
    if let Some(illegal) = trimmed
        .chars()
        .find(|ch| ILLEGAL_LAYOUT_NAME_CHARS.contains(ch))
    {
        return Err(CadError::InvalidInput(format!(
            "layout.invalid_name: layout name {:?} contains an AutoCAD-illegal character '{illegal}'",
            trimmed
        )));
    }
    Ok(trimmed.to_string())
}

/// The stable refusal for an id that names no layout in this drawing.
fn unknown_layout(id: LayoutId) -> CadError {
    CadError::InvalidInput(format!(
        "layout.unknown_id: this drawing has no layout with id {}",
        id.0
    ))
}

/// The reserved synthetic model row (`LayoutId(0)`) cannot be created or deleted
/// through these writes.
fn reserved_layout_id() -> CadError {
    CadError::InvalidInput(
        "layout.model_undeletable: layout id 0 is the reserved synthetic model row and cannot be \
         created or deleted"
            .into(),
    )
}

/// The stable refusal for deleting a layout (directly or through a batch) that
/// entities still live in.
fn in_use(id: LayoutId, name: &str, referencing: usize) -> CadError {
    CadError::InvalidInput(format!(
        "layout.in_use: layout '{name}' ({}) is still referenced by {referencing} entit{}; move or \
         delete them first",
        id.0,
        if referencing == 1 { "y" } else { "ies" }
    ))
}

/// How many entities have `space == SpaceId::Paper(id)`.
fn paper_entity_count(db: &DrawingDatabase, id: LayoutId) -> usize {
    db.entities
        .values()
        .filter(|entity| entity.space == SpaceId::Paper(id))
        .count()
}

fn finite_point(point: &Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

fn finite_matrix(matrix: &[[f64; 4]; 4]) -> bool {
    matrix.iter().flatten().all(|value| value.is_finite())
}
