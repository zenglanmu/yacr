//! The per-kind drag rules behind [`apply_grip`] — the write half of the grip
//! model.
//!
//! Everything here is pure: the rules take one entity's **current** geometry
//! plus a [`GripId`] and the drag target, and return the geometry the entity must
//! be updated to (or one of the stable `grip.*` refusals). No transaction, no
//! history and no marker is involved; the `GRIPSTRETCH` command
//! (`crate::app_drawing::grip`) is what stages the result, exactly once.
//!
//! Split out of `grips/mod.rs` so both files stay well under the 1000-line cap.
//! The handle lists, the hit-testing rules and the module-level documentation
//! (including every refusal code) live in the parent module.

use super::{
    arc_point, ellipse_axes, ellipse_quadrant_point, in_plane_distance, is_finite, is_full_ellipse,
    midpoint, plane_axes, quadrant_point, spline_is_consistent, GripId, GripKind,
    ARC_SWEEP_EPSILON, QUADRANT_DIRECTIONS, REASON_DEGENERATE_ARC, REASON_DEGENERATE_RADIUS,
    REASON_NON_FINITE_TARGET, REASON_PARTIAL_ELLIPSE, REASON_STALE_HANDLE, REASON_UNSUPPORTED_KIND,
    REASON_ZERO_DISPLACEMENT,
};
use cad_domain::{CadError, CadResult, Point3, SemanticGeometry};
use cad_geometry::{add, dot, length, scale, sub};

/// The in-plane **angle** of `to` as seen from `center`, or `None` when the
/// target has no in-plane direction at all (it sits on the centre, or on the
/// normal through it, or the projection is non-finite).
///
/// This is the arc-endpoint rule: the out-of-plane offset is discarded by
/// construction (`atan2` of the two in-plane components), so dragging an endpoint
/// can never tilt the arc out of its own plane.
fn in_plane_angle(center: Point3, axes: (Point3, Point3), to: Point3) -> Option<f64> {
    let offset = sub(to, center);
    let u = dot(offset, axes.0);
    let v = dot(offset, axes.1);
    if !u.is_finite() || !v.is_finite() || (u == 0.0 && v == 0.0) {
        return None;
    }
    Some(v.atan2(u))
}

/// Wrap an angle into `(0, 2π]` — the positive-direction extent between two
/// angles (the same normalization the importer's `normalize_sweep` performs).
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
/// for a clockwise / reflected one, so a drag never flips the arc's side. The
/// magnitude is the in-plane extent; it is the caller's job to reject a value
/// outside `(0, 2π)`.
fn sweep_in_direction(from: f64, to: f64, direction: f64) -> f64 {
    if direction < 0.0 {
        -wrap_positive(from - to)
    } else {
        wrap_positive(to - from)
    }
}

/// The `LINE` drag rules (two endpoints, then the translating midpoint).
fn apply_line(start: Point3, end: Point3, id: &GripId, to: Point3) -> CadResult<SemanticGeometry> {
    match (id.kind, id.index) {
        (GripKind::Endpoint, 0) => {
            finite_target(to)?;
            if to == start {
                return Err(zero_displacement("AcDbLine start endpoint"));
            }
            Ok(SemanticGeometry::Line { start: to, end })
        }
        (GripKind::Endpoint, 1) => {
            finite_target(to)?;
            if to == end {
                return Err(zero_displacement("AcDbLine end endpoint"));
            }
            Ok(SemanticGeometry::Line { start, end: to })
        }
        (GripKind::Endpoint, index) => Err(stale_handle(format!(
            "AcDbLine has no Endpoint grip at index {index} (0 = start, 1 = end)"
        ))),
        (GripKind::Midpoint, 2) => {
            finite_target(to)?;
            let from = midpoint(start, end);
            if to == from {
                return Err(zero_displacement("AcDbLine midpoint"));
            }
            // One delta for both endpoints: the line translates, so it stays
            // parallel to itself and keeps its length.
            let delta = sub(to, from);
            let new_start = add(start, delta);
            let new_end = add(end, delta);
            if !is_finite(new_start) || !is_finite(new_end) {
                // Finite inputs can still overflow to infinity; storing that
                // would corrupt the entity, so it is refused like any other
                // unstorable coordinate.
                return Err(overflowed("AcDbLine midpoint"));
            }
            Ok(SemanticGeometry::Line {
                start: new_start,
                end: new_end,
            })
        }
        (GripKind::Midpoint, index) => Err(stale_handle(format!(
            "AcDbLine has no Midpoint grip at index {index} (only 2)"
        ))),
        (
            GripKind::Center
            | GripKind::Quadrant
            | GripKind::Vertex
            | GripKind::ControlPoint
            | GripKind::Point,
            _,
        ) => Err(stale_handle(
            "AcDbLine exposes Endpoint (0 = start, 1 = end) and Midpoint (2) grips only"
                .to_string(),
        )),
    }
}

/// The `CIRCLE` drag rules (centre, then the four in-plane quadrants).
fn apply_circle(
    center: Point3,
    normal: Point3,
    radius: f64,
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    match (id.kind, id.index) {
        (GripKind::Center, 0) => {
            finite_target(to)?;
            if to == center {
                return Err(zero_displacement("AcDbCircle centre"));
            }
            Ok(SemanticGeometry::Circle {
                center: to,
                normal,
                radius,
            })
        }
        (GripKind::Center, index) => Err(stale_handle(format!(
            "AcDbCircle has no Center grip at index {index} (only 0)"
        ))),
        (GripKind::Quadrant, index @ 0..=3) => {
            finite_target(to)?;
            let axes = plane_axes(normal);
            let direction = QUADRANT_DIRECTIONS[usize::from(index)];
            if to == quadrant_point(center, radius, axes, direction) {
                return Err(zero_displacement("AcDbCircle quadrant"));
            }
            let new_radius = in_plane_distance(center, axes, to);
            if !new_radius.is_finite() || new_radius <= 0.0 {
                return Err(degenerate_radius(new_radius));
            }
            if new_radius == radius {
                // A target off the plane at exactly the stored radius has no
                // geometric effect (the radius rule ignores the out-of-plane
                // offset), so this is a zero-displacement refusal too: never an
                // `Ok` that would stage an unchanged entity.
                return Err(unchanged_radius(new_radius));
            }
            Ok(SemanticGeometry::Circle {
                center,
                normal,
                radius: new_radius,
            })
        }
        (GripKind::Quadrant, index) => Err(stale_handle(format!(
            "AcDbCircle has no Quadrant grip at index {index} \
             (0 = +ax, 1 = +ay, 2 = -ax, 3 = -ay)"
        ))),
        (
            GripKind::Endpoint
            | GripKind::Midpoint
            | GripKind::Vertex
            | GripKind::ControlPoint
            | GripKind::Point,
            _,
        ) => Err(stale_handle(
            "AcDbCircle exposes a Center grip (0) and Quadrant grips (0..=3) only".to_string(),
        )),
    }
}

/// The `ARC` drag rules: the rigid centre, then the two endpoints.
///
/// The endpoint rule (our choice, not an AutoCAD claim): the arc stays on the
/// same circle — `center`, `radius` and `normal` are returned bit-identically —
/// the dragged endpoint's angle becomes the in-plane direction of `to` from the
/// centre, the opposite endpoint keeps its angle (`start + sweep` for a start
/// drag, `start` for an end drag) and the sweep is recomputed **in the stored
/// direction**. A recomputed sweep outside the open interval `(0, 2π)` is refused
/// by name, never clamped.
fn apply_arc(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    match (id.kind, id.index) {
        (GripKind::Center, 0) => {
            finite_target(to)?;
            if to == center {
                return Err(zero_displacement("AcDbArc centre"));
            }
            Ok(SemanticGeometry::Arc {
                center: to,
                normal,
                radius,
                start,
                sweep,
            })
        }
        (GripKind::Center, index) => Err(stale_handle(format!(
            "AcDbArc has no Center grip at index {index} (only 0)"
        ))),
        (GripKind::Endpoint, index @ 0..=1) => {
            finite_target(to)?;
            let axes = plane_axes(normal);
            let is_start = index == 0;
            let end_angle = start + sweep;
            // The handle's own angle and the angle the drag must keep.
            let (handle_angle, anchored_angle) = if is_start {
                (start, end_angle)
            } else {
                (end_angle, start)
            };
            let handle = if is_start {
                "AcDbArc start endpoint"
            } else {
                "AcDbArc end endpoint"
            };
            if to == arc_point(center, radius, axes, handle_angle) {
                return Err(zero_displacement(handle));
            }
            let Some(angle) = in_plane_angle(center, axes, to) else {
                // A target on the centre (or on the normal through it) has no
                // in-plane direction, so there is no endpoint angle to store.
                return Err(degenerate_arc(handle, sweep));
            };
            // `sweep < 0.0` is the clockwise / reflected arc: the recomputation
            // must stay on the same side of the circle.
            let direction = if sweep < 0.0 { -1.0 } else { 1.0 };
            let (new_start, new_sweep) = if is_start {
                (angle, sweep_in_direction(angle, anchored_angle, direction))
            } else {
                (start, sweep_in_direction(anchored_angle, angle, direction))
            };
            if new_start == start && new_sweep == sweep {
                // An off-plane target along the same in-plane ray: the angle rule
                // ignores the out-of-plane part, so nothing changes.
                return Err(zero_displacement(handle));
            }
            let extent = new_sweep.abs();
            if !new_sweep.is_finite()
                || extent <= ARC_SWEEP_EPSILON
                || std::f64::consts::TAU - extent <= ARC_SWEEP_EPSILON
            {
                return Err(degenerate_arc(handle, new_sweep));
            }
            Ok(SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start: new_start,
                sweep: new_sweep,
            })
        }
        (GripKind::Endpoint, index) => Err(stale_handle(format!(
            "AcDbArc has no Endpoint grip at index {index} (0 = start point, 1 = end point)"
        ))),
        (GripKind::Quadrant, _) => Err(stale_handle(
            "AcDbArc exposes a Center grip (0) and Endpoint grips (0 = start, 1 = end), \
             not a Quadrant grip"
                .to_string(),
        )),
        (GripKind::Midpoint | GripKind::Vertex | GripKind::ControlPoint | GripKind::Point, _) => {
            Err(stale_handle(
                "AcDbArc exposes a Center grip (0) and Endpoint grips (0 = start, 1 = end) only"
                    .to_string(),
            ))
        }
    }
}

/// The `ELLIPSE` drag rules: the rigid centre, then the four axis quadrants of a
/// **full** ellipse.
///
/// The quadrant rule (our choice, **not an AutoCAD claim**) sets the radius that
/// the handle names from the target's **in-plane** distance to the centre: a
/// major handle (`0`/`2`) sets `|major_axis|` while keeping the stored **minor**
/// radius, so `ratio` is recomputed from the two stored values; a minor handle
/// (`1`/`3`) sets `ratio = distance / |major_axis|` and leaves the major vector
/// bit-identical. The **axis direction never rotates**: a major drag scales the
/// stored vector, so the ellipse cannot be turned by a target off the axis and
/// the two axes stay orthogonal by construction. A **partial** ellipse (a stored
/// parameter range that is not a full turn) exposes no quadrant handle at all and
/// answers a quadrant id with [`REASON_PARTIAL_ELLIPSE`].
///
/// The stored fields arrive as a tuple so the rule stays a three-argument
/// function (clippy's `too_many_arguments`; no wrapper struct is introduced for
/// what the domain already models).
type EllipseParts = (Point3, Point3, Point3, f64, f64, f64);

fn apply_ellipse(ellipse: EllipseParts, id: &GripId, to: Point3) -> CadResult<SemanticGeometry> {
    let (center, normal, major_axis, ratio, start, sweep) = ellipse;
    match (id.kind, id.index) {
        (GripKind::Center, 0) => {
            finite_target(to)?;
            if to == center {
                return Err(zero_displacement("AcDbEllipse centre"));
            }
            Ok(SemanticGeometry::Ellipse {
                center: to,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            })
        }
        (GripKind::Center, index) => Err(stale_handle(format!(
            "AcDbEllipse has no Center grip at index {index} (only 0)"
        ))),
        (GripKind::Quadrant, index @ 0..=3) => {
            // Handle identity first: a partial ellipse has no quadrant handle, so
            // this is not a "stale" id and not a target problem.
            if !is_full_ellipse(sweep) {
                return Err(partial_ellipse(index));
            }
            finite_target(to)?;
            let Some(axes) = ellipse_axes(normal, major_axis) else {
                return Err(degenerate_ellipse_radius(
                    "the stored plane frame is degenerate (major axis or normal)",
                ));
            };
            let direction = QUADRANT_DIRECTIONS[usize::from(index)];
            if to == ellipse_quadrant_point(center, major_axis, ratio, axes, direction) {
                return Err(zero_displacement("AcDbEllipse axis quadrant"));
            }
            let distance = in_plane_distance(center, axes, to);
            if !distance.is_finite() || distance <= 0.0 {
                return Err(degenerate_ellipse_radius(
                    "the drag target has no in-plane distance from the centre",
                ));
            }
            let major_len = length(major_axis);
            // The stored minor semi-axis (`|major_axis| * |ratio|`, the expression
            // the tessellator draws with) and the radius this handle names: the
            // major semi-axis for a major handle, the minor one otherwise.
            let minor_len = major_len * ratio.abs();
            let stored_len = if direction.0 == 0 {
                major_len
            } else {
                minor_len
            };
            if distance == stored_len {
                // The radius rule ignores where inside the plane the target sits,
                // and an ellipse axis never rotates, so a target at the stored
                // radius changes nothing — exactly like the circle rule. Refusing
                // also keeps a 1-ulp `distance / major_len` from being written as
                // a "change" that draws the same curve.
                return Err(unchanged_ellipse_radius(distance));
            }
            if direction.0 == 0 {
                // Major handle: the major radius is the measured distance, the
                // stored minor radius is kept, and the direction is the stored
                // one (the vector is scaled, never re-derived from the target).
                let new_ratio = minor_len / distance;
                if !new_ratio.is_finite() || new_ratio <= 0.0 {
                    return Err(degenerate_ellipse_radius("the recomputed ratio is invalid"));
                }
                Ok(SemanticGeometry::Ellipse {
                    center,
                    normal,
                    major_axis: scale(major_axis, distance / major_len),
                    ratio: new_ratio,
                    start,
                    sweep,
                })
            } else {
                // Minor handle: `ratio = minor / major`; the stored major vector is
                // bit-identical.
                let new_ratio = distance / major_len;
                if !new_ratio.is_finite() || new_ratio <= 0.0 {
                    return Err(degenerate_ellipse_radius("the recomputed ratio is invalid"));
                }
                if new_ratio == ratio {
                    return Err(unchanged_ellipse_radius(distance));
                }
                Ok(SemanticGeometry::Ellipse {
                    center,
                    normal,
                    major_axis,
                    ratio: new_ratio,
                    start,
                    sweep,
                })
            }
        }
        (GripKind::Quadrant, index) => Err(stale_handle(format!(
            "AcDbEllipse has no Quadrant grip at index {index} \
             (0 = +major, 1 = +minor, 2 = -major, 3 = -minor)"
        ))),
        (
            GripKind::Endpoint
            | GripKind::Midpoint
            | GripKind::Vertex
            | GripKind::ControlPoint
            | GripKind::Point,
            _,
        ) => Err(stale_handle(
            "AcDbEllipse exposes a Center grip (0) and, for a full ellipse, Quadrant grips \
             (0..=3) only"
                .to_string(),
        )),
    }
}

/// The `POLYLINE` drag rule: one vertex, in place, with every bulge preserved.
///
/// `points` are stored world-space vertices (the importer already maps the
/// LWPOLYLINE OCS to world, so there is no elevation/normal frame left to apply
/// here). The dragged vertex takes `x`/`y` from `to` and **keeps its own `z`**, so
/// a plan-view drag never changes an elevation; the other vertices, `closed` and
/// the whole `bulges` vector are returned verbatim. A bulge is therefore never
/// recomputed for the moved vertex (our reading — the segment entering and the
/// segment leaving keep the stored bulge — **not vendor-verified**).
fn apply_polyline(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    match id.kind {
        GripKind::Vertex => {
            let index = usize::from(id.index);
            let Some(stored) = points.get(index) else {
                return Err(stale_handle(format!(
                    "AcDbPolyline has no Vertex grip at index {index} \
                     (it has {} stored vertices)",
                    points.len()
                )));
            };
            finite_target(to)?;
            let moved = Point3 {
                x: to.x,
                y: to.y,
                z: stored.z,
            };
            if moved == *stored {
                // Also covers a z-only target: the drag keeps the stored z, so
                // such a target would stage an unchanged vertex.
                return Err(zero_displacement("AcDbPolyline vertex"));
            }
            let mut new_points = points.to_vec();
            new_points[index] = moved;
            Ok(SemanticGeometry::Polyline {
                points: new_points,
                bulges: bulges.to_vec(),
                closed,
            })
        }
        GripKind::Endpoint
        | GripKind::Midpoint
        | GripKind::Center
        | GripKind::Quadrant
        | GripKind::ControlPoint
        | GripKind::Point => Err(stale_handle(
            "AcDbPolyline exposes one Vertex grip per stored vertex (index = stored vertex \
             index) only"
                .to_string(),
        )),
    }
}

/// The `SPLINE` drag rule: one stored control point, every other field verbatim.
///
/// `control_points` are the **NURBS definition** points, not points on the drawn
/// curve: they are the `P_i` of `C(u) = sum(N_i(u) w_i P_i) / sum(N_i(u) w_i)`, so
/// an interior control point does not lie on the curve at all (only a clamped
/// spline's first/last one does). Dragging one therefore **redefines** that entry
/// and leaves `degree`, `knots` and `weights` **bit-identical**: no knot is
/// re-parameterised, no weight is renormalised, no fit is re-run. The result is a
/// **different NURBS** sharing the same parameterisation, which is exactly the
/// point of a control-point handle — the curve is reshaped, not translated — and
/// after such a drag the curve generally no longer passes through the points it
/// passed through before. That is our choice (AutoCAD also drags control points
/// without re-fitting, but **no vendor equivalence is claimed**).
///
/// The target is stored in all three coordinates (a control point is a free 3D
/// definition point, so nothing is projected; the plan view supplies the handle's
/// own `z`, so a plan-view drag cannot change an elevation).
///
/// An `index` outside the stored control points, **any** control-point id on an
/// internally inconsistent definition (`control_points.len() != weights.len()` or
/// `knots.len() != control_points.len() + degree + 1`, which exposes no grip at
/// all — see [`super::grips_for_entity`]) and any other [`GripKind`] are
/// [`REASON_STALE_HANDLE`]. Mismatched data is never repaired by guessing.
fn apply_spline(
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    match id.kind {
        GripKind::ControlPoint => {
            if !spline_is_consistent(degree, knots, control_points, weights) {
                return Err(stale_handle(format!(
                    "AcDbSpline stores an inconsistent NURBS definition ({} control points, \
                     {} weights, {} knots for degree {degree}); it exposes no ControlPoint grip \
                     and no knot or weight is recomputed here",
                    control_points.len(),
                    weights.len(),
                    knots.len()
                )));
            }
            let index = usize::from(id.index);
            let Some(stored) = control_points.get(index) else {
                return Err(stale_handle(format!(
                    "AcDbSpline has no ControlPoint grip at index {index} \
                     (it has {} stored control points)",
                    control_points.len()
                )));
            };
            finite_target(to)?;
            if to == *stored {
                return Err(zero_displacement("AcDbSpline control point"));
            }
            let mut new_points = control_points.to_vec();
            new_points[index] = to;
            Ok(SemanticGeometry::Spline {
                degree,
                knots: knots.to_vec(),
                control_points: new_points,
                // Copied verbatim: a control-point drag changes the definition
                // point only, never the weights or the knot vector.
                weights: weights.to_vec(),
            })
        }
        GripKind::Endpoint
        | GripKind::Midpoint
        | GripKind::Center
        | GripKind::Quadrant
        | GripKind::Vertex
        | GripKind::Point => Err(stale_handle(
            "AcDbSpline exposes one ControlPoint grip per stored control point (index = stored \
             control-point index) only"
                .to_string(),
        )),
    }
}

/// A `grip.unsupported_kind` refusal naming the DXF class.
fn unsupported_kind(class: &str) -> CadError {
    CadError::Unsupported(format!(
        "{REASON_UNSUPPORTED_KIND}: {class} exposes no grippable handle"
    ))
}

/// A `grip.stale_handle` refusal with the specific mismatch.
fn stale_handle(detail: String) -> CadError {
    CadError::InvalidInput(format!("{REASON_STALE_HANDLE}: {detail}"))
}

/// A `grip.zero_displacement` refusal naming the handle that did not move.
fn zero_displacement(handle: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_ZERO_DISPLACEMENT}: the {handle} is already at the drag target"
    ))
}

/// A `grip.zero_displacement` refusal for a quadrant drag whose radius is
/// already stored (an off-plane target at the current radius).
fn unchanged_radius(radius: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_ZERO_DISPLACEMENT}: the AcDbCircle quadrant drag leaves the radius at {radius}"
    ))
}

/// A `grip.degenerate_radius` refusal naming the rejected radius.
fn degenerate_radius(radius: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DEGENERATE_RADIUS}: the AcDbCircle quadrant drag yields radius {radius}; \
         a circle radius must be positive and finite (the centre is not moved by this handle)"
    ))
}

/// A `grip.degenerate_radius` refusal for an ellipse quadrant drag, naming the
/// cause (a measured radius or a stored frame that cannot yield one).
fn degenerate_ellipse_radius(cause: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DEGENERATE_RADIUS}: the AcDbEllipse quadrant drag yields no usable in-plane \
         radius: {cause}; an ellipse semi-axis must be positive and finite (the centre and the \
         normal are not moved by this handle)"
    ))
}

/// A `grip.zero_displacement` refusal for an ellipse quadrant drag whose measured
/// radius is already stored (the axis direction never rotates).
fn unchanged_ellipse_radius(radius: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_ZERO_DISPLACEMENT}: the AcDbEllipse quadrant drag leaves the axis radius at \
         {radius}"
    ))
}

/// A `grip.partial_ellipse` refusal naming the quadrant handle that a partial
/// ellipse (elliptical arc) does not expose.
fn partial_ellipse(index: u8) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_PARTIAL_ELLIPSE}: AcDbEllipse quadrant {index} is not a handle of a partial \
         ellipse; a stored parameter range that is not a full turn exposes the Center grip only"
    ))
}

/// A `grip.degenerate_arc` refusal naming the rejected sweep.
fn degenerate_arc(handle: &str, sweep: f64) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_DEGENERATE_ARC}: the {handle} drag yields sweep {sweep}; an arc sweep must \
         stay finite and strictly inside (0, 2*pi) in its stored direction \
         (the centre, radius and normal are not moved by this handle)"
    ))
}

/// A `grip.non_finite_target` refusal for a finite input whose result overflows.
fn overflowed(handle: &str) -> CadError {
    CadError::InvalidInput(format!(
        "{REASON_NON_FINITE_TARGET}: translating the {handle} produces a non-finite coordinate"
    ))
}

/// Refuse a target that could not be stored as a world point.
///
/// Exposed to the `GRIPSTRETCH` command so it can refuse a non-finite target
/// **before** it even reads the drawing, with the same code and message the drag
/// rule would produce.
pub(crate) fn finite_target(to: Point3) -> CadResult<()> {
    if is_finite(to) {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "{REASON_NON_FINITE_TARGET}: ({}, {}, {})",
            to.x, to.y, to.z
        )))
    }
}
/// Apply a single-handle drag to `geometry`, returning the geometry the entity
/// must be updated to.
///
/// * `LINE` endpoint: the dragged endpoint takes `to`, the other endpoint is
///   returned **bit-identically** (the very same `f64` values);
/// * `LINE` midpoint: both endpoints move by `to - midpoint`, so the line
///   translates rigidly (same direction, same length);
/// * `CIRCLE` centre: the centre takes `to`; `radius`/`normal` are bit-identical;
/// * `CIRCLE` quadrant: `radius` becomes the in-plane distance from the centre to
///   `to`; `center`/`normal` are bit-identical;
/// * `ARC` centre: the arc translates rigidly; `radius`/`normal`/`start`/`sweep`
///   are bit-identical;
/// * `ARC` endpoint: the dragged endpoint's angle becomes the in-plane direction
///   of `to` from the centre; `center`/`radius`/`normal` are bit-identical, the
///   opposite endpoint keeps its angle and the sweep is recomputed in the stored
///   direction (an arc that would collapse is refused, never clamped);
/// * `ELLIPSE` centre: the ellipse translates rigidly; `normal`/`major_axis`/
///   `ratio`/`start`/`sweep` are bit-identical;
/// * `ELLIPSE` quadrant (a full ellipse only): the corresponding radius becomes
///   the in-plane distance from the centre to `to`. A major handle sets
///   `|major_axis|` and keeps the stored minor radius (recomputing `ratio`); a
///   minor handle sets `ratio = distance / |major_axis|` and keeps `major_axis`
///   bit-identical. The axis direction never rotates and a partial ellipse refuses
///   the handle by name;
/// * `POLYLINE` vertex: that one vertex takes `x`/`y` from `to` and keeps its own
///   `z`; every other vertex and every bulge are bit-identical;
/// * `SPLINE` control point: that one **definition** point takes `to` verbatim and
///   `degree`/`knots`/`weights` plus every other control point are bit-identical.
///   No knot/weight is recomputed, so the curve is reshaped as a different NURBS
///   (a control point is not on the curve — see the module doc) and an internally
///   inconsistent definition refuses every control-point id as stale;
/// * `POINT`: the position becomes `to`;
/// * everything else, a [`GripId`] that does not name a handle of this geometry,
///   a non-finite target and a drag with no geometric effect are refused with the
///   stable reason codes documented in the module doc.
///
/// The function is pure: it writes nothing, opens no transaction and records no
/// history. The `GRIPSTRETCH` command calls it, and on `Err` skips the
/// transaction entirely — that is exactly why a zero displacement is an error
/// instead of an `Ok` with unchanged geometry (no empty undo entry).
pub fn apply_grip(
    geometry: &SemanticGeometry,
    id: &GripId,
    to: Point3,
) -> CadResult<SemanticGeometry> {
    match geometry {
        SemanticGeometry::Line { start, end } => apply_line(*start, *end, id, to),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => apply_circle(*center, *normal, *radius, id, to),
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => apply_arc(*center, *normal, *radius, *start, *sweep, id, to),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => apply_ellipse(
            (*center, *normal, *major_axis, *ratio, *start, *sweep),
            id,
            to,
        ),
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => apply_polyline(points, bulges, *closed, id, to),
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => apply_spline(*degree, knots, control_points, weights, id, to),
        SemanticGeometry::Point(position) => {
            match (id.kind, id.index) {
                (GripKind::Point, 0) => {}
                (GripKind::Point, index) => {
                    return Err(stale_handle(format!(
                        "AcDbPoint has no Point grip at index {index} (only 0)"
                    )));
                }
                (
                    GripKind::Endpoint
                    | GripKind::Midpoint
                    | GripKind::Center
                    | GripKind::Quadrant
                    | GripKind::Vertex
                    | GripKind::ControlPoint,
                    _,
                ) => {
                    return Err(stale_handle(
                        "AcDbPoint exposes a Point grip (0) only".to_string(),
                    ));
                }
            }
            finite_target(to)?;
            if to == *position {
                return Err(zero_displacement("AcDbPoint position"));
            }
            Ok(SemanticGeometry::Point(to))
        }
        other => Err(unsupported_kind(crate::app_drawing::class_name(other))),
    }
}
