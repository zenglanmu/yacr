//! `DRAWORDER FRONT|BACK|ABOVE|BELOW` (AutoCAD 2026), planned **per draw-order
//! scope**.
//!
//! Split out of `draw2_edit` (which keeps `TREX`/`EXTRIM`/`MPEDIT`) so both files
//! stay inside the code-size gate. The ordering rule is the one `HATCHTOBACK`
//! already follows: a *scope* is one [`SpaceId`] value — model space, one paper
//! layout, or one block definition — because that is the only grouping every
//! consumer of [`DbEntity::draw_order`] performs. There is no second ordering
//! mechanism and no cross-space sequence: a layout's draw order is only meaningful
//! against that layout's rows, and a block definition's members only against each
//! other.
//!
//! # Modes
//!
//! * `FRONT` — the selected objects of each scope receive the contiguous orders
//!   `max(non-selected) + 1 ..` in their existing relative order (id tie-break).
//! * `BACK` — the same below `min(non-selected) − n`.
//! * `ABOVE` / `BELOW` — the **first** selected object is the reference; every
//!   other selected object moves directly above/below it in the reference's own
//!   scope, shifting only that scope's rows on the same side when the gap is too
//!   small. A selected object in a different scope from the reference is the named
//!   `draworder.reference_scope` refusal, because "above object X" has no meaning
//!   across two independent sequences.
//!
//! A scope with no non-selected object keeps its rows untouched (there is nothing
//! to be in front of / behind); every scope is planned before anything is staged,
//! so an `i64` overflow in any scope is the whole command's
//! `draworder.order_exhausted` refusal and writes nothing.
//!
//! # Write
//!
//! Only rows whose order really changed are staged, with full before/after entity
//! patches, in one transaction / one undo step. Success is `draworder.applied`;
//! nothing to do is `draworder.unchanged` with no transaction and no revision bump.
//!
//! Work-only (the command layer refuses Viewer mode).

use std::collections::BTreeSet;

use super::*;

fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

fn order_exhausted() -> CadError {
    CadError::Unsupported(format!(
        "{}: the target draw order would overflow i64, so nothing was written",
        "draworder.order_exhausted"
    ))
}

fn unchanged_order(diagnostics: Vec<Diagnostic>, detail: &str) -> CommandOutcome {
    let mut diagnostics = diagnostics;
    diagnostics.push(Diagnostic {
        object: None,
        code: "draworder.unchanged".into(),
        message: format!("DRAWORDER: {detail}; nothing was written"),
    });
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics,
        measurement: None,
        point_report: None,
    }
}

impl Application {
    /// `DRAWORDER FRONT|BACK|ABOVE|BELOW` over the current selection, per scope.
    pub(crate) fn draw_order(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let mode = match &command.payload {
            CommandPayload::Draw2(payload) => match **payload {
                Draw2Payload::DrawOrder(mode) => mode,
                _ => {
                    return Err(refusal(
                        "draworder.payload",
                        "DRAWORDER needs a Draw2::DrawOrder(Front|Back|Above|Below) payload",
                    ));
                }
            },
            _ => {
                return Err(refusal(
                    "draworder.payload",
                    "DRAWORDER needs a Draw2::DrawOrder(Front|Back|Above|Below) payload",
                ));
            }
        };
        let refs = session.selection.refs();
        if refs.is_empty() {
            return Err(refusal(
                "draworder.selection",
                "DRAWORDER needs at least one selected object",
            ));
        }
        if matches!(mode, DrawOrderMode::Above | DrawOrderMode::Below) && refs.len() < 2 {
            return Err(refusal(
                "draworder.reference_required",
                "DRAWORDER ABOVE/BELOW uses the first selected object as the reference and \
                 needs at least one more selected object to move",
            ));
        }
        let document_id = command.document;
        let (entities, diagnostics) =
            self.resolve_batch_no_space(document_id, refs, "draworder")?;
        let mut selected: Vec<DbEntity> = entities;
        selected.sort_by_key(|entity| (entity.draw_order, entity.id.0));
        let selected_ids: BTreeSet<EntityId> = selected.iter().map(|entity| entity.id).collect();

        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| refusal("draworder.not_addressable", "document not open"))?;
        let drawing = document.drawing.as_ref();

        // `ABOVE`/`BELOW`: the reference defines the one scope that can move.
        let reference = if matches!(mode, DrawOrderMode::Above | DrawOrderMode::Below) {
            let reference_id = refs[0].entity;
            let reference = selected
                .iter()
                .find(|entity| entity.id == reference_id)
                .cloned()
                .ok_or_else(|| {
                    refusal(
                        "draworder.reference_required",
                        "the first selected object does not resolve to an entity",
                    )
                })?;
            let foreign: Vec<EntityId> = selected
                .iter()
                .filter(|entity| entity.id != reference.id && entity.space != reference.space)
                .map(|entity| entity.id)
                .collect();
            if !foreign.is_empty() {
                return Err(refusal(
                    "draworder.reference_scope",
                    format!(
                        "the reference {} is in {:?} but selected object(s) {:?} are in another \
                         scope; draw order is planned per space/block definition",
                        reference.id.0, reference.space, foreign
                    ),
                ));
            }
            Some(reference)
        } else {
            None
        };

        // `SpaceId` is not `Ord`; the population is tiny (one row per space with a
        // selected object), so a linear dedup is the honest shape.
        let mut scopes: Vec<SpaceId> = Vec::new();
        for entity in &selected {
            if !scopes.contains(&entity.space) {
                scopes.push(entity.space.clone());
            }
        }
        if let Some(reference) = &reference {
            scopes = vec![reference.space.clone()];
        }

        let mut planned: Vec<(DbEntity, i64)> = Vec::new();
        for space in scopes {
            let scope_selected: Vec<DbEntity> = selected
                .iter()
                .filter(|entity| entity.space == space.clone())
                .cloned()
                .collect();
            let mut others: Vec<DbEntity> = drawing
                .entities()
                .filter(|entity| entity.space == space.clone())
                .filter(|entity| !selected_ids.contains(&entity.id))
                .cloned()
                .collect();
            others.sort_by_key(|entity| (entity.draw_order, entity.id.0));
            match mode {
                DrawOrderMode::Front | DrawOrderMode::Back => {
                    if others.is_empty() {
                        // Nothing to be in front of / behind in this scope.
                        continue;
                    }
                    let mut target = if matches!(mode, DrawOrderMode::Front) {
                        others
                            .iter()
                            .map(|entity| entity.draw_order)
                            .max()
                            .unwrap_or(0)
                            .checked_add(1)
                            .ok_or_else(order_exhausted)?
                    } else {
                        others
                            .iter()
                            .map(|entity| entity.draw_order)
                            .min()
                            .unwrap_or(0)
                            .checked_sub(scope_selected.len() as i64)
                            .ok_or_else(order_exhausted)?
                    };
                    for entity in &scope_selected {
                        planned.push((entity.clone(), target));
                        target = target.checked_add(1).ok_or_else(order_exhausted)?;
                    }
                }
                DrawOrderMode::Above | DrawOrderMode::Below => {
                    let reference = reference.clone().ok_or_else(|| {
                        refusal("draworder.reference_required", "no ABOVE/BELOW reference")
                    })?;
                    let moving: Vec<DbEntity> = scope_selected
                        .iter()
                        .filter(|entity| entity.id != reference.id)
                        .cloned()
                        .collect();
                    let n = moving.len() as i64;
                    let reference_order = reference.draw_order;
                    let above = matches!(mode, DrawOrderMode::Above);
                    let side: Vec<DbEntity> = others
                        .iter()
                        .filter(|entity| {
                            if above {
                                entity.draw_order > reference_order
                            } else {
                                entity.draw_order < reference_order
                            }
                        })
                        .cloned()
                        .collect();
                    let gap: i64 = if above {
                        side.iter()
                            .map(|entity| entity.draw_order)
                            .min()
                            .map(|next| next - reference_order - 1)
                            .unwrap_or(i64::MAX / 4)
                    } else {
                        side.iter()
                            .map(|entity| entity.draw_order)
                            .max()
                            .map(|previous| reference_order - previous - 1)
                            .unwrap_or(i64::MAX / 4)
                    };
                    let shift = if gap >= n { 0 } else { n - gap };
                    let base = if above {
                        reference_order.checked_add(1).ok_or_else(order_exhausted)?
                    } else {
                        reference_order.checked_sub(n).ok_or_else(order_exhausted)?
                    };
                    for (index, entity) in moving.iter().enumerate() {
                        planned.push((
                            entity.clone(),
                            base.checked_add(index as i64).ok_or_else(order_exhausted)?,
                        ));
                    }
                    if shift > 0 {
                        for entity in side {
                            planned.push((
                                entity.clone(),
                                if above {
                                    entity
                                        .draw_order
                                        .checked_add(shift)
                                        .ok_or_else(order_exhausted)?
                                } else {
                                    entity
                                        .draw_order
                                        .checked_sub(shift)
                                        .ok_or_else(order_exhausted)?
                                },
                            ));
                        }
                    }
                }
            }
        }

        let mut changed: Vec<(DbEntity, i64)> = planned
            .into_iter()
            .filter(|(entity, order)| entity.draw_order != *order)
            .collect();
        changed.sort_by_key(|(entity, _)| (entity.draw_order, entity.id.0));
        if changed.is_empty() {
            return Ok(unchanged_order(
                diagnostics,
                "every target already has its draw order",
            ));
        }
        let mut candidate = drawing.clone();
        let drawing = &mut candidate;
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("draw order", transaction)?;
            for (entity, order) in &changed {
                let mut after = entity.clone();
                after.draw_order = *order;
                staged.update_entity(after)?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = changed
            .iter()
            .map(|(entity, _)| {
                let after = drawing.entity(entity.id).cloned().ok_or_else(|| {
                    CadError::Invariant(format!(
                        "DRAWORDER: entity {} vanished after commit",
                        entity.id.0
                    ))
                })?;
                Ok(drawing_patch(entity.id, Some(entity.clone()), Some(after)))
            })
            .collect::<CadResult<Vec<_>>>()?;
        let ids: Vec<ObjectId> = changed
            .iter()
            .map(|(entity, _)| ObjectId(entity.id.0))
            .collect();
        let moved = ids.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "draw order".into(),
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
        let mut diagnostics = diagnostics;
        diagnostics.push(Diagnostic {
            object: None,
            code: "draworder.applied".into(),
            message: format!(
                "DRAWORDER: rewrote the draw order of {moved} object(s) within their own scope \
                 in one transaction (one undo step)"
            ),
        });
        Ok(CommandOutcome {
            objects: ids,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}
