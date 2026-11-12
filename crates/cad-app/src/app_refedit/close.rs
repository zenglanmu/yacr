//! `REFCLOSE SAVE|DISCARD`: close the in-place block edit.
//!
//! `SAVE` writes the working set back into the block definition through
//! [`cad_db::DrawingDatabase::apply_drawing_changes_with_blocks`] — the single
//! block-table authority — as **one** transaction / one undo step: the updated
//! block row, every member's inverse-transformed geometry, added host objects
//! moved into the block space, removed members deleted and the working clones
//! deleted from model space. `DISCARD` deletes the extracted clones (one
//! transaction when there is anything to delete) and leaves the block definition
//! untouched. Both unhide the source INSERT and clear the session.

use super::*;

impl Application {
    /// `REFCLOSE SAVE|DISCARD`.
    pub(super) fn refclose(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let option = match &command.payload {
            CommandPayload::RefClose(option) => *option,
            CommandPayload::None => {
                return Err(invalid(
                    "refclose",
                    "needs_option",
                    "REFCLOSE needs SAVE or DISCARD",
                ))
            }
            _ => {
                return Err(invalid(
                    "refclose",
                    "invalid_option",
                    "REFCLOSE takes SAVE or DISCARD; no other payload is accepted",
                ))
            }
        };
        match option {
            RefCloseOption::Save => self.refclose_save(session, command),
            RefCloseOption::Discard => self.refclose_discard(session, command),
        }
    }

    /// `REFCLOSE SAVE`: write the working set back into the block definition.
    fn refclose_save(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let active = session
            .refedit
            .active()
            .cloned()
            .ok_or_else(|| invalid("refedit", "no_session", "no active in-place block edit"))?;
        let document_id = command.document;
        if active.document != document_id {
            return Err(invalid(
                "refedit",
                "no_session",
                "the active in-place block edit belongs to another document",
            ));
        }
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("refedit", "no_session", "document not open"))?
            .drawing
            .as_ref()
            .clone();
        let old_definition = candidate.block(active.block).cloned().ok_or_else(|| {
            invalid(
                "refedit",
                "block_missing",
                format!("block {} is not in the database", active.block.0),
            )
        })?;
        let old_metadata = candidate.block_metadata(active.block).cloned();
        let old_row = BlockRow {
            definition: old_definition.clone(),
            metadata: old_metadata.clone(),
        };
        let inverse = similarity_inverse(&active.insert_transform)?;

        let mut changes: Vec<(EntityId, Option<DbEntity>)> = Vec::new();
        let mut written_originals: BTreeSet<EntityId> = BTreeSet::new();

        // 1. Working clones still present: inverse-transform their geometry back
        //    onto the mapped original block member.
        for (&clone_id, &original) in &active.clone_of {
            if !active.working.contains(&clone_id) {
                continue;
            }
            let Some(clone) = candidate.entity(clone_id).cloned() else {
                // The host deleted the clone while editing: that member is gone.
                continue;
            };
            let Some(original_row) = candidate.entity(original).cloned() else {
                return Err(invalid(
                    "refedit",
                    "clone_failed",
                    format!(
                        "block member {} of block {} is no longer in the database",
                        original.0, active.block.0
                    ),
                ));
            };
            let geometry = place_in_block_space(&clone.geometry, &inverse)?;
            let mut after = original_row;
            after.geometry = geometry;
            changes.push((original, Some(after)));
            written_originals.insert(original);
        }

        // 2. Originals whose clone left the working set (REFSET REMOVE) or was
        //    deleted: delete them from the block. The model-space copy stays.
        for &original in active.clone_of.values() {
            if written_originals.contains(&original) {
                continue;
            }
            if candidate.entity(original).is_some() {
                changes.push((original, None));
            }
        }

        // 3. Added host objects in the working set: move them into the block.
        for &id in &active.working {
            if active.clone_of.contains_key(&id) {
                continue;
            }
            let Some(entity) = candidate.entity(id).cloned() else {
                continue; // deleted by the host: nothing to move
            };
            if entity.space != SpaceId::Model {
                return Err(invalid(
                    "refedit",
                    "clone_failed",
                    format!(
                        "working entity {} is not a model-space object and cannot move into \
                         block {}",
                        id.0, active.block.0
                    ),
                ));
            }
            let geometry = place_in_block_space(&entity.geometry, &inverse)?;
            let mut after = entity;
            after.geometry = geometry;
            after.space = SpaceId::Block(active.block);
            changes.push((id, Some(after)));
        }

        // 4. Delete the working clones from model space (a clone removed from the
        //    working set is not deleted: it is an ordinary host copy now).
        for &id in &active.working {
            if active.clone_of.contains_key(&id) && candidate.entity(id).is_some() {
                changes.push((id, None));
            }
        }

        let entities = expected_block_members(&candidate, active.block, &changes);
        let block_row = BlockRow {
            definition: BlockDefinition {
                id: active.block,
                entities,
                dynamic_visibility: old_definition.dynamic_visibility.clone(),
            },
            metadata: old_metadata,
        };
        let block_changes = vec![(active.block, Some(block_row.clone()))];

        let captured = capture_before(&candidate, &changes);
        let transaction = next_transaction_id();
        let change_set = candidate.apply_drawing_changes_with_blocks(
            "reference edit save",
            transaction,
            &block_changes,
            changes,
        )?;
        let patches = patches_after(&candidate, captured);
        let written = patches.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "reference edit save".into(),
                patches,
                block_patches: vec![BlockPatch {
                    id: active.block,
                    before: Some(old_row),
                    after: Some(block_row),
                }],
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        session.hidden_entities.remove(&active.source);
        session.refedit.clear();
        session.selection = SelectionSet::new();
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "refedit.save".into(),
                message: format!(
                    "REFCLOSE SAVE: wrote the working set back into block {} ({written} entity \
                     row(s) changed); the source reference is visible again",
                    active.block.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `REFCLOSE DISCARD`: delete the extracted clones, keep the definition.
    fn refclose_discard(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let active = session
            .refedit
            .active()
            .cloned()
            .ok_or_else(|| invalid("refedit", "no_session", "no active in-place block edit"))?;
        let document_id = command.document;
        if active.document != document_id {
            return Err(invalid(
                "refedit",
                "no_session",
                "the active in-place block edit belongs to another document",
            ));
        }
        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| invalid("refedit", "no_session", "document not open"))?
            .drawing
            .as_ref()
            .clone();
        let clones: Vec<EntityId> = active
            .clone_of
            .keys()
            .copied()
            .filter(|id| candidate.entity(*id).is_some())
            .collect();

        let change_set = if clones.is_empty() {
            // Nothing to delete: no transaction, no revision bump, no history.
            None
        } else {
            let changes: Vec<(EntityId, Option<DbEntity>)> =
                clones.iter().map(|id| (*id, None)).collect();
            let captured = capture_before(&candidate, &changes);
            let transaction = next_transaction_id();
            let change_set =
                candidate.apply_drawing_changes("reference edit discard", transaction, changes)?;
            let patches = patches_after(&candidate, captured);
            self.publish_candidate(
                document_id,
                candidate,
                DrawingUndoRecord {
                    transaction: change_set.transaction,
                    label: "reference edit discard".into(),
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
            Some(change_set)
        };

        session.hidden_entities.remove(&active.source);
        session.refedit.clear();
        session.selection = SelectionSet::new();
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: change_set.map(Arc::new),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "refedit.discard".into(),
                message: format!(
                    "REFCLOSE DISCARD: deleted {} extracted clone(s); block {} is unchanged",
                    clones.len(),
                    active.block.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}
