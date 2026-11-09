//! `REVERSE` and `OOPS` (AutoCAD 2026 2D modify commands).
//!
//! This module is the honest home of the two selection/database modify commands
//! that are not tied to the draw-capture state machine:
//!
//! * [`crate::CommandId::Reverse`] — reverse the vertices/direction of selected
//!   LINEs, LWPOLYLINEs and SPLINEs (selection-based, one transaction);
//! * [`crate::CommandId::Oops`] — reinsert the entities removed by the last
//!   ERASE (selection-free, one transaction with fresh ids).
//!
//! `OVERKILL` lives in [`super::overkill`] because its documented model
//! (tolerance, ignore toggles, collinear combines, polyline optimisation) is a
//! comparison engine of its own and would not fit this module's size budget.
//!
//! Every command in this module refuses **before** opening a transaction: a
//! refusal changes no revision, publishes no `ChangeSet` and records no undo
//! entry. Each successful write is exactly one transaction / one undo step.

use super::*;

impl Application {
    /// `REVERSE`: reverse the vertices/direction of every selected LINE,
    /// LWPOLYLINE and SPLINE in one transaction / one undo step.
    ///
    /// The selection is the whole input; see
    /// [`crate::command_id::reverse`] for the exact supported geometry, the
    /// bulge/knot reversal rules and every refusal code.
    pub(crate) fn reverse_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "reverse.payload: REVERSE takes no payload; the selection is the input".into(),
            ));
        }
        let refs = session.selection.refs();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "reverse.selection: REVERSE needs at least one selected entity".into(),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // Resolve and build every reversed row before the transaction opens, so a
        // refusal leaves the drawing and its id allocator untouched.
        let mut planned: Vec<(DbEntity, DbEntity)> = Vec::with_capacity(refs.len());
        for reference in refs {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(format!(
                    "reverse.not_addressable: entity {} belongs to another document",
                    reference.entity.0
                )));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(CadError::InvalidInput(format!(
                    "reverse.not_addressable: entity {} is referenced through a block instance or \
                     a sub-element; only directly addressable entities can be reversed",
                    reference.entity.0
                )));
            }
            let before = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "reverse.not_addressable: entity {} does not exist in this drawing",
                    reference.entity.0
                ))
            })?;
            if before.space != SpaceId::Model {
                return Err(CadError::InvalidInput(format!(
                    "reverse.not_addressable: entity {} is not in model space",
                    reference.entity.0
                )));
            }
            let geometry = reverse_geometry(&before.geometry)?;
            let mut after = before.clone();
            after.geometry = geometry;
            planned.push((before, after));
        }

        let transaction = next_transaction_id();
        let mut ids = Vec::with_capacity(planned.len());
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("reverse entities", transaction)?;
            for (_, after) in &planned {
                ids.push(after.id);
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = planned
            .into_iter()
            .map(|(before, after)| {
                let stored = drawing.entity(after.id).cloned().ok_or_else(|| {
                    CadError::Invariant(format!(
                        "REVERSE: entity {} vanished after commit",
                        after.id.0
                    ))
                })?;
                let _ = after;
                Ok(drawing_patch(before.id, Some(before), Some(stored)))
            })
            .collect::<CadResult<Vec<_>>>()?;
        let count = ids.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "reverse entities".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: ids.iter().map(|id| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "reverse.applied".into(),
                message: format!("REVERSE: reversed {count} entity/entities"),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `OOPS`: reinsert the entities removed by the **last ERASE** in one
    /// transaction / one undo step, with fresh entity ids.
    ///
    /// The snapshot is session state ([`crate::session_state::EraseSnapshot`])
    /// written by ERASE **after** its delete committed. It belongs to one
    /// document and is consumed by the first OOPS, so a second OOPS reports
    /// `oops.nothing` and writes nothing. Undo of OOPS removes the restored
    /// copies but does **not** recreate the snapshot (the user would have to use
    /// REDO to get the copies back).
    ///
    /// AutoCAD also restores objects removed by BLOCK/WBLOCK; this build records
    /// **ERASE only** (the declared v1 boundary), and the docs say so.
    pub(crate) fn oops_restore(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "oops.payload: OOPS takes no payload".into(),
            ));
        }
        let Some(snapshot) = session.last_erase().cloned() else {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "oops.nothing".into(),
                    message: "OOPS: no ERASE to restore".into(),
                }],
                measurement: None,
                point_report: None,
            });
        };
        if snapshot.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "oops.document: the last ERASE belongs to document {}, not {}",
                snapshot.document.0, command.document.0
            )));
        }
        if snapshot.entries.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object: None,
                    code: "oops.nothing".into(),
                    message: "OOPS: the last ERASE recorded no entities".into(),
                }],
                measurement: None,
                point_report: None,
            });
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // Allocate every fresh id before the transaction opens. The row keeps the
        // erased layer/space/draw order/type key; only its identity is new and the
        // import-only source handle is cleared (it described the erased row).
        let mut planned = Vec::with_capacity(snapshot.entries.len());
        for entry in snapshot.entries {
            let new_id = drawing.allocate_entity_id();
            let mut entity = entry.entity;
            entity.object.id = ObjectId(new_id.0);
            entity.object.source_handle = None;
            entity.id = new_id;
            planned.push((new_id, entity, entry.display));
        }

        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("oops restore", transaction)?;
            for (id, entity, display) in &planned {
                staged.insert_entity(entity.clone())?;
                staged.update_entity_display_state(*id, display)?;
            }
            staged.commit()?
        };

        let mut patches = Vec::with_capacity(planned.len());
        let mut objects = Vec::with_capacity(planned.len());
        for (id, _, display) in &planned {
            let stored = drawing.entity(*id).cloned().ok_or_else(|| {
                CadError::Invariant(format!("OOPS: restored entity {} vanished", id.0))
            })?;
            let mut patch = drawing_patch(*id, None, Some(stored));
            patch.after_state = Some(display.clone());
            patches.push(patch);
            objects.push(ObjectId(id.0));
        }
        let count = patches.len();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "oops restore".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        // Consumed only after the restore committed: a refused OOPS must leave the
        // snapshot intact for the next attempt.
        session.set_last_erase(None);

        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "oops.applied".into(),
                message: format!("OOPS: restored {count} entity/entities from the last ERASE"),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Reverse one supported geometry, or refuse the whole command by name.
///
/// The polyline bulge mapping is `b'[i] = −b[(n − 2 − i) mod n]`: segment `i` of
/// the reversed ring runs the original segment `(n − 2 − i) mod n` backwards, so
/// its signed sweep — and therefore its bulge — flips. This holds for closed
/// rings (`n` segments) and for the `n − 1` real segments of an open polyline.
///
/// `pub(crate)` because [`crate::CommandId::Pedit`]'s `REVERSE` option reuses
/// this exact rule instead of keeping a second, drifting copy.
pub(crate) fn reverse_geometry(geometry: &SemanticGeometry) -> CadResult<SemanticGeometry> {
    match geometry {
        SemanticGeometry::Line { start, end } => Ok(SemanticGeometry::Line {
            start: *end,
            end: *start,
        }),
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            let n = points.len();
            let segments = if *closed { n } else { n.saturating_sub(1) };
            let per_vertex = bulges.len() == n;
            let per_segment = bulges.len() == segments;
            if !per_vertex && !per_segment {
                return Err(CadError::InvalidInput(format!(
                    "reverse.geometry: polyline has {n} vertices but {} bulges; one bulge per \
                     vertex (or per segment) is required",
                    bulges.len()
                )));
            }
            let reversed_points: Vec<Point3> = points.iter().rev().copied().collect();
            let mut reversed_bulges = vec![0.0; bulges.len()];
            for (i, slot) in reversed_bulges.iter_mut().enumerate().take(segments) {
                // Closed: segment i runs original segment `(n − 2 − i) mod n`
                // backwards. Open: the `n − 1` real segments simply map
                // `i → (segments − 1 − i)`.
                let source = if *closed {
                    (n + n - 2 - i) % n
                } else {
                    segments - 1 - i
                };
                *slot = -bulges[source];
            }
            if per_vertex && !*closed {
                // The trailing entry of an open per-vertex array describes no
                // segment; carry it through instead of inventing a value.
                reversed_bulges[n - 1] = bulges[n - 1];
            }
            Ok(SemanticGeometry::Polyline {
                points: reversed_points,
                bulges: reversed_bulges,
                closed: *closed,
            })
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            let last = knots.len().checked_sub(1).ok_or_else(|| {
                CadError::InvalidInput("reverse.geometry: spline has an empty knot vector".into())
            })?;
            let u_last = knots[last];
            let reversed_knots: Vec<f64> =
                (0..knots.len()).map(|i| u_last - knots[last - i]).collect();
            let reversed_points: Vec<Point3> = control_points.iter().rev().copied().collect();
            let reversed_weights: Vec<f64> = weights.iter().rev().copied().collect();
            debug_assert_eq!(reversed_points.len(), control_points.len());
            debug_assert_eq!(reversed_knots.len(), knots.len());
            Ok(SemanticGeometry::Spline {
                degree: *degree,
                knots: reversed_knots,
                control_points: reversed_points,
                weights: reversed_weights,
            })
        }
        other => Err(CadError::Unsupported(format!(
            "reverse.unsupported: {} is not LINE, LWPOLYLINE or SPLINE; REVERSE v1 changes nothing",
            class_name(other)
        ))),
    }
}
