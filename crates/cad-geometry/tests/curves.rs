//! Curve semantics contract tests (spec §16.1, audit B23) for ellipses on
//! arbitrary planes, non-uniform affine conics, bulge preservation and
//! tessellation-independent curve intersections.

use cad_domain::*;
use cad_geometry::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn pz(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn non_uniform(sx: f64, sy: f64) -> Transform3 {
    let mut m = Transform3::identity().matrix;
    m[0][0] = sx;
    m[1][1] = sy;
    Transform3 { matrix: m }
}

fn default_params() -> TessellationParams {
    TessellationParams {
        tolerance: 1e-4,
        max_segments: 8192,
        min_segments: 8,
    }
}

fn max_abs(v: f64) -> f64 {
    v.abs()
}

// ---------------------------------------------------------------------------
// Ellipse on an arbitrary plane (stored normal, not cross(world_z, major)).
// ---------------------------------------------------------------------------

#[test]
fn ellipse_keeps_its_arbitrary_normal_plane() {
    // Plane normal +X: the ellipse lies in the Y-Z plane, major along Y.
    let ellipse = SemanticGeometry::Ellipse {
        center: pz(0.0, 0.0, 0.0),
        normal: pz(1.0, 0.0, 0.0),
        major_axis: pz(0.0, 3.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    // A very fine discretisation so the sampled extrema reach the true axes.
    let params = TessellationParams {
        tolerance: 1e-9,
        max_segments: 200_000,
        min_segments: 8,
    };
    let pts = tessellate_geometry(&ellipse, params);
    assert!(pts.len() > 8);
    let max_off_plane = pts.iter().map(|q| q.x.abs()).fold(0.0, f64::max);
    assert!(max_off_plane < 1e-7, "left its plane by {max_off_plane}");
    let max_y = pts.iter().map(|q| q.y.abs()).fold(0.0, f64::max);
    let max_z = pts.iter().map(|q| q.z.abs()).fold(0.0, f64::max);
    assert!((max_y - 3.0).abs() < 1e-6, "major preserved: {max_y}");
    assert!((max_z - 1.5).abs() < 1e-6, "minor/ratio preserved: {max_z}");
}

/// Implicit test: `q` lies on the ellipse described by the domain convention.
fn on_ellipse(
    q: Point3,
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    tol: f64,
) -> bool {
    let n = normalize(normal);
    let a = length(major_axis);
    if a < 1e-12 {
        return false;
    }
    let u = normalize(major_axis);
    let m = normalize(cross(n, u));
    let d = sub(q, center);
    if dot(d, n).abs() > tol {
        return false;
    }
    let du = dot(d, u) / a;
    let dv = dot(d, m) / (a * ratio);
    (du * du + dv * dv - 1.0).abs() <= tol
}

#[test]
fn ellipse_round_trips_through_a_non_uniform_transform() {
    // Any affine image of an ellipse is an ellipse; the points of the source
    // ellipse mapped directly must lie on the reported image.
    let src = SemanticGeometry::Ellipse {
        center: pz(1.0, -2.0, 4.0),
        normal: pz(0.0, 0.0, 1.0),
        major_axis: pz(4.0, 1.0, 0.0),
        ratio: 0.4,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    let t = non_uniform(2.0, 0.5);
    let out = DefaultGeometryEngine.transform(&src, &t).unwrap();
    let (center, normal, major_axis, ratio) = match &out {
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            ..
        } => (*center, *normal, *major_axis, *ratio),
        other => panic!("expected ellipse, got {other:?}"),
    };
    // Directly sample the source, map it, and check it satisfies the reported
    // ellipse's implicit equation. This proves exactness, not sampling density.
    let (cx, cy) = (1.0, -2.0);
    let (ux, uy) = (4.0, 1.0);
    for i in 0..64 {
        let th = std::f64::consts::TAU * (i as f64) / 64.0;
        // minor = ratio * cross(Z, major) = ratio * (-uy, ux)
        let mx = 0.4 * -uy;
        let my = 0.4 * ux;
        let sx = cx + ux * th.cos() + mx * th.sin();
        let sy = cy + uy * th.cos() + my * th.sin();
        let mapped = t.apply_point(pz(sx, sy, 4.0));
        assert!(
            on_ellipse(mapped, center, normal, major_axis, ratio, 1e-9),
            "θ={th}: {mapped:?} not on image ellipse"
        );
    }
}

#[test]
fn non_uniform_transform_of_tilted_circle_yields_a_planar_ellipse() {
    // A circle in the X-Z plane (normal +Y) scaled non-uniformly must become an
    // ellipse in that same mapped plane, not a circle and not folded onto XY.
    let circle = SemanticGeometry::Circle {
        center: pz(0.0, 0.0, 0.0),
        normal: pz(0.0, 1.0, 0.0),
        radius: 2.0,
    };
    let out = DefaultGeometryEngine
        .transform(&circle, &non_uniform(3.0, 1.0))
        .unwrap();
    match &out {
        SemanticGeometry::Ellipse { ratio, normal, .. } => {
            assert!((ratio - 1.0 / 3.0).abs() < 1e-9, "ratio {ratio}");
            assert!(normal.y.abs() > 0.99, "normal {normal:?}");
        }
        other => panic!("expected ellipse, got {other:?}"),
    }
    let pts = tessellate_geometry(&out, default_params());
    assert!(pts.len() > 8);
    // The image plane normal is the transformed +Y, so the circle's plane
    // becomes the plane y = 0 and every point has y = 0.
    let max_y = pts.iter().map(|q| q.y.abs()).fold(0.0, f64::max);
    assert!(max_y < 1e-6, "left the plane: {max_y}");
}

// ---------------------------------------------------------------------------
// Non-uniform scaling must not straighten a bulge.
// ---------------------------------------------------------------------------

#[test]
fn bulge_survives_non_uniform_scaling_as_an_elliptical_arc() {
    // A two-vertex polyline with bulge 1 is a semicircle sagging to (1, -1).
    // Scaling (2, 1) turns it into a half ellipse whose apex is (2, -1).
    let poly = SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(2.0, 0.0)],
        bulges: vec![1.0, 0.0],
        closed: false,
    };
    let out = DefaultGeometryEngine
        .transform(&poly, &non_uniform(2.0, 1.0))
        .unwrap();
    let pts = tessellate_geometry(&out, default_params());
    assert!(pts.len() > 4, "bulge was straightened: {pts:?}");
    let min_y = pts.iter().map(|q| q.y).fold(f64::INFINITY, f64::min);
    assert!((min_y + 1.0).abs() < 1e-6, "apex y = {min_y}");
    // Endpoints are preserved exactly.
    let has_start = pts.iter().any(|q| distance(*q, p(0.0, 0.0)) < 1e-9);
    let has_end = pts.iter().any(|q| distance(*q, p(4.0, 0.0)) < 1e-9);
    assert!(has_start && has_end, "endpoints moved: {pts:?}");
}

#[test]
fn uniform_rotation_shifts_arc_start_angle() {
    // A 90° rotation about Z maps an arc starting at (2, 0) to one starting at
    // (0, 2); the stored start angle must shift with the OCS frame.
    let arc = SemanticGeometry::Arc {
        center: pz(0.0, 0.0, 0.0),
        normal: pz(0.0, 0.0, 1.0),
        radius: 2.0,
        start: 0.0,
        sweep: std::f64::consts::FRAC_PI_2,
    };
    let mut m = Transform3::identity().matrix;
    m[0][0] = 0.0;
    m[0][1] = -1.0;
    m[1][0] = 1.0;
    m[1][1] = 0.0;
    let rot = Transform3 { matrix: m };
    let out = DefaultGeometryEngine.transform(&arc, &rot).unwrap();
    match &out {
        SemanticGeometry::Arc { start, .. } => {
            assert!(
                (start - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
                "start {start}"
            );
        }
        other => panic!("expected arc, got {other:?}"),
    }
    let pts = tessellate_geometry(&out, default_params());
    assert!(distance(pts[0], p(0.0, 2.0)) < 1e-9, "first {pts:?}");
}

#[test]
fn reflection_flips_bulge_side() {
    // Mirroring across the X axis must move the sagging arc above the chord.
    let poly = SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(2.0, 0.0)],
        bulges: vec![1.0, 0.0],
        closed: false,
    };
    let mut m = Transform3::identity().matrix;
    m[1][1] = -1.0;
    let mirror = Transform3 { matrix: m };
    let out = DefaultGeometryEngine.transform(&poly, &mirror).unwrap();
    let pts = tessellate_geometry(&out, default_params());
    let max_y = pts.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max);
    assert!((max_y - 1.0).abs() < 1e-6, "max_y = {max_y}");
}

// ---------------------------------------------------------------------------
// Analytic curve intersections.
// ---------------------------------------------------------------------------

fn circle_at(center: Point3, radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center,
        normal: pz(0.0, 0.0, 1.0),
        radius,
    }
}

#[test]
fn segment_circle_intersection_is_analytic() {
    let line = SemanticGeometry::Line {
        start: p(-10.0, 3.0),
        end: p(10.0, 3.0),
    };
    let circle = circle_at(p(0.0, 0.0), 5.0);
    let hits = DefaultGeometryEngine
        .intersect_local(&line, &circle, &TolerancePolicy::default())
        .unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().any(|q| distance(*q, p(-4.0, 3.0)) < 1e-9));
    assert!(hits.iter().any(|q| distance(*q, p(4.0, 3.0)) < 1e-9));
}

#[test]
fn segment_arc_intersection_respects_the_sweep() {
    let line_through = SemanticGeometry::Line {
        start: p(-10.0, 3.0),
        end: p(10.0, 3.0),
    };
    let upper = SemanticGeometry::Arc {
        center: p(0.0, 0.0),
        normal: pz(0.0, 0.0, 1.0),
        radius: 5.0,
        start: 0.0,
        sweep: std::f64::consts::PI,
    };
    let hits = DefaultGeometryEngine
        .intersect_local(&line_through, &upper, &TolerancePolicy::default())
        .unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");

    let below = SemanticGeometry::Line {
        start: p(-10.0, -3.0),
        end: p(10.0, -3.0),
    };
    let none = DefaultGeometryEngine
        .intersect_local(&below, &upper, &TolerancePolicy::default())
        .unwrap();
    assert!(none.is_empty(), "arc outside its sweep: {none:?}");
}

#[test]
fn circle_circle_intersection_is_analytic() {
    let a = circle_at(p(0.0, 0.0), 5.0);
    let b = circle_at(p(6.0, 0.0), 5.0);
    let hits = DefaultGeometryEngine
        .intersect_local(&a, &b, &TolerancePolicy::default())
        .unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().all(|q| (q.x - 3.0).abs() < 1e-9));
    assert!(hits.iter().any(|q| (q.y - 4.0).abs() < 1e-9));
    assert!(hits.iter().any(|q| (q.y + 4.0).abs() < 1e-9));
}

#[test]
fn segment_ellipse_intersection_is_analytic() {
    let ellipse = SemanticGeometry::Ellipse {
        center: p(0.0, 0.0),
        normal: pz(0.0, 0.0, 1.0),
        major_axis: p(5.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    let line = SemanticGeometry::Line {
        start: p(-10.0, 0.0),
        end: p(10.0, 0.0),
    };
    let hits = DefaultGeometryEngine
        .intersect_local(&line, &ellipse, &TolerancePolicy::default())
        .unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().any(|q| distance(*q, p(-5.0, 0.0)) < 1e-9));
    assert!(hits.iter().any(|q| distance(*q, p(5.0, 0.0)) < 1e-9));
}

#[test]
fn intersection_is_independent_of_display_lod_for_curves() {
    // The same circle/segment pair intersected under a tiny and a huge display
    // pixel budget must produce exactly the same measured points; only
    // `display_pixels` changes (audit B23/F06).
    let line = SemanticGeometry::Line {
        start: p(-10.0, 3.5),
        end: p(10.0, 3.5),
    };
    let circle = circle_at(p(0.0, 0.0), 5.0);
    let coarse = TolerancePolicy {
        display_pixels: 0.001,
        ..TolerancePolicy::default()
    };
    let fine = TolerancePolicy {
        display_pixels: 500.0,
        ..TolerancePolicy::default()
    };
    let a = DefaultGeometryEngine
        .intersect_local(&line, &circle, &coarse)
        .unwrap();
    let b = DefaultGeometryEngine
        .intersect_local(&line, &circle, &fine)
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(a.len(), 2);
}

#[test]
fn analytic_intersection_matches_sampled_intersection() {
    // A circle and an arc that do not share a centre exercises the analytic
    // path against the sampling fallback for a spline nearby.
    let circle = circle_at(p(0.0, 0.0), 5.0);
    let arc = SemanticGeometry::Arc {
        center: p(3.0, 0.0),
        normal: pz(0.0, 0.0, 1.0),
        radius: 4.0,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    let hits = DefaultGeometryEngine
        .intersect_local(&circle, &arc, &TolerancePolicy::default())
        .unwrap();
    assert_eq!(hits.len(), 2, "{hits:?}");
    // Both points are on both circles.
    for q in &hits {
        assert!(max_abs(distance(*q, p(0.0, 0.0)) - 5.0) < 1e-9);
        assert!(max_abs(distance(*q, p(3.0, 0.0)) - 4.0) < 1e-9);
    }
}
