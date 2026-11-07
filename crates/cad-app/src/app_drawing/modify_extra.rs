//! `REVERSE`, `OVERKILL` and `OOPS` (AutoCAD 2026 2D modify commands).
//!
//! This module is the honest home of the three selection/database modify
//! commands that are not tied to the draw-capture state machine:
//!
//! * [`crate::CommandId::Reverse`] — reverse the vertices/direction of selected
//!   LINEs, LWPOLYLINEs and SPLINEs (selection-based, one transaction);
//! * [`crate::CommandId::Overkill`] — remove **exact duplicate** selected
//!   entities (v1, one transaction; see [`Application::overkill`]);
//! * [`crate::CommandId::Oops`] — reinsert the entities removed by the last
//!   ERASE (selection-free, one transaction with fresh ids).
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
fn reverse_geometry(geometry: &SemanticGeometry) -> CadResult<SemanticGeometry> {
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

/// The exact-duplicate comparison tolerance (world units, and radians for angles).
const OVERKILL_EPSILON: f64 = 1e-9;

fn nearly_equal(a: f64, b: f64) -> bool {
    (a - b).abs() <= OVERKILL_EPSILON
}

fn points_equal(a: Point3, b: Point3) -> bool {
    nearly_equal(a.x, b.x) && nearly_equal(a.y, b.y) && nearly_equal(a.z, b.z)
}

/// Exact-duplicate geometry comparison for OVERKILL v1.
///
/// Only the five kinds with a documented comparison are equal to anything:
/// LINE (unordered endpoint pair), LWPOLYLINE (points pairwise, segment bulges,
/// closed flag), CIRCLE (centre/normal/radius), ARC
/// (centre/normal/radius/start/sweep) and SPLINE
/// (degree/knots/control points/weights). Every other geometry kind — including
/// a kind paired with itself — is `false`: v1 never merges what it cannot
/// compare exactly.
fn same_duplicate_geometry(a: &SemanticGeometry, b: &SemanticGeometry) -> bool {
    match (a, b) {
        (
            SemanticGeometry::Line { start: a0, end: a1 },
            SemanticGeometry::Line { start: b0, end: b1 },
        ) => {
            (points_equal(*a0, *b0) && points_equal(*a1, *b1))
                || (points_equal(*a0, *b1) && points_equal(*a1, *b0))
        }
        (
            SemanticGeometry::Polyline {
                points: a_points,
                bulges: a_bulges,
                closed: a_closed,
            },
            SemanticGeometry::Polyline {
                points: b_points,
                bulges: b_bulges,
                closed: b_closed,
            },
        ) => {
            if a_closed != b_closed || a_points.len() != b_points.len() {
                return false;
            }
            if !a_points
                .iter()
                .zip(b_points)
                .all(|(a, b)| points_equal(*a, *b))
            {
                return false;
            }
            // Compare real segments only; the unused trailing entry of an open
            // per-vertex bulge array describes no segment.
            let segments = if *a_closed {
                a_points.len()
            } else {
                a_points.len().saturating_sub(1)
            };
            (0..segments).all(|i| {
                nearly_equal(
                    a_bulges.get(i).copied().unwrap_or(0.0),
                    b_bulges.get(i).copied().unwrap_or(0.0),
                )
            })
        }
        (
            SemanticGeometry::Circle {
                center: a_center,
                normal: a_normal,
                radius: a_radius,
            },
            SemanticGeometry::Circle {
                center: b_center,
                normal: b_normal,
                radius: b_radius,
            },
        ) => {
            points_equal(*a_center, *b_center)
                && points_equal(*a_normal, *b_normal)
                && nearly_equal(*a_radius, *b_radius)
        }
        (
            SemanticGeometry::Arc {
                center: a_center,
                normal: a_normal,
                radius: a_radius,
                start: a_start,
                sweep: a_sweep,
            },
            SemanticGeometry::Arc {
                center: b_center,
                normal: b_normal,
                radius: b_radius,
                start: b_start,
                sweep: b_sweep,
            },
        ) => {
            points_equal(*a_center, *b_center)
                && points_equal(*a_normal, *b_normal)
                && nearly_equal(*a_radius, *b_radius)
                && nearly_equal(*a_start, *b_start)
                && nearly_equal(*a_sweep, *b_sweep)
        }
        (
            SemanticGeometry::Spline {
                degree: a_degree,
                knots: a_knots,
                control_points: a_points,
                weights: a_weights,
            },
            SemanticGeometry::Spline {
                degree: b_degree,
                knots: b_knots,
                control_points: b_points,
                weights: b_weights,
            },
        ) => {
            a_degree == b_degree
                && a_knots.len() == b_knots.len()
                && a_points.len() == b_points.len()
                && a_weights.len() == b_weights.len()
                && a_knots
                    .iter()
                    .zip(b_knots)
                    .all(|(a, b)| nearly_equal(*a, *b))
                && a_points
                    .iter()
                    .zip(b_points)
                    .all(|(a, b)| points_equal(*a, *b))
                && a_weights
                    .iter()
                    .zip(b_weights)
                    .all(|(a, b)| nearly_equal(*a, *b))
        }
        _ => false,
    }
}

impl Application {
    /// `OVERKILL` v1: delete **exact duplicate** selected entities.
    ///
    /// Two selected entities are duplicates when they are directly addressable,
    /// in the same space and on the same layer, have the same geometry kind and
    /// compare equal within [`OVERKILL_EPSILON`] (see
    /// [`same_duplicate_geometry`]). The keeper of each group is the lowest
    /// `draw_order` (ties: lowest entity id); every other member is deleted in
    /// one transaction / one undo step through the shared delete path, which
    /// captures the display state for undo.
    ///
    /// v1 is deliberately **exact duplicates only**. Partial overlaps, collinear
    /// merges, arc-over-circle replacement, polyline segment optimization, the
    /// tolerance option and the ignore-layer/colour options are NOT IMPLEMENTED
    /// and are reported by the `overkill.v1_exact_duplicates_only` diagnostic.
    /// Nothing is ever merged or deleted across layers or spaces.
    pub(crate) fn overkill(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "overkill.payload: OVERKILL takes no payload; the selection is the input".into(),
            ));
        }
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(CadError::InvalidInput(
                "overkill.selection: OVERKILL needs at least one selected entity".into(),
            ));
        }
        let document_id = command.document;
        let drawing = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref();

        // Resolve the directly addressable rows first; a foreign or
        // block-instance/sub-element ref is a named refusal (never silently
        // skipped, because the user asked to compare that object).
        let mut seen: Vec<EntityId> = Vec::new();
        let mut entities: Vec<DbEntity> = Vec::new();
        for reference in references {
            if reference.document != document_id {
                return Err(CadError::InvalidInput(format!(
                    "overkill.not_addressable: entity {} belongs to another document",
                    reference.entity.0
                )));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(CadError::InvalidInput(format!(
                    "overkill.not_addressable: entity {} is referenced through a block instance \
                     or a sub-element; only directly addressable entities can be compared",
                    reference.entity.0
                )));
            }
            if seen.contains(&reference.entity) {
                continue;
            }
            seen.push(reference.entity);
            let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "overkill.target_missing: entity {} does not exist in this drawing",
                    reference.entity.0
                ))
            })?;
            entities.push(entity);
        }

        // Greedy grouping: every candidate is compared with each group's keeper
        // (its lowest-draw-order member). Same space and layer first, so a
        // cross-layer or cross-space pair can never be a group.
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for (index, entity) in entities.iter().enumerate() {
            let mut placed = false;
            for group in &mut groups {
                let keeper = &entities[group[0]];
                if keeper.space == entity.space
                    && keeper.layer == entity.layer
                    && same_duplicate_geometry(&keeper.geometry, &entity.geometry)
                {
                    group.push(index);
                    placed = true;
                    break;
                }
            }
            if !placed {
                groups.push(vec![index]);
            }
        }

        let mut doomed: Vec<EntityId> = Vec::new();
        for mut group in groups {
            if group.len() < 2 {
                continue;
            }
            // Lowest draw_order wins; ties break on the lowest entity id.
            group.sort_by_key(|index| {
                let entity = &entities[*index];
                (entity.draw_order, entity.id.0)
            });
            for index in group.into_iter().skip(1) {
                doomed.push(entities[index].id);
            }
        }

        let informational = Diagnostic {
            object: None,
            code: "overkill.v1_exact_duplicates_only".into(),
            message: "OVERKILL v1 removes exact duplicate entities only; partial overlaps, \
                      collinear merges, arc-over-circle replacement, polyline segment \
                      optimization, a tolerance option and ignore-layer/colour options are \
                      NOT IMPLEMENTED"
                .into(),
        };
        if doomed.is_empty() {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![
                    informational,
                    Diagnostic {
                        object: None,
                        code: "overkill.unchanged".into(),
                        message: "OVERKILL: no exact duplicates among the selected entities".into(),
                    },
                ],
                measurement: None,
                point_report: None,
            });
        }

        let count = doomed.len();
        let refs: Vec<SelectionRef> = doomed
            .iter()
            .map(|entity| SelectionRef {
                document: document_id,
                entity: *entity,
                instance: InstancePath::default(),
                sub_element: None,
            })
            .collect();
        let (changes, mut diagnostics) = self.delete_entity_refs(document_id, &refs, "OVERKILL")?;
        diagnostics.insert(0, informational);
        diagnostics.push(Diagnostic {
            object: None,
            code: "overkill.applied".into(),
            message: format!("OVERKILL: deleted {count} exact duplicate(s)"),
        });
        Ok(CommandOutcome {
            objects: doomed.iter().map(|id| ObjectId(id.0)).collect(),
            changes,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}
