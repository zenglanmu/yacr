//! Layer **selection** commands (AutoCAD 2026).
//!
//! These are the layer operations that act on the current selection rather than
//! on a layer name typed into the palette: `LAYCUR` in this commit, with the
//! rest of the run (`LAYM C U R`, `LAYFRZ`, `LAYTHW`, `LAYLCK`, `LAYULK`,
//! `SETBYLAYER`) added beside it.
//!
//! ## Declared divergence: pre-selection instead of "Select objects"
//!
//! AutoCAD prompts *Select objects:* after these commands start. This build's
//! fixed-step capture machine cannot ask that prompt, so every command here uses
//! the **current session selection** as its whole input
//! ([`crate::CommandPayload::None`]), exactly like `MATCHPROP`, `LAYISO` and the
//! Object mode of `REVCLOUD`. A bare invocation with nothing selected is an
//! explicit refusal (`<command>.selection`), never an empty success.
//!
//! ## What "selected object" means here
//!
//! Only directly addressable **model-space entity rows** are accepted. The
//! preflight refuses, by name ([`ensure_model_space_addressable`]):
//!
//! * a ref from another document (`<command>.not_addressable`);
//! * a block-instance member (`InstancePath`) or a sub-element
//!   (`<command>.not_addressable`, `Unsupported`: those have no entity identity
//!   of their own, exactly like the property-write path);
//! * an entity that does not resolve (`<command>.not_addressable`);
//! * an entity in paper space or a block definition
//!   (`<command>.not_addressable`, `Unsupported`: there is no model-space layer
//!   row to act on).
//!
//! ## Writes
//!
//! Entity-layer writes (`LAYCUR`) go through one
//! `begin_drawing_transaction` + `update_entity` per changed entity + one
//! `commit`, recorded as **one** undo step with `DrawingUndoRecord` entity
//! patches, exactly like `app_drawing::revision_cloud` / `properties`.
//!
//! Layer-**table** writes use
//! [`cad_db::DrawingDatabase::set_layer_attributes`]. They raise the revision
//! but are **not undoable** (cad-history has no layer patch kind); the success
//! diagnostic says so, and no undo entry is recorded.
//!
//! Every refusal happens before anything is published: the original drawing
//! keeps its revision, its layer rows and its history. A locked destination
//! layer is refused by the database's own entity-write rule (`layer.locked`);
//! there is no second lock check here.
//!
//! ## `LAYCUR`
//!
//! "Changes the layer property of selected objects to the current layer." Sets
//! every selected entity's `layer` to `SessionState::active_layer`. The active
//! layer must exist in the drawing (`laycur.active_layer_missing`). Entities
//! already on it are not staged; when the whole batch is already there the
//! command opens no transaction and reports `laycur.unchanged`. Success is
//! `laycur.applied` with the layer name and the entity count. Work-only.
//!
//! ## `LAYM C U R`
//!
//! "Sets the current layer to that of a selected object." Uses the **first**
//! selected entity's stored layer (`laymcur.first_pick` names the extra picks
//! when more than one object is selected) and resolves the row
//! (`laymcur.unknown_layer` when it is absent). It then reuses the one
//! current-layer rule, [`Application::ensure_layer_can_be_current`], so a
//! locked/frozen/off layer is refused with the existing
//! `layer.current_locked` / `layer.current_frozen` / `layer.current_off` codes
//! before the session is touched. Session-only: no database write, no revision
//! bump, no history. Success is `laymcur.applied` with the layer name. Work-only.

use super::*;

/// A named, non-capability refusal (`<command>.<code>: <detail>`).
fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

/// A named capability-gap refusal (`<command>.<code>: <detail>`).
fn unsupported(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{prefix}.{code}: {detail}"))
}

/// These commands read the selection, so a payload is refused rather than
/// silently ignored.
fn ensure_no_payload(command: &Command, prefix: &str, label: &str) -> CadResult<()> {
    if matches!(command.payload, CommandPayload::None) {
        Ok(())
    } else {
        Err(invalid(
            prefix,
            "payload",
            format!("{label} takes no payload; the current selection is the input"),
        ))
    }
}

/// Refuse a selected ref that is not a directly addressable model-space entity.
fn ensure_model_space_addressable(
    drawing: &DrawingDatabase,
    command: &Command,
    reference: &SelectionRef,
    prefix: &str,
) -> CadResult<()> {
    if reference.document != command.document {
        return Err(invalid(
            prefix,
            "not_addressable",
            format!(
                "the selected ref for entity {} belongs to another document",
                reference.entity.0
            ),
        ));
    }
    if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
        return Err(unsupported(
            prefix,
            "not_addressable",
            format!(
                "entity {} is referenced through a block instance or a sub-element and has no \
                 directly addressable identity",
                reference.entity.0
            ),
        ));
    }
    let Some(entity) = drawing.entity(reference.entity) else {
        return Err(invalid(
            prefix,
            "not_addressable",
            format!(
                "entity {} does not exist in this drawing",
                reference.entity.0
            ),
        ));
    };
    if entity.space != SpaceId::Model {
        return Err(unsupported(
            prefix,
            "not_addressable",
            format!(
                "entity {} is not in model space; only model-space entities can be edited by this \
                 command",
                reference.entity.0
            ),
        ));
    }
    Ok(())
}

/// Take the selection snapshot and refuse an empty one.
fn selection_snapshot(
    session: &SessionState,
    prefix: &str,
    label: &str,
) -> CadResult<Vec<SelectionRef>> {
    let references = session.selection.refs().to_vec();
    if references.is_empty() {
        return Err(invalid(
            prefix,
            "selection",
            format!("{label} needs at least one selected model-space object"),
        ));
    }
    Ok(references)
}

impl Application {
    /// Dispatch a layer **selection** command.
    pub(crate) fn layer_selection_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.id {
            CommandId::LayCur => self.laycur(session, command),
            CommandId::LayMCur => self.laymcur(session, command),
            _ => Err(CadError::InvalidInput(
                "not a layer selection command".to_string(),
            )),
        }
    }

    /// `LAYM C U R`: make the first selected entity's layer the current layer.
    fn laymcur(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "laymcur", "LAYMCUR")?;
        let references = selection_snapshot(session, "laymcur", "LAYMCUR")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        for reference in &references {
            ensure_model_space_addressable(drawing.as_ref(), command, reference, "laymcur")?;
        }

        let first = &references[0];
        let layer = drawing
            .entity(first.entity)
            .expect("preflight verified the entity resolves")
            .layer;
        let layer_name = drawing
            .layer(layer)
            .map(|row| row.name.clone())
            .ok_or_else(|| {
                invalid(
                    "laymcur",
                    "unknown_layer",
                    format!(
                        "the first selected entity's layer ({}) does not exist in this drawing",
                        layer.0
                    ),
                )
            })?;

        // The one current-layer rule: locked / frozen / off are refused with the
        // existing `layer.current_*` codes before the session is touched.
        self.ensure_layer_can_be_current(document_id, layer)?;
        session.active_layer = layer;

        let mut diagnostics = vec![Diagnostic {
            object: None,
            code: "laymcur.applied".into(),
            message: format!("LAYMCUR: the current layer is now '{layer_name}'"),
        }];
        if references.len() > 1 {
            diagnostics.push(Diagnostic {
                object: None,
                code: "laymcur.first_pick".into(),
                message: format!(
                    "LAYMCUR: {} objects were selected; the current layer follows the first \
                     selected object",
                    references.len()
                ),
            });
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// `LAYCUR`: move every selected entity to the session's current layer.
    fn laycur(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        ensure_no_payload(command, "laycur", "LAYCUR")?;
        let references = selection_snapshot(session, "laycur", "LAYCUR")?;
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .clone();

        let active_layer = session.active_layer;
        let layer_name = drawing
            .layer(active_layer)
            .map(|row| row.name.clone())
            .ok_or_else(|| {
                invalid(
                    "laycur",
                    "active_layer_missing",
                    format!(
                        "the session's current layer ({}) does not exist in this drawing",
                        active_layer.0
                    ),
                )
            })?;

        for reference in &references {
            ensure_model_space_addressable(drawing.as_ref(), command, reference, "laycur")?;
        }

        let mut changed_before: Vec<DbEntity> = Vec::new();
        for reference in &references {
            let entity = drawing
                .entity(reference.entity)
                .expect("preflight verified the entity resolves");
            if entity.layer != active_layer {
                changed_before.push(entity.clone());
            }
        }

        if changed_before.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "laycur.unchanged".into(),
                    message: format!(
                        "LAYCUR: every selected entity is already on layer '{layer_name}'"
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let transaction = next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction("laycur", transaction)?;
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
                .ok_or_else(|| CadError::Invariant("LAYCUR: edited entity vanished".into()))?;
            patches.push(drawing_patch(id, Some(before), Some(after)));
            objects.push(ObjectId(id.0));
        }
        let count = patches.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "laycur".to_string(),
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
                code: "laycur.applied".into(),
                message: format!(
                    "LAYCUR: moved {count} entities to the current layer '{layer_name}'"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
