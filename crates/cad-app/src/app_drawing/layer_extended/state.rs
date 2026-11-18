//! `LAYERSTATE`, `LAYERPMODE` and `LTSCALE`: the bounded named layer-state store,
//! the `LAYERP` snapshot gate and the drawing-global linetype-scale header write.
//!
//! Split out of `layer_extended` so both files stay inside the code-size gate; the
//! per-command semantics, writes and undo behaviour are documented on each
//! method below.

use std::collections::BTreeMap;

use cad_db::{ChangeScope, LayerAttributes};

use super::*;
use crate::command_payload::LayerStateCommand;

impl Application {
    /// `LAYERSTATE <option>`: bounded named layer states.
    pub(crate) fn layer_state(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        match &command.payload {
            CommandPayload::None => Err(invalid(
                "layerstate",
                "needs_option",
                "LAYERSTATE needs SAVE <name>, RESTORE <name>, LIST or DELETE <name>",
            )),
            CommandPayload::LayerState(LayerStateCommand::Save(name)) => {
                let attributes: BTreeMap<LayerId, LayerAttributes> = drawing
                    .layers()
                    .map(|row| (row.id, drawing.layer_attributes(row.id)))
                    .collect();
                let layer_count = attributes.len();
                let replaced = session.layer_states.save(
                    name,
                    attributes,
                    session.active_layer,
                    session.layer_overrides.clone(),
                );
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "layerstate.saved".into(),
                        message: format!(
                            "LAYERSTATE: {} named state '{name}' with {layer_count} layer(s) \
                             (session only; {} states retained, cap {})",
                            if replaced { "replaced" } else { "saved" },
                            session.layer_states.len(),
                            session.layer_states.cap()
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandPayload::LayerState(LayerStateCommand::List) => {
                let names: Vec<String> = session
                    .layer_states
                    .snapshots()
                    .map(|snapshot| snapshot.name.clone())
                    .collect();
                let (code, message) = if names.is_empty() {
                    (
                        "layerstate.list_empty",
                        "LAYERSTATE: no named layer states are saved in this session".to_string(),
                    )
                } else {
                    (
                        "layerstate.list",
                        format!(
                            "LAYERSTATE: {} saved state(s): {}",
                            names.len(),
                            names.join(", ")
                        ),
                    )
                };
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: code.into(),
                        message,
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandPayload::LayerState(LayerStateCommand::Delete(name)) => {
                if !session.layer_states.remove(name) {
                    return Err(invalid(
                        "layerstate",
                        "unknown_name",
                        format!("no named layer state is called '{name}'"),
                    ));
                }
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "layerstate.deleted".into(),
                        message: format!("LAYERSTATE: deleted the named state '{name}'"),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandPayload::LayerState(LayerStateCommand::Restore(name)) => {
                let snapshot = session.layer_states.resolve(name).cloned().ok_or_else(|| {
                    invalid(
                        "layerstate",
                        "unknown_name",
                        format!("no named layer state is called '{name}'"),
                    )
                })?;
                // Validate **every** stored row before touching the candidate.
                for layer in snapshot.attributes.keys() {
                    if drawing.layer(*layer).is_none() {
                        return Err(invalid(
                            "layerstate",
                            "dangling_layer",
                            format!(
                                "the saved state '{name}' names layer {} which no longer exists; \
                                 delete the state or recreate the layer first",
                                layer.0
                            ),
                        ));
                    }
                }
                if drawing.layer(snapshot.active_layer).is_none() {
                    return Err(invalid(
                        "layerstate",
                        "dangling_active_layer",
                        format!(
                            "the saved state '{name}' names active layer {} which no longer exists",
                            snapshot.active_layer.0
                        ),
                    ));
                }
                if let Some(attributes) = snapshot.attributes.get(&snapshot.active_layer) {
                    if !attributes.is_visible() {
                        return Err(invalid(
                            "layerstate",
                            "active_layer_hidden",
                            format!(
                                "the saved state '{name}' stores its active layer {} as off or \
                                 frozen; restoring it would receive invisible new geometry",
                                snapshot.active_layer.0
                            ),
                        ));
                    }
                }

                let transaction = next_transaction_id();
                let before_revision = drawing.revision();
                let mut candidate = drawing.as_ref().clone();
                let mut patches = Vec::new();
                let mut scope = ChangeScope::default();
                for (layer, attributes) in &snapshot.attributes {
                    if drawing.layer_attributes(*layer) != *attributes {
                        let before = layer_row(drawing.as_ref(), *layer);
                        let write = candidate.set_layer_attributes(
                            *layer,
                            attributes.clone(),
                            transaction,
                            "restore layer state",
                        )?;
                        scope.merge(&write.scope);
                        patches.push(cad_history::layer_patch(
                            *layer,
                            before,
                            layer_row(&candidate, *layer),
                        ));
                    }
                }
                let changed = patches.len();
                let changes = (changed > 0).then(|| {
                    Arc::new(ChangeSet {
                        database: candidate.id(),
                        before: before_revision,
                        after: candidate.revision(),
                        transaction,
                        reason: "restore layer state".to_string(),
                        changes: Vec::new(),
                        scope,
                    })
                });
                if changed > 0 {
                    self.publish_layer_record(
                        document_id,
                        candidate,
                        transaction,
                        "layerstate restore",
                        patches,
                    )?;
                }
                // Session half: the saved active layer and override set replace the
                // current ones. This is session state, not a layer-table patch, so
                // undo restores the attributes but not the active-layer/override
                // choice; no `LAYERP` snapshot is pushed.
                session.active_layer = snapshot.active_layer;
                session.layer_overrides = snapshot.overrides.clone();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "layerstate.restored".into(),
                        message: format!(
                            "LAYERSTATE: restored '{name}' ({changed} layer row(s) changed in one \
                             transaction / one undo step, active layer {}; {} override(s) \
                             replaced; the active-layer/override choice is session state and is \
                             not undone)",
                            snapshot.active_layer.0,
                            snapshot.overrides.len()
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            _ => Err(invalid(
                "layerstate",
                "invalid_payload",
                "LAYERSTATE takes one typed SAVE/RESTORE/LIST/DELETE argument",
            )),
        }
    }

    /// `LAYERPMODE [ON|OFF]`: gate `LAYERP` snapshot recording.
    ///
    /// Bare `LAYERPMODE` (`CommandPayload::None`) reports the current setting and
    /// changes nothing. `ON` is the default and preserves the existing behaviour;
    /// `OFF` suppresses the snapshot push in
    /// [`SessionState::push_layer_override_snapshot`] without changing which
    /// override is applied.
    pub(super) fn layer_pmode(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::None => {
                let state = if session.layer_states.snapshot_enabled() {
                    "ON"
                } else {
                    "OFF"
                };
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "layerpmode.report".into(),
                        message: format!("LAYERPMODE is {state}"),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            CommandPayload::LayerPmode(enabled) => {
                session.layer_states.set_snapshot_enabled(*enabled);
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "layerpmode.applied".into(),
                        message: format!(
                            "LAYERPMODE is now {}: layer overrides {} record LAYERP snapshots",
                            if *enabled { "ON" } else { "OFF" },
                            if *enabled { "will" } else { "will not" }
                        ),
                    }],
                    measurement: None,
                    point_report: None,
                })
            }
            _ => Err(invalid(
                "layerpmode",
                "invalid_payload",
                "LAYERPMODE takes ON or OFF (or no argument to report the current setting)",
            )),
        }
    }

    /// `LTSCALE <value>`: write the drawing-global linetype scale.
    pub(super) fn linetype_scale(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::LinetypeScale(scale) = command.payload else {
            return Err(invalid(
                "ltscale",
                "needs_value",
                "LTSCALE needs one positive finite decimal value",
            ));
        };
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let mut candidate = drawing.as_ref().clone();
        let changeset =
            candidate.set_linetype_scale(scale, next_transaction_id(), "set linetype scale")?;
        let changed = changeset.before != changeset.after;
        // Publish the validated candidate. No history record: `cad-history` has no
        // header-variable patch kind.
        self.workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
            .drawing = Arc::new(candidate);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(changeset)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "drawing.linetype_scale".into(),
                message: format!(
                    "linetype scale: {scale} ($LTSCALE){}",
                    if changed {
                        ""
                    } else {
                        " (already stored, no change)"
                    }
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
