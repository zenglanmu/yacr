//! The 2026-11-07 layer batch: `LAYMCH`, `LAYMRG`, `LAYDEL`, `LAYERSTATE`,
//! `LAYERPMODE`, `LTSCALE` and `LAYWALK`.
//!
//! Split out of `layer_commands` / `application` so every file stays inside the
//! 1000-line code gate; the command semantics are documented per method.
//!
//! ## Writes and undo
//!
//! * `LAYMCH`, `LAYMRG` and `LAYDEL` stage their entity rows through the
//!   established `begin_drawing_transaction` → `update_entity` /
//!   `delete_entity` → `commit` → `publish_candidate` pattern: **one transaction
//!   and one undo step** for the entity rows.
//! * Layer-**table** writes (`delete_layer`, `set_layer_attributes`) raise the
//!   revision once per changed row, and the command records the matching
//!   before/after [`cad_db::LayerPatch`] rows in the **same** `DrawingUndoRecord`
//!   as the entity patches. A merge/delete is therefore one atomic undo step:
//!   undo restores the deleted layer rows before (and together with) the entity
//!   rows, and redo re-applies both. The published `ChangeSet` still spans the
//!   entity commit plus the layer-table bumps. `LAYMRG`/`LAYDEL` prune the
//!   per-viewport overrides naming the removed layers as [`cad_db::LayoutPatch`]
//!   rows of that same record; `LAYDEL` also deletes entities in **every** space
//!   (model, paper, block member — one `entities` table).
//! * `LAYERSTATE RESTORE` applies stored attributes with `set_layer_attributes`
//!   (one revision bump per changed layer) and records all changed rows as one
//!   layer-patch undo step. The restored active layer and override set are
//!   session state and are not part of history.
//! * `LTSCALE` writes the drawing header (`$LTSCALE`): one revision bump, **no**
//!   undo entry (there is no header-variable patch kind).
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
use super::layer_record::layer_row;
use super::layout_record::layout_row;
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
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
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
    /// source layer is then deleted from the layer table in the same transaction
    /// and the same undo record (entity patches + one layer patch per source row).
    /// Every paper viewport override keyed by a source layer is pruned there too
    /// (one layout patch per pruned layout), so no override outlives its layer.
    /// Sources
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
        let layout_patches = prune_views(drawing.as_ref(), &mut candidate, &sources, transaction)?;
        // Each source row is recorded as a layer patch in the same undo step, so
        // undo restores the row before the entity rows are put back onto it.
        let mut layer_patches = Vec::with_capacity(sources.len());
        for source in &sources {
            let before = layer_row(drawing.as_ref(), *source);
            candidate.delete_layer(*source, transaction, "merge layer")?;
            layer_patches.push(cad_history::layer_patch(*source, before, None));
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
                layer_patches,
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches,
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
                     current layer '{active_name}'; the source rows were deleted (one \
                     transaction / one undo step)",
                    sources.len()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYDEL` / `-LAYDEL`: delete the selection's layers and everything on them.
    ///
    /// The source layers are the distinct layers of the selection, which names a
    /// layer through a **top-level** object in model space *or* a paper-space
    /// layout (see [`laydel_layer_of`]). A layer is referenced by **one** table
    /// here — `cad-db` stores model-space entities, every paper-space layout's
    /// entities and every block definition's members in the same `entities` map —
    /// so deleting by layer covers all three spaces and `delete_layer` afterwards
    /// sees no reference. The other stored reference to a layer row is a paper
    /// viewport's per-layer override map (`VPLAYER`/`LAYVPI`); those keys are
    /// pruned in the same transaction so no override outlives its layer.
    ///
    /// Entities, the pruned viewport rows and the layer rows are all staged on
    /// **one** candidate with **one** transaction and one undo record (entity
    /// patches with the display state a delete prunes, one layout patch per pruned
    /// layout, one layer patch per deleted row), so one undo restores all of it.
    /// A delete the database refuses (a locked layer's entities: `layer.locked`)
    /// publishes nothing, so a failure never becomes a silent success. The active
    /// layer (`laydel.current_layer`), the default layer `0`
    /// (`laydel.default_layer`) and an empty selection are refused.
    fn laydel(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "laydel", "LAYDEL")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();
        // A top-level drawing-space object names a layer (model or paper space).
        let mut sources: Vec<LayerId> = Vec::new();
        for reference in session.selection.refs() {
            let layer = laydel_layer_of(drawing.as_ref(), command, reference)?;
            if !sources.contains(&layer) {
                sources.push(layer);
            }
        }
        if sources.is_empty() {
            return Err(invalid(
                "laydel",
                "selection",
                "LAYDEL needs at least one selected object in model space or a paper-space layout",
            ));
        }

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
        let entity_rows: Vec<DbEntity> = drawing
            .entities()
            .filter(|entity| sources.contains(&entity.layer))
            .cloned()
            .collect();
        let display_states: Vec<_> = entity_rows
            .iter()
            .map(|entity| drawing.entity_display_state(entity.id))
            .collect();
        let before_revision = drawing.revision();
        let transaction = next_transaction_id();
        // All halves share one candidate, one transaction and one undo record;
        // nothing is published until the layer rows are deleted below.
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged =
                candidate.begin_drawing_transaction("delete layer entities", transaction)?;
            for entity in &entity_rows {
                staged.delete_entity(entity.id)?;
            }
            staged.commit()?
        };
        let mut patches = Vec::with_capacity(entity_rows.len());
        for (entity, state) in entity_rows.into_iter().zip(display_states) {
            patches.push(drawing_patch_deleted(entity.id, entity, state));
        }
        let entity_count = patches.len();
        let layout_patches = prune_views(drawing.as_ref(), &mut candidate, &sources, transaction)?;
        let mut layer_patches = Vec::with_capacity(sources.len());
        for source in &sources {
            let before = layer_row(drawing.as_ref(), *source);
            candidate.delete_layer(*source, transaction, "delete layer")?;
            layer_patches.push(cad_history::layer_patch(*source, before, None));
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
            changes: change_set.changes.clone(),
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction,
                label: "laydel".to_string(),
                patches,
                block_patches: Vec::new(),
                layer_patches,
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches,
                merge_key: None,
            },
        )?;
        // The deleted entities must not stay in the session selection.
        self.reconcile_selection(session);
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(combined)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "laydel.applied".into(),
                message: format!(
                    "LAYDEL: deleted {entity_count} entities (model space, paper space and block \
                     definitions) and {} layer(s) in one transaction / one undo step",
                    sources.len()
                ),
            }],
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
                let mut patches = Vec::new();
                for (layer, attributes) in &snapshot.attributes {
                    if drawing.layer_attributes(*layer) != *attributes {
                        let before = layer_row(drawing.as_ref(), *layer);
                        candidate.set_layer_attributes(
                            *layer,
                            attributes.clone(),
                            transaction,
                            "restore layer state",
                        )?;
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

/// `LAYDEL`'s selection rule: one top-level **drawing-space** object (model space
/// or any paper-space layout — the delete covers every space) names one layer;
/// block-instance members, sub-elements and block-definition members are refused
/// by name (`laydel.block_space`).
fn laydel_layer_of(
    drawing: &DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
) -> CadResult<LayerId> {
    let id = reference.entity.0;
    let blocked = format!(
        "entity {id} is a block-instance member, a sub-element or a block-definition member; \
         LAYDEL needs a top-level object in model space or a paper-space layout"
    );
    if reference.document != command.document {
        return Err(invalid(
            "laydel",
            "not_addressable",
            format!("the selected ref for entity {id} belongs to another document"),
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(invalid("laydel", "block_space", &blocked));
    }
    match drawing.entity(reference.entity) {
        None => Err(invalid(
            "laydel",
            "not_addressable",
            format!("entity {id} does not exist in this drawing"),
        )),
        Some(entity) if matches!(entity.space, SpaceId::Model | SpaceId::Paper(_)) => {
            Ok(entity.layer)
        }
        Some(_) => Err(invalid("laydel", "block_space", &blocked)),
    }
}

/// Prune every deleted source layer from each layout viewport's `layer_overrides`
/// map, staging the rewritten layouts on `candidate` as layout patches of its one
/// undo step so **no override outlives its layer**.
fn prune_views(
    drawing: &DrawingDatabase,
    candidate: &mut DrawingDatabase,
    sources: &[LayerId],
    transaction: TransactionId,
) -> CadResult<Vec<cad_db::LayoutPatch>> {
    let mut patches = Vec::new();
    for layout in drawing.layouts() {
        let mut viewports = layout.viewports.clone();
        let mut pruned = false;
        for viewport in &mut viewports {
            for source in sources {
                pruned |= viewport.layer_overrides.remove(source).is_some();
            }
        }
        if pruned {
            let before = layout_row(drawing, layout.id);
            candidate.set_layout_viewports(layout.id, viewports, transaction, "prune overrides")?;
            let after = layout_row(candidate, layout.id);
            patches.push(cad_history::layout_patch(layout.id, before, after));
        }
    }
    Ok(patches)
}
