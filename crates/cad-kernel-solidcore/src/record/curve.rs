//! Upstream edge curves → neutral analytic curves.
//!
//! `Line`, a **closed** `Circle`, a **non-closed** `Circle` (an open circle is an
//! arc: the record's curves are unbounded, so the edge `interval` is the trim and
//! its two vertices are the ends), and a `Spline` whose control points all lie on
//! its chord are mapped. A circle edge whose start and end vertex coincide is the
//! full circle; when they differ the arc is reconstructed from the circle's frame
//! and the **authoritative vertices** — but only when both endpoints really lie on
//! the circle within the edge's tolerance. An open circle whose endpoints do not
//! lie on it is refused by name, never drawn as the nearest arc.
//!
//! The record's `curve` is the analytic *label*; when the edge also carries a
//! `nurbs` form the form is the geometry and the label a proved name for it, so
//! mapping the label keeps a cylinder cap circle analytic.
//!
//! # Straight splines
//!
//! The pinned kernel writes a chamfer's straight edges (and other new edges
//! between planes) as cubic Béziers whose control points are collinear and
//! spread along the segment. Such a curve's image **is** the straight segment,
//! so mapping it to [`cad_domain::BrepCurve::Line`] is exact, not an
//! approximation — and it is what keeps an all-planar chamfer product liftable.
//! The check is geometric and local: every control point must lie on the chord
//! within the edge's tolerance, and the chord's ends must be the edge's own
//! vertices. A spline that leaves its chord (any real curve) stays refused.

use cad_domain::{BrepCurve, Point3};
use solidcore_record::brep::{Brep as Record, Curve, Edge};

use super::units::point;
use super::{add, cross, dot, is_axis, length, mul, sub};
use crate::error::LiftRefusal;

/// The record's own name for a curve, used in refusal details.
pub(crate) fn type_key(curve: &Curve) -> &'static str {
    match curve {
        Curve::Line { .. } => "line",
        Curve::Circle { .. } => "circle",
        Curve::Ellipse { .. } => "ellipse",
        Curve::Polyline { .. } => "polyline",
        Curve::Spline { .. } => "spline",
        Curve::RationalSpline { .. } => "rational-spline",
        Curve::SurfaceIntersection { .. } => "surface-intersection",
    }
}

/// Whether the edge starts and ends at the same vertex (a closed edge).
pub(crate) fn closed(edge: &Edge) -> bool {
    edge.vertices[0] == edge.vertices[1]
}

/// The circle an edge's curve describes, when it is a `Circle` label.
pub(crate) fn circle(curve: &Curve) -> Option<(Point3, Point3, Point3, f64)> {
    match curve {
        Curve::Circle { frame, radius } => {
            let center = point(frame.origin);
            let x = point(frame.x_axis);
            let y = point(frame.y_axis);
            let normal = cross(x, y);
            Some((
                center,
                super::normalize(normal),
                super::normalize(x),
                *radius,
            ))
        }
        _ => None,
    }
}

/// The endpoint vertices of an edge, in its own direction.
///
/// The edge's curve stores its own endpoints, but a kernel-produced record can
/// be one ULP off them (the curve and the vertex are separate records with
/// separate tolerances). The **vertex** is authoritative: using it keeps the
/// neutral loops exactly chained, which the reverse conversion requires.
pub(crate) fn endpoints(record: &Record, edge: &Edge, forward: bool) -> Option<(Point3, Point3)> {
    let start = record.vertices.get(edge.vertices[0] as usize)?;
    let end = record.vertices.get(edge.vertices[1] as usize)?;
    let (start, end) = (point(start.point), point(end.point));
    Some(if forward { (start, end) } else { (end, start) })
}

/// Map one oriented edge use to a neutral curve, already oriented in loop
/// direction.
pub(crate) fn to_brep(
    record: &Record,
    edge: &Edge,
    forward: bool,
) -> Result<BrepCurve, LiftRefusal> {
    if matches!(edge.curve, Curve::Line { .. }) {
        let (start, end) = endpoints(record, edge, forward)
            .ok_or_else(|| LiftRefusal::unsupported("a line edge references a missing vertex"))?;
        // A collapsed line is the record's degenerate-edge marker (a cone apex
        // uses it); as a standalone boundary curve it means nothing to us.
        if start == end {
            return Err(LiftRefusal::unsupported(
                "a collapsed line edge cannot be mapped to a boundary curve",
            ));
        }
        return Ok(BrepCurve::Line { start, end });
    }
    if let Some((center, normal, u_dir, radius)) = circle(&edge.curve) {
        if !closed(edge) {
            return arc_from_circle(record, edge, forward, center, normal, u_dir, radius);
        }
        return Ok(BrepCurve::Circle {
            center,
            normal,
            u_dir,
            radius,
        });
    }
    if let Some(line) = straight_spline(record, edge, forward) {
        return Ok(line);
    }
    let key = type_key(&edge.curve);
    Err(LiftRefusal::unsupported(format!(
        "a '{key}' edge curve is not mapped to the neutral B-rep in P1 \
         (only lines, circles (full or open) and exactly straight splines are)"
    )))
}

/// The neutral arc an **open** circle edge describes, or a named refusal.
///
/// The circle record is unbounded and the edge's `interval` is its trim; the
/// edge's **vertices** are authoritative for the ends (a kernel record can be one
/// ULP off between its curve and its vertex). The result is oriented so the sweep
/// from `start` to `end` runs counter-clockwise about `normal`:
///
/// - a use walking the edge `forward` runs interval min → max, which for a
///   `Curve::Circle` label is counter-clockwise about the frame's `z = x × y`;
/// - a use walking it backwards runs `end → start`, which is counter-clockwise
///   about `-z`, so the endpoints and the normal both flip.
///
/// Both endpoints must lie on the circle (radius **and** plane) within the edge's
/// own tolerance, and the span they describe must be a proper part of one turn.
/// Otherwise the record is internally inconsistent and is refused — a "nearly
/// right" arc is not re-projected onto the circle.
fn arc_from_circle(
    record: &Record,
    edge: &Edge,
    forward: bool,
    center: Point3,
    normal: Point3,
    u_dir: Point3,
    radius: f64,
) -> Result<BrepCurve, LiftRefusal> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(LiftRefusal::unsupported(format!(
            "an arc edge has radius {radius}; the radius must be finite and positive"
        )));
    }
    let Some((start, end)) = endpoints(record, edge, forward) else {
        return Err(LiftRefusal::unsupported(
            "an arc edge (an open circle) references a missing vertex",
        ));
    };
    let normal = if forward { normal } else { mul(normal, -1.0) };
    if !is_axis(normal) || !is_axis(u_dir) {
        return Err(LiftRefusal::unsupported(
            "an arc edge has a degenerate circle frame",
        ));
    }
    // The same idiom as a straight spline's ends: the edge's own tolerance with a
    // tiny relative floor, so a zero tolerance still admits the kernel's rounding.
    let allowed = edge
        .tolerance
        .max(1e-9 * radius.abs())
        .max(f64::MIN_POSITIVE);
    let v_dir = cross(normal, u_dir);
    for p in [start, end] {
        let offset = sub(p, center);
        if (length(offset) - radius).abs() > allowed || dot(offset, normal).abs() > allowed {
            return Err(LiftRefusal::unsupported(format!(
                "an arc edge (an open circle) has an endpoint that is not on its circle (radius \
                 or plane off by more than {allowed}); it is not re-projected"
            )));
        }
    }
    let angle = |p: Point3| {
        let offset = sub(p, center);
        dot(offset, v_dir).atan2(dot(offset, u_dir))
    };
    let delta = angle(end) - angle(start);
    let sweep = if delta > 0.0 {
        delta
    } else {
        delta + std::f64::consts::TAU
    };
    if !sweep.is_finite() || !(sweep > 1e-9 && sweep < std::f64::consts::TAU - 1e-9) {
        return Err(LiftRefusal::unsupported(format!(
            "an arc edge (an open circle) covers {sweep} rad between its vertices; an arc is a \
             proper part of one turn, so this is refused rather than drawn as a full circle"
        )));
    }
    Ok(BrepCurve::Arc {
        center,
        normal,
        u_dir,
        radius,
        start,
        end,
    })
}

/// The straight segment a `Spline` edge describes *exactly*, when it does.///
/// See the module docs: a Bézier/B-spline whose control polygon lies on its
/// chord has the chord as its image, so it is a line. The endpoints are taken
/// from the edge's authoritative **vertices** (kernel records can be one ULP off
/// between the curve and the vertex), and the orientation follows `forward`.
///
/// Returns `None` for anything that is not proved straight, so the caller still
/// refuses it as a spline.
fn straight_spline(record: &Record, edge: &Edge, forward: bool) -> Option<BrepCurve> {
    let Curve::Spline { points, .. } = &edge.curve else {
        return None;
    };
    let first = points.first()?;
    let last = points.last()?;
    let start_vertex = record.vertices.get(edge.vertices[0] as usize)?;
    let end_vertex = record.vertices.get(edge.vertices[1] as usize)?;
    let (start, end) = (point(start_vertex.point), point(end_vertex.point));

    let from = point(*first);
    let to = point(*last);
    let chord = sub(to, from);
    let squared = dot(chord, chord);
    if !squared.is_finite() || squared <= 0.0 {
        return None;
    }
    let chord_length = squared.sqrt();
    // The edge's own tolerance, with a tiny relative floor so a zero tolerance
    // still admits the kernel's rounding and nothing more.
    let allowed = edge
        .tolerance
        .max(1e-9 * chord_length)
        .max(f64::MIN_POSITIVE);

    let ends_match = |a: Point3, b: Point3| length(sub(a, b)) <= allowed;
    let ordered = ends_match(from, start) && ends_match(to, end);
    let reversed = ends_match(from, end) && ends_match(to, start);
    if !ordered && !reversed {
        return None;
    }
    for control in points {
        let p = point(*control);
        let t = dot(sub(p, from), chord) / squared;
        // Outside the chord's span: the curve leaves the segment.
        if !(0.0..=1.0).contains(&t) {
            return None;
        }
        let on_chord = add(from, mul(chord, t));
        if length(sub(p, on_chord)) > allowed {
            return None;
        }
    }
    // `forward` is the loop direction; the control points run the edge's own
    // (min → max) direction, which `start`/`end` already follow.
    Some(if forward {
        BrepCurve::Line { start, end }
    } else {
        BrepCurve::Line {
            start: end,
            end: start,
        }
    })
}
