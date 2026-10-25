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
    arc_point, in_plane_distance, is_finite, midpoint, plane_axes, quadrant_point, GripId,
    GripKind, ARC_SWEEP_EPSILON, QUADRANT_DIRECTIONS, REASON_DEGENERATE_ARC,
    REASON_DEGENERATE_RADIUS, REASON_NON_FINITE_TARGET, REASON_STALE_HANDLE,
    REASON_UNSUPPORTED_KIND, REASON_ZERO_DISPLACEMENT,
};
use cad_domain::{CadError, CadResult, Point3, SemanticGeometry};
use cad_geometry::{add, dot, sub};

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
        (GripKind::Center | GripKind::Quadrant | GripKind::Vertex | GripKind::Point, _) => {
            Err(stale_handle(
                "AcDbLine exposes Endpoint (0 = start, 1 = end) and Midpoint (2) grips only"
                    .to_string(),
            ))
        }
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
        (GripKind::Endpoint | GripKind::Midpoint | GripKind::Vertex | GripKind::Point, _) => {
            Err(stale_handle(
                "AcDbCircle exposes a Center grip (0) and Quadrant grips (0..=3) only".to_string(),
            ))
        }
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
        (GripKind::Midpoint | GripKind::Vertex | GripKind::Point, _) => Err(stale_handle(
            "AcDbArc exposes a Center grip (0) and Endpoint grips (0 = start, 1 = end) only"
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
        | GripKind::Point => Err(stale_handle(
            "AcDbPolyline exposes one Vertex grip per stored vertex (index = stored vertex \
             index) only"
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
/// * `POLYLINE` vertex: that one vertex takes `x`/`y` from `to` and keeps its own
///   `z`; every other vertex and every bulge are bit-identical;
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
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => apply_polyline(points, bulges, *closed, id, to),
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
                    | GripKind::Vertex,
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
