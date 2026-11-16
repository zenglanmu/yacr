//! Upstream edge curves → neutral analytic curves.
//!
//! `Line`, a **closed** `Circle`, and a `Spline` whose control points all lie on
//! its chord are mapped. A record edge is unbounded (its bounds are the edge
//! `interval`), so a circle edge that is not closed — its start and end vertex
//! differ — is an arc, and the neutral model has no arc variant: it is refused,
//! never drawn as a full circle.
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
use super::{add, cross, dot, length, mul, sub};
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
            return Err(LiftRefusal::unsupported(
                "a circular arc edge (open circle) is not mapped in P1; only full circles are",
            ));
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
         (only lines, full circles and exactly straight splines are)"
    )))
}

/// The straight segment a `Spline` edge describes *exactly*, when it does.
///
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
