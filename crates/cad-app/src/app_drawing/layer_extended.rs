//! The 2026-11-07 layer batch: `LAYMCH`, `LAYMRG`, `LAYDEL`, `LAYERSTATE`,
//! `LAYERPMODE`, `LTSCALE` and `LAYWALK`.
//!
//! Split out of `layer_commands` / `application` so every file stays inside the
//! 1000-line code gate; the command semantics are documented per method.
//!
//! ## Writes and undo
//!
//! * `LAYMCH` / `LAYMCH` and the entity half of `LAYMGR` / `LAYMCH` and `LAYDEL`
//!   use the established `begin_drawing_transaction` → `update_entity` /
//!   `delete_entity` → `commit` → `publish_candidate` pattern: **one transaction
//!   and one undo step** for the entity rows.
//! * Layer-**table** writes (`delete_layer`, `set_layer_attributes`) raise the
//!   revision once per changed row but are **not undoable** (`cad-history` has no
//!   layer patch kind). A merge/delete therefore publishes one `ChangeSet` whose
//!   `before`/`after` span the entity commit plus the layer-table bumps, while the
//!   undo record carries only the entity patches. Undoing a `LAYDEL`/`LAYMRG`
//!   restores the entity rows but **not** the deleted layer rows: an undo can leave
//!   an entity pointing at a layer that no longer exists. That is a documented
//!   boundary of this build, not a silent success.
//! * `LAYERSTATE RESTORE` applies stored attributes with `set_layer_attributes`
//!   (one revision bump per changed layer, no undo).
//! * `LTSCALE` writes the drawing header (`$LTSCALE`): one revision bump, **no**
//!   undo entry.
//! * `LAYERPMODE` and `LAYWALK` are session state only: no database write, no
//!   revision bump, no history.

use std::collections::{BTreeMap, BTreeSet};

use cad_db::{DbEntity, LayerAttributes, DEFAULT_LAYER_NAME};
use cad_domain::SelectionRef;

use super::layer_commands::{
    ensure_model_space_addressable, ensure_no_payload, invalid, selected_distinct_layers,
    selection_snapshot,
};
use super::layer_edit::resolve_layer;
use super::*;
use crate::command_payload::LayerStateCommand;

impl Application {
    /// Dispatch the 2026-11-07 layer batch.
    pub(crate) fn layer_extended_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::LayMCh => self.laymch(session, command),
            CommandId::LayMrg => self.laymrg(session, command),
            CommandId::LayDel => self.laydel(session, command),
            CommandId::LayerState => self.layer_state(session, command),
            CommandId::LayerPmode => self.layer_pmode(session, command),
            CommandId::SetLinetypeScale => self.linetype_scale(command),
            CommandId::LayWalk => self.laywalk(session, command),
            _ => Err(CadError::InvalidInput(
                "not a layer batch command".to_string(),
            )),
        }
    }

    /// `LAYMCH` / `-LAYMCH`: move the selection to the first pick's layer.
    ///
    /// The destination is the **first** selected entity's stored layer; every
    /// other selected model-space entity is moved there in one transaction / one
    /// undo step. An empty selection is refused (`laymch.selection`), a selection
    /// of one is refused (`laymch.single_selection`, there is nothing to move), and
    /// a payload, a foreign/instance/sub-element ref or a non-model-space entity is
    /// refused before any write. A destination the database refuses (locked layer)
    /// keeps the whole command refused by the database's own entity-write rule.
    fn laymch(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "laymch", "LAYMCH")?;
        let references = selection_snapshot(session, "laymch", "LAYMCH")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        if references.len() < 2 {
            return Err(invalid(
                "laymch",
                "single_selection",
                "LAYMCH needs at least two selected objects: the first pick names the destination \
                 layer and the rest are moved onto it",
            ));
        }
        for reference in &references {
            ensure_model_space_addressable(drawing.as_ref(), command, reference, "laymch")?;
        }

        let destination = drawing
            .entity(references[0].entity)
            .expect("preflight verified the entity resolves")
            .layer;
        let layer_name = drawing
            .layer(destination)
            .map(|row| row.name.clone())
            .ok_or_else(|| {
                invalid(
                    "laymch",
                    "destination_layer_missing",
                    format!(
                        "the first selected entity's layer ({}) does not exist in this drawing",
                        destination.0
                    ),
                )
            })?;

        let mut changed_before: Vec<DbEntity> = Vec::new();
        for reference in &references[1..] {
            let entity = drawing
                .entity(reference.entity)
                .expect("preflight verified the entity resolves");
            if entity.layer != destination {
                changed_before.push(entity.clone());
            }
        }
        if changed_before.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "laymch.unchanged".into(),
                    message: format!(
                        "LAYMCH: every selected entity is already on layer '{layer_name}'"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("laymch", transaction)?;
            for before in &changed_before {
                let mut after = before.clone();
                after.layer = destination;
                staged.update_entity(after)?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(changed_before.len());
        let mut objects = Vec::with_capacity(changed_before.len());
        for before in changed_before {
            let id = before.id;
            let after = candidate
                .entity(id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("LAYMCH: edited entity vanished".into()))?;
            patches.push(drawing_patch(id, Some(before), Some(after)));
            objects.push(ObjectId(id.0));
        }
        let count = patches.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "laymch".to_string(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "laymch.applied".into(),
                message: format!(
                    "LAYMCH: moved {count} entities onto the layer of the first selected object \
                     ('{layer_name}')"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYMRG` / `-LAYMRG`: merge the selection's layers into the **active** layer.
    ///
    /// Declared deviation: AutoCAD asks for a source and a target layer; this build
    /// has no in-command prompts, so the sources are the distinct layers of the
    /// current selection and the target is the session's active layer. Every
    /// entity on a source layer (model space, paper space or block member) is
    /// rewritten to the active layer in one transaction / one undo step; each
    /// source layer is then deleted from the layer table (not undoable). Sources
    /// equal to the active layer (`laymrg.source_is_active`) and the default layer
    /// `0` (`laymrg.default_layer`) are refused because the layer table cannot
    /// represent either merge. Empty selection is `laymrg.selection`.
    fn laymrg(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "laymrg", "LAYMRG")?;
        let references = selection_snapshot(session, "laymrg", "LAYMRG")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let sources = selected_distinct_layers(drawing.as_ref(), command, &references, "laymrg")?;

        let active_layer = session.active_layer;
        let active_name = drawing
            .layer(active_layer)
            .map(|row| row.name.clone())
            .ok_or_else(|| {
                invalid(
                    "laymrg",
                    "active_layer_missing",
                    format!(
                        "the session's current layer ({}) does not exist in this drawing",
                        active_layer.0
                    ),
                )
            })?;
        if sources.contains(&active_layer) {
            return Err(invalid(
                "laymrg",
                "source_is_active",
                format!(
                    "layer '{active_name}' is both the target (current) layer and a source; switch \
                     the current layer first"
                ),
            ));
        }
        for source in &sources {
            let row = drawing.layer(*source).ok_or_else(|| {
                invalid(
                    "laymrg",
                    "unknown_layer",
                    format!("layer {} does not exist in this drawing", source.0),
                )
            })?;
            if row.name == DEFAULT_LAYER_NAME {
                return Err(invalid(
                    "laymrg",
                    "default_layer",
                    format!(
                        "layer '{}' is the drawing's default layer and cannot be a merge source",
                        DEFAULT_LAYER_NAME
                    ),
                ));
            }
        }

        let changed_before: Vec<DbEntity> = drawing
            .entities()
            .filter(|entity| sources.contains(&entity.layer))
            .cloned()
            .collect();

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("laymrg", transaction)?;
            for before in &changed_before {
                let mut after = before.clone();
                after.layer = active_layer;
                staged.update_entity(after)?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(changed_before.len());
        let mut objects = Vec::with_capacity(changed_before.len());
        for before in changed_before {
            let id = before.id;
            let after = candidate
                .entity(id)
                .cloned()
                .ok_or_else(|| CadError::Invariant("LAYMRG: edited entity vanished".into()))?;
            patches.push(drawing_patch(id, Some(before), Some(after)));
            objects.push(ObjectId(id.0));
        }
        let entity_count = patches.len();
        let before_revision = drawing.revision();
        // Layer-table deletes raise the revision per row and are not undoable; the
        // published change set spans both halves.
        for source in &sources {
            candidate.delete_layer(*source, transaction, "merge layer")?;
        }
        let after_revision = candidate.revision();
        for source in &sources {
            session.layer_overrides.remove(*source);
        }
        let combined = ChangeSet {
            database: candidate.id(),
            before: before_revision,
            after: after_revision,
            transaction,
            reason: "laymrg".to_string(),
            changes: change_set.changes.clone(),
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction,
                label: "laymrg".to_string(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(combined)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "laymrg.applied".into(),
                message: format!(
                    "LAYMRG: merged {entity_count} entities from {} source layer(s) into the \
                     current layer '{active_name}'; the source rows were deleted (layer-table \
                     edits are not undoable)",
                    sources.len()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYDEL` / `-LAYDEL`: delete the selection's layers and their model entities.
    ///
    /// The source layers are the distinct layers of the current selection. Every
    /// **model-space** entity on those layers is deleted through
    /// [`Application::delete_entity_refs`] (one transaction / one undo step); each
    /// source layer is then deleted from the layer table (not undoable) and its
    /// session override is dropped. Entities in paper space or block definitions
    /// are **not** deleted by this command: the preflight refuses a source layer
    /// that still has such members (`laydel.layer_in_use`) so the subsequent
    /// database delete cannot fail halfway. The active layer and the default layer
    /// `0` are refused, as is an empty selection.
    fn laydel(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "laydel", "LAYDEL")?;
        let references = selection_snapshot(session, "laydel", "LAYDEL")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        let sources = selected_distinct_layers(drawing.as_ref(), command, &references, "laydel")?;

        let active_layer = session.active_layer;
        if sources.contains(&active_layer) {
            let name = drawing
                .layer(active_layer)
                .map(|row| row.name.as_str())
                .unwrap_or("<unknown>");
            return Err(invalid(
                "laydel",
                "current_layer",
                format!(
                    "layer '{name}' is the current layer and cannot be deleted; switch the \
                     current layer first"
                ),
            ));
        }
        for source in &sources {
            let row = drawing.layer(*source).ok_or_else(|| {
                invalid(
                    "laydel",
                    "unknown_layer",
                    format!("layer {} does not exist in this drawing", source.0),
                )
            })?;
            if row.name == DEFAULT_LAYER_NAME {
                return Err(invalid(
                    "laydel",
                    "default_layer",
                    format!(
                        "layer '{}' is the drawing's default layer and cannot be deleted",
                        DEFAULT_LAYER_NAME
                    ),
                ));
            }
        }
        // References the entity delete below will not remove (paper space / block
        // members) would make `delete_layer` fail after the entities were already
        // gone. Refuse the whole command before any write instead.
        let blocking = drawing
            .entities()
            .filter(|entity| sources.contains(&entity.layer) && entity.space != SpaceId::Model)
            .count();
        if blocking > 0 {
            return Err(invalid(
                "laydel",
                "layer_in_use",
                format!(
                    "{blocking} entity/entities in paper space or block definitions still reference \
                     the selected layer(s); LAYDEL only deletes model-space entities"
                ),
            ));
        }

        let model_refs: Vec<SelectionRef> = drawing
            .entities()
            .filter(|entity| sources.contains(&entity.layer) && entity.space == SpaceId::Model)
            .map(|entity| SelectionRef {
                document: document_id,
                entity: entity.id,
                instance: cad_domain::InstancePath::default(),
                sub_element: None,
            })
            .collect();
        let before_revision = drawing.revision();
        let (entity_changes, mut diagnostics) =
            self.delete_entity_refs(document_id, &model_refs, "LAYDEL")?;

        let drawing_after = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::Invariant("document disappeared after LAYDEL delete".into()))?
            .drawing
            .clone();
        let mut candidate = drawing_after.as_ref().clone();
        let transaction = next_transaction_id();
        for source in &sources {
            candidate.delete_layer(*source, transaction, "delete layer")?;
        }
        for source in &sources {
            session.layer_overrides.remove(*source);
        }
        let after_revision = candidate.revision();
        let combined = ChangeSet {
            database: candidate.id(),
            before: before_revision,
            after: after_revision,
            transaction,
            reason: "laydel".to_string(),
            changes: entity_changes
                .as_ref()
                .map(|changes| changes.changes.clone())
                .unwrap_or_default(),
        };
        self.publish_layer_table(document_id, candidate)?;
        // The deleted entities must not stay in the session selection.
        self.reconcile_selection(session);
        diagnostics.push(Diagnostic {
            object: None,
            code: "laydel.applied".into(),
            message: format!(
                "LAYDEL: deleted {} model-space entities and {} layer(s); layer-table edits are \
                 not undoable and entities in paper space/block definitions are not deleted",
                model_refs.len(),
                sources.len()
            ),
        });
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(combined)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

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
                let mut changed = 0;
                for (layer, attributes) in &snapshot.attributes {
                    if drawing.layer_attributes(*layer) != *attributes {
                        candidate.set_layer_attributes(
                            *layer,
                            attributes.clone(),
                            transaction,
                            "restore layer state",
                        )?;
                        changed += 1;
                    }
                }
                let changes = (changed > 0).then(|| {
                    Arc::new(ChangeSet {
                        database: candidate.id(),
                        before: before_revision,
                        after: candidate.revision(),
                        transaction,
                        reason: "restore layer state".to_string(),
                        changes: Vec::new(),
                    })
                });
                if changed > 0 {
                    self.publish_layer_table(document_id, candidate)?;
                }
                // Session half: the saved active layer and override set replace the
                // current ones. This is a named-state restore, not an undoable
                // layer-table edit, so no `LAYERP` snapshot is pushed.
                session.active_layer = snapshot.active_layer;
                session.layer_overrides = snapshot.overrides.clone();
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes,
                    diagnostics: vec![Diagnostic {
                        object: None,
                        code: "layerstate.restored".into(),
                        message: format!(
                            "LAYERSTATE: restored '{name}' ({changed} layer row(s) changed, \
                             active layer {}; {} override(s) replaced; layer-table edits are not \
                             undoable)",
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
    fn layer_pmode(
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
    fn linetype_scale(&mut self, command: &Command) -> CadResult<CommandOutcome> {
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

    /// `LAYWALK <name>[,<name>...]`: show only the named layers plus the active layer.
    ///
    /// Overrides only: the drawing layer table, its revision and `cad-history` are
    /// untouched. The hides go through the same validated atomic batch as `LAYISO`
    /// (`hide_layers_atomically`), so a refusal changes nothing. `LAYUNISO` /
    /// `RESTORELAYERS` clears the result, exactly as it clears an isolation.
    fn laywalk(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::LayerWalk(names) = &command.payload else {
            return Err(invalid(
                "layers.walk",
                "needs_argument",
                "LAYWALK needs one or more layer names separated by commas",
            ));
        };
        if names.is_empty() {
            return Err(invalid(
                "layers.walk",
                "needs_argument",
                "LAYWALK needs at least one layer name",
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        let mut keep: BTreeSet<LayerId> = BTreeSet::new();
        keep.insert(session.active_layer);
        for name in names {
            let id = resolve_layer(drawing.as_ref(), name.trim()).map_err(|error| {
                invalid(
                    "layers.walk",
                    "unknown_layer",
                    format!("cannot resolve layer '{}': {error}", name.trim()),
                )
            })?;
            keep.insert(id);
        }
        let hide: Vec<LayerId> = drawing
            .layers()
            .filter(|layer| !keep.contains(&layer.id))
            .map(|layer| layer.id)
            .collect();
        if hide.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "layers.walk.noop".into(),
                    message: "LAYWALK: every layer is already in the keep-set; no override written"
                        .into(),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let mut outcome = self.hide_layers_atomically(session, document_id, &hide)?;
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "layers.walk.applied".into(),
            message: format!(
                "LAYWALK: {} named layer(s) plus the current layer {} kept visible, {} hidden \
                 (session overrides only; LAYUNISO/RESTORELAYERS clears it)",
                names.len(),
                session.active_layer.0,
                hide.len()
            ),
        });
        Ok(outcome)
    }
}
