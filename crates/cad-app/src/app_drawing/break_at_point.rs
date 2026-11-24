//! `BREAKATPOINT`: split one object into two at a single point.
//!
//! Split out of `break_curve.rs` (the code-size limit); it is a private helper
//! module of that file, not a second command surface.
//!
//! ## Supported targets (exact)
//!
//! Every target kind whose break-at-point geometry is representable in this
//! build's stored model:
//!
//! * LINE → two LINEs;
//! * ARC → two ARCs;
//! * **open** LWPOLYLINE → two open polylines (a bulged segment splits into two
//!   arcs with matching bulges);
//! * **closed** LWPOLYLINE → **one** open polyline that starts and ends at the
//!   break point and keeps every segment/bulge of the loop (a single point
//!   cannot leave two non-degenerate pieces of a closed loop);
//! * **open** SPLINE → two exact sub-splines (same degree/weights, both ends
//!   clamped);
//! * CIRCLE → **one** full-turn ARC opened at the break point (`start` is the
//!   projected angle, `sweep` is one full turn) — one point cannot create a gap,
//!   so there is no second piece;
//! * ELLIPSE / elliptical arc → the same two shapes as an ARC: an elliptical
//!   arc keeps two pieces, a full ellipse opens into one full-turn elliptical
//!   arc.
//!
//! A **closed SPLINE** is refused by name (`breakatpoint.closed`): opening it at
//! an arbitrary point would need a rotated knot domain, which this build's NURBS
//! sub-curve planner does not do — refusing is honest, guessing a seam is not.
//! Every other kind (TEXT, IMAGE, HATCH, DIMENSION, MULTILEADER, TABLE, MESH,
//! BODY3D, …) is `breakatpoint.unsupported`, named in the message.
//!
//! ## Refusals (all before the transaction)
//!
//! | code | meaning |
//! | --- | --- |
//! | `breakatpoint.payload` | a payload other than `BreakAtPoint` |
//! | `breakatpoint.invalid_point` | a non-finite break point |
//! | `breakatpoint.not_addressable` | foreign document, block-instance member, sub-element or non-model space |
//! | `breakatpoint.target_missing` | the target ref does not resolve |
//! | `breakatpoint.closed` | a **closed SPLINE** (its seam cannot be rotated) |
//! | `breakatpoint.unsupported` | any other non-curve kind, or a non-`+Z` bulged polyline |
//! | `breakatpoint.projection_failed` | the point has no projection (degenerate target / on the axis) |
//! | `breakatpoint.point_outside_span` | the projection falls outside an open target's span |
//! | `breakatpoint.point_at_endpoint` | the break point projects onto an endpoint of an open target |
//! | `breakatpoint.degenerate_piece` | one side would be empty / has no representable sub-curve |
//!
//! ## The write
//!
//! Everything is resolved as plain geometry before the transaction opens. The
//! source is retained as the **first** piece (same id, layer, space and draw
//! order; geometry and type key change) and, when the result really has two
//! pieces, the second is inserted with a **fresh id** copying the source's
//! display attributes. One transaction / one undo step; undo restores the source
//! and removes the inserted piece.

use super::super::polyline_curve::PolylineCurve;
use super::super::trim_curves::{cut_tolerance, TargetCurve};
use super::super::*;
use cad_db::DbObject;

/// The target kinds this command can split, named in every refusal.
const SUPPORTED: &str = "LINE, ARC, CIRCLE, ELLIPSE, open SPLINE, LWPOLYLINE";

/// A `BREAKATPOINT` refusal.
fn at_point_refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// `BREAKATPOINT`: split one object into two at a single point.
    ///
    /// See the module docs for the exact supported set and every named refusal.
    /// The break point is **projected** onto the target (the host resolved the
    /// entity with its pickbox), so a click a few pixels off the curve is still
    /// valid. One transaction / one undo step; a single-piece result (a closed
    /// loop opened at the point) is reported in the diagnostic.
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

        let pieces = match &before.geometry {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
                ..
            } => {
                let curve = PolylineCurve::new(points, bulges, *closed, "breakatpoint")?;
                if *closed {
                    // A closed loop opened at one point is one open polyline; it
                    // has no second piece because the loop only has one seam.
                    vec![curve.open_at_point(*point, tol)?]
                } else {
                    let (first, second) = curve.split_at_point(*point, tol)?;
                    vec![first, second]
                }
            }
            SemanticGeometry::Spline { closed: true, .. } => {
                return Err(CadError::Unsupported(format!(
                    "breakatpoint.closed: a closed SPLINE cannot be opened at a single point \
                     without rotating its knot domain, which this build's sub-curve planner \
                     does not do; supported targets are {SUPPORTED}"
                )));
            }
            SemanticGeometry::Line { .. }
            | SemanticGeometry::Arc { .. }
            | SemanticGeometry::Circle { .. }
            | SemanticGeometry::Ellipse { .. }
            | SemanticGeometry::Spline { .. } => {
                curve_pieces_at_point(&before.geometry, *point, tol)?
            }
            other => {
                return Err(CadError::Unsupported(format!(
                    "breakatpoint.unsupported: {} is not one of {SUPPORTED}; \
                     BREAKATPOINT changes nothing",
                    class_name(other)
                )));
            }
        };

        let source_id = before.id;
        let source_display = document.drawing.entity_display_state(source_id);
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;
        let new_display = cad_db::EntityDisplayState {
            attributes: source_display.attributes.clone(),
            visibility: Default::default(),
        };
        let mut pieces = pieces.into_iter();
        let first = pieces
            .next()
            .expect("every supported target resolves to at least one piece");
        let second = pieces.next();
        let new_id = second.as_ref().map(|_| drawing.allocate_entity_id());
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction("break at point", transaction)?;
            let mut after = before.clone();
            after.geometry = first;
            after.object.type_key = class_name(&after.geometry).to_string();
            staged.update_entity(after)?;
            if let (Some(second), Some(new_id)) = (second, new_id) {
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
            }
            staged.commit()?
        };

        let stored_first = drawing
            .entity(source_id)
            .cloned()
            .ok_or_else(|| CadError::Invariant("split entity vanished after commit".into()))?;
        let opened_kind = class_name(&stored_first.geometry);
        let mut patches = vec![drawing_patch(source_id, Some(before), Some(stored_first))];
        let mut objects = vec![ObjectId(source_id.0)];
        if let Some(new_id) = new_id {
            let stored_second = drawing.entity(new_id).cloned().ok_or_else(|| {
                CadError::Invariant("inserted split piece vanished after commit".into())
            })?;
            let mut second_patch = drawing_patch(new_id, None, Some(stored_second));
            second_patch.after_state = Some(new_display.clone());
            patches.push(second_patch);
            objects.push(ObjectId(new_id.0));
        }
        let message = if objects.len() == 1 {
            format!(
                "BREAKATPOINT: opened entity {} into one {opened_kind} at the projected point; \
                 a single point cannot leave two non-degenerate pieces of a closed loop",
                source_id.0
            )
        } else {
            format!(
                "BREAKATPOINT: split entity {} into two pieces at the projected point",
                source_id.0
            )
        };
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: "break at point".into(),
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
                object: Some(ObjectId(source_id.0)),
                code: "breakatpoint.applied".into(),
                message,
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve the one or two pieces a LINE/ARC/CIRCLE/ELLIPSE/SPLINE becomes at the
/// projection of `point` (one parameter on the target's own span).
///
/// A periodic target (a full CIRCLE or a full ELLIPSE) has no ends: opening it
/// at one point yields the **one** full-turn arc/elliptical arc starting there.
/// An open target must have the point strictly inside its span and yields two
/// sub-curves.
fn curve_pieces_at_point(
    geometry: &SemanticGeometry,
    point: Point3,
    tol: f64,
) -> CadResult<Vec<SemanticGeometry>> {
    let curve = TargetCurve::new_trim_extend(
        geometry,
        "BREAKATPOINT",
        "LINE, ARC, CIRCLE, ELLIPSE or SPLINE",
    )?;
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
    if curve.is_periodic() {
        // No endpoint rule: every point of a closed loop is a legal seam, so the
        // result is the whole curve opened there (one piece, not two).
        if !u.is_finite() {
            return Err(at_point_refusal(
                "breakatpoint.projection_failed",
                format!(
                    "the point has no finite projection onto the {}",
                    curve.label()
                ),
            ));
        }
        let u = u.rem_euclid(1.0);
        let opened = curve
            .with_span(u, 1.0)
            .map_err(|reason| at_point_refusal("breakatpoint.degenerate_piece", reason))?;
        return Ok(vec![opened]);
    }
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
    Ok(vec![first, second])
}
