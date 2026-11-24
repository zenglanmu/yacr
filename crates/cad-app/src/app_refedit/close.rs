//! `REFCLOSE SAVE|DISCARD`: close the in-place block edit.
//!
//! `SAVE` writes the working set back into the block definition through
//! [`cad_db::DrawingDatabase::apply_drawing_changes_with_blocks`] — the single
//! block-table authority — as **one** transaction / one undo step: the updated
//! block row, every member's inverse-transformed geometry **and layer, draw
//! order and changed render attributes**, added host objects moved into the block
//! space (also with a block-relative draw order), removed members deleted —
//! including pruning them from the block's dynamic-visibility descriptor — and
//! the working clones deleted from model space. `DISCARD` deletes the extracted clones (one
//! transaction when there is anything to delete) and leaves the block definition
//! untouched. Both unhide the source INSERT and clear the session.

use super::*;

/// The named refusal for a draw order that cannot be represented in `i64`.
///
/// Checked while planning, so it fires before the batch is built: nothing is
/// written, no revision, no history.
fn order_exhausted() -> CadError {
    invalid(
        "refedit",
        "order_exhausted",
        "the block draw order would overflow i64, so nothing was written",
    )
}

/// The draw order every member row takes on `SAVE`.
///
/// Draw order is only meaningful **within one scope**, and a working clone's
/// `draw_order` is a model-space value while a member's is a block-space value:
/// the two sequences are not comparable. The plan therefore never copies a
/// clone's number into the block. Surviving members keep their existing slot
/// values and the members that have a clone are re-sequenced into the clones'
/// relative order — a permutation inside those slots — so an untouched member
/// never moves and no unrelated row is rewritten. Host objects added to the
/// working set are appended above the survivors in their own model-space order.
fn planned_member_draw_orders(
    candidate: &DrawingDatabase,
    block: BlockId,
    written: &BTreeMap<EntityId, EntityId>,
    removed: &BTreeSet<EntityId>,
    added: &[EntityId],
) -> CadResult<BTreeMap<EntityId, i64>> {
    // The surviving block members in their stored (draw_order, id) order; their
    // orders are the slots the re-sequencing permutes inside.
    let mut slots: Vec<(i64, u128, EntityId)> = candidate
        .entities()
        .filter(|entity| entity.space == SpaceId::Block(block) && !removed.contains(&entity.id))
        .map(|entity| (entity.draw_order, entity.id.0, entity.id))
        .collect();
    slots.sort_unstable();
    let slot_orders: Vec<i64> = slots.iter().map(|(order, _, _)| *order).collect();

    // The members represented by a clone, ordered the way the user left the
    // clones in model space.
    let mut clone_order: Vec<(i64, u128, EntityId)> = written
        .iter()
        .map(|(&original, &clone)| {
            let row = candidate.entity(clone).ok_or_else(|| {
                CadError::Invariant(format!(
                    "reference edit: working clone {} vanished before the draw order plan",
                    clone.0
                ))
            })?;
            Ok((row.draw_order, clone.0, original))
        })
        .collect::<CadResult<_>>()?;
    clone_order.sort_unstable();

    let queue: Vec<EntityId> = clone_order
        .iter()
        .map(|(_, _, original)| *original)
        .collect();
    let mut queue_index = 0usize;
    let mut planned: BTreeMap<EntityId, i64> = BTreeMap::new();
    for (index, (_, _, id)) in slots.iter().enumerate() {
        if written.contains_key(id) {
            let original = *queue.get(queue_index).ok_or_else(|| {
                CadError::Invariant("reference edit: clone order and member slots disagree".into())
            })?;
            queue_index += 1;
            planned.insert(original, slot_orders[index]);
        }
    }
    if queue_index != queue.len() {
        return Err(CadError::Invariant(
            "reference edit: a written member is not a block-space survivor".into(),
        ));
    }

    // Added host objects are new members: append them above every survivor in
    // their own model-space relative order.
    let mut added_rows: Vec<(i64, u128, EntityId)> = added
        .iter()
        .map(|id| {
            let row = candidate.entity(*id).ok_or_else(|| {
                CadError::Invariant(format!(
                    "reference edit: added object {} vanished before the draw order plan",
                    id.0
                ))
            })?;
            Ok((row.draw_order, id.0, *id))
        })
        .collect::<CadResult<_>>()?;
    added_rows.sort_unstable();
    let mut next = match slot_orders.last() {
        Some(order) => order.checked_add(1).ok_or_else(order_exhausted)?,
        None => 0,
    };
    for (_, _, id) in added_rows {
        planned.insert(id, next);
        next = next.checked_add(1).ok_or_else(order_exhausted)?;
    }
    Ok(planned)
}

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

        // Resolve the three member groups before any row is built: a written
        // member has a surviving working clone, a removed member's clone left
        // the working set (or was deleted by the host), and an added object is a
        // working id that was not extracted from the block.
        let mut written: BTreeMap<EntityId, EntityId> = BTreeMap::new();
        for (&clone_id, &original) in &active.clone_of {
            if active.working.contains(&clone_id) && candidate.entity(clone_id).is_some() {
                // A member the host removed while the session was active cannot
                // be written back: refuse by name before the draw-order plan or
                // any row is built.
                if candidate.entity(original).is_none() {
                    return Err(invalid(
                        "refedit",
                        "clone_failed",
                        format!(
                            "block member {} of block {} is no longer in the database",
                            original.0, active.block.0
                        ),
                    ));
                }
                written.insert(original, clone_id);
            }
        }
        let mut removed: BTreeSet<EntityId> = BTreeSet::new();
        for &original in active.clone_of.values() {
            if !written.contains_key(&original) && candidate.entity(original).is_some() {
                removed.insert(original);
            }
        }
        let added: Vec<EntityId> = active
            .working
            .iter()
            .copied()
            .filter(|id| !active.clone_of.contains_key(id) && candidate.entity(*id).is_some())
            .collect();
        let draw_orders =
            planned_member_draw_orders(&candidate, active.block, &written, &removed, &added)?;

        let mut changes: Vec<(EntityId, Option<DbEntity>)> = Vec::new();
        // A member's render attributes (color/linetype/lineweight/transparency)
        // are editable on the clone; a changed set is written back with the same
        // display path, while the member's visibility membership (a property of
        // the block, not of the clone) is kept exactly.
        let mut display: Vec<(EntityId, EntityDisplayState)> = Vec::new();

        // 1. Working clones still present: inverse-transform their geometry and
        //    write the clone's layer and planned draw order back onto the mapped
        //    original block member.
        for (&original, &clone_id) in &written {
            let clone = candidate.entity(clone_id).cloned().ok_or_else(|| {
                CadError::Invariant(format!(
                    "reference edit: working clone {} vanished before the save",
                    clone_id.0
                ))
            })?;
            let original_row = candidate.entity(original).cloned().ok_or_else(|| {
                invalid(
                    "refedit",
                    "clone_failed",
                    format!(
                        "block member {} of block {} is no longer in the database",
                        original.0, active.block.0
                    ),
                )
            })?;
            let geometry = place_in_block_space(&clone.geometry, &inverse)?;
            let mut after = original_row;
            after.geometry = geometry;
            after.layer = clone.layer;
            after.draw_order = *draw_orders.get(&original).ok_or_else(|| {
                CadError::Invariant(format!(
                    "reference edit: no draw order planned for member {}",
                    original.0
                ))
            })?;
            let clone_attributes = candidate.entity_render_attributes(clone_id);
            if clone_attributes != candidate.entity_render_attributes(original) {
                display.push((
                    original,
                    EntityDisplayState {
                        attributes: clone_attributes,
                        visibility: candidate.entity_visibility_membership(original),
                    },
                ));
            }
            changes.push((original, Some(after)));
        }

        // 2. Originals whose clone left the working set (REFSET REMOVE) or was
        //    deleted: delete them from the block. The model-space copy stays.
        for &original in &removed {
            changes.push((original, None));
        }

        // 3. Added host objects in the working set: move them into the block,
        //    keeping their layer and giving them a block-relative draw order.
        for &id in &added {
            let entity = candidate.entity(id).cloned().ok_or_else(|| {
                CadError::Invariant(format!(
                    "reference edit: added object {} vanished before the save",
                    id.0
                ))
            })?;
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
            after.draw_order = *draw_orders.get(&id).ok_or_else(|| {
                CadError::Invariant(format!(
                    "reference edit: no draw order planned for added object {}",
                    id.0
                ))
            })?;
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
        // A deleted member must also leave the block's dynamic-visibility
        // descriptor: the descriptor is member-id keyed, and a dangling id would
        // be unreachable (and is rejected by the builder's own validation on the
        // next full build). Pruning is the representable outcome for a
        // `REFSET REMOVE` of a governed member; a hidden member that was never
        // extracted keeps its membership untouched.
        let surviving: BTreeSet<EntityId> = entities.iter().copied().collect();
        let mut dynamic_visibility = old_definition.dynamic_visibility.clone();
        if let Some(descriptor) = dynamic_visibility.as_mut() {
            descriptor
                .member_entities
                .retain(|id| surviving.contains(id));
            for state in &mut descriptor.states {
                state.entities.retain(|id| surviving.contains(id));
            }
        }
        let block_row = BlockRow {
            definition: BlockDefinition {
                id: active.block,
                entities,
                dynamic_visibility,
            },
            metadata: old_metadata,
        };
        let block_changes = vec![(active.block, Some(block_row.clone()))];

        let captured = capture_before(&candidate, &changes);
        let transaction = next_transaction_id();
        let change_set = candidate.apply_drawing_changes_with_blocks_and_display(
            "reference edit save",
            transaction,
            &block_changes,
            changes,
            &display,
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
                     row(s) changed, including member layer and draw order); the source \
                     reference is visible again",
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
