//! The recording half of layer-table writes.
//!
//! A layer-table edit that a handler wants on the undo stack records one
//! [`cad_db::LayerPatch`] per row (create = `before: None`, delete =
//! `after: None`) and publishes the candidate that produced it. Replay goes
//! through the database's validated combined path
//! ([`cad_db::DrawingDatabase::apply_drawing_changes_with_blocks_display_and_layers`]),
//! so the layer rows land in the same revision and `ChangeSet` shape as every
//! other edit and a delete is applied after its entity deletes.
//!
//! Kept beside `layer_commands` rather than inside it so that file stays inside
//! the project's code-size limit.

use super::*;
use cad_db::LayerAttributes;

/// Read one layer table row (the [`cad_db::Layer`] plus its attributes) as the
/// before/after unit a [`cad_db::LayerPatch`] carries.
///
/// Returns `None` when the id names no layer row, so a create records
/// `before: None` and a delete records `after: None` without inventing a row.
pub(super) fn layer_row(database: &DrawingDatabase, id: LayerId) -> Option<cad_db::LayerRow> {
    database.layer(id).map(|layer| cad_db::LayerRow {
        layer: layer.clone(),
        attributes: database.layer_attributes(id),
    })
}

impl Application {
    /// Apply one boolean layer-attribute change to every layer in `layers`.
    ///
    /// Each changed row is one [`DrawingDatabase::set_layer_attributes`] call, so
    /// a multi-layer command raises the revision once per changed layer (a
    /// documented limitation), but the whole batch records **one** undo step:
    /// one [`cad_db::LayerPatch`] per changed layer is published through
    /// [`Self::publish_layer_record`] with a single transaction. `mutate` returns
    /// whether the row actually changed; unchanged rows are not staged, so the
    /// revision does not move for them and they record no patch. Nothing is
    /// published when no row changes. A successful write reports one combined
    /// revision span and the changed-row count.
    pub(super) fn write_layer_flags(
        &mut self,
        document_id: DocumentId,
        layers: &[LayerId],
        reason: &str,
        mutate: impl Fn(&mut LayerAttributes) -> bool,
    ) -> CadResult<(Option<Arc<ChangeSet>>, usize)> {
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let mut candidate = drawing.as_ref().clone();
        let before = candidate.revision();
        let transaction = next_transaction_id();
        let mut changed = 0;
        let mut patches = Vec::new();
        for id in layers {
            let before_row = layer_row(drawing.as_ref(), *id);
            let mut attributes = candidate.layer_attributes(*id);
            if mutate(&mut attributes) {
                candidate.set_layer_attributes(*id, attributes, transaction, reason)?;
                changed += 1;
                if let Some(before_row) = before_row {
                    patches.push(cad_history::layer_patch(
                        *id,
                        Some(before_row),
                        layer_row(&candidate, *id),
                    ));
                }
            }
        }
        if changed == 0 {
            return Ok((None, 0));
        }
        let changeset = ChangeSet {
            database: candidate.id(),
            before,
            after: candidate.revision(),
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
        };
        self.publish_layer_record(
            document_id,
            candidate,
            transaction,
            &format!("layer-{reason}"),
            patches,
        )?;
        Ok((Some(Arc::new(changeset)), changed))
    }

    /// Publish an already-validated layer-table candidate **without history**.
    ///
    /// Only [`Self::layer_command_without_history`] (the legacy `PURGE` delete)
    /// uses this now: every ordinary layer-table command records a patch through
    /// [`Self::publish_layer_record`]. Do not add new callers.
    pub(super) fn publish_layer_table(
        &mut self,
        document_id: DocumentId,
        candidate: DrawingDatabase,
    ) -> CadResult<()> {
        self.workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
            .drawing = Arc::new(candidate);
        Ok(())
    }

    /// Record one layer-table undo step and publish the candidate that produced
    /// it.
    ///
    /// The record carries `layer_patches` and no entity/block patches, so
    /// undo/redo replays it through the same validated combined write path
    /// (layer upserts first, layer deletes last, one revision and one
    /// `ChangeSet`). History is validated **before** the candidate replaces the
    /// authoritative drawing, so a rejected record leaves both the drawing and
    /// the history untouched.
    pub(super) fn publish_layer_record(
        &mut self,
        document_id: DocumentId,
        candidate: DrawingDatabase,
        transaction: TransactionId,
        label: &str,
        layer_patches: Vec<cad_db::LayerPatch>,
    ) -> CadResult<()> {
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction,
                label: label.to_string(),
                patches: Vec::new(),
                block_patches: Vec::new(),
                layer_patches,
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )
    }
}
