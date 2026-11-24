//! `LAYCUR` and `LAYM C U R`: move the selection onto the session's current
//! layer, and make the first selected entity's layer current.
//!
//! Split out of `layer_commands` so both files stay inside the code-size gate;
//! the two write/refusal rules are documented on the methods below.

use super::*;

impl Application {
    /// `LAYM C U R`: make the first selected entity's layer the current layer.
    pub(super) fn laymcur(
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

        // The first pick names the layer; every other ref is still preflighted so
        // an unusable selection is refused before the session changes. A layer is
        // named through any drawing space — model, a paper layout, a block
        // definition or a nested pick's effective layer (`layer_named_by`), the
        // same rule `LAYLCK`/`LAYFRZ` use, because this command only reads it.
        let first_layer = layer_named_by(drawing.as_ref(), command, &references[0], "laymcur")?;
        for reference in &references[1..] {
            layer_named_by(drawing.as_ref(), command, reference, "laymcur")?;
        }
        let layer_name = drawing
            .layer(first_layer)
            .map(|row| row.name.clone())
            .ok_or_else(|| {
                invalid(
                    "laymcur",
                    "unknown_layer",
                    format!(
                        "the first selected entity's layer ({}) does not exist in this drawing",
                        first_layer.0
                    ),
                )
            })?;

        // The one current-layer rule: locked / frozen / off are refused with the
        // existing `layer.current_*` codes before the session is touched.
        self.ensure_layer_can_be_current(document_id, first_layer)?;
        session.active_layer = first_layer;

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
    pub(super) fn laycur(
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
            ensure_drawing_space_addressable(drawing.as_ref(), command, reference, "laycur")?;
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
