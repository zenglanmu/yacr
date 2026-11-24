//! `LAYERP` (AutoCAD's previous layer state): pop one bounded `LAYERP` snapshot
//! and restore both halves of it.
//!
//! A snapshot is one [`crate::LayerPreviousState`]:
//!
//! * the session's temporary visibility overrides, restored as session view
//!   state (no database write, no revision bump);
//! * the **pre-change layer-table attributes** of every row the recorded command
//!   rewrote — written through the same validated `set_layer_attributes` path as
//!   every other layer write and recorded as **one** undo step with one
//!   [`cad_db::LayerPatch`] per changed row.
//!
//! The command never pushes a snapshot (see [`crate::layer_history`]), so a
//! held-down `LAYERP` walks monotonically back through the recorded states.
//!
//! ## Refuse before writing
//!
//! A snapshot that names a layer row deleted since it was recorded is refused as
//! a whole (`layers.previous.dangling_layer`) before the candidate database is
//! touched. The snapshot stays on the stack, so the history can still be walked
//! after the layer is restored (or a different step is chosen). A pure-override
//! snapshot therefore keeps the original "session-only, no revision bump, no
//! history entry" contract; only a snapshot with attribute rows writes and
//! records an undo step.

use super::layer_record::layer_row;
use super::*;
use cad_db::ChangeScope;

impl Application {
    /// `LAYERP`: restore the newest recorded layer state, or report the named
    /// empty-history no-op.
    pub(crate) fn layer_previous(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        // `LAYERP` takes no input at all: a caller that supplies one is refused
        // rather than silently ignored (the selection is not its input either).
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "layers.previous.invalid_payload: LAYERP takes no payload".into(),
            ));
        }
        let Some(snapshot) = session.layer_history.latest().cloned() else {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "layers.previous.empty".into(),
                    message: "LAYERP: there is no previous layer state to restore in this \
                              session; nothing changed"
                        .into(),
                }],
                measurement: None,
                point_report: None,
            });
        };

        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        // Validate **every** recorded row before touching the candidate: a layer
        // deleted after the snapshot is a named refusal, never a partial restore.
        for layer in snapshot.attributes.keys() {
            if drawing.layer(*layer).is_none() {
                return Err(CadError::InvalidInput(format!(
                    "layers.previous.dangling_layer: the recorded layer state names layer {} \
                     which no longer exists; undo the deletion or recreate the layer first",
                    layer.0
                )));
            }
        }

        let transaction = next_transaction_id();
        let before_revision = drawing.revision();
        let mut candidate = drawing.as_ref().clone();
        let mut patches = Vec::new();
        let mut scope = ChangeScope::default();
        for (layer, attributes) in &snapshot.attributes {
            if drawing.layer_attributes(*layer) == *attributes {
                continue;
            }
            let before = layer_row(drawing.as_ref(), *layer);
            let write = candidate.set_layer_attributes(
                *layer,
                attributes.clone(),
                transaction,
                "layer previous",
            )?;
            scope.merge(&write.scope);
            patches.push(cad_history::layer_patch(
                *layer,
                before,
                layer_row(&candidate, *layer),
            ));
        }
        let changed = patches.len();
        let changes = (changed > 0).then(|| {
            Arc::new(ChangeSet {
                database: candidate.id(),
                before: before_revision,
                after: candidate.revision(),
                transaction,
                reason: "layer previous".to_string(),
                changes: Vec::new(),
                scope,
            })
        });
        if changed > 0 {
            self.publish_layer_record(
                document_id,
                candidate,
                transaction,
                "layer previous",
                patches,
            )?;
        }

        // The restore is published (or was a pure session-override restore):
        // consume the snapshot only now, so a refused write leaves it intact.
        let overrides_changed = snapshot.overrides != session.layer_overrides;
        session.layer_history.pop();
        session.layer_overrides = snapshot.overrides.clone();
        let restored_overrides = snapshot.overrides.len();
        let (code, message) = if changed == 0 && !overrides_changed {
            (
                "layers.previous.unchanged",
                "LAYERP: the previous layer state is identical to the current state; the step \
                 was consumed and nothing changed"
                    .to_string(),
            )
        } else {
            (
                "layers.previous",
                format!(
                    "LAYERP: restored the previous layer state ({restored_overrides} temporary \
                     override(s), {changed} layer row(s) in one transaction / one undo step)"
                ),
            )
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes,
            diagnostics: vec![Diagnostic {
                object: None,
                code: code.into(),
                message,
            }],
            measurement: None,
            point_report: None,
        })
    }
}
