//! Layer-table rows as a writable unit for **combined** batches.
//!
//! A layer is one row in [`DrawingDatabase::layers`] plus its
//! [`LayerAttributes`] beside it, exactly like a block is a [`BlockDefinition`]
//! plus [`BlockMetadata`] (see `drawing::block_edit`). Replaying an undo step
//! that creates, renames, re-flags or deletes a layer therefore needs both
//! halves, and it often has to land in the **same revision** as an entity batch
//! (a layer delete is only legal once its entities are gone). Doing that through
//! two writes would publish two revisions and two undo steps.
//!
//! [`DrawingDatabase::apply_drawing_changes_with_blocks_display_and_layers`] is
//! the single validated entry for such a batch. Its contract, in order:
//!
//! 1. entity identity, the layer-lock chokepoint and the display-state checks
//!    (the existing combined path's phase 1);
//! 2. the layer batch: names trimmed and validated like
//!    [`DrawingDatabase::create_layer`]/[`DrawingDatabase::rename_layer`],
//!    uniqueness case-insensitively across the **final** layer set, no delete of
//!    the default layer (`0`) and no delete that still has a reference in the
//!    post-batch entity view (block members included — they are one table);
//! 3. the block batch, then the entity batch against the staged blocks **and**
//!    the staged layers (so an entity can be restored onto a layer the same
//!    record restores);
//! 4. mutate layer upserts first, then block upserts, the entity batch, the
//!    display states, block deletes, and layer deletes **last**;
//! 5. raise the revision exactly once and publish one entity-scoped
//!    [`ChangeSet`].
//!
//! An upsert always re-derives [`Layer::visible`] from
//! [`LayerAttributes::is_visible`] (`!(off || frozen)`); a copied `visible` flag
//! is never trusted, so the one invariant the rest of the build reads cannot be
//! recorded inconsistently.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::*;

use super::layer_edit::{validated_layer_name, DEFAULT_LAYER_NAME};
use super::{DrawingDatabase, EntityDisplayState};
use crate::change::ChangeSet;
use crate::entity::DbEntity;
use crate::tables::{Layer, LayerAttributes};

/// One layer table row as a writable unit.
///
/// [`Layer`] stays the three-field row every consumer reads; the attribute set
/// lives beside it exactly as it does in the database (see
/// [`DrawingDatabase::layer_attributes`]), so a restore never invents an
/// attribute state.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerRow {
    pub layer: Layer,
    pub attributes: LayerAttributes,
}

/// A before/after record of one layer row.
///
/// `None` means the row did not exist (`before`) / no longer exists (`after`).
/// The type is public so `cad-history` (and the layer commands it will carry)
/// does not have to invent a second, divergent row shape.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPatch {
    pub id: LayerId,
    pub before: Option<LayerRow>,
    pub after: Option<LayerRow>,
}

impl DrawingDatabase {
    /// [`Self::apply_drawing_changes_with_blocks_and_display`] plus a layer-table
    /// batch, committed as **one** revision and **one** [`ChangeSet`].
    ///
    /// `layer_changes` maps a layer id to its new row (`None` = delete); the
    /// other three batches keep that method's contract. A `None` display entry
    /// means "no display state staged for this entity". See the module docs for
    /// the phase order and the validation rules. Every refusal happens before
    /// any mutation, so a refused batch leaves the tables, the entity rows, the
    /// display maps, the allocators and the revision exactly as they were.
    pub fn apply_drawing_changes_with_blocks_display_and_layers(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        block_changes: &[(BlockId, Option<super::BlockRow>)],
        entity_changes: Vec<(EntityId, Option<DbEntity>)>,
        display_changes: &[(EntityId, Option<EntityDisplayState>)],
        layer_changes: &[(LayerId, Option<LayerRow>)],
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
            &[],
        )
    }

    /// Store one validated layer row, keeping `visible` derived and the id
    /// allocator ahead.
    ///
    /// Validation already proved the name is non-empty, legal and unique in the
    /// final set; it is re-trimmed here because the stored spelling is always
    /// trimmed ([`Self::create_layer`] does the same).
    pub(super) fn install_layer_row(&mut self, id: LayerId, row: &LayerRow) {
        let name = row.layer.name.trim().to_string();
        self.layers.insert(
            id,
            Layer {
                id,
                name,
                // Never trust a copied flag: `visible` is defined by the
                // attribute set (`!(off || frozen)`).
                visible: row.attributes.is_visible(),
            },
        );
        self.layer_attributes.insert(id, row.attributes.clone());
        self.next_layer_id = self.next_layer_id.max(id.0.saturating_add(1));
    }
}

/// Validate the whole layer batch; returns the staged (created/updated) ids.
///
/// `post_batch` is the entity view the batch leaves behind, sorted by
/// `draw_order`; delete safety counts references there so deleting a layer and
/// all its entities in the same record is accepted, while a surviving reference
/// is still refused.
pub(super) fn validate_layer_batch(
    db: &DrawingDatabase,
    layer_changes: &[(LayerId, Option<LayerRow>)],
    post_batch: &[&DbEntity],
) -> CadResult<BTreeSet<LayerId>> {
    let mut seen = BTreeSet::new();
    let mut staged = BTreeSet::new();
    for (id, change) in layer_changes {
        if !seen.insert(*id) {
            return Err(CadError::Invariant(format!(
                "transaction modifies layer {id:?} twice"
            )));
        }
        if let Some(row) = change {
            if row.layer.id != *id {
                return Err(CadError::Invariant(format!(
                    "change key {id:?} does not match layer id {:?}",
                    row.layer.id
                )));
            }
            validated_layer_name(&row.layer.name)?;
            staged.insert(*id);
        }
    }

    // Uniqueness is checked against the **final** set: a name freed by a delete
    // in the same batch is available, and a name taken by an upsert is not.
    let mut final_names: BTreeMap<String, (LayerId, String)> = BTreeMap::new();
    for layer in db.layers.values().filter(|layer| !seen.contains(&layer.id)) {
        final_names.insert(layer.name.to_lowercase(), (layer.id, layer.name.clone()));
    }
    for (id, change) in layer_changes {
        let Some(row) = change else { continue };
        let name = row.layer.name.trim().to_string();
        if let Some((existing, existing_name)) =
            final_names.insert(name.to_lowercase(), (*id, name.clone()))
        {
            return Err(CadError::InvalidInput(format!(
                "layer.duplicate_name: this drawing already has a layer named '{existing_name}' \
                 ({}) and the batch also stages layer '{name}' ({id:?}); names are unique \
                 case-insensitively",
                existing.0
            )));
        }
    }

    for (id, change) in layer_changes {
        if change.is_some() {
            continue;
        }
        let stored = db.layers.get(id).ok_or_else(|| {
            CadError::Invariant(format!(
                "layer.unknown_id: cannot delete layer {id:?}; it does not exist"
            ))
        })?;
        if stored.name == DEFAULT_LAYER_NAME {
            return Err(CadError::InvalidInput(format!(
                "layer.default_undeletable: layer '{}' is the drawing's default layer and cannot \
                 be deleted",
                DEFAULT_LAYER_NAME
            )));
        }
        let referencing = post_batch
            .iter()
            .filter(|entity| entity.layer == *id)
            .count();
        if referencing > 0 {
            return Err(CadError::InvalidInput(format!(
                "layer.in_use: layer '{}' ({}) is still referenced by {referencing} entit{}; \
                 move or delete them first",
                stored.name,
                id.0,
                if referencing == 1 { "y" } else { "ies" }
            )));
        }
    }
    Ok(staged)
}
