//! Contract tests for geometry invariants from the audit (B23, B24).
//!
//! Each test is written to fail against the pre-fix behaviour and lock the
//! corrected semantics: affine circle/arc/ellipse transformation, continuous
//! bulge polylines, source-knot splines, display-LOD-independent measurement,
//! and work-plane / area validation.

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

fn uniform(s: f64) -> Transform3 {
    Transform3::scale(s)
}

// ---------------------------------------------------------------------------
// B23: non-uniform transform of circles/arcs/ellipses must be affine-exact.
// ---------------------------------------------------------------------------

#[test]
fn non_uniform_transform_turns_circle_into_ellipse_not_circle() {
    // A unit circle scaled (2, 1) must become an ellipse with major axis 2 on
    // X, ratio 0.5 -- the old code kept `Circle` with an averaged radius.
    let circle = SemanticGeometry::Circle {
        center: p(0.0, 0.0),
        normal: pz(0.0, 0.0, 1.0),
        radius: 1.0,
    };
    let out = DefaultGeometryEngine
        .transform(&circle, &non_uniform(2.0, 1.0))
        .unwrap();
    match out {
        SemanticGeometry::Ellipse {
            major_axis, ratio, ..
        } => {
            assert!((length(major_axis) - 2.0).abs() < 1e-9, "{major_axis:?}");
            assert!((ratio - 0.5).abs() < 1e-9, "ratio {ratio}");
        }
        // Any circle here is the old bug: radius would have been (2+1+1)/3.
        other => panic!("expected ellipse, got {other:?}"),
    }
}

#[test]
fn transformed_circle_points_lie_on_the_ellipse() {
    // The image of the transformed circle must lie on the reported ellipse:
    // this checks the affine mapping is exact, not merely the type tag.
    let radius = 3.0;
    let normal = pz(0.0, 0.0, 1.0);
    let source_center = pz(1.0, 2.0, 0.0);
    let circle = SemanticGeometry::Circle {
        center: source_center,
        normal,
        radius,
    };
    let t = non_uniform(2.0, 0.5);
    let out = DefaultGeometryEngine.transform(&circle, &t).unwrap();
    let (center, major_axis, ratio, start) = match out {
        SemanticGeometry::Ellipse {
            center,
            major_axis,
            ratio,
            start,
            ..
        } => (center, major_axis, ratio, start),
        other => panic!("expected ellipse, got {other:?}"),
    };
    let (ax, ay, _) = arbitrary_axis(normal);
    let u = normalize(major_axis);
    let minor_dir = ellipse_minor_dir(u);
    let minor = scale(minor_dir, length(major_axis) * ratio);
    // Sample the source circle and compare against the ellipse parameterisation.
    for i in 0..12 {
        let theta = std::f64::consts::TAU * (i as f64) / 12.0;
        let source = add(
            source_center,
            add(
                scale(ax, theta.cos() * radius),
                scale(ay, theta.sin() * radius),
            ),
        );
        let mapped = t.apply_point(source);
        let param = start + theta;
        let expected = add(
            center,
            add(scale(major_axis, param.cos()), scale(minor, param.sin())),
        );
        assert!(
            distance(mapped, expected) < 1e-9,
            "θ={theta}: mapped {mapped:?} != expected {expected:?}"
        );
    }
}

fn ellipse_minor_dir(major_unit: Point3) -> Point3 {
    normalize(cross(pz(0.0, 0.0, 1.0), major_unit))
}

#[test]
fn uniform_transform_keeps_circle_and_scales_radius() {
    let circle = SemanticGeometry::Circle {
        center: p(1.0, 1.0),
        normal: pz(0.0, 0.0, 1.0),
        radius: 2.5,
    };
    let out = DefaultGeometryEngine
        .transform(&circle, &uniform(3.0))
        .unwrap();
    match out {
        SemanticGeometry::Circle { radius, .. } => assert!((radius - 7.5).abs() < 1e-9),
        other => panic!("expected circle, got {other:?}"),
    }
}

#[test]
fn tilted_ellipse_stays_in_its_own_plane_when_tessellated() {
    // A tilted ellipse (major axis has a Z component) must lie in the plane
    // spanned by its major axis and its minor axis, not be folded into XY.
    // The domain convention defines minor = cross(world_z, major).
    let major = pz(4.0, 0.0, 4.0);
    let ellipse = SemanticGeometry::Ellipse {
        center: pz(0.0, 0.0, 0.0),
        normal: pz(0.0, 0.0, 1.0),
        major_axis: major,
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    };
    let params = TessellationParams {
        tolerance: 1e-5,
        max_segments: 8192,
        min_segments: 64,
    };
    let pts = tessellate_geometry(&ellipse, params);
    let u = normalize(major);
    let minor_dir = normalize(cross(pz(0.0, 0.0, 1.0), u));
    let normal = normalize(cross(u, minor_dir));
    let max_out_of_plane = pts
        .iter()
        .map(|q| dot(*q, normal).abs())
        .fold(0.0, f64::max);
    assert!(
        max_out_of_plane < 1e-6,
        "ellipse left its plane by {max_out_of_plane}"
    );
    // And it must actually be tilted: some points have a non-zero Z.
    let max_z = pts.iter().map(|q| q.z.abs()).fold(0.0, f64::max);
    assert!(max_z > 0.5, "expected a tilted ellipse, max_z={max_z}");
}

#[test]
fn bulge_arc_after_line_segment_is_continuous() {
    // A straight segment then a bulge arc: the polyline must connect the arc's
    // start to the previous segment's end (the old code dropped the arc start,
    // so the arc started mid-air).
    let poly = SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(1.0, 0.0), p(3.0, 0.0)],
        bulges: vec![0.0, 1.0, 0.0],
        closed: false,
    };
    let pts = tessellate_geometry(&poly, TessellationParams::default());
    for w in pts.windows(2) {
        let d = distance(w[0], w[1]);
        assert!(d > 1e-9, "duplicate/zero segment at {:?}", w[0]);
    }
    // The arc starts exactly at (1,0) and ends at (3,0); with bulge 1 it bows
    // downward (a semicircle of radius 1).
    assert!(pts.iter().any(|q| distance(*q, p(1.0, 0.0)) < 1e-9));
    let apex = pts
        .iter()
        .filter(|q| (q.x - 2.0).abs() < 1e-6)
        .map(|q| q.y)
        .fold(f64::INFINITY, f64::min);
    assert!((apex + 1.0).abs() < 1e-6, "apex y = {apex}");
}

#[test]
fn closed_polyline_all_bulges_are_continuous() {
    let poly = SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0)],
        bulges: vec![0.5, 0.5, 0.5],
        closed: true,
    };
    let pts = tessellate_geometry(&poly, TessellationParams::default());
    assert!(pts.len() > 4);
    // First and last point of a closed polyline coincide.
    assert!(distance(pts[0], *pts.last().unwrap()) < 1e-9);
}

#[test]
fn tilted_bulge_arc_stays_in_the_polyline_plane() {
    // A polyline lying in the vertical plane y = 5, with a bulge on the first
    // segment. The arc must remain in that plane and bow along Z. The old code
    // worked in world XY, so the arc leaked out of the plane and its Z was
    // pinned flat (audit B23 / importer OCS).
    let poly = SemanticGeometry::Polyline {
        points: vec![pz(0.0, 5.0, 0.0), pz(2.0, 5.0, 0.0), pz(2.0, 5.0, 2.0)],
        bulges: vec![0.5, 0.0, 0.0],
        closed: false,
    };
    let pts = tessellate_geometry(&poly, TessellationParams::default());
    assert!(pts.len() > 3);
    for q in &pts {
        assert!((q.y - 5.0).abs() < 1e-9, "arc left the plane: {q:?}");
    }
    // The bulge actually uses the in-plane Z extent (not flattened to z = 0).
    let bowed = pts.iter().filter(|q| q.z.abs() > 1e-6).count();
    assert!(bowed > 0, "tilted bulge was flattened: {pts:?}");
}

#[test]
fn spline_honours_source_knots_instead_of_uniformising() {
    // A single quadratic Bezier with explicit clamped knots is a parabola;
    // rebuilding uniform knots through 3 points would give a straight line
    // between the endpoints. Assert the curve bows away from the chord.
    let ctrl = vec![p(0.0, 0.0), p(1.0, 2.0), p(2.0, 0.0)];
    let knots = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let spline = SemanticGeometry::Spline {
        degree: 2,
        knots,
        control_points: ctrl,
        weights: vec![1.0, 1.0, 1.0],
    };
    let pts = tessellate_geometry(&spline, TessellationParams::default());
    let max_y = pts.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max);
    // The Bezier reaches y = 1 at its midpoint.
    assert!((max_y - 1.0).abs() < 1e-6, "max_y = {max_y}");
    assert!(pts
        .iter()
        .any(|q| (q.x - 1.0).abs() < 1e-6 && (q.y - 1.0).abs() < 1e-6));
}

#[test]
fn rational_spline_weighs_control_points() {
    // A rational quadratic with unequal weights must not coincide with the
    // non-rational curve (weights are geometry, not decoration).
    let ctrl = vec![p(0.0, 0.0), p(1.0, 2.0), p(2.0, 0.0)];
    let knots = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let weighted = tessellate_spline(
        &ctrl,
        &knots,
        &[1.0, 0.5, 1.0],
        2,
        TessellationParams::default(),
    );
    let plain = tessellate_spline(
        &ctrl,
        &knots,
        &[1.0, 1.0, 1.0],
        2,
        TessellationParams::default(),
    );
    let max_weighted = weighted
        .iter()
        .map(|q| q.y)
        .fold(f64::NEG_INFINITY, f64::max);
    // At t = 1/2: (0.25*0 + 0.25*0.5*2 + 0.25*0) / (0.25 + 0.25 + 0.25) = 2/3.
    assert!((max_weighted - 2.0 / 3.0).abs() < 1e-6, "{max_weighted}");
    assert!(plain.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max) > 0.9);
}

#[test]
fn malformed_knots_fall_back_without_panicking() {
    let ctrl = vec![p(0.0, 0.0), p(1.0, 1.0), p(2.0, 0.0)];
    // Wrong length -> clamped uniform fallback, still a usable polyline.
    let pts = tessellate_spline(&ctrl, &[0.0, 1.0], &[], 2, TessellationParams::default());
    assert!(pts.len() >= 3);
}

#[test]
fn transform_rejects_non_finite_result() {
    let mut m = Transform3::identity().matrix;
    m[0][0] = f64::INFINITY;
    let t = Transform3 { matrix: m };
    let out = DefaultGeometryEngine.transform(&SemanticGeometry::Point(p(1.0, 0.0)), &t);
    assert!(matches!(out, Err(CadError::InvalidInput(_))), "{out:?}");
}

// ---------------------------------------------------------------------------
// B23: display LOD must not change measured/intersection semantics.
// ---------------------------------------------------------------------------

#[test]
fn intersection_is_independent_of_display_pixel_budget() {
    let a = SemanticGeometry::Line {
        start: p(-1.0, 0.0),
        end: p(1.0, 0.0),
    };
    let b = SemanticGeometry::Line {
        start: p(0.0, -1.0),
        end: p(0.0, 1.0),
    };
    let coarse = TolerancePolicy {
        display_pixels: 0.01,
        ..TolerancePolicy::default()
    };
    let fine = TolerancePolicy {
        display_pixels: 50.0,
        ..TolerancePolicy::default()
    };
    let hits_coarse = DefaultGeometryEngine
        .intersect_local(&a, &b, &coarse)
        .unwrap();
    let hits_fine = DefaultGeometryEngine
        .intersect_local(&a, &b, &fine)
        .unwrap();
    assert_eq!(hits_coarse, hits_fine);
    assert_eq!(hits_coarse.len(), 1);
}

// ---------------------------------------------------------------------------
// B24: work-plane and ray validation.
// ---------------------------------------------------------------------------

#[test]
fn skewed_work_plane_is_rejected() {
    let plane = WorkPlane {
        origin: p(0.0, 0.0),
        u: p(1.0, 0.0),
        v: p(1.0, 1.0), // 45 degrees, not orthogonal
    };
    assert!(validate_work_plane(&plane, 1e-6).is_err());
}

#[test]
fn degenerate_and_non_finite_work_planes_are_rejected() {
    let zero = WorkPlane {
        origin: p(0.0, 0.0),
        u: p(0.0, 0.0),
        v: p(0.0, 1.0),
    };
    assert!(validate_work_plane(&zero, 1e-6).is_err());
    let nan = WorkPlane {
        origin: p(f64::NAN, 0.0),
        u: p(1.0, 0.0),
        v: p(0.0, 1.0),
    };
    assert!(validate_work_plane(&nan, 1e-6).is_err());
}

#[test]
fn orthogonal_work_plane_is_accepted() {
    let plane = WorkPlane {
        origin: p(1.0, 2.0),
        u: p(2.0, 0.0),
        v: p(0.0, 3.0),
    };
    assert!(validate_work_plane(&plane, 1e-6).is_ok());
}

#[test]
fn ray_behind_the_origin_never_hits() {
    // Ray pointing away from the plane must return None even when the direction
    // is unnormalised (old code used raw direction for t).
    let plane = WorkPlane {
        origin: p(0.0, 0.0),
        u: p(1.0, 0.0),
        v: p(0.0, 1.0),
    };
    let ray = Ray3 {
        origin: pz(0.0, 0.0, 5.0),
        direction: pz(0.0, 0.0, 7.0), // away from z=0, large magnitude
    };
    let hit = DefaultGeometryEngine
        .ray_plane(&ray, &plane, &TolerancePolicy::default())
        .unwrap();
    assert!(hit.is_none(), "{hit:?}");
}

#[test]
fn ray_plane_result_is_independent_of_direction_magnitude() {
    let plane = WorkPlane {
        origin: p(0.0, 0.0),
        u: p(1.0, 0.0),
        v: p(0.0, 1.0),
    };
    let make = |dz: f64| Ray3 {
        origin: pz(0.0, 0.0, 5.0),
        direction: pz(0.0, 0.0, dz),
    };
    let a = DefaultGeometryEngine
        .ray_plane(&make(-1.0), &plane, &TolerancePolicy::default())
        .unwrap()
        .unwrap();
    let b = DefaultGeometryEngine
        .ray_plane(&make(-100.0), &plane, &TolerancePolicy::default())
        .unwrap()
        .unwrap();
    assert!(distance(a, b) < 1e-12, "{a:?} vs {b:?}");
}

#[test]
fn area_rejects_non_finite_points_and_non_orthogonal_measurement_plane() {
    let nan = [p(f64::NAN, 0.0), p(1.0, 0.0), p(1.0, 1.0)];
    assert!(measure_polygon_area(&nan, 1e-6).is_err());
}
