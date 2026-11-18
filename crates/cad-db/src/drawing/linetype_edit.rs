//! Post-import write for the named **linetype** table.
//!
//! The importer fills `DB.LTYPE` through
//! [`crate::DrawingDatabaseBuilder::insert_linetype`]. Until this module existed
//! that was the only writer, so `RENAME LINETYPE` had no API and was refused by
//! name. This module is the sanctioned post-import path for the three linetype
//! table writes this build models: **rename** (`RENAME LINETYPE`), **create**
//! ([`DrawingDatabase::create_linetype`], the typed `LINETYPE CREATE` form) and
//! **delete** ([`DrawingDatabase::delete_linetype`], `LINETYPE DELETE` and
//! `PURGE LINETYPE`). A `None` batch entry is a validated delete; a `Some` entry
//! at a missing id is a create (or the undo replay that reinstalls a deleted
//! row).
//!
//! # A rename cascades, and the database owns the cascade
//!
//! A linetype **name** is stored in two other places:
//!
//! * `LayerAttributes::linetype` (the layer row's `DXF 6` name), which the
//!   representation resolves for every `ByLayer` entity;
//! * [`EntityLineType::Explicit::name`](crate::EntityLineType), the diagnostic
//!   copy an entity with an explicit linetype keeps beside its resolved pattern.
//!
//! Renaming the row without retargeting both would leave dangling name
//! references that a later DXF/DWG save or export would write, so the cascade is
//! part of the same mutation as the row write — not a second command the caller
//! has to remember. [`DrawingDatabase::install_linetype_batch`] rewrites the
//! references from the **stored** name to the new one, which also makes an undo
//! replay (which installs the `before` row) cascade back automatically.
//!
//! # A create is validated against the renderable pattern subset
//!
//! [`DrawingDatabase::create_linetype`] refuses an empty pattern, a non-finite or
//! zero-length element and a pattern with no visible dash
//! (`linetype.bad_pattern`), because `LinetypePattern::from_elements` would
//! otherwise silently drop an unrenderable entry. A **stored** row is not
//! re-validated (a real drawing can carry importer patterns outside that subset,
//! and undoing a delete must restore it verbatim).
//!
//! # The default linetype cannot be renamed
//!
//! [`DEFAULT_LINETYPE_NAME`] (`Continuous`) is the implicit value of every layer
//! row that has no stored attributes (`LayerAttributes::default`) and the
//! fallback the representation uses for an unresolvable name. Those implicit
//! references are not rows this batch could retarget, so renaming the row would
//! break them; the write is refused by name
//! (`linetype.continuous_unrenameable`) exactly like the layer `0` / DIMSTYLE
//! `Standard` protections; creating a row with that name is refused as
//! `linetype.continuous_reserved`, and deleting it as
//! `linetype.continuous_undeletable`.
//!
//! # One revision, one `ChangeSet`
//!
//! The row write, the layer-attribute cascade and the entity-attribute cascade
//! raise the revision once and publish one [`ChangeSet`]. A cascade that
//! retargeted an entity's explicit name also publishes that entity as an
//! `Update` with `ChangeMask::STYLE`, so its representation is rebuilt; a
//! `ByLayer` reference needs no entity update because the layer row (plus the
//! revision stamp) already invalidates it.
//!
//! This module records no undo step itself: the command layer keeps the
//! before/after [`LinetypePatch`] in one `cad-history` step.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::*;

use super::style_edit::validated_style_name;
use super::DrawingDatabase;
use crate::change::ChangeSet;
use crate::entity::{EntityLineType, LinetypePattern};
use crate::tables::{LineType, DEFAULT_LINETYPE_NAME};

/// A before/after record of one linetype table row.
///
/// `None` means the row did not exist (`before`) / no longer exists (`after`).
/// The type is public so `cad-history` carries the exact row shape instead of
/// inventing a second one.
#[derive(Debug, Clone, PartialEq)]
pub struct LinetypePatch {
    pub id: LinetypeId,
    pub before: Option<LineType>,
    pub after: Option<LineType>,
}

impl DrawingDatabase {
    /// The full combined write path plus the linetype-table batch.
    ///
    /// See `Self::apply_drawing_changes_combined` for the entity/block/layer/
    /// layout/style contract and this module for the linetype half. The linetype
    /// batch is not a style table: linetypes are their own symbol table, and
    /// mixing the two would let a caller forget the cascade rules above.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        block_changes: &[(BlockId, Option<super::BlockRow>)],
        entity_changes: Vec<(EntityId, Option<crate::DbEntity>)>,
        display_changes: &[(EntityId, Option<super::EntityDisplayState>)],
        layer_changes: &[(LayerId, Option<super::LayerRow>)],
        layout_changes: &[(LayoutId, Option<super::LayoutRow>)],
        style_changes: super::StyleTableChanges<'_>,
        linetype_changes: &[(LinetypeId, Option<LineType>)],
    ) -> CadResult<ChangeSet> {
        let display: Vec<(EntityId, super::EntityDisplayState)> = display_changes
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
            style_changes,
            linetype_changes,
            super::EntityGeometryCheck::NewWrite,
        )
    }

    /// The **history-replay** sibling of
    /// [`Self::apply_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes`].
    ///
    /// Semantically identical — same batch shape, same phase order, same
    /// all-or-nothing validation, same single revision and `ChangeSet` — with
    /// exactly one difference: each entity row is **restored verbatim** instead of
    /// being geometry-validated. Undo/redo puts back rows this database itself
    /// held a moment earlier; the importer is permissive and can store geometry
    /// that the write validator rejects (a zero-length line inside a block
    /// definition or a paper layout is the real-machine case), so re-validating on
    /// the way back in would make the database refuse to restore its own stored
    /// state and break the undo step.
    ///
    /// Replay therefore skips `validate_geometry` only. Every reference and
    /// structural rule still runs: the entity/`ObjectId` identity relationship,
    /// layer/block/layout existence (widened by the ids the same batch stages),
    /// the `SpaceId` shape, an `INSERT`'s block target, the display-state rules,
    /// the block member-list invariant, the layer/block/layout/style/linetype
    /// batch rules and the layer-lock rule. Nothing here lets a *command* skip
    /// validation: this entry point is named for replay, the mode type is
    /// crate-private, and a brand-new degenerate insert through
    /// [`Self::apply_drawing_changes`] is still refused.
    ///
    /// See `drawing::block_edit` for the module-level rationale.
    #[allow(clippy::too_many_arguments)]
    pub fn replay_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        block_changes: &[(BlockId, Option<super::BlockRow>)],
        entity_changes: Vec<(EntityId, Option<crate::DbEntity>)>,
        display_changes: &[(EntityId, Option<super::EntityDisplayState>)],
        layer_changes: &[(LayerId, Option<super::LayerRow>)],
        layout_changes: &[(LayoutId, Option<super::LayoutRow>)],
        style_changes: super::StyleTableChanges<'_>,
        linetype_changes: &[(LinetypeId, Option<LineType>)],
    ) -> CadResult<ChangeSet> {
        let display: Vec<(EntityId, super::EntityDisplayState)> = display_changes
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
            style_changes,
            linetype_changes,
            super::EntityGeometryCheck::RestoreStored,
        )
    }

    /// Rename one linetype row and retarget every stored reference to it.
    ///
    /// The single-purpose entry the `RENAME LINETYPE` command uses: validate,
    /// write the row, cascade (`Continuous` is refused), one revision. A
    /// refusal leaves the table, the layer rows, the entity attributes and the
    /// revision untouched (the caller works on a candidate clone, but the batch
    /// itself also validates before the first mutation).
    pub fn apply_linetype_rename(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        after: &LineType,
    ) -> CadResult<ChangeSet> {
        let changes = [(after.id, Some(after.clone()))];
        self.apply_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes(
            reason,
            transaction,
            &[],
            Vec::new(),
            &[],
            &[],
            &[],
            super::StyleTableChanges::default(),
            &changes,
        )
    }

    /// Allocate the next linetype id.
    ///
    /// Monotonic like [`DrawingDatabase::allocate_layer_id`]: initialised by the
    /// builder from `max(stored id) + 1` on the import path and from the
    /// persisted `next_linetype_id` counter on a load, bumped by every installed
    /// row, and never rewound by a delete, so a deleted id is never handed out
    /// again.
    pub fn allocate_linetype_id(&mut self) -> LinetypeId {
        let id = LinetypeId(self.next_linetype_id);
        self.next_linetype_id = self.next_linetype_id.saturating_add(1);
        id
    }

    /// Create one linetype table row from a typed name + group-49 pattern.
    ///
    /// The **single** create path (the typed `LINETYPE CREATE` form). Validation
    /// happens before any mutation, so a refusal leaves the table, the layer rows,
    /// the entity attributes and the revision untouched:
    ///
    /// | refusal | rule |
    /// | --- | --- |
    /// | `linetype.name_empty` / `linetype.name_illegal_char` | the name grammar the rename path uses |
    /// | `linetype.exists` | the name already resolves (ASCII case-insensitive) |
    /// | `linetype.continuous_reserved` | `Continuous` is the drawing default and is never created |
    /// | `linetype.bad_pattern` | empty, non-finite, or an element with no length |
    ///
    /// # Ids
    ///
    /// The id comes from the persisted monotonic allocator
    /// [`DrawingDatabase::allocate_linetype_id`], exactly like every other table row:
    /// a create → delete → create sequence never hands the deleted id out again
    /// (ADR-0037 D5 — `SourceRef` maps and the shard assignment depend on ids
    /// never being reused). Allocation is not a content write by itself; a
    /// refusal below leaves the allocator where it was.
    ///
    /// # Pattern grammar
    ///
    /// `pattern` is the DXF group-49 element list: a positive length is a dash, a
    /// negative one a gap, `0.0` a dot. [`LinetypePattern::from_elements`] only
    /// keeps finite elements, so a `NaN`/infinite entry would silently vanish —
    /// that is why the raw list is validated here first, and why a list with no
    /// visible dash (`>= 0.0`) is refused instead of stored as an invisible line.
    ///
    /// Returns the new id and the change set (one revision).
    pub fn create_linetype(
        &mut self,
        name: &str,
        pattern: &[f64],
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<(LinetypeId, ChangeSet)> {
        let name = validated_style_name(name.trim(), "linetype")?;
        validate_linetype_pattern(pattern)?;
        if name.eq_ignore_ascii_case(DEFAULT_LINETYPE_NAME) {
            return Err(CadError::InvalidInput(format!(
                "linetype.continuous_reserved: linetype '{DEFAULT_LINETYPE_NAME}' is the \
                 drawing's default linetype and cannot be created as a table row"
            )));
        }
        if let Some(existing) = self
            .linetypes
            .values()
            .find(|row| row.name.trim().eq_ignore_ascii_case(&name))
        {
            return Err(CadError::InvalidInput(format!(
                "linetype.exists: this drawing already has a linetype named '{}' ({}); names are \
                 unique ASCII case-insensitively",
                existing.name, existing.id.0
            )));
        }
        let id = self.allocate_linetype_id();
        let row = LineType {
            id,
            name,
            pattern: LinetypePattern::from_elements(pattern.iter().copied()),
            complex: false,
        };
        let changes = [(id, Some(row))];
        let change_set = self
            .apply_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes(
                reason,
                transaction,
                &[],
                Vec::new(),
                &[],
                &[],
                &[],
                super::StyleTableChanges::default(),
                &changes,
            )?;
        Ok((id, change_set))
    }

    /// Install one validated linetype row and cascade its name references.
    ///
    /// Returns the entity ids whose explicit linetype name was retargeted, so the
    /// write path can announce their `STYLE` update. The cascade source is the
    /// **stored** row's name: (re)installing a row therefore renames whatever
    /// references currently carry the previous spelling, which is exactly what
    /// an undo replay needs.
    pub(super) fn install_linetype_batch(
        &mut self,
        rows: &[(LinetypeId, Option<LineType>)],
    ) -> BTreeSet<EntityId> {
        let mut touched = BTreeSet::new();
        for (id, change) in rows {
            let Some(row) = change else {
                // A validated delete: the row had no surviving name reference
                // (validation refused otherwise), so removing it cannot dangle.
                self.linetypes.remove(id);
                continue;
            };
            let previous = self
                .linetypes
                .get(id)
                .map(|stored| stored.name.trim().to_string());
            let mut installed = row.clone();
            installed.id = *id;
            installed.name = installed.name.trim().to_string();
            self.linetypes.insert(*id, installed.clone());
            // A caller-supplied id (an undo reinstall, or a load) must leave the
            // allocator ahead of it, so a later create cannot collide with a row
            // that already exists. The persisted high-water mark only grows.
            self.next_linetype_id = self.next_linetype_id.max(id.0.saturating_add(1));
            if let Some(previous) = previous {
                if previous != installed.name {
                    self.cascade_linetype_name(&previous, &installed.name, &mut touched);
                }
            }
        }
        touched
    }

    /// Retarget every stored name reference from `old` to `new`.
    ///
    /// Both stores compare ASCII case-insensitively (the importer can preserve a
    /// source spelling that differs from the table row's case) and write the
    /// table row's canonical spelling. An entity's **resolved pattern** is not
    /// touched: a rename does not change the dash pattern, only the name.
    fn cascade_linetype_name(&mut self, old: &str, new: &str, touched: &mut BTreeSet<EntityId>) {
        for attributes in self.layer_attributes.values_mut() {
            if attributes.linetype.eq_ignore_ascii_case(old) {
                attributes.linetype = new.to_string();
            }
        }
        for (id, attributes) in self.render_attributes.iter_mut() {
            if let EntityLineType::Explicit { name, .. } = &mut attributes.linetype {
                if name.eq_ignore_ascii_case(old) {
                    *name = new.to_string();
                    touched.insert(*id);
                }
            }
        }
    }
}

/// Validate one typed group-49 pattern for a **new** linetype row.
///
/// The rules are the renderable subset the dash model can draw:
///
/// * at least one element (`linetype.bad_pattern` otherwise — an "empty pattern"
///   is `Continuous`, which is a built-in name, not a creatable row);
/// * every element finite and of non-zero length (a `0.0` is DXF's *dot*, which
///   this model cannot distinguish from a gap, and a non-finite length would be
///   silently dropped by `LinetypePattern::from_elements`);
/// * at least one **visible** dash (positive length), so a row never stores an
///   all-gap pattern that draws nothing.
///
/// A **stored** row is not re-validated here: a real drawing can carry importer
/// patterns outside this subset, and an undo that restores such a row must not be
/// refused (the same reason `EntityGeometryCheck::RestoreStored` exists).
fn validate_linetype_pattern(pattern: &[f64]) -> CadResult<()> {
    if pattern.is_empty() {
        return Err(CadError::InvalidInput(
            "linetype.bad_pattern: a linetype needs at least one dash length (an empty pattern is \
             Continuous, which is a built-in name)"
                .into(),
        ));
    }
    if let Some(bad) = pattern
        .iter()
        .find(|element| !element.is_finite() || element.abs() <= 0.0)
    {
        return Err(CadError::InvalidInput(format!(
            "linetype.bad_pattern: element {bad} is not a finite, non-zero length; positive is a \
             dash, negative a gap"
        )));
    }
    if !pattern.iter().any(|element| *element > 0.0) {
        return Err(CadError::InvalidInput(
            "linetype.bad_pattern: the pattern has no visible dash (every element is a gap); \
             positive is a dash, negative a gap"
                .into(),
        ));
    }
    Ok(())
}

/// Validate the whole linetype batch.
///
/// Returns the ids of the rows that stay in the table (creates/renames); a
/// `None` row is a delete and is validated against the same in-use rules
/// [`DrawingDatabase::ensure_linetype_deletable`] states for a caller. Every
/// refusal happens before the first mutation: the table, the layer rows, the
/// entity attributes and the revision are untouched.
pub(super) fn validate_linetype_batch(
    db: &DrawingDatabase,
    rows: &[(LinetypeId, Option<LineType>)],
) -> CadResult<BTreeSet<LinetypeId>> {
    let mut seen = BTreeSet::new();
    let mut staged = BTreeSet::new();
    let mut deleted = BTreeSet::new();
    for (id, change) in rows {
        if !seen.insert(*id) {
            return Err(CadError::Invariant(format!(
                "transaction modifies linetype {id:?} twice"
            )));
        }
        let Some(row) = change else {
            // Delete: the row must exist, must not be the drawing default and must
            // have no surviving name reference — the exact rules the public
            // `delete_linetype` predicate states for the user.
            let stored = db.linetypes.get(id).ok_or_else(|| {
                CadError::Invariant(format!(
                    "linetype.unknown_id: cannot delete linetype {id:?}; it does not exist"
                ))
            })?;
            if stored
                .name
                .trim()
                .eq_ignore_ascii_case(DEFAULT_LINETYPE_NAME)
            {
                return Err(CadError::InvalidInput(format!(
                    "linetype.continuous_undeletable: linetype '{DEFAULT_LINETYPE_NAME}' is the \
                     drawing's default linetype and the implicit value of every layer row with no \
                     stored attributes, so it cannot be deleted"
                )));
            }
            let layers = db
                .layer_attributes
                .values()
                .filter(|attributes| attributes.linetype.eq_ignore_ascii_case(stored.name.trim()))
                .count();
            let entities = db
                .render_attributes
                .values()
                .filter(|attributes| {
                    matches!(
                        &attributes.linetype,
                        EntityLineType::Explicit { name, .. }
                            if name.eq_ignore_ascii_case(stored.name.trim())
                    )
                })
                .count();
            if layers + entities > 0 {
                return Err(CadError::InvalidInput(format!(
                    "linetype.in_use: linetype '{}' ({}) is still referenced by {layers} layer \
                     row(s) and {entities} entity attribute(s); retarget them first",
                    stored.name, id.0
                )));
            }
            deleted.insert(*id);
            continue;
        };
        if row.id != *id {
            return Err(CadError::Invariant(format!(
                "change key {id:?} does not match the linetype row id {:?}",
                row.id
            )));
        }
        // A stored row is a **rename** and keeps its protections. A missing id is a
        // **create** (`LINETYPE CREATE`) or the history replay of a delete (undo
        // reinstalls the row it removed); neither may ever be the drawing default.
        match db.linetypes.get(id) {
            Some(stored) => {
                if stored
                    .name
                    .trim()
                    .eq_ignore_ascii_case(DEFAULT_LINETYPE_NAME)
                {
                    return Err(CadError::InvalidInput(format!(
                        "linetype.continuous_unrenameable: linetype '{DEFAULT_LINETYPE_NAME}' is \
                         the drawing's default linetype and the implicit value of every layer row \
                         with no stored attributes, so it cannot be renamed"
                    )));
                }
            }
            None => {
                if row.name.trim().eq_ignore_ascii_case(DEFAULT_LINETYPE_NAME) {
                    return Err(CadError::InvalidInput(format!(
                        "linetype.continuous_reserved: linetype '{DEFAULT_LINETYPE_NAME}' is the \
                         drawing's default linetype and cannot be created as a table row"
                    )));
                }
            }
        }
        validated_style_name(&row.name, "linetype")?;
        staged.insert(*id);
    }

    // Uniqueness is checked against the **final** name set, so a name freed by a
    // delete in the same batch is available to the same batch.
    let mut final_names: BTreeMap<String, String> = BTreeMap::new();
    for row in db
        .linetypes
        .values()
        .filter(|row| !staged.contains(&row.id) && !deleted.contains(&row.id))
    {
        final_names.insert(row.name.trim().to_lowercase(), row.name.clone());
    }
    for (id, change) in rows {
        let Some(row) = change else { continue };
        let name = row.name.trim().to_string();
        if let Some(previous) = final_names.insert(name.to_lowercase(), name.clone()) {
            return Err(CadError::InvalidInput(format!(
                "linetype.duplicate_name: this drawing already has a linetype named '{previous}' \
                 and the batch also stages '{name}' ({id:?}); names are unique \
                 case-insensitively"
            )));
        }
    }
    Ok(staged)
}
