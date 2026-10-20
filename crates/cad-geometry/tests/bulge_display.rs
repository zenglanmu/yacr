//! Bulge **display** contract: the tessellator must place a bulge arc on the
//! side the (signed) bulge encodes, in every vertex order and both bulge signs.
//!
//! Regression context: `append_bulge_arc` used to displace the arc centre to the
//! *apex* side (`mid - u * r * cos(theta/2)`). That mirrors every arc across its
//! chord, so a four-quadrant circle loop drawn as `tan(pi/8)` quarter arcs showed
//! four large arcs plus four straight overshoot segments (max radius 21.95 for a
//! radius-10 circle). A semicircle cannot see the defect (`cos(theta/2) = 0`),
//! which is why the older tests missed it.
//!
//! Conventions pinned here (measured against an independent expansion and the
//! hatch fill sampler):
//!
//! * `bulge = tan(theta/4)`; a **positive** bulge sweeps the arc counter-clockwise
//!   in the polyline plane's `+Z` frame, so its apex lies to the *right* of the
//!   chord direction and its centre to the *left*;
//! * a polyline lying in a plane parallel to world XY is 2D data: its bulges are
//!   read against `+Z`, never against the vertex winding. Reversing the vertices
//!   is therefore not a way to flip an arc — the bulge sign is.

use cad_domain::{Point3, SemanticGeometry};
use cad_geometry::{bulge_arc_geometry, tessellate_geometry, TessellationParams};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn radius_of(q: &Point3) -> f64 {
    q.x.hypot(q.y)
}

/// Four quadrant vertices of a radius-`radius` circle.
fn quadrant_vertices(radius: f64, ccw: bool) -> Vec<Point3> {
    let mut points: Vec<Point3> = (0..4)
        .map(|index| {
            let angle = index as f64 * std::f64::consts::FRAC_PI_2;
            let (sin, cos) = angle.sin_cos();
            p(radius * cos, radius * sin)
        })
        .collect();
    if !ccw {
        points.reverse();
    }
    points
}

fn closed_bulged_loop(points: Vec<Point3>, bulge: f64) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points,
        bulges: vec![bulge; 4],
        closed: true,
    }
}

fn tessellate(geometry: &SemanticGeometry) -> Vec<Point3> {
    tessellate_geometry(geometry, TessellationParams::default())
}

/// The apex radius of the *inward* quarter-arc scallop a four-quadrant loop
/// produces when its bulge signs do not match the arc side: each chord's arc is
/// centred at the opposite quadrant vertex, so it dips to `radius * (sqrt(2) - 1)`.
fn scallop_apex_radius(radius: f64) -> f64 {
    radius * (std::f64::consts::SQRT_2 - 1.0)
}

#[test]
fn four_quadrant_bulged_circle_display_stays_on_the_analytic_circle() {
    let radius = 10.0;
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    for ccw in [true, false] {
        // The correct encoding of a circle traces outward arcs: +bulge when the
        // vertices run counter-clockwise around +Z, -bulge when they run clockwise.
        let outward = if ccw { bulge } else { -bulge };
        let points = tessellate(&closed_bulged_loop(quadrant_vertices(radius, ccw), outward));
        for q in &points {
            assert!(
                (radius_of(q) - radius).abs() < 1e-9,
                "ccw={ccw} outward bulge: sample {q:?} is {} from the analytic circle",
                (radius_of(q) - radius).abs()
            );
        }
        // Endpoints are shared exactly between segments, so the ring is continuous.
        assert!(
            (radius_of(points.first().unwrap()) - radius).abs() < 1e-9
                && (radius_of(points.last().unwrap()) - radius).abs() < 1e-9
        );
    }
}

#[test]
fn four_quadrant_bulged_circle_display_never_overshoots_the_circle() {
    let radius = 10.0;
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    let anchor = scallop_apex_radius(radius);
    for ccw in [true, false] {
        for sign in [1.0, -1.0] {
            let points = tessellate(&closed_bulged_loop(
                quadrant_vertices(radius, ccw),
                sign * bulge,
            ));
            assert!(points.len() > 8);
            let (min, max) = points.iter().fold((f64::INFINITY, 0.0f64), |acc, q| {
                (acc.0.min(radius_of(q)), acc.1.max(radius_of(q)))
            });
            // No sample may leave the circle the vertices sit on: the mirrored
            // centre produced a radius of `radius + 2 * radius * cos(pi/4)`.
            assert!(
                max <= radius + 1e-9,
                "ccw={ccw} sign={sign}: a sample overshoots the circle (max radius {max})"
            );
            // Inward signs give the scallop, outward signs the circle itself.
            if ccw == (sign > 0.0) {
                assert!(
                    (min - radius).abs() < 1e-9,
                    "ccw={ccw} sign={sign}: expected the analytic circle, min radius {min}"
                );
            } else {
                assert!(
                    (min - anchor).abs() < 1e-9,
                    "ccw={ccw} sign={sign}: expected the inward scallop (apex {anchor}), \
                     min radius {min}"
                );
            }
        }
    }
}

/// Root-cause regression for the mirrored centre: a 53-degree arc (neither a
/// semicircle nor a full turn) must land on the circle its chord and signed
/// sagitta define.
#[test]
fn quarter_arc_display_centre_is_opposite_the_apex() {
    // bulge = 0.5 => theta = 4*atan(0.5) ~ 0.9273 rad; chord 2 => sagitta 0.5,
    // so the arc is the circle of radius 1.25 centred at (1, 0.75) through
    // (0, 0) and (2, 0), whose apex is (1, -0.5).
    let geometry = SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(2.0, 0.0)],
        bulges: vec![0.5, 0.0],
        closed: false,
    };
    let points = tessellate(&geometry);
    let center = p(1.0, 0.75);
    let radius = 1.25;
    for q in &points {
        let d = (q.x - center.x).hypot(q.y - center.y);
        assert!(
            (d - radius).abs() < 1e-9,
            "sample {q:?} is not on the analytic arc circle (distance {d})"
        );
    }
    let apex_y = points
        .iter()
        .filter(|q| (q.x - 1.0).abs() < 1e-9)
        .map(|q| q.y)
        .fold(f64::INFINITY, f64::min);
    assert!(
        (apex_y + 0.5).abs() < 1e-9,
        "a positive bulge must bow away from the centre side, apex y = {apex_y}"
    );
    // The mirror image of the same arc (negative bulge) bows to the other side.
    let mirrored = SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(2.0, 0.0)],
        bulges: vec![-0.5, 0.0],
        closed: false,
    };
    let apex_y = tessellate(&mirrored)
        .iter()
        .filter(|q| (q.x - 1.0).abs() < 1e-9)
        .map(|q| q.y)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (apex_y - 0.5).abs() < 1e-9,
        "a negative bulge must bow to the other side, apex y = {apex_y}"
    );
}

/// `EXPLODE` recovers an analytic `Arc` from the same bulge; it must name the
/// same centre the display draws, otherwise exploding a bulged polyline moves it.
#[test]
fn exploded_arc_matches_the_displayed_bulge_arc() {
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let arc = bulge_arc_geometry(p(0.0, 0.0), p(2.0, 0.0), 0.5, normal)
        .expect("a nonzero bulge is an arc");
    let SemanticGeometry::Arc {
        center,
        radius,
        start,
        sweep,
        ..
    } = arc
    else {
        panic!("expected an Arc");
    };
    assert!((center.x - 1.0).abs() < 1e-9 && (center.y - 0.75).abs() < 1e-9);
    assert!((radius - 1.25).abs() < 1e-9);
    assert!((sweep - 4.0 * 0.5f64.atan()).abs() < 1e-12);

    // The recovered arc samples onto the same circle, through the same apex.
    let samples = tessellate(&SemanticGeometry::Arc {
        center,
        normal,
        radius,
        start,
        sweep,
    });
    let apex_y = samples
        .iter()
        .filter(|q| (q.x - 1.0).abs() < 1e-9)
        .map(|q| q.y)
        .fold(f64::INFINITY, f64::min);
    assert!(
        (apex_y + 0.5).abs() < 1e-9,
        "the exploded arc bows the wrong way, apex y = {apex_y}"
    );
}

/// The hatch **boundary display** and the hatch **fill** must read a loop's
/// bulges in the same frame. They used to diverge for a clockwise-wound loop,
/// because the display re-derived the plane normal from the winding while the
/// fill used the stored hatch plane (`+Z`), mirroring every arc of one of them.
#[test]
fn hatch_boundary_display_and_fill_agree_for_either_winding() {
    use cad_domain::{
        HatchFill, HatchGeometry, HatchLoop, HatchLoopFlags, HatchPatternType, HatchPlane,
        HatchStyle,
    };

    let radius = 10.0;
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    for ccw in [true, false] {
        for sign in [1.0, -1.0] {
            let vertices = quadrant_vertices(radius, ccw);
            let loop_ = HatchLoop::new(
                vertices.iter().map(|q| [q.x, q.y]).collect(),
                vec![sign * bulge; 4],
                HatchLoopFlags::default(),
            );
            let hatch = HatchGeometry::new(
                vec![loop_],
                HatchPlane::default(),
                HatchStyle::Normal,
                HatchFill::Solid,
                Vec::new(),
                None,
                HatchPatternType::UserDefined,
            )
            .expect("a bulged quadrant loop is a valid boundary");

            let mut boundary = None;
            let mut fill = Vec::new();
            for geometry in hatch.display() {
                match geometry {
                    SemanticGeometry::Polyline { .. } => {
                        boundary = Some(tessellate(geometry));
                    }
                    SemanticGeometry::Mesh(mesh) => fill.extend(mesh.vertices.iter().copied()),
                    other => panic!("unexpected hatch display child: {other:?}"),
                }
            }
            let boundary = boundary.expect("the boundary polyline is always displayed");
            assert!(!fill.is_empty(), "a solid hatch must produce a fill mesh");
            let expect_circle = ccw == (sign > 0.0);
            for (what, samples) in [("boundary", &boundary), ("fill", &fill)] {
                let (min, max) = samples.iter().fold((f64::INFINITY, 0.0f64), |acc, q| {
                    (acc.0.min(radius_of(q)), acc.1.max(radius_of(q)))
                });
                assert!(
                    max <= radius + 1e-6,
                    "ccw={ccw} sign={sign} {what}: overshoots the circle (max radius {max})"
                );
                if expect_circle {
                    assert!(
                        (min - radius).abs() < 1e-6,
                        "ccw={ccw} sign={sign} {what}: not the analytic circle (min radius {min})"
                    );
                } else {
                    assert!(
                        (min - scallop_apex_radius(radius)).abs() < 1e-6,
                        "ccw={ccw} sign={sign} {what}: not the inward scallop (min radius {min})"
                    );
                }
            }
        }
    }
}
