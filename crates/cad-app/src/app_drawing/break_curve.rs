//! `BREAK`: remove the span between two projected picks and keep the
//! complementary piece(s).
//!
//! AutoCAD's two-point `BREAK` takes an entity pick (which is also the first
//! break point) and a second break point. This build implements exactly that
//! shape for **LINE**, **ARC**, **CIRCLE** and **SPLINE** targets. The `F`
//! first-point override and multiple-break mode are deliberately **NOT
//! IMPLEMENTED**.
//!
//! The picks are projected onto the target's own parameter space with the same
//! [`TargetCurve`] abstraction TRIM/EXTEND use, so there is one parameter model
//! and one tolerance helper ([`cut_tolerance`]) rather than a second break-only
//! rule:
//!
//! * an open target (LINE/ARC/SPLINE) uses `u ∈ [0, 1]`; both picks must project
//!   inside that span (no clamping from another curve). The pieces `[0, u1]` and
//!   `[u2, 1]` are kept, each only when it is longer than the parameter tolerance;
//! * a periodic target (a full CIRCLE) normalises both parameters into `[0, 1)`
//!   and keeps the **one** complement starting at `u2` and wrapping through the
//!   seam. The stored entity therefore becomes an ARC.
//!
//! ## Refusals (all before the transaction)
//!
//! | code | meaning |
//! | --- | --- |
//! | `break.invalid_point` | `first`/`second` has a NaN/infinite coordinate |
//! | `break.not_addressable` | block-instance member, sub-element, or non-model-space target |
//! | `break.target_missing` | the target ref does not resolve (or is another document's) |
//! | `break.projection_failed` | a pick has no projection at all (degenerate target, pick on the ARC/CIRCLE axis) |
//! | `break.pick_outside_span` | a pick projects outside `[0, 1]` on an open target |
//! | `break.zero_span` | the two parameters are the same (nothing to remove) |
//! | `break.whole_curve` | the span covers the whole curve (the object would vanish) |
//! | `break.degenerate_piece` | `with_span` cannot keep a well-formed piece (e.g. too few SPLINE control points) |
//!
//! A LWPOLYLINE (or any other kind) is the explicit `Unsupported` from
//! [`TargetCurve::new`], whose `supported` name is `LINE, ARC, CIRCLE or SPLINE`.
//! A refusal writes nothing: no revision bump, no `ChangeSet`, no undo entry.
//!
//! ## Transaction shape
//!
//! Everything is computed before the transaction is opened. On success the source
//! entity is **retained as the first piece** (same id, layer, space, draw order
//! and display sidecar; only its geometry and `type_key` change). When a second
//! piece exists it is inserted with a fresh id on the source's layer/space/draw
//! order and the source's display attributes. One transaction / one undo step;
//! undo restores the original geometry and removes the inserted piece.

use super::trim_curves::{cut_tolerance, norm_tau, TargetCurve};
use super::*;
use cad_db::DbObject;
use cad_geometry::{arbitrary_axis, bulge_arc_geometry};

/// A plan-level BREAK refusal: `InvalidInput` carrying its stable code.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// `BREAK` one LINE/ARC/CIRCLE/SPLINE between two picked points.
    ///
    /// See the module docs for the supported subset, the named refusals and the
    /// one-transaction/one-undo-step shape. The source is retained as the first
    /// piece; the kept complement is inserted when it is non-degenerate.
    pub(crate) fn break_curve(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Break {
            target,
            first,
            second,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "BREAK needs a Break { target, first, second } payload".into(),
            ));
        };
        // 1. Finite picks: refused before the drawing is read at all.
        if !is_finite_point(*first) || !is_finite_point(*second) {
            return Err(refusal(
                "break.invalid_point",
                "BREAK break points must be finite",
            ));
        }
        // 2. Directly addressable, model-space entities only.
        if !target.instance.0.is_empty() || target.sub_element.is_some() {
            return Err(refusal(
                "break.not_addressable",
                format!(
                    "BREAK target {} is a block-instance member or sub-element; only \
                     directly addressable model-space entities can be broken",
                    target.entity.0
                ),
            ));
        }
        if target.document != command.document {
            return Err(refusal(
                "break.target_missing",
                "BREAK target belongs to another document",
            ));
        }

        let document_id = command.document;
        let tol = cut_tolerance();
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;

        // 3. Load the target and build the shared parameter model.
        let before = document
            .drawing
            .entity(target.entity)
            .cloned()
            .ok_or_else(|| {
                refusal(
                    "break.target_missing",
                    format!("BREAK target {} does not exist", target.entity.0),
                )
            })?;
        if before.space != SpaceId::Model {
            return Err(refusal(
                "break.not_addressable",
                format!(
                    "BREAK target {} is not a model-space entity; only model space is \
                     supported",
                    before.id.0
                ),
            ));
        }
        let curve = TargetCurve::new(&before.geometry, "BREAK", "LINE, ARC, CIRCLE or SPLINE")?;

        // 4. Project both picks onto the target's own parameter space.
        let Some(u_a) = curve.project_param(*first, tol) else {
            return Err(refusal(
                "break.projection_failed",
                format!(
                    "the first BREAK pick has no projection onto the target {}",
                    curve.label()
                ),
            ));
        };
        let Some(u_b) = curve.project_param(*second, tol) else {
            return Err(refusal(
                "break.projection_failed",
                format!(
                    "the second BREAK pick has no projection onto the target {}",
                    curve.label()
                ),
            ));
        };

        let span_len = curve.length();
        if !span_len.is_finite() || span_len <= tol {
            return Err(refusal(
                "break.projection_failed",
                format!(
                    "the BREAK target {} is degenerate (zero length)",
                    curve.label()
                ),
            ));
        }
        let param_tol = (tol / span_len).max(1e-12);

        // 5./6. Resolve the kept piece(s) as plain geometry before any write.
        let pieces = if curve.is_periodic() {
            periodic_pieces(&curve, u_a, u_b, param_tol)?
        } else {
            open_pieces(&curve, u_a, u_b, param_tol)?
        };
        debug_assert!(!pieces.is_empty(), "BREAK always keeps at least one piece");

        // 7. One transaction: update the source into the first piece, insert the
        // second when present, then read both back for the undo patches.
        let source_id = before.id;
        let source_display = document.drawing.entity_display_state(source_id);
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        // Allocate the fresh id before the transaction borrows the drawing. The
        // id is only consumed when a second piece exists.
        let new_id = pieces.get(1).map(|_| drawing.allocate_entity_id());
        // The new piece copies the source's **display attributes** (colour,
        // linetype, lineweight, opacity); it is a fresh model-space entity, so it
        // carries no dynamic-visibility/block membership.
        let new_display = new_id.map(|_| cad_db::EntityDisplayState {
            attributes: source_display.attributes.clone(),
            visibility: Default::default(),
        });
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("break curve", transaction)?;
            let first_piece = pieces
                .first()
                .expect("a successful BREAK keeps at least one piece");
            let mut after = before.clone();
            after.geometry = first_piece.clone();
            after.object.type_key = class_name(first_piece).to_string();
            staged.update_entity(after)?;
            if let (Some(new_id), Some(geometry)) = (new_id, pieces.get(1)) {
                let entity = DbEntity {
                    object: DbObject {
                        id: ObjectId(new_id.0),
                        type_key: class_name(geometry).to_string(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id: new_id,
                    layer: before.layer,
                    space: before.space.clone(),
                    geometry: geometry.clone(),
                    draw_order: before.draw_order,
                };
                staged.insert_entity(entity)?;
                // Display attributes live beside the entity row; the shared write
                // path applies them after the insert, in this same revision.
                if let Some(display) = &new_display {
                    staged.update_entity_display_state(new_id, display)?;
                }
            }
            staged.commit()?
        };

        // Read the committed rows back: the patches are the exact stored values.
        let stored_first = drawing
            .entity(source_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("broken entity vanished after commit".into()))?;
        let stored_second = match new_id {
            Some(id) => Some(drawing.entity(id).cloned().ok_or_else(|| {
                CadError::Invariant("inserted BREAK piece vanished after commit".into())
            })?),
            None => None,
        };

        let mut patches = vec![drawing_patch(
            source_id,
            Some(before.clone()),
            Some(stored_first),
        )];
        if let (Some(id), Some(stored)) = (new_id, &stored_second) {
            let mut patch = drawing_patch(id, None, Some(stored.clone()));
            // Record the copied display attributes so a redo re-applies them,
            // exactly like the shared create funnel does for a captured display.
            patch.after_state = new_display.clone();
            patches.push(patch);
        }

        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "break curve".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        let kinds: Vec<&str> = pieces.iter().map(class_name).collect();
        let mut objects = vec![ObjectId(source_id.0)];
        if let Some(id) = new_id {
            objects.push(ObjectId(id.0));
        }
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(source_id.0)),
                code: "drawing.break".into(),
                message: format!(
                    "BREAK: source {} kept as {} piece(s) ({})",
                    source_id.0,
                    pieces.len(),
                    kinds.join(", ")
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve the kept piece(s) of an **open** target (LINE/ARC/SPLINE).
fn open_pieces(
    curve: &TargetCurve,
    u_a: f64,
    u_b: f64,
    param_tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    for u in [u_a, u_b] {
        if !u.is_finite() || u < -param_tol || u > 1.0 + param_tol {
            return Err(refusal(
                "break.pick_outside_span",
                format!(
                    "a BREAK pick projects outside the target {} (parameter {u:.9}, the \
                     target spans [0, 1])",
                    curve.label()
                ),
            ));
        }
    }
    // Only a within-tolerance overshoot is folded onto the end; a pick from
    // another curve is refused above, never silently clamped.
    let u1 = u_a.clamp(0.0, 1.0).min(u_b.clamp(0.0, 1.0));
    let u2 = u_a.clamp(0.0, 1.0).max(u_b.clamp(0.0, 1.0));
    if u2 - u1 <= param_tol {
        return Err(refusal(
            "break.zero_span",
            "the two BREAK picks project to the same point; there is no span to remove",
        ));
    }
    if u1 <= param_tol && u2 >= 1.0 - param_tol {
        return Err(refusal(
            "break.whole_curve",
            "the two BREAK picks span the whole target; BREAK does not delete the object \
             (use ERASE)",
        ));
    }

    let mut pieces = Vec::with_capacity(2);
    if u1 > param_tol {
        pieces.push(
            curve
                .with_span(0.0, u1)
                .map_err(|reason| refusal("break.degenerate_piece", reason))?,
        );
    }
    if u2 < 1.0 - param_tol {
        pieces.push(
            curve
                .with_span(u2, 1.0 - u2)
                .map_err(|reason| refusal("break.degenerate_piece", reason))?,
        );
    }
    Ok(pieces)
}

/// Resolve the kept complement of a **periodic** target (a full CIRCLE).
///
/// Both projected parameters are wrapped into `[0, 1)`, ordered, and the removed
/// span is `[u1, u2]`; the kept piece starts at `u2` and wraps through the seam
/// with fraction `1 - (u2 - u1)`. The result is an ARC, which is what the caller
/// stores in the original CIRCLE entity.
fn periodic_pieces(
    curve: &TargetCurve,
    u_a: f64,
    u_b: f64,
    param_tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    if !u_a.is_finite() || !u_b.is_finite() {
        return Err(refusal(
            "break.projection_failed",
            "a BREAK pick projected to a non-finite parameter",
        ));
    }
    // `rem_euclid` folds both the seam and any wrap (a negative-sweep full turn)
    // into the target's own single period.
    let u_a = u_a.rem_euclid(1.0);
    let u_b = u_b.rem_euclid(1.0);
    let (u1, u2) = if u_a <= u_b { (u_a, u_b) } else { (u_b, u_a) };
    let removed = u2 - u1;
    if removed <= param_tol {
        return Err(refusal(
            "break.zero_span",
            "the two BREAK picks project to the same point; there is no span to remove",
        ));
    }
    let fraction = 1.0 - removed;
    if fraction <= param_tol {
        return Err(refusal(
            "break.whole_curve",
            "the two BREAK picks span the whole target; BREAK does not delete the object \
             (use ERASE)",
        ));
    }
    let geometry = curve
        .with_span(u2, fraction)
        .map_err(|reason| refusal("break.degenerate_piece", reason))?;
    Ok(vec![geometry])
}

/// A `BREAKATPOINT` refusal.
fn at_point_refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// `BREAKATPOINT`: split one object into two at a single point.
    ///
    /// Supported targets are LINE, ARC and an **open** LWPOLYLINE. AutoCAD's
    /// BREAKATPOINT refuses closed objects, and so does this build: a CIRCLE or a
    /// closed polyline is `breakatpoint.closed`; a SPLINE is
    /// `breakatpoint.spline_unsupported`; every other kind is
    /// `breakatpoint.unsupported`. The break point is **projected** onto the
    /// target (the host resolved the entity with its pickbox), so a click a few
    /// pixels off the curve is still valid.
    ///
    /// The source entity is retained as the first piece (same id/layer/space/
    /// draw order/display state); the second piece is a fresh-id entity copying
    /// the source's display attributes. One transaction / one undo step.
    pub(crate) fn break_at_point(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::BreakAtPoint { target, point } = &command.payload else {
            return Err(at_point_refusal(
                "breakatpoint.payload",
                "BREAKATPOINT needs a BreakAtPoint { target, point } payload",
            ));
        };
        if !is_finite_point(*point) {
            return Err(at_point_refusal(
                "breakatpoint.invalid_point",
                "the break point must be finite",
            ));
        }
        if !target.instance.0.is_empty() || target.sub_element.is_some() {
            return Err(at_point_refusal(
                "breakatpoint.not_addressable",
                format!(
                    "target {} is a block-instance member or sub-element; only directly \
                     addressable model-space entities can be split",
                    target.entity.0
                ),
            ));
        }
        if target.document != command.document {
            return Err(at_point_refusal(
                "breakatpoint.target_missing",
                "the target belongs to another document",
            ));
        }

        let document_id = command.document;
        let tol = cut_tolerance();
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let before = document
            .drawing
            .entity(target.entity)
            .cloned()
            .ok_or_else(|| {
                at_point_refusal(
                    "breakatpoint.target_missing",
                    format!("target {} does not exist", target.entity.0),
                )
            })?;
        if before.space != SpaceId::Model {
            return Err(at_point_refusal(
                "breakatpoint.not_addressable",
                format!(
                    "target {} is not a model-space entity; only model space is supported",
                    before.id.0
                ),
            ));
        }

        let (first, second) = match &before.geometry {
            SemanticGeometry::Circle { .. } => {
                return Err(at_point_refusal(
                    "breakatpoint.closed",
                    "BREAKATPOINT does not split a closed object such as a CIRCLE",
                ));
            }
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => {
                if *closed {
                    return Err(at_point_refusal(
                        "breakatpoint.closed",
                        "BREAKATPOINT does not split a closed polyline",
                    ));
                }
                polyline_split_at_point(points, bulges, *point, tol)?
            }
            SemanticGeometry::Spline { .. } => {
                return Err(CadError::Unsupported(
                    "breakatpoint.spline_unsupported: BREAKATPOINT does not split a SPLINE; \
                     only LINE, ARC and an open LWPOLYLINE are supported"
                        .into(),
                ));
            }
            SemanticGeometry::Line { .. } | SemanticGeometry::Arc { .. } => {
                curve_split_at_point(&before.geometry, *point, tol)?
            }
            other => {
                return Err(CadError::Unsupported(format!(
                    "breakatpoint.unsupported: {} is not LINE, ARC or an open LWPOLYLINE; \
                     BREAKATPOINT changes nothing",
                    class_name(other)
                )));
            }
        };

        let source_id = before.id;
        let source_display = document.drawing.entity_display_state(source_id);
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let new_id = drawing.allocate_entity_id();
        let new_display = cad_db::EntityDisplayState {
            attributes: source_display.attributes.clone(),
            visibility: Default::default(),
        };
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("break at point", transaction)?;
            let mut after = before.clone();
            after.geometry = first;
            after.object.type_key = class_name(&after.geometry).to_string();
            staged.update_entity(after)?;
            let entity = DbEntity {
                object: DbObject {
                    id: ObjectId(new_id.0),
                    type_key: class_name(&second).to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: new_id,
                layer: before.layer,
                space: before.space.clone(),
                geometry: second,
                draw_order: before.draw_order,
            };
            staged.insert_entity(entity)?;
            staged.update_entity_display_state(new_id, &new_display)?;
            staged.commit()?
        };

        let stored_first = drawing
            .entity(source_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("split entity vanished after commit".into()))?;
        let stored_second = drawing.entity(new_id).cloned().ok_or_else(|| {
            CadError::Invariant("inserted split piece vanished after commit".into())
        })?;
        let mut second_patch = drawing_patch(new_id, None, Some(stored_second));
        second_patch.after_state = Some(new_display);
        let patches = vec![
            drawing_patch(source_id, Some(before), Some(stored_first)),
            second_patch,
        ];
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "break at point".into(),
                patches,
                block_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        Ok(CommandOutcome {
            objects: vec![ObjectId(source_id.0), ObjectId(new_id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(source_id.0)),
                code: "breakatpoint.applied".into(),
                message: format!(
                    "BREAKATPOINT: split entity {} into two pieces at the projected point",
                    source_id.0
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Split a LINE or ARC at the projection of `point` (one parameter on the
/// target's own span).
fn curve_split_at_point(
    geometry: &SemanticGeometry,
    point: Point3,
    tol: f64,
) -> CadResult<(SemanticGeometry, SemanticGeometry)> {
    let curve = TargetCurve::new(geometry, "BREAKATPOINT", "LINE, ARC or open LWPOLYLINE")?;
    let Some(u) = curve.project_param(point, tol) else {
        return Err(at_point_refusal(
            "breakatpoint.projection_failed",
            format!("the point has no projection onto the {}", curve.label()),
        ));
    };
    let span = curve.length();
    if !span.is_finite() || span <= tol {
        return Err(at_point_refusal(
            "breakatpoint.projection_failed",
            format!("the {} is degenerate (zero length)", curve.label()),
        ));
    }
    let param_tol = (tol / span).max(1e-12);
    if !u.is_finite() || u < -param_tol || u > 1.0 + param_tol {
        return Err(at_point_refusal(
            "breakatpoint.point_outside_span",
            format!(
                "the point projects outside the {} (parameter {u:.9}, the target spans [0, 1])",
                curve.label()
            ),
        ));
    }
    let u = u.clamp(0.0, 1.0);
    if u <= param_tol || u >= 1.0 - param_tol {
        return Err(at_point_refusal(
            "breakatpoint.point_at_endpoint",
            format!(
                "the break point coincides with an endpoint of the {}; there is no interior \
                 split to make",
                curve.label()
            ),
        ));
    }
    let first = curve
        .with_span(0.0, u)
        .map_err(|reason| at_point_refusal("breakatpoint.degenerate_piece", reason))?;
    let second = curve
        .with_span(u, 1.0 - u)
        .map_err(|reason| at_point_refusal("breakatpoint.degenerate_piece", reason))?;
    Ok((first, second))
}

/// The nearest point of one polyline segment and its parameter `t ∈ [0, 1]`.
///
/// A straight segment projects in 3D; a bulged one is resolved through
/// [`bulge_arc_geometry`] on the `+Z` plane and projected radially.
fn project_on_segment(
    a: Point3,
    b: Point3,
    bulge: f64,
    point: Point3,
    tol: f64,
) -> CadResult<(f64, f64)> {
    if bulge.abs() <= 1e-9 {
        let d = cad_geometry::sub(b, a);
        let len2 = cad_geometry::dot(d, d);
        if !len2.is_finite() || len2 <= 0.0 {
            return Ok((0.0, f64::INFINITY));
        }
        let t = (cad_geometry::dot(cad_geometry::sub(point, a), d) / len2).clamp(0.0, 1.0);
        let closest = cad_geometry::add(a, cad_geometry::scale(d, t));
        return Ok((t, cad_geometry::distance(closest, point)));
    }
    let Some(SemanticGeometry::Arc {
        center,
        normal,
        radius,
        start,
        sweep,
    }) = bulge_arc_geometry(
        a,
        b,
        bulge,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
    )
    else {
        return Err(at_point_refusal(
            "breakatpoint.degenerate_piece",
            "a bulged polyline segment could not be resolved as a circular arc on the +Z plane",
        ));
    };
    if !radius.is_finite() || radius <= tol || sweep.abs() <= 1e-12 {
        return Err(at_point_refusal(
            "breakatpoint.degenerate_piece",
            "a bulged polyline segment is degenerate",
        ));
    }
    let (ax, ay, _) = arbitrary_axis(normal);
    let v = cad_geometry::sub(point, center);
    let (rx, ry) = (cad_geometry::dot(v, ax), cad_geometry::dot(v, ay));
    if rx.hypot(ry) <= tol {
        return Err(at_point_refusal(
            "breakatpoint.projection_failed",
            "the point lies on the arc's axis and has no radial projection",
        ));
    }
    let angle = ry.atan2(rx);
    let raw = if sweep >= 0.0 {
        norm_tau(angle - start)
    } else {
        -norm_tau(start - angle)
    };
    let t = (raw / sweep).clamp(0.0, 1.0);
    let closest = cad_geometry::add(
        center,
        cad_geometry::add(
            cad_geometry::scale(ax, radius * (start + sweep * t).cos()),
            cad_geometry::scale(ay, radius * (start + sweep * t).sin()),
        ),
    );
    Ok((t, cad_geometry::distance(closest, point)))
}

/// Split an **open** LWPOLYLINE at the projection of `point`, at a vertex or on
/// a segment (with bulge splitting). Emits two open polylines with one bulge per
/// vertex (the trailing entry of an open polyline is unused and zero).
fn polyline_split_at_point(
    points: &[Point3],
    bulges: &[f64],
    point: Point3,
    tol: f64,
) -> CadResult<(SemanticGeometry, SemanticGeometry)> {
    let n = points.len();
    if n < 2 {
        return Err(at_point_refusal(
            "breakatpoint.degenerate_piece",
            format!("the polyline has {n} vertices; at least two are needed"),
        ));
    }
    if bulges.len() != n && bulges.len() != n.saturating_sub(1) {
        return Err(at_point_refusal(
            "breakatpoint.unsupported",
            format!(
                "the polyline has {n} vertices but {} bulges; one bulge per vertex (or per \
                 segment) is required",
                bulges.len()
            ),
        ));
    }
    let has_bulge = (0..n - 1).any(|i| bulges.get(i).copied().unwrap_or(0.0).abs() > 1e-9);
    if has_bulge && !cad_geometry::is_xy_parallel(points) {
        return Err(CadError::Unsupported(
            "breakatpoint.unsupported: a bulged polyline must be +Z planar to split its arc \
             segments"
                .into(),
        ));
    }
    let bulge_at = |i: usize| bulges.get(i).copied().unwrap_or(0.0);

    // Nearest segment, then decide vertex vs interior by the segment parameter.
    let mut best: Option<(usize, f64, f64)> = None;
    for i in 0..n - 1 {
        let (t, dist) = project_on_segment(points[i], points[i + 1], bulge_at(i), point, tol)?;
        if best.is_none_or(|(_, _, best_dist)| dist < best_dist) {
            best = Some((i, t, dist));
        }
    }
    let Some((segment, t, _)) = best else {
        return Err(at_point_refusal(
            "breakatpoint.projection_failed",
            "the point has no projection onto the polyline",
        ));
    };
    let segment_len = cad_geometry::distance(points[segment], points[segment + 1]);
    if !segment_len.is_finite() || segment_len <= tol {
        return Err(at_point_refusal(
            "breakatpoint.degenerate_piece",
            "the polyline has a zero-length segment at the break point",
        ));
    }
    let param_tol = (tol / segment_len).max(1e-12);

    if t <= param_tol {
        return split_at_vertex(points, n, segment, &bulge_at);
    }
    if t >= 1.0 - param_tol {
        return split_at_vertex(points, n, segment + 1, &bulge_at);
    }

    // Interior of the segment: split the (possibly bulged) segment in two.
    let a = points[segment];
    let b = points[segment + 1];
    let bulge = bulge_at(segment);
    let (break_point, first_bulge, second_bulge) = if bulge.abs() <= 1e-9 {
        let d = cad_geometry::sub(b, a);
        (cad_geometry::add(a, cad_geometry::scale(d, t)), 0.0, 0.0)
    } else {
        let Some(SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        }) = bulge_arc_geometry(
            a,
            b,
            bulge,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        )
        else {
            return Err(at_point_refusal(
                "breakatpoint.degenerate_piece",
                "the bulged segment could not be resolved as a circular arc",
            ));
        };
        let (ax, ay, _) = arbitrary_axis(normal);
        let angle = start + sweep * t;
        let at = cad_geometry::add(
            center,
            cad_geometry::add(
                cad_geometry::scale(ax, radius * angle.cos()),
                cad_geometry::scale(ay, radius * angle.sin()),
            ),
        );
        let theta = 4.0 * bulge.atan();
        let Some(first) = cad_geometry::arc_sweep_to_bulge(theta * t) else {
            return Err(at_point_refusal(
                "breakatpoint.degenerate_piece",
                "the first arc piece has no representable bulge",
            ));
        };
        let Some(second) = cad_geometry::arc_sweep_to_bulge(theta * (1.0 - t)) else {
            return Err(at_point_refusal(
                "breakatpoint.degenerate_piece",
                "the second arc piece has no representable bulge",
            ));
        };
        (at, first, second)
    };

    // First piece: original vertices 0..=segment, then the break point.
    let mut first_points: Vec<Point3> = points[..=segment].to_vec();
    first_points.push(break_point);
    let mut first_bulges: Vec<f64> = (0..segment).map(bulge_at).collect();
    first_bulges.push(first_bulge);
    first_bulges.push(0.0);

    // Second piece: the break point, then original vertices segment+1..n.
    let mut second_points: Vec<Point3> = vec![break_point];
    second_points.extend_from_slice(&points[segment + 1..]);
    let mut second_bulges: Vec<f64> = vec![second_bulge];
    second_bulges.extend((segment + 1..n - 1).map(bulge_at));
    second_bulges.push(0.0);

    Ok((
        SemanticGeometry::Polyline {
            points: first_points,
            bulges: first_bulges,
            closed: false,
        },
        SemanticGeometry::Polyline {
            points: second_points,
            bulges: second_bulges,
            closed: false,
        },
    ))
}

/// Split an open polyline at interior vertex `k`, emitting two open polylines.
fn split_at_vertex(
    points: &[Point3],
    n: usize,
    k: usize,
    bulge_at: &impl Fn(usize) -> f64,
) -> CadResult<(SemanticGeometry, SemanticGeometry)> {
    if k == 0 || k + 1 >= n {
        return Err(at_point_refusal(
            "breakatpoint.point_at_endpoint",
            "the break point coincides with an endpoint of the polyline; there is no interior \
             split to make",
        ));
    }
    let first_points: Vec<Point3> = points[..=k].to_vec();
    let mut first_bulges: Vec<f64> = (0..k).map(bulge_at).collect();
    first_bulges.push(0.0);

    let second_points: Vec<Point3> = points[k..].to_vec();
    let mut second_bulges: Vec<f64> = (k..n - 1).map(bulge_at).collect();
    second_bulges.push(0.0);

    Ok((
        SemanticGeometry::Polyline {
            points: first_points,
            bulges: first_bulges,
            closed: false,
        },
        SemanticGeometry::Polyline {
            points: second_points,
            bulges: second_bulges,
            closed: false,
        },
    ))
}
