use cad_domain::Point3;

use super::math::finite;
use super::*;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn square_loop(z: f64, ccw: bool) -> BrepLoop {
    let pts = if ccw {
        vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]
    } else {
        vec![[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]]
    };
    BrepLoop {
        edges: pts
            .iter()
            .map(|q| BrepCurve::Line {
                start: p(q[0], q[1], z),
                end: p(q[0], q[1], z),
            })
            .collect(),
    }
}

#[test]
fn planar_square_triangulates_to_two_triangles() {
    let face = BrepFace {
        id: 0,
        surface: BrepSurface::Plane {
            origin: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
        },
        reversed: false,
        loops: vec![square_loop(0.0, true)],
    };
    let m = tessellate_face(&face, 0.01, 0.35).unwrap();
    assert_eq!(m.triangles.len(), 2);
    let area: f64 = m
        .triangles
        .iter()
        .map(|t| {
            let a = m.vertices[t[0] as usize];
            let b = m.vertices[t[1] as usize];
            let c = m.vertices[t[2] as usize];
            length(cross(sub(b, a), sub(c, a))) * 0.5
        })
        .sum();
    assert!((area - 4.0).abs() < 1e-9, "area {area}");
    for n in &m.normals {
        assert!((n.z - 1.0).abs() < 1e-9);
    }
}

#[test]
fn reversed_planar_face_flips_the_normal() {
    let face = BrepFace {
        id: 0,
        surface: BrepSurface::Plane {
            origin: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
        },
        reversed: true,
        loops: vec![square_loop(0.0, true)],
    };
    let m = tessellate_face(&face, 0.01, 0.35).unwrap();
    assert!(m.normals.iter().all(|n| n.z < -0.9));
}

#[test]
fn planar_face_with_square_hole_keeps_the_hole_open() {
    // A 4x4 outer square with a 2x2 inner hole: area 16 - 4 = 12.
    let outer = BrepLoop {
        edges: [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]
            .iter()
            .map(|q| BrepCurve::Line {
                start: p(q[0], q[1], 0.0),
                end: p(q[0], q[1], 0.0),
            })
            .collect(),
    };
    // Hole wound opposite to the outer boundary.
    let hole = BrepLoop {
        edges: [[1.0, 1.0], [1.0, 3.0], [3.0, 3.0], [3.0, 1.0]]
            .iter()
            .map(|q| BrepCurve::Line {
                start: p(q[0], q[1], 0.0),
                end: p(q[0], q[1], 0.0),
            })
            .collect(),
    };
    let face = BrepFace {
        id: 0,
        surface: BrepSurface::Plane {
            origin: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
        },
        reversed: false,
        loops: vec![outer, hole],
    };
    let m = tessellate_face(&face, 0.01, 0.35).unwrap();
    let area: f64 = m
        .triangles
        .iter()
        .map(|t| {
            let a = m.vertices[t[0] as usize];
            let b = m.vertices[t[1] as usize];
            let c = m.vertices[t[2] as usize];
            length(cross(sub(b, a), sub(c, a))) * 0.5
        })
        .sum();
    assert!((area - 12.0).abs() < 1e-9, "area {area}");
}

#[test]
fn arc_segments_are_monotone_in_tolerance() {
    let coarse = arc_segments(1.0, std::f64::consts::TAU, 0.5, 0.35);
    let fine = arc_segments(1.0, std::f64::consts::TAU, 0.01, 0.35);
    assert!(fine >= coarse, "{fine} < {coarse}");
}

#[test]
fn unsupported_surface_is_an_error_not_a_mesh() {
    let face = BrepFace {
        id: 3,
        surface: BrepSurface::Unsupported {
            type_key: "nurbs-surface".into(),
        },
        reversed: false,
        loops: Vec::new(),
    };
    let err = tessellate_face(&face, 0.01, 0.35).unwrap_err();
    assert!(err.is_unsupported());
    assert!(err.reason().contains("nurbs-surface"));
}

// ---- Cone and torus (F15) ----

fn circle_loop(center: Point3, normal: Point3, u_dir: Point3, radius: f64) -> BrepLoop {
    BrepLoop {
        edges: vec![BrepCurve::Circle {
            center,
            normal,
            u_dir,
            radius,
        }],
    }
}

fn apex_loop() -> BrepLoop {
    // The degenerate apex edge acadrust's cone builder emits: one coedge
    // whose underlying curve pointer is NULL.
    BrepLoop {
        edges: vec![BrepCurve::Unsupported {
            type_key: "edge-without-curve".into(),
        }],
    }
}

fn cone_face(radius: f64, height: f64, loops: Vec<BrepLoop>) -> BrepFace {
    let hyp = (radius * radius + height * height).sqrt();
    BrepFace {
        id: 0,
        surface: BrepSurface::Cone {
            origin: p(0.0, 0.0, 0.0),
            axis: p(0.0, 0.0, 1.0),
            ref_dir: p(1.0, 0.0, 0.0),
            radius,
            sin_half_angle: -radius / hyp,
            cos_half_angle: height / hyp,
        },
        reversed: false,
        loops,
    }
}

fn triangle_area(m: &FaceMesh, t: [u32; 3]) -> f64 {
    let a = m.vertices[t[0] as usize];
    let b = m.vertices[t[1] as usize];
    let c = m.vertices[t[2] as usize];
    length(cross(sub(b, a), sub(c, a))) * 0.5
}

#[test]
fn full_cone_fans_base_to_apex_without_nan() {
    // radius 1, height 2, apex at z = 2 (the fixture geometry).
    let face = cone_face(
        1.0,
        2.0,
        vec![
            circle_loop(p(0.0, 0.0, 0.0), p(0.0, 0.0, -1.0), p(1.0, 0.0, 0.0), 1.0),
            apex_loop(),
        ],
    );
    let m = tessellate_face(&face, 0.01, 0.35).unwrap();
    assert!(m.curved);
    let n = m.edges[0].len();
    assert_eq!(m.triangles.len(), n, "one fan triangle per base segment");
    assert!(m
        .vertices
        .iter()
        .all(|q| q.x.is_finite() && q.y.is_finite() && q.z.is_finite()));
    assert!(m
        .normals
        .iter()
        .all(|q| q.x.is_finite() && q.y.is_finite() && q.z.is_finite()));
    // The apex is the single highest vertex.
    let zmax = m.vertices.iter().fold(f64::NEG_INFINITY, |a, q| a.max(q.z));
    assert!((zmax - 2.0).abs() < 1e-9, "apex at {zmax}");
    // Lateral area = π r s, s = sqrt(1 + 4).
    let area: f64 = m.triangles.iter().map(|t| triangle_area(&m, *t)).sum();
    let expected = std::f64::consts::PI * 1.0 * (1.0f64 + 4.0).sqrt();
    assert!(
        (area - expected).abs() / expected < 0.02,
        "area {area} vs {expected}"
    );
    // Error bound must be finite and positive.
    let bound = m.error_bound.unwrap();
    assert!(bound.is_finite() && bound > 0.0);
}

#[test]
fn truncated_cone_stitches_two_rings_on_generators() {
    // A frustum from r = 1.0 at z = 0 to r = 0.5 at z = 1.0. The slope is
    // -0.5, so the surface reference radius at z = 0 is 1.0.
    let face = BrepFace {
        id: 0,
        surface: BrepSurface::Cone {
            origin: p(0.0, 0.0, 0.0),
            axis: p(0.0, 0.0, 1.0),
            ref_dir: p(1.0, 0.0, 0.0),
            radius: 1.0,
            sin_half_angle: -0.5 / (1.0f64 + 0.25).sqrt(),
            cos_half_angle: 1.0 / (1.0f64 + 0.25).sqrt(),
        },
        reversed: false,
        loops: vec![
            circle_loop(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0), p(1.0, 0.0, 0.0), 1.0),
            circle_loop(p(0.0, 0.0, 1.0), p(0.0, 0.0, 1.0), p(1.0, 0.0, 0.0), 0.5),
        ],
    };
    let m = tessellate_face(&face, 0.01, 0.35).unwrap();
    assert!(m.curved);
    let na = m.edges[0].len();
    let nb = m.edges[1].len();
    assert_eq!(m.triangles.len(), na + nb, "merge emits na + nb triangles");
    assert!(m.vertices.iter().all(|q| finite(*q)));
    // Lateral area of a frustum: π (r0 + r1) s, s = sqrt(0.25 + 1).
    let area: f64 = m.triangles.iter().map(|t| triangle_area(&m, *t)).sum();
    let expected = std::f64::consts::PI * 1.5 * (0.25f64 + 1.0).sqrt();
    assert!(
        (area - expected).abs() / expected < 0.03,
        "area {area} vs {expected}"
    );
    assert!(m.error_bound.unwrap() > 0.0);
}

#[test]
fn cone_without_a_circle_is_unsupported_not_faked() {
    let face = cone_face(1.0, 2.0, vec![apex_loop()]);
    let err = tessellate_face(&face, 0.01, 0.35).unwrap_err();
    assert!(err.is_unsupported());
}

#[test]
fn torus_is_closed_with_a_chordal_bound() {
    let face = BrepFace {
        id: 0,
        surface: BrepSurface::Torus {
            center: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            major_radius: 3.0,
            minor_radius: 1.0,
            u_dir: p(1.0, 0.0, 0.0),
        },
        reversed: false,
        loops: Vec::new(),
    };
    let m = tessellate_face(&face, 0.01, 0.35).unwrap();
    assert!(m.curved && !m.triangles.is_empty());
    assert!(m.vertices.iter().all(|q| finite(*q)));
    assert!(m.normals.iter().all(|q| finite(*q)));
    // Every edge in the torus grid is shared by exactly two triangles once
    // welded by position (the seam closes).
    let open = crate::brep_tessellator::detect_open_edges(&m.vertices, &m.triangles);
    assert!(open.is_empty(), "torus seam must close: {open:?}");
    let bound = m.error_bound.unwrap();
    assert!(bound.is_finite() && bound > 0.0);
}
