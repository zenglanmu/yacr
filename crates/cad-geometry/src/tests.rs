//! Unit tests.

use super::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn line_bounds_are_exact() {
    let g = SemanticGeometry::Line {
        start: p(1.0, 2.0),
        end: p(5.0, -3.0),
    };
    let b = DefaultGeometryEngine.bounds(&g).unwrap();
    assert_eq!(b.min, p(1.0, -3.0));
    assert_eq!(b.max, p(5.0, 2.0));
}

#[test]
fn circle_tessellation_respects_tolerance() {
    let g = SemanticGeometry::Circle {
        center: p(0.0, 0.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        radius: 100.0,
    };
    let coarse = tessellate_geometry(
        &g,
        TessellationParams {
            tolerance: 1.0,
            max_segments: 4096,
            min_segments: 8,
        },
    );
    let fine = tessellate_geometry(
        &g,
        TessellationParams {
            tolerance: 0.01,
            max_segments: 4096,
            min_segments: 8,
        },
    );
    assert!(fine.len() > coarse.len());
    // All points lie on the circle.
    for q in &fine {
        assert!((length(*q) - 100.0).abs() < 1e-6);
    }
}

#[test]
fn bulge_semicircle_hits_lower_apex_for_ccw() {
    let pts = vec![p(0.0, 0.0), p(2.0, 0.0)];
    let out = polyline_with_bulges(&pts, &[1.0, 0.0], false, TessellationParams::default());
    let apex = out.iter().find(|q| (q.x - 1.0).abs() < 1e-6).unwrap();
    assert!((apex.y + 1.0).abs() < 1e-6, "apex y = {}", apex.y);
}

#[test]
fn transform_round_trips_translation() {
    let mut m = [[0.0f64; 4]; 4];
    m[0][0] = 1.0;
    m[1][1] = 1.0;
    m[2][2] = 1.0;
    m[3][3] = 1.0;
    m[0][3] = 10.0;
    m[1][3] = 20.0;
    let t = Transform3 { matrix: m };
    let g = SemanticGeometry::Point(p(1.0, 1.0));
    let out = DefaultGeometryEngine.transform(&g, &t).unwrap();
    assert_eq!(out, SemanticGeometry::Point(p(11.0, 21.0)));
}

#[test]
fn ray_hits_work_plane() {
    let plane = WorkPlane {
        origin: p(0.0, 0.0),
        u: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        v: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    };
    let ray = Ray3 {
        origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 5.0,
        },
        direction: Point3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
    };
    let hit = DefaultGeometryEngine
        .ray_plane(&ray, &plane, &TolerancePolicy::default())
        .unwrap();
    assert!(hit.is_some());
    assert!(distance(hit.unwrap(), p(0.0, 0.0)) < 1e-9);
}

#[test]
fn local_intersection_finds_crossing_lines() {
    let a = SemanticGeometry::Line {
        start: p(-1.0, 0.0),
        end: p(1.0, 0.0),
    };
    let b = SemanticGeometry::Line {
        start: p(0.0, -1.0),
        end: p(0.0, 1.0),
    };
    let hits = DefaultGeometryEngine
        .intersect_local(&a, &b, &TolerancePolicy::default())
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(distance(hits[0], p(0.0, 0.0)) < 1e-9);
}

#[test]
fn infinite_line_intersection_distinguishes_point_parallel_and_collinear() {
    let tol = 1e-9;
    // Two crossing lines meet at exactly one point.
    match infinite_line_intersection(p(-1.0, 0.0), p(1.0, 0.0), p(0.0, -1.0), p(0.0, 1.0), tol) {
        LineIntersection::Point(c) => {
            assert!(c.x.abs() < 1e-9 && c.y.abs() < 1e-9, "{c:?}");
        }
        other => panic!("crossing lines must report a point, got {other:?}"),
    }
    // Parallel, distinct lines: no intersection (never a fabricated corner).
    assert_eq!(
        infinite_line_intersection(p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0), p(1.0, 1.0), tol),
        LineIntersection::Parallel
    );
    // Collinear lines share infinitely many points: distinct from Parallel.
    assert_eq!(
        infinite_line_intersection(p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0), p(3.0, 0.0), tol),
        LineIntersection::Collinear
    );
    // The extension case FILLET/CHAMFER need: the segments do not touch, but the
    // infinite lines cross at (2, 0).
    match infinite_line_intersection(p(0.0, 0.0), p(1.0, 0.0), p(2.0, 3.0), p(2.0, 1.0), tol) {
        LineIntersection::Point(c) => {
            assert!((c.x - 2.0).abs() < 1e-9, "corner x = {}", c.x);
            assert!(c.y.abs() < 1e-9, "corner y = {}", c.y);
        }
        other => panic!("the lines cross beyond the segments, got {other:?}"),
    }
    // A degenerate (zero-length) line has no direction: reported as Parallel.
    assert_eq!(
        infinite_line_intersection(p(1.0, 1.0), p(1.0, 1.0), p(0.0, 0.0), p(1.0, 0.0), tol),
        LineIntersection::Parallel
    );
}

#[test]
fn bulge_arc_geometry_recovers_a_semicircle_and_has_no_zero_bulge_arc() {
    // bulge = 1 ⇒ tan(θ/4) = 1 ⇒ θ = π: a unit-radius semicircle over a 2-unit
    // chord. The recovered arc must pass through the two chord endpoints.
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let arc = bulge_arc_geometry(p(0.0, 0.0), p(2.0, 0.0), 1.0, normal)
        .expect("a nonzero bulge is an arc");
    match arc {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => {
            assert!((radius - 1.0).abs() < 1e-9, "radius = {radius}");
            assert!(
                (sweep.abs() - std::f64::consts::PI).abs() < 1e-9,
                "sweep = {sweep}"
            );
            let at = |angle: f64| Point3 {
                x: center.x + radius * angle.cos(),
                y: center.y + radius * angle.sin(),
                z: center.z,
            };
            let first = at(start);
            let last = at(start + sweep);
            assert!(
                distance(first, p(0.0, 0.0)) < 1e-9 && distance(last, p(2.0, 0.0)) < 1e-9,
                "the arc must span the chord: {first:?} .. {last:?}"
            );
        }
        other => panic!("expected an Arc, got {other:?}"),
    }
    // A zero bulge is a straight segment: no arc to recover.
    assert!(bulge_arc_geometry(p(0.0, 0.0), p(2.0, 0.0), 0.0, normal).is_none());
}

fn sample_image() -> SemanticGeometry {
    SemanticGeometry::Image {
        origin: p(1.0, 2.0),
        u: Point3 {
            x: 0.5,
            y: 0.0,
            z: 0.0,
        },
        v: Point3 {
            x: 0.0,
            y: 0.25,
            z: 0.0,
        },
        pixels: [640.0, 480.0],
        file: Some("logo.png".to_string()),
        clip: None,
        visible: true,
    }
}

#[test]
fn image_and_mask_are_finite_with_finite_placement() {
    assert!(geometry_is_finite(&sample_image()));
    assert!(geometry_is_finite(&SemanticGeometry::Mask {
        boundary: vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)],
        inverted: false,
    }));
    let poisoned = SemanticGeometry::Image {
        origin: p(f64::NAN, 0.0),
        u: Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        v: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        pixels: [10.0, 10.0],
        file: None,
        clip: None,
        visible: true,
    };
    assert!(!geometry_is_finite(&poisoned));
}

#[test]
fn transform_moves_image_origin_but_not_pixels() {
    let mut m = [[0.0f64; 4]; 4];
    m[0][0] = 2.0;
    m[1][1] = 2.0;
    m[2][2] = 2.0;
    m[3][3] = 1.0;
    m[0][3] = 10.0;
    m[1][3] = 20.0;
    let t = Transform3 { matrix: m };
    let out = DefaultGeometryEngine
        .transform(&sample_image(), &t)
        .unwrap();
    match out {
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            ..
        } => {
            assert_eq!(origin, p(12.0, 24.0));
            assert_eq!(u, p(1.0, 0.0));
            assert_eq!(v, p(0.0, 0.5));
            assert_eq!(pixels, [640.0, 480.0]);
        }
        other => panic!("expected Image, got {other:?}"),
    }
}

#[test]
fn mask_tessellates_to_a_closed_boundary() {
    let mask = SemanticGeometry::Mask {
        boundary: vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)],
        inverted: false,
    };
    let out = tessellate_geometry(&mask, TessellationParams::default());
    assert_eq!(out.len(), 4);
    assert_eq!(out.first(), out.last());
}

// ---------------------------------------------------------------------------
// Bounded approximate intersection fallback
// ---------------------------------------------------------------------------

fn circle_with_radius(radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center: p(0.0, 0.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        radius,
    }
}

/// An open polyline of `segments` unit chords, each with the same bulge.
fn dense_bulge_polyline(segments: usize, bulge: f64) -> SemanticGeometry {
    let points: Vec<Point3> = (0..=segments).map(|i| p(i as f64, 0.0)).collect();
    let bulges = vec![bulge; segments];
    SemanticGeometry::Polyline {
        points,
        bulges,
        closed: false,
    }
}

#[test]
fn fallback_params_are_scale_relative_and_budgeted() {
    let policy = TolerancePolicy::default();
    let mut counts = Vec::new();
    for radius in [5.0, 5_000.0] {
        let circle = circle_with_radius(radius);
        let params = fallback_tessellation_params(&circle, &policy);
        assert_eq!(params.max_segments, FALLBACK_MAX_SEGMENTS);
        assert_eq!(params.min_segments, 4);
        // The chord follows the entity's diameter, not a world constant.
        let expected = 2.0 * radius * FALLBACK_CHORD_FRACTION;
        assert!(
            (params.tolerance - expected).abs() <= expected * 1e-12,
            "chord {} != expected {expected}",
            params.tolerance
        );
        let points = tessellate_geometry(&circle, params);
        assert!(
            points.len() <= FALLBACK_MAX_SEGMENTS + 1,
            "{} samples exceeds the {} segment budget",
            points.len(),
            FALLBACK_MAX_SEGMENTS
        );
        assert!(points.len() > 32, "circle must not be collapsed");
        counts.push(points.len());
    }
    assert_eq!(
        counts[0], counts[1],
        "the same curve at 1000x scale must use the same segment count"
    );
}

#[test]
fn fallback_bounds_a_dense_bulge_polyline_to_the_segment_budget() {
    let policy = TolerancePolicy::default();
    let geometry = dense_bulge_polyline(200, 0.2);
    let params = fallback_tessellation_params(&geometry, &policy);
    assert_eq!(params.max_segments, FALLBACK_MAX_SEGMENTS);

    let raw = tessellate_geometry(&geometry, params);
    assert!(
        raw.len() > FALLBACK_MAX_SEGMENTS + 1,
        "fixture must exceed one curve's budget: {} samples",
        raw.len()
    );
    let bounded = bound_fallback_samples(raw.clone());
    assert!(
        bounded.len() <= FALLBACK_MAX_SEGMENTS + 1,
        "{} samples after bounding",
        bounded.len()
    );
    assert_eq!(bounded.first(), raw.first(), "first vertex kept");
    assert_eq!(bounded.last(), raw.last(), "last vertex kept");
    // Decimation only drops samples; it never reorders or duplicates.
    let mut cursor = 0;
    for q in &bounded {
        let found = raw[cursor..].iter().position(|r| r == q);
        assert!(found.is_some(), "bounded sample is out of order: {q:?}");
        cursor += found.unwrap();
    }
}

#[test]
fn fallback_chord_never_drops_below_the_computation_tolerance() {
    // A zero-extent description (a single-vertex polyline) still respects the
    // world predicate tolerance as the floor.
    let policy = TolerancePolicy {
        computation_world: 1e-7,
        ..TolerancePolicy::default()
    };
    let point = SemanticGeometry::Polyline {
        points: vec![p(1.0, 2.0)],
        bulges: Vec::new(),
        closed: false,
    };
    let params = fallback_tessellation_params(&point, &policy);
    assert_eq!(params.tolerance, 1e-7);
    assert_eq!(params.max_segments, FALLBACK_MAX_SEGMENTS);
}
