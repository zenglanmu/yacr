//! STRETCH edit command (ADR 0016 decision E).
//!
//! The interaction is four points: the two opposite corners of a **crossing
//! window** (cpolygon/lasso are NOT IMPLEMENTED; the payload is always treated as
//! crossing), then a base point and a displacement point. Enclosure is recomputed
//! from the rectangle and the geometry, never stored in the selection.
//!
//! Behaviour (Autodesk's two-way rule):
//!
//! * **fully enclosed ⇒ move** the whole entity (`transform_geometry`, exactly
//!   what MOVE does);
//! * **partially enclosed ⇒ stretch** only the vertices/endpoints inside the
//!   window. Per-vertex kinds: `LINE` (start/end), `POLYLINE`/`LWPOLYLINE` (every
//!   stored vertex; bulges kept; no width/tangent/curve-fit field is touched),
//!   `SPLINE` (stored control points; a control point is not on the curve, so
//!   this reshapes the whole curve) and `ARC`
//!   (the two stored endpoints move on their own circle; our declared endpoint
//!   rule is the arc grip's rule);
//! * `CIRCLE` cannot be stretched and moves **only when its centre** is enclosed;
//! * `TEXT`/`MTEXT` move **only when their insertion point** is enclosed (the
//!   documented MTEXT rule, declared for TEXT too);
//! * `INSERT` moves rigidly only when its insertion point is enclosed; block
//!   members are never stretched individually;
//! * `DIMENSION` definition points inside the window move and the display is
//!   re-synthesized (Autodesk documents dimensions as stretchable);
//! * `ELLIPSE` and `HATCH` are move-only: a partial enclosure leaves them
//!   unchanged with a named diagnostic (HATCH is not documented in STRETCH, so it
//!   is not invented here).
//!
//! `MULTILEADER` still fails the whole command (its compound display has no
//! per-vertex semantics). An in-window entity that cannot be handled fails the
//! whole command and writes nothing; one transaction / one undo step.
//!
//! **UNVERIFIED**: Autodesk documents only that STRETCH does not modify
//! width/tangency/curve-fit information and does not document a bulge when a
//! bulge-segment endpoint is dragged; our declared rule keeps the stored bulge
//! value unchanged (the tests pin it numerically).

use super::*;

use cad_spatial::pick_world_bounds;

use crate::select_box::{geometry_hits_rect, Rect};

/// The enclosure classification of one entity against the crossing window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Enclosure {
    /// The geometry neither intersects nor is contained by the window.
    Outside,
    /// The geometry hits the window but is not fully enclosed.
    Partial,
    /// The geometry is fully enclosed by the window.
    Full,
}

/// The planned action for one in-window entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StretchAction {
    /// Translate the whole entity by the displacement: a fully enclosed entity, or a
    /// partially enclosed `INSERT` whose insertion point is inside the window (a
    /// block reference moves rigidly, see the module doc).
    Move,
    /// Partially enclosed `LINE`/`POLYLINE`/`SPLINE`/`ARC`: move only the
    /// in-window vertices, control points or arc endpoints.
    Stretch,
    /// Partially enclosed, not stretchable: unchanged plus a diagnostic.
    Unchanged,
}

impl Application {
    /// STRETCH the entities inside a crossing window (ADR 0016 decision E).
    ///
    /// The window corners are canvas-logical (`rect_logical`) and are mapped
    /// through the active viewport's orthographic plan view, exactly like
    /// `BoxSelect`; the effective translation is `delta − base`. Every input is
    /// validated and the enclosure classification is resolved for the whole
    /// drawing **before** any write, so a refusal changes nothing (the database
    /// transaction is all-or-nothing and no history is recorded).
    ///
    /// Refusals (nothing written): a non-finite window corner / base /
    /// displacement point; a degenerate window (zero width or height); a zero
    /// displacement (`delta == base`, nothing to do); a non-model space or
    /// non-plan view; a visible tree the pick builder cannot enumerate (opaque
    /// geometry, cyclic/depth-limited INSERTs, missing block members); and any
    /// in-window entity that cannot be handled (TEXT, or a kind with no
    /// per-vertex edit and no declared move-only rule).
    pub(crate) fn stretch_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Stretch {
            rect_logical,
            base,
            delta,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "StretchEntities needs a Stretch { rect_logical, base, delta } payload".into(),
            ));
        };
        let (rect_logical, base, delta) = (*rect_logical, *base, *delta);
        if !rect_logical.iter().flatten().all(|value| value.is_finite()) {
            return Err(CadError::InvalidInput(
                "STRETCH window corners must be finite".into(),
            ));
        }
        if !is_finite_point(base) || !is_finite_point(delta) {
            return Err(CadError::InvalidInput(
                "STRETCH base and displacement points must be finite".into(),
            ));
        }

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        if document.id != document_id {
            return Err(CadError::StaleResult);
        }
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "STRETCH currently supports model space only".into(),
            ));
        }
        let viewport = self
            .workspace
            .viewports
            .get(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let size = viewport.logical_size;
        if !size.iter().all(|value| value.is_finite() && *value > 0.0) {
            return Err(CadError::InvalidInput(
                "STRETCH viewport size must be finite and positive".into(),
            ));
        }
        // The plan mapping is the same one picking/box-selection use. A
        // perspective/3D view has no plan mapping: 3D STRETCH is NOT IMPLEMENTED.
        let first = viewport
            .screen_to_world(rect_logical[0], size)
            .ok_or_else(|| {
                CadError::Unsupported("STRETCH requires the orthographic plan view".into())
            })?;
        let second = viewport
            .screen_to_world(rect_logical[1], size)
            .ok_or_else(|| {
                CadError::Unsupported("STRETCH requires the orthographic plan view".into())
            })?;
        let rect = Rect::from_corners(first, second);
        let tol = stretch_tolerance();
        if !rect.width().is_finite()
            || !rect.height().is_finite()
            || rect.width() <= tol
            || rect.height() <= tol
        {
            return Err(CadError::InvalidInput(
                "STRETCH window must be non-degenerate (zero width or height)".into(),
            ));
        }
        let translation = Point3 {
            x: delta.x - base.x,
            y: delta.y - base.y,
            z: delta.z - base.z,
        };
        let displacement_length = (translation.x * translation.x
            + translation.y * translation.y
            + translation.z * translation.z)
            .sqrt();
        if !is_finite_point(translation)
            || !displacement_length.is_finite()
            || displacement_length <= tol
        {
            return Err(CadError::InvalidInput(
                "STRETCH displacement must be non-zero and finite (nothing to do)".into(),
            ));
        }

        self.stretch_windows(session, command, vec![rect], translation)
    }

    /// The shared STRETCH planner over one or more crossing windows.
    ///
    /// `stretch_entities` calls it with one window; `MSTRETCH` calls it with the
    /// typed list. An entity is fully enclosed when one window encloses it, is
    /// partial when any window hits it, and its vertices/endpoints move when they
    /// lie inside **any** window. The geometry edit itself is the existing
    /// [`stretched_geometry`] / [`moved_geometry`] planner, never a second copy.
    pub(super) fn stretch_windows(
        &mut self,
        session: &SessionState,
        command: &Command,
        rects: Vec<Rect>,
        translation: Point3,
    ) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // The pick builder silently skips cycles/depth limits; preflight the same
        // visible tree `select_box` refuses on before trusting its catalogue.
        let scope = drawing.root_layer_scope();
        for entity in drawing.model_space() {
            crate::select_all::validate_branch(drawing, session, entity, scope, &mut Vec::new())
                .map_err(stretch_tree_refusal)?;
        }

        // Group the visible pick items by their top-level model-space owner: a
        // bare entity owns itself, an INSERT owns its expanded children (whose
        // `InstancePath` starts at the insert entity).
        let items = drawing_pick_items(drawing, document_id);
        let mut groups: Vec<(EntityId, Vec<usize>)> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let visible = crate::picking::ref_visibility(
                drawing,
                &session.layer_overrides,
                &session.hidden_entities,
                scope,
                &item.source,
            )
            .ok_or(CadError::StaleResult)?;
            if !visible {
                continue;
            }
            let owner = item
                .source
                .instance
                .0
                .first()
                .copied()
                .unwrap_or(item.source.entity);
            match groups.iter_mut().find(|(id, _)| *id == owner) {
                Some((_, indices)) => indices.push(index),
                None => groups.push((owner, vec![index])),
            }
        }

        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let mut plan: Vec<(EntityId, StretchAction)> = Vec::new();
        for (owner, indices) in groups {
            let owner_entity = drawing
                .entity(owner)
                .cloned()
                .ok_or(CadError::StaleResult)?;
            let owner_items: Vec<&crate::PickItem> =
                indices.iter().map(|index| &items[*index]).collect();
            let enclosure = classify_enclosure(&owner_entity.geometry, &owner_items, &rects);
            if enclosure == Enclosure::Outside {
                continue;
            }
            // MULTILEADER: the whole command still fails, exactly as the pre-P7b
            // `Compound` did (both partial and full enclosure). It is refused
            // explicitly by type now that the variant has its own transform arm;
            // no `Unchanged` arm is added, so STRETCH does not fork by content
            // type (ADR 0019 decision G).
            if matches!(&owner_entity.geometry, SemanticGeometry::MultiLeader(_)) {
                return Err(CadError::Unsupported(
                    "STRETCH cannot handle MULTILEADER: its display combines leader paths, \
                     arrowheads, a dogleg and text, so a per-vertex stretch has no defined \
                     semantics; the whole command is refused (the pre-P7b compound behaviour is \
                     preserved)"
                        .into(),
                ));
            }
            match enclosure {
                Enclosure::Full => plan.push((owner, StretchAction::Move)),
                Enclosure::Partial => match &owner_entity.geometry {
                    // The per-vertex stretchable kinds: LINE and LWPOLYLINE
                    // (including bulge segments, whose stored bulges are kept),
                    // plus SPLINE (stored control points) and ARC (stored
                    // start/end points on the unchanged circle).
                    SemanticGeometry::Line { .. }
                    | SemanticGeometry::Polyline { .. }
                    | SemanticGeometry::Spline { .. }
                    | SemanticGeometry::Arc { .. } => {
                        plan.push((owner, StretchAction::Stretch));
                    }
                    // CIRCLE cannot be stretched at all. It moves as a whole only
                    // when its **centre** is enclosed (AutoCAD's documented rule);
                    // a circle whose centre is outside the crossing window is left
                    // unchanged with the named reason `circle_center_outside`.
                    SemanticGeometry::Circle { center, .. } => {
                        if rects.iter().any(|rect| rect.contains_point(*center)) {
                            plan.push((owner, StretchAction::Move));
                        } else {
                            diagnostics
                                .push(stretch_unchanged_diagnostic(owner, "circle_center_outside"));
                        }
                    }
                    // ELLIPSE cannot be stretched: move-only when fully enclosed,
                    // unchanged here.
                    SemanticGeometry::Ellipse { .. } => {
                        plan.push((owner, StretchAction::Unchanged));
                    }
                    // TEXT/MTEXT follow the **insertion-point** rule: move the whole
                    // run iff its stored anchor (`position`) is inside the crossing
                    // window; otherwise leave it unchanged with the named reason
                    // `text_outside`. Autodesk documents this rule for MTEXT and is
                    // silent for TEXT; using the same rule for both is our declared
                    // choice (and is the only rule this flat-text domain can state
                    // honestly, since it stores an anchor and no per-glyph vertices).
                    SemanticGeometry::Text { position, .. } => {
                        if rects.iter().any(|rect| rect.contains_point(*position)) {
                            plan.push((owner, StretchAction::Move));
                        } else {
                            diagnostics.push(stretch_unchanged_diagnostic(owner, "text_outside"));
                        }
                    }
                    // INSERT: the reference has no per-vertex edit, so the declared
                    // rule is rigid: translate the whole reference by the
                    // displacement **iff** its insertion point (the transformed
                    // block-space origin) is inside the crossing window, otherwise
                    // leave it bit-identically unchanged and report the reason by
                    // name (`drawing.stretch.insert_outside`). Both arms are
                    // pre-existing paths — `Move` is the ordinary whole-entity
                    // `transform_geometry` translation (so the linear part of the
                    // placement is preserved bit for bit) and the diagnostic is an
                    // ordinary per-entity "unchanged" report, so nothing is staged.
                    SemanticGeometry::Insert { transform, .. } => {
                        if rects
                            .iter()
                            .any(|rect| rect.contains_point(insertion_point(transform)))
                        {
                            plan.push((owner, StretchAction::Move));
                        } else {
                            diagnostics.push(stretch_unchanged_diagnostic(owner, "insert_outside"));
                        }
                    }
                    // A dimension's **definition points** stretch: every point
                    // inside the crossing window moves by the displacement and the
                    // derived display is re-synthesized. Autodesk documents
                    // dimensions as stretchable; see `stretched_dimension`.
                    SemanticGeometry::Dimension(_) => {
                        plan.push((owner, StretchAction::Stretch));
                    }
                    // A hatch follows `compatibility.md`'s existing rule: a
                    // partial enclosure leaves it unchanged with a diagnostic,
                    // a full enclosure moves it as a whole (ADR 0018 E).
                    SemanticGeometry::Hatch(_) => {
                        diagnostics.push(stretch_unchanged_diagnostic(owner, "hatch"));
                        plan.push((owner, StretchAction::Unchanged));
                    }
                    // A kind with no per-vertex edit and no declared move-only
                    // rule fails the whole command (refusal granularity).
                    other => {
                        return Err(CadError::Unsupported(format!(
                            "STRETCH cannot handle {} inside the window",
                            geometry_kind(other)
                        )));
                    }
                },
                Enclosure::Outside => unreachable!("Outside was filtered above"),
            }
        }

        // Build the updates (pure). A fully enclosed entity that cannot be
        // translated fails the whole command (never a partial success).
        let mut before_states: Vec<DbEntity> = Vec::new();
        let mut updates: Vec<DbEntity> = Vec::new();
        for (owner, action) in plan {
            let before = drawing
                .entity(owner)
                .cloned()
                .ok_or(CadError::StaleResult)?;
            match action {
                StretchAction::Move => {
                    let geometry = moved_geometry(&before.geometry, translation).map_err(
                        |error| match error {
                            CadError::Unsupported(reason) => CadError::Unsupported(format!(
                                "STRETCH cannot move {}: {reason}",
                                geometry_kind(&before.geometry)
                            )),
                            other => other,
                        },
                    )?;
                    let mut after = before.clone();
                    after.geometry = geometry;
                    before_states.push(before);
                    updates.push(after);
                }
                StretchAction::Stretch => {
                    match stretched_geometry(&before.geometry, &rects, translation) {
                        Ok(geometry) => {
                            let mut after = before.clone();
                            after.geometry = geometry;
                            before_states.push(before);
                            updates.push(after);
                        }
                        // The window crosses the entity but nothing moved: the
                        // geometry is unchanged plus an explicit diagnostic
                        // (never silent, never a fabricated edit).
                        Err(reason) => {
                            diagnostics.push(stretch_unchanged_diagnostic(owner, reason))
                        }
                    }
                }
                StretchAction::Unchanged => {
                    diagnostics.push(stretch_unchanged_diagnostic(owner, "not_stretchable"));
                }
            }
        }

        if updates.is_empty() {
            // Nothing in the window changed: 0 is a result, reported explicitly,
            // and no transaction/history is recorded.
            diagnostics.push(Diagnostic {
                object: None,
                code: "drawing.stretch".into(),
                message: "STRETCH: nothing changed in the window".into(),
            });
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics,
                measurement: None,
                point_report: None,
            });
        }

        let changed_count = updates.len();
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("stretch entities", transaction)?;
            for after in &updates {
                staged.update_entity(after.clone())?;
            }
            staged.commit()?
        };
        // Read each committed entity back so the undo patch's `after` is the exact
        // stored value, never a re-derivation.
        let patches: Vec<DrawingPatch> = before_states
            .into_iter()
            .map(|before| {
                let after = drawing.entity(before.id).cloned().ok_or_else(|| {
                    CadError::Invariant(format!(
                        "stretched entity {} vanished after commit",
                        before.id.0
                    ))
                })?;
                Ok(drawing_patch(before.id, Some(before), Some(after)))
            })
            .collect::<CadResult<Vec<_>>>()?;
        let changed_ids: Vec<ObjectId> = updates.iter().map(|after| ObjectId(after.id.0)).collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "stretch entities".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        diagnostics.push(Diagnostic {
            object: None,
            code: "drawing.stretch".into(),
            message: format!("STRETCH: {changed_count} entity(ies) changed (crossing window)"),
        });
        Ok(CommandOutcome {
            objects: changed_ids,
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }
}

/// The insertion point of an `INSERT`: the block-space origin mapped through the
/// placement (the translation column). Mapping the origin stays honest for any
/// affine placement; a non-finite placement lies in no window.
fn insertion_point(transform: &Transform3) -> Point3 {
    transform.apply_point(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
}

/// Classify one entity's geometry against the crossing window.
///
/// "Fully enclosed" is two-way (ADR 0016 decision E): a straight `LINE`/polyline
/// needs every vertex inside one window; every other kind (curves can bow out of
/// the vertex hull) needs its conservative world AABB fully inside one window.
/// "Partial" is a hit on the window (the shared crossing hit test) that is not
/// fully enclosed; the per-kind edit is decided independently.
fn classify_enclosure(
    geometry: &SemanticGeometry,
    items: &[&crate::PickItem],
    rects: &[Rect],
) -> Enclosure {
    // "Fully enclosed" must be one **single** window: two disjoint windows each
    // holding one endpoint do not enclose the segment between them.
    let full = match geometry {
        SemanticGeometry::Line { start, end } => rects
            .iter()
            .any(|rect| rect.contains_point(*start) && rect.contains_point(*end)),
        SemanticGeometry::Polyline { points, bulges, .. }
            if !points.is_empty() && bulges.iter().all(|bulge| bulge.abs() <= 1e-12) =>
        {
            rects
                .iter()
                .any(|rect| points.iter().all(|point| rect.contains_point(*point)))
        }
        // CIRCLE/ELLIPSE/INSERT, bulge polylines and every other kind use the
        // conservative bounds fully-inside **one** window path.
        _ => match union_bounds(items) {
            Some((min, max)) => rects.iter().any(|rect| rect.contains_aabb(min, max)),
            // No computable box: never fabricate an inclusion.
            None => return Enclosure::Outside,
        },
    };
    if full {
        return Enclosure::Full;
    }
    let hits = rects.iter().any(|rect| {
        items
            .iter()
            .any(|item| geometry_hits_rect(&item.geometry, &item.transform, *rect))
    });
    if hits {
        Enclosure::Partial
    } else {
        Enclosure::Outside
    }
}

/// The union of the conservative world AABBs of the visible pick items owned by
/// one top-level entity, or `None` when no item contributes a computable box.
fn union_bounds(items: &[&crate::PickItem]) -> Option<(Point3, Point3)> {
    let mut acc: Option<(Point3, Point3)> = None;
    for item in items {
        let Some((min, max)) = pick_world_bounds(&item.geometry, &item.transform) else {
            continue;
        };
        acc = Some(match acc {
            Some((acc_min, acc_max)) => (
                Point3 {
                    x: acc_min.x.min(min.x),
                    y: acc_min.y.min(min.y),
                    z: acc_min.z.min(min.z),
                },
                Point3 {
                    x: acc_max.x.max(max.x),
                    y: acc_max.y.max(max.y),
                    z: acc_max.z.max(max.z),
                },
            ),
            None => (min, max),
        });
    }
    acc
}

/// The whole-entity translation used by `Enclosure::Full` — exactly MOVE's
/// result, through the same `cad-db` `transform_geometry` (a NURBS is affinely
/// invariant), so the two commands cannot disagree.
fn moved_geometry(geometry: &SemanticGeometry, translation: Point3) -> CadResult<SemanticGeometry> {
    transform_geometry(geometry, &Transform3::translation(translation))
}

/// Build the per-vertex stretched geometry for `LINE`, `POLYLINE`, `SPLINE`,
/// `ARC` or `DIMENSION`, moving only what lies inside the window.
///
/// `Err(reason)` means nothing moved and carries the stable diagnostic reason, so
/// an in-window entity is never silent. Bulges are kept as-is (UNVERIFIED by
/// Autodesk; our declared rule); a spline's stored control points move with its
/// `degree`/`knots`/`weights` copied verbatim; the arc helpers below reproduce the
/// tessellator's angle frame term for term and use the grip model's sweep slack.
fn stretched_geometry(
    geometry: &SemanticGeometry,
    rects: &[Rect],
    delta: Point3,
) -> Result<SemanticGeometry, &'static str> {
    let inside = |point: Point3| rects.iter().any(|rect| rect.contains_point(point));
    let shift = |point: Point3| Point3 {
        x: point.x + delta.x,
        y: point.y + delta.y,
        z: point.z + delta.z,
    };
    match geometry {
        SemanticGeometry::Line { start, end } => {
            let new_start = if inside(*start) {
                shift(*start)
            } else {
                *start
            };
            let new_end = if inside(*end) { shift(*end) } else { *end };
            if new_start == *start && new_end == *end {
                return Err("no_vertex");
            }
            Ok(SemanticGeometry::Line {
                start: new_start,
                end: new_end,
            })
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            let new_points: Vec<Point3> = points
                .iter()
                .map(|point| {
                    if inside(*point) {
                        shift(*point)
                    } else {
                        *point
                    }
                })
                .collect();
            if new_points == *points {
                return Err("no_vertex");
            }
            Ok(SemanticGeometry::Polyline {
                points: new_points,
                // UNVERIFIED: the stored bulge values are preserved as-is.
                bulges: bulges.clone(),
                closed: *closed,
            })
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            // A stored definition that is not indexable cannot be edited
            // faithfully: refuse by name instead of staging vectors that no
            // consumer (`cad-geometry::NurbsCurve`, the control-point grips) can
            // evaluate. This is the same relation `cad-db` enforces on write, with
            // the same two empty-vector shorthands (`knots: []` = clamped uniform,
            // `weights: []` = all ones) exempt in their own respect only.
            let expected_knots = control_points.len() + *degree as usize + 1;
            let weights_indexable = weights.is_empty() || weights.len() == control_points.len();
            let knots_indexable = knots.is_empty() || knots.len() == expected_knots;
            if !weights_indexable || !knots_indexable {
                return Err("spline_inconsistent");
            }
            let new_points: Vec<Point3> = control_points
                .iter()
                .map(|point| {
                    if inside(*point) {
                        shift(*point)
                    } else {
                        *point
                    }
                })
                .collect();
            if new_points == *control_points {
                return Err("no_vertex");
            }
            // `degree`, `knots` and `weights` are copied verbatim: the moved
            // points are a different NURBS with the same parameterisation, so the
            // structural invariants above still hold.
            Ok(SemanticGeometry::Spline {
                degree: *degree,
                knots: knots.clone(),
                control_points: new_points,
                weights: weights.clone(),
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let axes = arc_plane_axes(*normal);
            let end_angle = start + sweep;
            let start_point = arc_point(*center, *radius, axes, *start);
            let end_point = arc_point(*center, *radius, axes, end_angle);
            let start_inside = inside(start_point);
            let end_inside = inside(end_point);
            if !start_inside && !end_inside {
                return Err("no_vertex");
            }
            // Each in-window endpoint moves to the in-plane direction of its
            // translated position; the other one keeps its stored angle.
            let mut new_start = *start;
            let mut new_end = end_angle;
            if start_inside {
                new_start =
                    in_plane_angle(*center, axes, shift(start_point)).ok_or("arc_no_direction")?;
            }
            if end_inside {
                new_end =
                    in_plane_angle(*center, axes, shift(end_point)).ok_or("arc_no_direction")?;
            }
            let direction = if *sweep < 0.0 { -1.0 } else { 1.0 };
            let new_sweep = sweep_in_direction(new_start, new_end, direction);
            let extent = new_sweep.abs();
            if !new_sweep.is_finite()
                || extent <= crate::grips::ARC_SWEEP_EPSILON
                || std::f64::consts::TAU - extent <= crate::grips::ARC_SWEEP_EPSILON
            {
                return Err("arc_degenerate_sweep");
            }
            if new_start == *start && new_sweep == *sweep {
                // The in-plane rule left the arc identical: the displacement has
                // no tangential component in the arc's own plane (purely radial or
                // purely out-of-plane), so "the endpoints move" is a no-op here.
                return Err("arc_unchanged");
            }
            Ok(SemanticGeometry::Arc {
                center: *center,
                normal: *normal,
                radius: *radius,
                start: new_start,
                sweep: new_sweep,
            })
        }
        SemanticGeometry::Dimension(dimension) => {
            stretched_dimension(dimension, &inside, &shift).ok_or("dimension_unchanged")
        }
        // The plan only ever routes the four kinds above (plus DIMENSION) into
        // `Stretch`; anything else here would be a planning defect, reported
        // rather than guessed.
        _ => Err("not_stretchable"),
    }
}

/// Stretch a dimension by moving only its **definition points** that lie inside
/// the crossing window.
///
/// Autodesk documents dimensions as stretchable: the extension-line origins and
/// the dimension-line location move with the window, and the measurement follows.
/// Every point slot of [`DimensionPoints`] plus the group-11 text middle point is
/// moved independently; `kind`, `text_rotation` (an explicit rotation is kept),
/// `normal`, the style snapshot and the text override are copied. The display is
/// rebuilt through the domain's single synthesizer ([`DimensionGeometry::synthesized`])
/// so measurement and drawn number stay in step (ADR 0017 A.4 invariant 1) — this
/// command never hand-builds dimension geometry.
///
/// `None` means nothing moved (no point inside the window) or the stretched
/// definition cannot be synthesized (e.g. it collapsed); the caller reports the
/// named `dimension_unchanged` diagnostic and writes nothing for this entity.
/// This is **our declared choice**, not an Autodesk assertion about which
/// individual definition point moves first.
fn stretched_dimension(
    dimension: &DimensionGeometry,
    inside: &impl Fn(Point3) -> bool,
    shift: &impl Fn(Point3) -> Point3,
) -> Option<SemanticGeometry> {
    let source = dimension.definition();
    let moved = |point: Point3| {
        if inside(point) {
            shift(point)
        } else {
            point
        }
    };
    let points = match &source.points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => DimensionPoints::Linear {
            first: moved(*first),
            second: moved(*second),
            dimension_line: moved(*dimension_line),
        },
        DimensionPoints::Radial {
            center,
            point_on_arc,
            leader_length,
        } => DimensionPoints::Radial {
            center: moved(*center),
            point_on_arc: moved(*point_on_arc),
            leader_length: *leader_length,
        },
        DimensionPoints::Angular2Ln {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            arc_point,
        } => DimensionPoints::Angular2Ln {
            line1_start: moved(*line1_start),
            line1_end: moved(*line1_end),
            line2_start: moved(*line2_start),
            line2_end: moved(*line2_end),
            arc_point: moved(*arc_point),
        },
        DimensionPoints::Angular3Pt {
            vertex,
            first,
            second,
            arc_point,
        } => DimensionPoints::Angular3Pt {
            vertex: moved(*vertex),
            first: moved(*first),
            second: moved(*second),
            arc_point: moved(*arc_point),
        },
        DimensionPoints::Ordinate {
            feature,
            leader_end,
            base,
        } => DimensionPoints::Ordinate {
            feature: moved(*feature),
            leader_end: moved(*leader_end),
            base: moved(*base),
        },
    };
    let text_middle = moved(source.text_middle);
    if points == source.points && text_middle == source.text_middle {
        return None;
    }
    let definition = DimensionDefinition {
        kind: source.kind,
        points,
        text_middle,
        text_rotation: source.text_rotation,
        normal: source.normal,
    };
    let rebuilt = DimensionGeometry::synthesized(
        definition,
        dimension.style_name().to_string(),
        dimension.style().clone(),
        dimension.text_override().map(str::to_string),
    )
    .ok()?;
    Some(SemanticGeometry::Dimension(Box::new(rebuilt)))
}

/// The explicit "unchanged" diagnostic for an in-window entity that is not
/// stretched, never a silent drop. `reason` is the stable suffix of
/// `drawing.stretch.<reason>`; see the match arms below for the full set.
fn stretch_unchanged_diagnostic(entity: EntityId, reason: &str) -> Diagnostic {
    let move_only = format!(
        "STRETCH: entity {} crosses the window but cannot be stretched (move-only kind); \
         left unchanged",
        entity.0
    );
    let message = match reason {
        "no_vertex" => format!(
            "STRETCH: entity {} crosses the window but has no vertex inside; left unchanged",
            entity.0
        ),
        "spline_inconsistent" => format!(
            "STRETCH: entity {} crosses the window but its stored spline definition is \
             inconsistent (knot or weight count does not match its control points), so it cannot \
             be edited faithfully; left unchanged",
            entity.0
        ),
        "arc_no_direction" => format!(
            "STRETCH: entity {} crosses the window but an in-window arc endpoint has no in-plane \
             direction from the arc centre (it lies on the centre or on the normal through it), so \
             no endpoint angle exists; left unchanged",
            entity.0
        ),
        "arc_degenerate_sweep" => format!(
            "STRETCH: entity {} crosses the window but moving its in-window endpoint(s) on the same \
             circle in the stored direction would give a sweep of 0 or a full turn, which is not a \
             storable arc; left unchanged",
            entity.0
        ),
        "arc_unchanged" => format!(
            "STRETCH: entity {} crosses the window but its in-window endpoint(s) keep their stored \
             angles on the arc's circle (the displacement has no tangential in-plane component); \
             left unchanged",
            entity.0
        ),
        // Our declared INSERT rule: a block reference only moves rigidly, and only
        // when its insertion point is in the window. The message names the
        // precondition so a partially enclosed reference is never silently dropped.
        "insert_outside" => format!(
            "STRETCH: block reference {} crosses the window but its insertion point (the \
             transformed block-space origin) is outside it, and STRETCH moves a block reference \
             only rigidly by its insertion point (block members are not stretched individually); \
             left unchanged",
            entity.0
        ),
        // Documented AutoCAD circle rule: a circle cannot be stretched, and it
        // only moves as a whole when its centre is in the crossing window.
        "circle_center_outside" => format!(
            "STRETCH: circle {} crosses the window but its centre is outside it; a circle cannot \
             be stretched and only moves (as a whole) when its centre is enclosed, so it is left \
             unchanged",
            entity.0
        ),
        // Our declared TEXT/MTEXT rule (Autodesk documents it for MTEXT): the run
        // moves only by its insertion point.
        "text_outside" => format!(
            "STRETCH: text {} crosses the window but its insertion point is outside it; TEXT and \
             MTEXT move only when their insertion point is enclosed, so it is left unchanged",
            entity.0
        ),
        "dimension_unchanged" => format!(
            "STRETCH: dimension {} crosses the window but no definition point is inside it (or \
             the stretched definition cannot be synthesized); left unchanged",
            entity.0
        ),
        _ => move_only,
    };
    Diagnostic {
        object: Some(ObjectId(entity.0)),
        code: format!("drawing.stretch.{reason}"),
        message,
    }
}

/// The arc's plane frame `(ax, ay)`: the AutoCAD arbitrary-axis frame, i.e. the
/// same axes the tessellator, the representation provider and the arc endpoint
/// grips measure `start`/`sweep` in. Sharing the frame is what makes an angle here
/// denote the point the display draws for the same stored values.
///
/// Duplicating the two-line frame locally (as `select_box`/`trim_boundary` also
/// do) is deliberate: the grip helpers are module-private and STRETCH must not
/// reach into the grip model.
fn arc_plane_axes(normal: Point3) -> (Point3, Point3) {
    let (ax, ay, _) = cad_geometry::arbitrary_axis(normal);
    (ax, ay)
}

/// One point on the arc: `centre + radius·(cos θ·ax + sin θ·ay)`, the
/// tessellator's expression term for term.
fn arc_point(center: Point3, radius: f64, axes: (Point3, Point3), angle: f64) -> Point3 {
    cad_geometry::add(
        center,
        cad_geometry::add(
            cad_geometry::scale(axes.0, angle.cos() * radius),
            cad_geometry::scale(axes.1, angle.sin() * radius),
        ),
    )
}

/// The in-plane angle of `to` as seen from `center`, or `None` when it has no
/// in-plane direction at all (it sits on the centre, on the normal through it, or
/// the projection is non-finite). The out-of-plane offset is discarded by
/// construction, so stretching an arc endpoint can never tilt the arc out of its
/// own plane.
fn in_plane_angle(center: Point3, axes: (Point3, Point3), to: Point3) -> Option<f64> {
    let offset = cad_geometry::sub(to, center);
    let u = cad_geometry::dot(offset, axes.0);
    let v = cad_geometry::dot(offset, axes.1);
    if !u.is_finite() || !v.is_finite() || (u == 0.0 && v == 0.0) {
        return None;
    }
    Some(v.atan2(u))
}

/// Wrap an angle into `(0, 2π]` — the positive-direction extent between two
/// angles, the same normalization the importer's `normalize_sweep` performs.
fn wrap_positive(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut wrapped = angle % tau;
    if wrapped <= 0.0 {
        wrapped += tau;
    }
    wrapped
}

/// The signed sweep from `from` to `to` **in the stored direction**.
///
/// `direction` is `+1.0` for a counter-clockwise (positive-sweep) arc and `-1.0`
/// for a clockwise/reflected one, so a stretch never flips the arc's side. The
/// caller rejects a value of 0 or a full turn.
fn sweep_in_direction(from: f64, to: f64, direction: f64) -> f64 {
    if direction < 0.0 {
        -wrap_positive(from - to)
    } else {
        wrap_positive(to - from)
    }
}

/// Relabel the shared tree-preflight refusal with the STRETCH command context.
fn stretch_tree_refusal(error: CadError) -> CadError {
    match error {
        CadError::Unsupported(_) => CadError::Unsupported(
            "STRETCH cannot enumerate the visible entity tree (opaque geometry, cyclic or \
             depth-limited INSERTs, or missing block members)"
                .into(),
        ),
        other => other,
    }
}

/// The world tolerance shared by the STRETCH degeneracy checks (window area and
/// displacement magnitude).
pub(super) fn stretch_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}
