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
