//! Post-import write for the named **linetype** table.
//!
//! The importer fills `DB.LTYPE` through
//! [`crate::DrawingDatabaseBuilder::insert_linetype`]. Until this module existed
//! that was the only writer, so `RENAME LINETYPE` had no API and was refused by
//! name. This module is the sanctioned post-import path for the one linetype
//! write this build models — a **rename**. Creating and deleting linetypes stay
//! unsupported (`PURGE LINETYPE` is still a named refusal), so the batch below
//! deliberately has no create/delete arm: a `None` entry is refused instead of
//! silently deleting a row.
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
//! # The default linetype cannot be renamed
//!
//! [`DEFAULT_LINETYPE_NAME`] (`Continuous`) is the implicit value of every layer
//! row that has no stored attributes (`LayerAttributes::default`) and the
//! fallback the representation uses for an unresolvable name. Those implicit
//! references are not rows this batch could retarget, so renaming the row would
//! break them; the write is refused by name
//! (`linetype.continuous_unrenameable`) exactly like the layer `0` / DIMSTYLE
//! `Standard` protections.
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
use crate::entity::EntityLineType;
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
                // Validation refuses deletes; keep the mutation total instead of
                // panicking so a future caller cannot crash the host.
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

/// Validate the whole linetype batch.
///
/// Returns the staged (renamed) ids. Every refusal happens before the first
/// mutation: the table, the layer rows, the entity attributes and the revision
/// are untouched.
pub(super) fn validate_linetype_batch(
    db: &DrawingDatabase,
    rows: &[(LinetypeId, Option<LineType>)],
) -> CadResult<BTreeSet<LinetypeId>> {
    let mut seen = BTreeSet::new();
    let mut staged = BTreeSet::new();
    for (id, change) in rows {
        if !seen.insert(*id) {
            return Err(CadError::Invariant(format!(
                "transaction modifies linetype {id:?} twice"
            )));
        }
        let Some(row) = change else {
            return Err(CadError::InvalidInput(format!(
                "linetype.delete_unsupported: this build has no linetype delete API; row {id:?} \
                 was not removed"
            )));
        };
        if row.id != *id {
            return Err(CadError::Invariant(format!(
                "change key {id:?} does not match the linetype row id {:?}",
                row.id
            )));
        }
        let stored = db.linetypes.get(id).ok_or_else(|| {
            CadError::Invariant(format!(
                "linetype.unknown_id: cannot rename linetype {id:?}; it does not exist and this \
                 build has no create API"
            ))
        })?;
        if stored
            .name
            .trim()
            .eq_ignore_ascii_case(DEFAULT_LINETYPE_NAME)
        {
            return Err(CadError::InvalidInput(format!(
                "linetype.continuous_unrenameable: linetype '{DEFAULT_LINETYPE_NAME}' is the \
                 drawing's default linetype and the implicit value of every layer row with no \
                 stored attributes, so it cannot be renamed"
            )));
        }
        validated_style_name(&row.name, "linetype")?;
        staged.insert(*id);
    }

    // Uniqueness is checked against the **final** name set: a name freed by the
    // same batch would be available, but rename-only batches never free one.
    let mut final_names: BTreeMap<String, String> = BTreeMap::new();
    for row in db.linetypes.values().filter(|row| !seen.contains(&row.id)) {
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
