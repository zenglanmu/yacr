//! The arc and circle half of `to_record`: a circular boundary in the record.
//!
//! The record treats `Curve::Circle` as a *label* and requires its NURBS form,
//! and the edge's `interval` is the trim. So a circular boundary is written as:
//!
//! * the analytic label (`Curve::Circle` with `x_axis = u_dir`, so a round trip
//!   gives the same `u_dir` back),
//! * the exact NURBS form of the spanned arc — or of the whole circle — from
//!   `solidcore`'s own `eval::form::curve`, on the **same quarter grid the face it
//!   bounds carries**, rescaled so one turn is `2π` (see [`super::surfaces`]). The
//!   interior parameter is the conic's, **not** the angle; what the shared grid
//!   buys is that a boundary whose ends fall on a quarter line has an exact pcurve
//!   on the face (the two parameterisations agree at every knot and between them).
//!   An arc whose ends are **off** that grid has no exact pcurve on a curved face:
//!   `solidcore_ops::pcurve` refuses it (`solid.pcurve_not_exact`) rather than
//!   fitting one — it still lifts on a plane, where the pcurve is the affine image
//!   of this same form.
//! * `interval`, the span in that form's parameters, and the two **vertex**
//!   records at its ends (a full circle's two ends are the same seam vertex).
//!
//! # Two uses of one circle
//!
//! Two uses of one arc (the two faces sharing it) must collapse to one record
//! edge, or the closed-shell check sees each use once. Arcs are matched by their
//! **point set** — centre, radius and the arc's own midpoint — because both a
//! reversed traversal and a differently-oriented `u_dir` reproduce those three
//! exactly. (The midpoint is the point at half the sweep *in the arc's own frame*;
//! rotating `u_dir` about `normal` shifts every frame angle by the same amount, so
//! the point itself does not move. A chord-bisector shortcut would need a sign for
//! sweeps over `pi` and would collapse a half arc to its centre — hence the point,
//! not the bisector.) A full circle is matched by its circle: centre, radius and
//! plane.
//!
//! A full circle has no endpoints to tell a reversed use from a forward one, so
//! its traversal is resolved after every face's loops exist, from the planar face
//! that shares it (`to_record::resolve_closed_circles`): the cap's loop runs
//! counter-clockwise about its own plane normal, and the other use necessarily
//! runs the other way.
//!
//! The use's pcurve on a **plane** is the arc's (or circle's) own NURBS
//! **projected** into the face's `(u, v)`. The projection from the plane to
//! `(u, v)` is affine and a NURBS is invariant under an affine map of its control
//! points, so that pcurve is exact and shares the edge's parameter by
//! construction. This matters: the kernel integrates the trimmed face's area,
//! centroid and inertia over the pcurve, so a chord polyline would report a wrong
//! area even with a right volume. On a **curved** face the pcurve comes from
//! `solidcore_ops::pcurve::pcurves_within` instead (see [`super::loops`]), which
//! is exact or refuses.
//!
//! # What this does *not* prove by itself
//!
//! The kernel's `record_check::agreement` proves the loop chains, the pcurves lift
//! onto their edges and the outer loop turns the right way — i.e. the arc's ends
//! and its area behaviour. It does **not** prove the arc's *bulge*: a pcurve (and
//! a 3D net) could bulge the wrong way and still chain. That is what the
//! independent assertions pin — `an_arc_pcurve_is_the_exact_uv_image_of_the_arc`
//! checks every sample is exactly `radius` from the centre, and
//! `arc_boundary_is_an_approximation_with_a_chordal_bound` checks the tessellated
//! quarter disc's area against `π/4`.

use std::collections::HashMap;

use cad_domain::{BrepCurve, Point3};
use solidcore_record::brep::{Brep as Record, Curve, Curve2, Edge, Frame};

use super::loops::{boundary_kind, edge_use, vertex, ArcEdge, Authored, CircleEdge, Segment};
use super::surfaces::circle_form;
use super::units::{bits, record_point};
use super::{add, cross, dot, finite_point, is_axis, length, mul, normalize, sub};
use crate::error::LiftRefusal;

/// A neutral circular arc with its frame resolved to unit vectors.
struct ArcSeg {
    center: Point3,
    normal: Point3,
    u_dir: Point3,
    radius: f64,
    start: Point3,
    end: Point3,
}

/// Map one authored boundary curve to its record edge (creating it on first use).
pub(super) fn authored_segment(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    curve: &BrepCurve,
    face: u32,
) -> Result<Authored, LiftRefusal> {
    authored_with_seam(record, vertex_of, edge_of, arcs, circles, curve, face, None)
}

/// [`authored_segment`] with an **explicit seam point** for a full circle.
///
/// Two circles of one seam loop (a torus's tube circle and its outer equator)
/// must share one record vertex, and vertices dedup on exact bits. Their seams
/// computed independently — `(origin + x·R) + x·r` versus `origin + x·(R + r)` —
/// differ by a rounding step for most inputs, so the loop would not chain. The
/// caller that knows both circles computes the shared point once and passes it
/// here for both.
#[allow(clippy::too_many_arguments)]
pub(super) fn authored_with_seam(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    edge_of: &mut HashMap<([u64; 3], [u64; 3]), u32>,
    arcs: &mut Vec<ArcEdge>,
    circles: &mut Vec<CircleEdge>,
    curve: &BrepCurve,
    face: u32,
    seam: Option<Point3>,
) -> Result<Authored, LiftRefusal> {
    match curve {
        BrepCurve::Line { start, end } => {
            if !finite_point(*start) || !finite_point(*end) {
                return Err(LiftRefusal::unsupported(format!(
                    "face {face} has a boundary point that is not finite"
                )));
            }
            let (edge, forward) = edge_use(record, vertex_of, edge_of, (*start, *end), face)?;
            Ok(Authored {
                edge,
                forward,
                segment: Segment::Line {
                    start: *start,
                    end: *end,
                },
                domain_samples: vec![*start, *end],
            })
        }
        BrepCurve::Arc {
            center,
            normal,
            u_dir,
            radius,
            start,
            end,
        } => {
            let arc = ArcSeg {
                center: *center,
                normal: *normal,
                u_dir: *u_dir,
                radius: *radius,
                start: *start,
                end: *end,
            };
            let (edge, forward) = arc_edge_use(record, vertex_of, arcs, &arc, face)?;
            // The face's parameter box must contain the whole arc, and a chord
            // estimate would cut its bulge; the circle's axis box is the safe
            // superset (a plane has no bounded natural domain, so a loose box
            // costs nothing here).
            let mut domain_samples = vec![*start, *end];
            for (dx, dy, dz) in [
                (-1.0, 0.0, 0.0),
                (1.0, 0.0, 0.0),
                (0.0, -1.0, 0.0),
                (0.0, 1.0, 0.0),
                (0.0, 0.0, -1.0),
                (0.0, 0.0, 1.0),
            ] {
                domain_samples.push(Point3 {
                    x: center.x + radius * dx,
                    y: center.y + radius * dy,
                    z: center.z + radius * dz,
                });
            }
            Ok(Authored {
                edge,
                forward,
                segment: Segment::Arc,
                domain_samples,
            })
        }
        BrepCurve::Circle {
            center,
            normal,
            u_dir,
            radius,
        } => {
            let (edge, forward) = circle_edge_use(
                record, vertex_of, circles, *center, *normal, *u_dir, *radius, face, seam,
            )?;
            // The face's parameter box must contain the whole circle and every
            // arc a trimmed face is bounded by; the circle's axis box is the safe
            // superset for a plane's box (a plane has no bounded natural domain,
            // so a loose box costs nothing).
            let mut domain_samples = Vec::with_capacity(6);
            for (dx, dy, dz) in [
                (-1.0, 0.0, 0.0),
                (1.0, 0.0, 0.0),
                (0.0, -1.0, 0.0),
                (0.0, 1.0, 0.0),
                (0.0, 0.0, -1.0),
                (0.0, 0.0, 1.0),
            ] {
                domain_samples.push(Point3 {
                    x: center.x + radius * dx,
                    y: center.y + radius * dy,
                    z: center.z + radius * dz,
                });
            }
            Ok(Authored {
                edge,
                forward,
                segment: Segment::Circle,
                domain_samples,
            })
        }
        other => Err(LiftRefusal::non_planar(format!(
            "face {face} has a boundary curve that is not a line, a circle or a circular arc \
             ({}); only line-, circle- or arc-bounded bodies are lifted",
            boundary_kind(other)
        ))),
    }
}

/// Create (or find) the record edge for one **full** circle.
///
/// The record's `Curve::Circle` is a label and the geometry is the edge's NURBS
/// form; the edge's `interval` is the form's own parameter range (a whole turn on
/// the surface's quarter grid), and both vertices are the **seam point** — the
/// point at the label's `u_dir`, which is where a cap's loop and a lateral face's
/// seam edge meet it.
#[allow(clippy::too_many_arguments)] // one circle's own frame, named rather than packed
fn circle_edge_use(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    circles: &mut Vec<CircleEdge>,
    center: Point3,
    normal: Point3,
    u_dir: Point3,
    radius: f64,
    face: u32,
    seam: Option<Point3>,
) -> Result<(u32, bool), LiftRefusal> {
    if !finite_point(center) {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a circular boundary with a non-finite centre"
        )));
    }
    let normal = normalize(normal);
    let x = normalize(sub(u_dir, mul(normal, dot(u_dir, normal))));
    if !is_axis(normal) || !is_axis(x) {
        return Err(LiftRefusal::placement(format!(
            "face {face} has a circular boundary whose frame is degenerate"
        )));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a circular boundary with radius {radius}; the radius must be finite \
             and positive"
        )));
    }
    let y = normalize(cross(normal, x));
    if !is_axis(y) {
        return Err(LiftRefusal::placement(format!(
            "face {face} has a circular boundary whose frame is degenerate"
        )));
    }
    // The same circle described twice is one record edge.
    let allowed = record.tolerance.max(1e-9 * radius);
    for existing in circles.iter() {
        let same_plane = dot(sub(center, existing.center), existing.normal).abs() <= allowed;
        let same_axis = dot(normal, existing.normal).abs() >= 1.0 - 1e-9;
        if same_plane
            && same_axis
            && (existing.radius - radius).abs() <= allowed
            && length(sub(center, existing.center)) <= allowed
        {
            return Ok((existing.edge, true));
        }
    }
    // The seam vertex: the caller's point when two circles must share one (see
    // [`authored_with_seam`]), else this circle's own reference point.
    let seam = match seam {
        Some(given) => {
            // A caller-supplied seam is the shared point of a seam loop; it must
            // be this circle's point at angle zero, within rounding.
            let allowed = record.tolerance.max(1e-9 * radius);
            if length(sub(given, add(center, mul(x, radius)))) > allowed {
                return Err(LiftRefusal::unsupported(format!(
                    "face {face} has a circular boundary whose shared seam point is not on its \
                     reference radius (off by more than {allowed})"
                )));
            }
            given
        }
        None => add(center, mul(x, radius)),
    };
    let seam_vertex = vertex(record, vertex_of, bits(record_point(seam)), seam);
    let (form, interval) = circle_form(
        face,
        center,
        normal,
        x,
        radius,
        [0.0, std::f64::consts::TAU],
    )?;
    let tolerance = record.tolerance;
    record.edges.push(Edge {
        // The circle is the analytic **label**; the rational NURBS form above is
        // the geometry (the record requires both for a circle), and the interval
        // is the form's own parameter range.
        curve: Curve::Circle {
            frame: Frame {
                origin: record_point(center),
                x_axis: record_point(x),
                y_axis: record_point(y),
            },
            radius,
        },
        nurbs: Some(form),
        interval,
        vertices: [seam_vertex, seam_vertex],
        tolerance,
        uses: Vec::new(),
    });
    let edge = u32::try_from(record.edges.len())
        .map_err(|_| LiftRefusal::unsupported("more edges than a record index can name"))?
        - 1;
    circles.push(CircleEdge {
        center,
        normal,
        radius,
        edge,
    });
    Ok((edge, true))
}

/// Create (or find) the record edge for one circular arc.
///
/// The record's `Curve::Circle` is unbounded, so the open span lives in the
/// edge's `interval`; the circle frame keeps the arc's own `u_dir` as `x_axis`,
/// so the interval is the arc's **angular** span `[start angle, end angle]` about
/// `normal`, the same parameter its NURBS form uses. The edge's two vertices are
/// the arc's endpoints, in the arc's own direction.
fn arc_edge_use(
    record: &mut Record,
    vertex_of: &mut HashMap<[u64; 3], u32>,
    arcs: &mut Vec<ArcEdge>,
    arc: &ArcSeg,
    face: u32,
) -> Result<(u32, bool), LiftRefusal> {
    let ArcSeg {
        center,
        normal,
        u_dir,
        radius,
        start,
        end,
    } = *arc;
    if !finite_point(center) || !finite_point(start) || !finite_point(end) {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a circular arc with a non-finite point"
        )));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a circular arc with radius {radius}; the radius must be finite and \
             positive"
        )));
    }
    let normal = normalize(normal);
    if !is_axis(normal) {
        return Err(LiftRefusal::placement(format!(
            "face {face} has a circular arc with a degenerate normal"
        )));
    }
    let u_hint = normalize(u_dir);
    if !is_axis(u_hint) || dot(u_hint, normal).abs() > 1e-6 {
        return Err(LiftRefusal::placement(format!(
            "face {face} has a circular arc whose u direction is not an in-plane axis"
        )));
    }
    // The record circle keeps the arc's own frame (`x_axis = u_dir`), so the
    // record's parameter is the angle from `u_dir` and a round trip through the
    // record gives the same `u_dir` back.
    let x = u_hint;
    let y = normalize(cross(normal, x));
    if !is_axis(y) {
        return Err(LiftRefusal::placement(format!(
            "face {face} has a circular arc whose frame is degenerate"
        )));
    }
    // A producer's endpoints can be a rounding step off the circle; anything
    // beyond that is a different geometry, not something to re-project.
    let allowed = (radius * 1e-7).max(record.tolerance);
    for p in [start, end] {
        let d = sub(p, center);
        if (length(d) - radius).abs() > allowed || dot(d, normal).abs() > allowed {
            return Err(LiftRefusal::unsupported(format!(
                "face {face} has a circular arc endpoint that is not on its circle (radius or \
                 plane off by more than {allowed})"
            )));
        }
    }
    let angle = |p: Point3| {
        let d = sub(p, center);
        dot(d, y).atan2(dot(d, x))
    };
    let start_angle = angle(start);
    let end_angle = angle(end);
    let delta = end_angle - start_angle;
    let sweep = if delta > 0.0 {
        delta
    } else {
        delta + std::f64::consts::TAU
    };
    if !(sweep > 1e-9 && sweep < std::f64::consts::TAU - 1e-9) {
        return Err(LiftRefusal::unsupported(format!(
            "face {face} has a circular arc sweeping {sweep} rad; an arc is a proper part of one \
             turn, and a full circle is not an arc"
        )));
    }
    let half = start_angle + 0.5 * sweep;
    let midpoint = add(
        center,
        add(mul(x, radius * half.cos()), mul(y, radius * half.sin())),
    );
    // One record edge per arc: the two faces sharing it reach the same point set.
    let same = |a: Point3, b: Point3| length(sub(a, b)) <= allowed;
    for existing in arcs.iter() {
        if same(existing.center, center)
            && (existing.radius - radius).abs() <= allowed
            && same(existing.midpoint, midpoint)
        {
            let forward = if same(existing.start, start) && same(existing.end, end) {
                true
            } else if same(existing.start, end) && same(existing.end, start) {
                false
            } else {
                return Err(LiftRefusal::unsupported(format!(
                    "face {face} uses a circular arc whose endpoints disagree with the edge that \
                     already carries it; a loop must chain through the same vertices"
                )));
            };
            return Ok((existing.edge, forward));
        }
    }

    let first = vertex(record, vertex_of, bits(record_point(start)), start);
    let second = vertex(record, vertex_of, bits(record_point(end)), end);
    let tolerance = record.tolerance;
    // The form `solidcore` builds for a circle is the same quarter-grid form the
    // face it bounds carries (see `super::surfaces`), so its interval is the
    // edge's trim in the form's parameters and the use's pcurve is exact.
    let (nurbs, interval) = circle_form(
        face,
        center,
        normal,
        x,
        radius,
        [start_angle, start_angle + sweep],
    )?;
    record.edges.push(Edge {
        // The open circle: the `Circle` record is the analytic **label** and the
        // NURBS form below is the geometry (the record requires both for a
        // circle), with the interval on the NURBS's own parameter.
        curve: Curve::Circle {
            frame: Frame {
                origin: record_point(center),
                x_axis: record_point(x),
                y_axis: record_point(y),
            },
            radius,
        },
        nurbs: Some(nurbs),
        interval,
        vertices: [first, second],
        tolerance,
        uses: Vec::new(),
    });
    let edge = u32::try_from(record.edges.len())
        .map_err(|_| LiftRefusal::unsupported("more edges than a record index can name"))?
        - 1;
    arcs.push(ArcEdge {
        center,
        radius,
        midpoint,
        start,
        end,
        edge,
    });
    Ok((edge, true))
}

/// The exact `(u, v)` image of an arc edge, on the **edge's** interval and in the
/// edge's own direction (min → max), whatever way the loop traverses it.
///
/// The projection from the plane to `(u, v)` is affine and a NURBS is invariant
/// under an affine map of its control points, so projecting the arc's own NURBS
/// form is *exact* — and its parameter is the edge's by construction. The kernel
/// integrates the trimmed face's area and inertia over these pcurves, so a chord
/// polyline here would report a wrong area even with a right volume.
pub(super) fn arc_pcurve(
    record: &Record,
    edge: u32,
    origin: Point3,
    u_axis: Point3,
    v_axis: Point3,
) -> Result<Curve2, LiftRefusal> {
    let Some(stored) = record.edges.get(edge as usize) else {
        return Err(LiftRefusal::unsupported(
            "an arc use references an edge the record does not hold",
        ));
    };
    let Some(Curve::RationalSpline { knots, homogeneous }) = &stored.nurbs else {
        return Err(LiftRefusal::unsupported(
            "an arc edge has no rational NURBS form to project into the face",
        ));
    };
    let uv = |p: Point3| {
        let offset = sub(p, origin);
        [dot(offset, u_axis), dot(offset, v_axis)]
    };
    let mut projected = Vec::with_capacity(homogeneous.len());
    for point in homogeneous {
        let weight = point[3];
        if !weight.is_finite() || weight <= 0.0 {
            return Err(LiftRefusal::unsupported(
                "an arc edge's NURBS form has a non-positive weight",
            ));
        }
        let cartesian = Point3 {
            x: point[0] / weight,
            y: point[1] / weight,
            z: point[2] / weight,
        };
        let [u, v] = uv(cartesian);
        projected.push([u * weight, v * weight, weight]);
    }
    Ok(Curve2::RationalSpline {
        knots: knots.clone(),
        homogeneous: projected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::point;
    use crate::record::{from_record, to_record, DEFAULT_TOLERANCE};
    use cad_domain::{BrepData, BrepFace, BrepSurface};
    use solidcore_record::brep::{Loop, Surface};

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    // -----------------------------------------------------------------------
    // Arcs
    // -----------------------------------------------------------------------

    fn half_disc(z: f64) -> BrepCurve {
        BrepCurve::Arc {
            center: p(0.0, 0.0, z),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
            radius: 1.0,
            start: p(1.0, 0.0, z),
            end: p(-1.0, 0.0, z),
        }
    }

    fn lower_half_disc(z: f64) -> BrepCurve {
        BrepCurve::Arc {
            center: p(0.0, 0.0, z),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
            radius: 1.0,
            start: p(-1.0, 0.0, z),
            end: p(1.0, 0.0, z),
        }
    }

    /// A **disc of zero thickness**: two coplanar faces sharing the two half-arc
    /// boundary edges, wound oppositely.
    ///
    /// This is the only closed shell whose faces are all planes and whose edges
    /// include an arc: an arc lies in exactly one plane, so the face on either
    /// side of it lies in that plane too, and a *solid* with arc edges therefore
    /// always has a curved neighbour (P2.1b). The disc exercises the record
    /// conversion itself — arc edge, interval, vertices, pcurve, shell closure —
    /// which is what this cut adds; lifting an arc-bounded **solid** stays
    /// refused until curved faces lift.
    fn flat_disc() -> BrepData {
        let loops = vec![cad_domain::BrepLoop {
            edges: vec![half_disc(0.0), lower_half_disc(0.0)],
        }];
        let face = |id: u32, reversed: bool| BrepFace {
            id,
            surface: BrepSurface::Plane {
                origin: p(0.0, 0.0, 0.0),
                normal: p(0.0, 0.0, 1.0),
                u_dir: p(1.0, 0.0, 0.0),
            },
            reversed,
            loops: loops.clone(),
            domain: None,
            tolerance: None,
        };
        BrepData {
            shells: vec![cad_domain::BrepShell {
                id: 0,
                faces: vec![face(0, false), face(1, true)],
            }],
            placement: None,
            tolerance: None,
        }
    }

    #[test]
    fn an_arc_is_written_as_an_open_circle_with_its_vertices() {
        let record = to_record(&flat_disc(), DEFAULT_TOLERANCE).expect("flat disc lifts");
        assert_eq!(record.faces.len(), 2);
        assert_eq!(record.edges.len(), 2, "one record edge per arc, used twice");
        assert_eq!(record.vertices.len(), 2);
        for edge in &record.edges {
            let Curve::Circle { frame, radius } = &edge.curve else {
                panic!("an arc must be a circle record: {:?}", edge.curve);
            };
            assert!((radius - 1.0).abs() < 1e-12);
            assert_eq!(point(frame.origin), p(0.0, 0.0, 0.0));
            // Half a turn, open: the interval is the trim, not a full circle.
            let span = edge.interval[1] - edge.interval[0];
            assert!((span - std::f64::consts::PI).abs() < 1e-12, "{span}");
            assert_ne!(
                edge.vertices[0], edge.vertices[1],
                "an arc is not a closed edge"
            );
            assert_eq!(edge.uses.len(), 2, "both faces use each arc");
        }
        // The two arcs carry different halves: their intervals and their frames
        // must differ (a half turn each, on opposite sides).
        assert_ne!(record.edges[0].interval, record.edges[1].interval);
    }

    #[test]
    fn an_arc_pcurve_is_the_exact_uv_image_of_the_arc() {
        let record = to_record(&flat_disc(), DEFAULT_TOLERANCE).expect("flat disc lifts");
        let mut checked = 0;
        for (face_index, face) in record.faces.iter().enumerate() {
            let Surface::Plane {
                origin,
                u_axis,
                v_axis,
            } = &face.surface
            else {
                panic!("the disc's faces are planes");
            };
            let (origin, u, v) = (point(*origin), point(*u_axis), point(*v_axis));
            for loop_ in &face.loops {
                let Loop::Edges { edges } = loop_ else {
                    panic!("the disc's loops hold edges");
                };
                for use_ in edges {
                    let edge = &record.edges[use_.edge as usize];
                    let Curve::Circle { frame, radius } = &edge.curve else {
                        panic!("arc edge");
                    };
                    let center = point(frame.origin);
                    let centre_uv = {
                        let offset = sub(center, origin);
                        [dot(offset, u), dot(offset, v)]
                    };
                    let [t0, t1] = edge.interval;
                    let mut previous = f64::NEG_INFINITY;
                    let mut total = 0.0;
                    for step in 0..=32 {
                        let t = t0 + (t1 - t0) * f64::from(step) / 32.0;
                        let got = solidcore_ops::pcurve::eval(&use_.pcurve, t)
                            .unwrap_or_else(|| panic!("pcurve evaluates at {t}"));
                        // Exactly on the circle: the pcurve's image is the arc, not a
                        // chord polyline.
                        let radius_uv = ((got[0] - centre_uv[0]).powi(2)
                            + (got[1] - centre_uv[1]).powi(2))
                        .sqrt();
                        assert!(
                            (radius_uv - radius).abs() < 1e-12,
                            "face {face_index} t={t}: radius {radius_uv} vs {radius}"
                        );
                        // Monotone turn about the centre, from start to end.
                        let angle = (got[1] - centre_uv[1]).atan2(got[0] - centre_uv[0]);
                        if step > 0 {
                            let mut delta = angle - previous;
                            while delta <= -std::f64::consts::PI {
                                delta += std::f64::consts::TAU;
                            }
                            total += delta;
                        }
                        previous = angle;
                        checked += 1;
                    }
                    // Signed, in the face's `(u, v)` frame: the magnitude alone
                    // would also accept a traversal running the wrong way round
                    // the arc.
                    assert!(
                        (total - std::f64::consts::PI).abs() < 1e-9,
                        "the pcurve must turn through the arc's sweep in the edge's own                          direction: {total}"
                    );
                    // And its ends are the projected endpoints the edge chains by.
                    let angle_of = |target: Point3| {
                        let offset = sub(target, center);
                        dot(offset, point(frame.y_axis)).atan2(dot(offset, point(frame.x_axis)))
                    };
                    let start_uv = solidcore_ops::pcurve::eval(&use_.pcurve, t0).expect("start");
                    let end_uv = solidcore_ops::pcurve::eval(&use_.pcurve, t1).expect("end");
                    let start_angle =
                        angle_of(point(record.vertices[edge.vertices[0] as usize].point));
                    let end_angle =
                        angle_of(point(record.vertices[edge.vertices[1] as usize].point));
                    let angle_of_uv =
                        |q: [f64; 2]| (q[1] - centre_uv[1]).atan2(q[0] - centre_uv[0]);
                    assert!(
                        (angle_of_uv(start_uv) - start_angle).abs() < 1e-12,
                        "pcurve start {start_uv:?} vs vertex angle {start_angle}"
                    );
                    assert!(
                        (angle_of_uv(end_uv) - end_angle).abs() < 1e-12,
                        "pcurve end {end_uv:?} vs vertex angle {end_angle}"
                    );
                }
            }
        }
        assert!(checked >= 4 * 33, "every use of every arc was checked");
    }

    #[test]
    fn the_second_face_may_describe_the_shared_arc_in_a_rotated_frame() {
        // One arc's point set, described by the two faces with opposite `u_dir`
        // (the same geometry, a frame rotated by 180° about the normal). The dedupe
        // key must be the arc's *point* set, not anything read off the frame, or
        // the same physical edge becomes two record edges and the closed-shell
        // check refuses the body.
        let mut flipped = flat_disc();
        let face = &mut flipped.shells[0].faces[1];
        for curve in &mut face.loops[0].edges {
            let BrepCurve::Arc { u_dir, .. } = curve else {
                panic!("the disc's boundary is arcs");
            };
            *u_dir = Point3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            };
        }
        let record = to_record(&flipped, DEFAULT_TOLERANCE)
            .expect("a rotated arc frame still names the same edge");
        assert_eq!(
            record.edges.len(),
            2,
            "one record edge per arc, not per use"
        );
        assert!(record.edges.iter().all(|edge| edge.uses.len() == 2));
        // The frame is a free choice, so the record carries whichever use created
        // the edge; what must not happen is a duplicate edge or a refusal.
        let from_flipped = from_record(&record).expect("the record reads back");
        assert_eq!(from_flipped.face_count(), 2);
    }

    #[test]
    fn an_arc_record_reads_back_as_the_same_arc() {
        let brep = flat_disc();
        let record = to_record(&brep, DEFAULT_TOLERANCE).expect("flat disc lifts");
        let back = from_record(&record).expect("the arc record reads back");
        let mut arcs = Vec::new();
        for face in &back.shells[0].faces {
            for loop_ in &face.loops {
                for curve in &loop_.edges {
                    let BrepCurve::Arc {
                        center,
                        normal,
                        u_dir,
                        radius,
                        start,
                        end,
                    } = curve
                    else {
                        panic!("expected an arc, got {curve:?}");
                    };
                    arcs.push((*center, *normal, *u_dir, *radius, *start, *end));
                }
            }
        }
        assert_eq!(arcs.len(), 4, "two faces, two arcs each");
        let close = |a: Point3, b: Point3| crate::record::length(sub(a, b)) <= 1e-12;
        for (center, normal, u_dir, radius, start, end) in &arcs {
            assert!(close(*center, p(0.0, 0.0, 0.0)));
            assert!((*radius - 1.0).abs() < 1e-12);
            assert!(close(normalize(*u_dir), p(1.0, 0.0, 0.0)));
            // Either half of the disc, oriented so the sweep runs counter-clockwise
            // about the arc's own normal.
            let upper = close(*start, p(1.0, 0.0, 0.0)) && close(*end, p(-1.0, 0.0, 0.0));
            let lower = close(*start, p(-1.0, 0.0, 0.0)) && close(*end, p(1.0, 0.0, 0.0));
            assert!(upper || lower, "{start:?} -> {end:?}");
            let turning = cross(sub(*start, *center), sub(*end, *center));
            let sign = dot(turning, *normal);
            assert!(
                sign > 0.0 || sign.abs() <= 1e-12,
                "the sweep must be counter-clockwise about `normal`"
            );
        }
    }
}
