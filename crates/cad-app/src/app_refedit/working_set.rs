//! `REFSET ADD|REMOVE`: the in-place block edit's working set.
//!
//! Session state only: no database write, no revision bump, no history entry and
//! no `ChangeSet`. The selection is the whole input; `ADD` may name plain host
//! objects (they move into the block on save), `REMOVE` drops an entry (its
//! original member is deleted from the block on save while the model-space copy
//! stays). A bare `REFSET` is `ADD`, matching AutoCAD's prompt default.

use super::*;

impl Application {
    /// `REFSET ADD|REMOVE`: edit the active session's working set.
    pub(super) fn refset(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let option = match &command.payload {
            CommandPayload::None => RefSetOption::Add,
            CommandPayload::RefSet(option) => *option,
            _ => {
                return Err(invalid(
                    "refset",
                    "invalid_option",
                    "REFSET takes ADD or REMOVE; no other payload is accepted",
                ))
            }
        };
        if !session.refedit.is_active() {
            return Err(invalid(
                "refedit",
                "no_session",
                "REFSET needs an active in-place block edit; start one with REFEDIT",
            ));
        }
        let edited_block = session
            .refedit
            .active()
            .map(|active| active.block)
            .ok_or_else(|| invalid("refedit", "no_session", "no active in-place block edit"))?;
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(invalid(
                "refedit",
                "selection_empty",
                "REFSET needs at least one selected object",
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("refedit", "no_session", "document not open"))?
            .drawing
            .clone();
        let mut ids = Vec::with_capacity(refs.len());
        for reference in &refs {
            if reference.document != document_id
                || !reference.instance.0.is_empty()
                || reference.sub_element.is_some()
            {
                return Err(invalid(
                    "refedit",
                    "selection",
                    format!(
                        "entity {} is not a directly addressable model-space object",
                        reference.entity.0
                    ),
                ));
            }
            let entity = drawing.entity(reference.entity).ok_or_else(|| {
                invalid(
                    "refedit",
                    "selection",
                    format!("entity {} does not exist", reference.entity.0),
                )
            })?;
            if entity.space != SpaceId::Model {
                return Err(invalid(
                    "refedit",
                    "selection",
                    format!("entity {} is not in model space", entity.id.0),
                ));
            }
            // A reference to the block being edited cannot join that block's own
            // working set: saving it would make the definition contain itself.
            if option == RefSetOption::Add {
                if let SemanticGeometry::Insert {
                    block: referenced, ..
                } = &entity.geometry
                {
                    if *referenced == edited_block {
                        return Err(invalid(
                            "refedit",
                            "selection",
                            format!(
                                "entity {} is a reference to block {} itself; a block cannot \
                                 contain an instance of itself",
                                entity.id.0, edited_block.0
                            ),
                        ));
                    }
                }
            }
            ids.push(entity.id);
        }

        let active = session
            .refedit
            .active_mut()
            .ok_or_else(|| invalid("refedit", "no_session", "no active in-place block edit"))?;
        let mut changed = 0usize;
        for id in ids {
            let updated = match option {
                RefSetOption::Add => active.working.insert(id),
                RefSetOption::Remove => active.working.remove(&id),
            };
            if updated {
                changed += 1;
            }
        }
        let working = active.working.len();
        let (code, verb) = match option {
            RefSetOption::Add => ("refset.add", "added"),
            RefSetOption::Remove => ("refset.remove", "removed"),
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: None,
                code: code.into(),
                message: format!(
                    "REFSET {verb} {changed} object(s); the working set now holds {working} \
                     object(s) (session state only)"
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
