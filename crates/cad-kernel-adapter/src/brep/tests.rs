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
        tolerance: None,
        domain: None,
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
        tolerance: None,
        domain: None,
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
        tolerance: None,
        domain: None,
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
        tolerance: None,
        domain: None,
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
        tolerance: None,
        domain: None,
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
        tolerance: None,
        domain: None,
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
        tolerance: None,
        domain: None,
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

// ---------------------------------------------------------------------------
// `BrepCurve::Arc`
// ---------------------------------------------------------------------------

/// The quarter disc of radius 1 in `z = 0`: a quarter arc from `(1, 0)` to
/// `(0, 1)` about `+z`, then the two radii back to the centre.
fn quarter_disc_loop() -> BrepLoop {
    BrepLoop {
        edges: vec![
            BrepCurve::Arc {
                center: p(0.0, 0.0, 0.0),
                normal: p(0.0, 0.0, 1.0),
                u_dir: p(1.0, 0.0, 0.0),
                radius: 1.0,
                start: p(1.0, 0.0, 0.0),
                end: p(0.0, 1.0, 0.0),
            },
            BrepCurve::Line {
                start: p(0.0, 1.0, 0.0),
                end: p(0.0, 0.0, 0.0),
            },
            BrepCurve::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
        ],
    }
}

fn quarter_disc_face() -> BrepFace {
    BrepFace {
        id: 0,
        surface: BrepSurface::Plane {
            origin: p(0.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            u_dir: p(1.0, 0.0, 0.0),
        },
        reversed: false,
        loops: vec![quarter_disc_loop()],
        tolerance: None,
        domain: None,
    }
}

#[test]
fn arc_loop_samples_a_closed_ring_from_its_own_start() {
    let ring =
        super::tessellate::sample_loop(&quarter_disc_loop(), 1e-5, 0.35).expect("arc loop samples");
    assert!(ring.len() > 4, "the arc contributes intermediate samples");
    // The ring is a cycle: the first point is the arc's start and the last is a
    // distinct point (no duplicated closing vertex to weld by hand).
    assert_eq!(ring[0], p(1.0, 0.0, 0.0));
    let last = ring[ring.len() - 1];
    assert!(
        length(sub(ring[0], last)) > 1e-9,
        "a ring must not repeat its first point"
    );
    // Every arc sample sits on the circle, inside the quarter and never past the
    // next edge's start; the two line edges contribute their own points exactly.
    for q in &ring {
        assert!(length(*q) <= 1.0 + 1e-12, "outside the disc: {q:?}");
    }
    // The polygon closes: shoelace area equals the quarter disc's closed form.
    let mut area = 0.0;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        area += a.x * b.y - b.x * a.y;
    }
    let area = area / 2.0;
    let expected = std::f64::consts::FRAC_PI_4;
    assert!(
        (area - expected).abs() / expected < 1e-3,
        "quarter-disc area {area} vs {expected} (0.1 % budget)"
    );
}

#[test]
fn arc_boundary_is_an_approximation_with_a_chordal_bound() {
    let mesh = tessellate_face(&quarter_disc_face(), 1e-5, 0.35).expect("quarter disc");
    // A planar face bounded by an arc is *not* analytic: its boundary is a
    // polyline on the circle, so the mesh must say so.
    assert!(mesh.curved, "an arc boundary is a chordal approximation");
    let bound = mesh.error_bound.expect("a chordal bound");
    assert!(bound.is_finite() && bound > 0.0 && bound < 1e-5);
    let area: f64 = mesh
        .triangles
        .iter()
        .map(|t| {
            let a = mesh.vertices[t[0] as usize];
            let b = mesh.vertices[t[1] as usize];
            let c = mesh.vertices[t[2] as usize];
            length(cross(sub(b, a), sub(c, a))) * 0.5
        })
        .sum();
    let expected = std::f64::consts::FRAC_PI_4;
    assert!(
        (area - expected).abs() / expected < 1e-3,
        "triangulated area {area} vs {expected}"
    );
}

#[test]
fn arc_samples_join_the_next_edge_exactly() {
    let loop_ = quarter_disc_loop();
    let ring = super::tessellate::sample_loop(&loop_, 1e-5, 0.35).expect("ring");
    // The arc's last sample is followed by the next edge's start, which is the
    // arc's own end: the chain has no gap and no duplicated point.
    let BrepCurve::Line { start, .. } = loop_.edges[1] else {
        panic!("the second edge is a line");
    };
    assert!(
        ring.windows(2)
            .all(|pair| length(sub(pair[0], pair[1])) > 1e-12),
        "no consecutive duplicates after dedupe"
    );
    let second_edge_index = ring
        .iter()
        .position(|q| length(sub(*q, start)) <= 1e-12)
        .expect("the line's start is a ring vertex");
    assert!(second_edge_index > 1, "the arc contributed samples first");
}

#[test]
fn a_half_arc_bounds_its_bulge() {
    // A half circle from (1, 0) to (-1, 0) about +z bulges to y = 2, which an
    // endpoint-only estimate would miss entirely.
    let brep = BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: vec![BrepFace {
                id: 0,
                surface: BrepSurface::Plane {
                    origin: p(0.0, 0.0, 0.0),
                    normal: p(0.0, 0.0, 1.0),
                    u_dir: p(1.0, 0.0, 0.0),
                },
                reversed: false,
                loops: vec![BrepLoop {
                    edges: vec![BrepCurve::Arc {
                        center: p(0.0, 0.0, 0.0),
                        normal: p(0.0, 0.0, 1.0),
                        u_dir: p(1.0, 0.0, 0.0),
                        radius: 2.0,
                        start: p(2.0, 0.0, 0.0),
                        end: p(-2.0, 0.0, 0.0),
                    }],
                }],
                tolerance: None,
                domain: None,
            }],
        }],
        placement: None,
        tolerance: None,
    };
    let (min, max) = crate::solid_tolerance::brep_bounds(&brep).expect("bounds");
    assert!(max.y >= 2.0 - 1e-12, "the bulge reaches y = 2: {max:?}");
    assert!(
        min.x <= -2.0 + 1e-12 && max.x >= 2.0 - 1e-12,
        "the ends bound x: {min:?} {max:?}"
    );
}

#[test]
fn an_arc_on_a_cylinder_side_face_is_refused_by_name() {
    let face = BrepFace {
        id: 0,
        surface: BrepSurface::Cylinder {
            origin: p(0.0, 0.0, 0.0),
            axis: p(0.0, 0.0, 1.0),
            ref_dir: p(1.0, 0.0, 0.0),
            radius: 1.0,
        },
        reversed: false,
        loops: vec![
            BrepLoop {
                edges: vec![BrepCurve::Arc {
                    center: p(0.0, 0.0, 0.0),
                    normal: p(0.0, 0.0, 1.0),
                    u_dir: p(1.0, 0.0, 0.0),
                    radius: 1.0,
                    start: p(1.0, 0.0, 0.0),
                    end: p(0.0, 1.0, 0.0),
                }],
            },
            BrepLoop {
                edges: vec![BrepCurve::Arc {
                    center: p(0.0, 0.0, 1.0),
                    normal: p(0.0, 0.0, 1.0),
                    u_dir: p(1.0, 0.0, 0.0),
                    radius: 1.0,
                    start: p(1.0, 0.0, 1.0),
                    end: p(0.0, 1.0, 1.0),
                }],
            },
        ],
        tolerance: None,
        domain: None,
    };
    let err = tessellate_face(&face, 0.01, 0.35).expect_err("trimmed cylinder is P2.1b");
    assert!(err.is_unsupported());
    assert!(
        err.reason().contains("circular arc"),
        "the refusal names the arc: {}",
        err.reason()
    );
}

/// A 3×2 control net (`degree 2` in `u`, `1` in `v`), all weights 1: a surface
/// that bulges to `z = 0.5` between the control rows.
fn nurbs_hull_net() -> Vec<Vec<[f64; 4]>> {
    vec![
        vec![[0.0, 0.0, 0.0, 1.0], [0.0, 2.0, 0.0, 1.0]],
        vec![[1.0, 0.0, 1.0, 1.0], [1.0, 2.0, 1.0, 1.0]],
        vec![[2.0, 0.0, 0.0, 1.0], [2.0, 2.0, 0.0, 1.0]],
    ]
}

fn nurbs_face(id: u32) -> BrepFace {
    BrepFace {
        id,
        surface: BrepSurface::Nurbs {
            u_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            v_knots: vec![0.0, 0.0, 1.0, 1.0],
            points: nurbs_hull_net(),
        },
        reversed: false,
        loops: Vec::new(),
        domain: Some([[0.0, 1.0], [0.0, 1.0]]),
        tolerance: None,
    }
}

#[test]
fn a_nurbs_face_is_refused_naming_solidcore_tess() {
    let face = nurbs_face(4);
    let err = tessellate_face(&face, 0.01, 0.35).expect_err("no NURBS mesher in this build");
    assert!(err.is_unsupported(), "{err:?}");
    assert!(
        err.reason().contains("solidcore-tess"),
        "the refusal names the missing mesher: {}",
        err.reason()
    );
    assert!(
        err.reason().contains("nurbs"),
        "the refusal names the surface: {}",
        err.reason()
    );
}

/// The bounds estimate uses the **dehomogenised control net**, so it is a
/// superset of the surface (convex hull), never an under-estimate.
#[test]
fn the_nurbs_control_box_contains_sampled_points_of_the_surface() {
    let brep = BrepData {
        shells: vec![BrepShell {
            id: 0,
            faces: vec![nurbs_face(0)],
        }],
        placement: None,
        tolerance: None,
    };
    let (min, max) = crate::solid_tolerance::brep_bounds(&brep).expect("bounds");
    // The control net's own extent: u in [0, 2], v in [0, 2], z in [0, 1].
    assert!(
        min.x.abs() < 1e-12 && (max.x - 2.0).abs() < 1e-12,
        "{min:?} {max:?}"
    );
    assert!(
        min.y.abs() < 1e-12 && (max.y - 2.0).abs() < 1e-12,
        "{min:?} {max:?}"
    );
    assert!(
        min.z.abs() < 1e-12 && (max.z - 1.0).abs() < 1e-12,
        "{min:?} {max:?}"
    );

    // Evaluate the exact surface (a Bézier patch: degree (2, 1) with the knots
    // above) at a grid and check every point is inside the box.
    let net = nurbs_hull_net();
    let bernstein2 = |t: f64| [(1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t];
    let bernstein1 = |s: f64| [1.0 - s, s];
    let mut saw_bulge = false;
    for i in 0..=10 {
        for j in 0..=10 {
            let u = i as f64 / 10.0;
            let v = j as f64 / 10.0;
            let bu = bernstein2(u);
            let bv = bernstein1(v);
            let mut point = p(0.0, 0.0, 0.0);
            for (a, row) in net.iter().enumerate() {
                for (b, coefficient) in row.iter().enumerate() {
                    let basis = bu[a] * bv[b];
                    point.x += basis * coefficient[0];
                    point.y += basis * coefficient[1];
                    point.z += basis * coefficient[2];
                }
            }
            assert!(
                point.x >= min.x - 1e-12 && point.x <= max.x + 1e-12,
                "{point:?}"
            );
            assert!(
                point.y >= min.y - 1e-12 && point.y <= max.y + 1e-12,
                "{point:?}"
            );
            assert!(
                point.z >= min.z - 1e-12 && point.z <= max.z + 1e-12,
                "{point:?}"
            );
            if point.z > 0.25 {
                saw_bulge = true;
            }
        }
    }
    assert!(
        saw_bulge,
        "the sampled surface really leaves the control rows"
    );
    assert!(
        max.z >= 1.0 - 1e-12,
        "the box comes from the control net, which reaches z = 1"
    );
}
