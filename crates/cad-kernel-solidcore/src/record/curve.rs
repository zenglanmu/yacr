//! Upstream edge curves → neutral analytic curves.
//!
//! Only `Line` and a **closed** `Circle` are mapped. A record edge is unbounded
//! (its bounds are the edge `interval`), so a circle edge that is not closed —
//! its start and end vertex differ — is an arc, and the neutral model has no arc
//! variant: it is refused, never drawn as a full circle.
//!
//! The record's `curve` is the analytic *label*; when the edge also carries a
//! `nurbs` form the form is the geometry and the label a proved name for it, so
//! mapping the label keeps a cylinder cap circle analytic.

use cad_domain::{BrepCurve, Point3};
use solidcore_record::brep::{Brep as Record, Curve, Edge};

use super::cross;
use super::units::point;
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
    let key = type_key(&edge.curve);
    Err(LiftRefusal::unsupported(format!(
        "a '{key}' edge curve is not mapped to the neutral B-rep in P1 \
         (only lines and full circles are)"
    )))
}
