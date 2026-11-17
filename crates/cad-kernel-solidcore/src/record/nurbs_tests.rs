//! P2.1b: the neutral model reads NURBS **form** surfaces and a face's `(u, v)`
//! box (part 1), and writes a form face back with its stated box or refuses it by
//! name (part 2 — see [`surfaces`](super::surfaces) for the writer).
//!
//! These live in their own module because `from_record.rs` is close to the
//! 1000-line file ceiling; the tests are about the record seam, not about any one
//! conversion file.

use cad_domain::{BrepData, BrepFace, BrepLoop, BrepPlacement, BrepSurface, Point3};
use solidcore_record::brep::{Brep as Record, Surface};

use super::{bake, from_record, to_record, DEFAULT_TOLERANCE};
use crate::error::LIFT_UNSUPPORTED;
use crate::polyhedra;

/// A record with one planar box whose first face's **label** is a 3×3 control
/// net, plus the `(u, v)` box the producer states for it.
///
/// The net is a genuine degree (2, 2) tensor product: 3 control points per
/// direction, `3 + 2 + 1 = 6` knots, a non-empty span — what the record's own
/// validation accepts.
struct SplineFace {
    record: Record,
    points: Vec<Vec<[f64; 3]>>,
    domain: [[f64; 2]; 2],
}

fn spline_face_record() -> SplineFace {
    let mut record = to_record(
        &polyhedra::box_brep(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            1.0,
            2.0,
            3.0,
        ),
        DEFAULT_TOLERANCE,
    )
    .expect("authored box lifts");
    let points: Vec<Vec<[f64; 3]>> = vec![
        vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.25], [0.0, 2.0, 0.0]],
        vec![[1.0, 0.0, 0.5], [1.0, 1.0, 0.75], [1.0, 2.0, 0.5]],
        vec![[2.0, 0.0, 0.0], [2.0, 1.0, 0.25], [2.0, 2.0, 0.0]],
    ];
    let domain = [[-1.5, 2.5], [0.25, 3.75]];
    record.faces[0].surface = Surface::Spline {
        u_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        v_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        points: points.clone(),
    };
    record.faces[0].domain = domain;
    SplineFace {
        record,
        points,
        domain,
    }
}

fn nurbs_faces(brep: &BrepData) -> Vec<&BrepFace> {
    brep.shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .filter(|face| matches!(face.surface, BrepSurface::Nurbs { .. }))
        .collect()
}

/// A neutral body whose first face is a form surface and whose other faces are
/// the authored box's planes.
fn nurbs_body(points: Vec<Vec<[f64; 4]>>, domain: Option<[[f64; 2]; 2]>) -> BrepData {
    let mut brep = polyhedra::box_brep(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        1.0,
        2.0,
        3.0,
    );
    let face = &mut brep.shells[0].faces[0];
    face.surface = BrepSurface::Nurbs {
        u_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        v_knots: vec![0.0, 0.0, 1.0, 1.0],
        points,
    };
    face.domain = domain;
    brep
}

fn two_by_three() -> Vec<Vec<[f64; 4]>> {
    vec![
        vec![[0.0, 0.0, 0.0, 1.0], [0.0, 2.0, 0.0, 1.0]],
        vec![[1.0, 0.0, 1.0, 1.0], [1.0, 2.0, 1.0, 1.0]],
        vec![[2.0, 0.0, 0.0, 1.0], [2.0, 2.0, 0.0, 1.0]],
    ]
}

#[test]
fn a_spline_surface_face_reads_back_as_nurbs_with_its_domain() {
    let SplineFace {
        record,
        points,
        domain,
    } = spline_face_record();
    let brep = from_record(&record).expect("a spline-labelled face maps to NURBS");
    let faces = nurbs_faces(&brep);
    assert_eq!(faces.len(), 1, "exactly the mutated face is a form surface");
    let face = faces[0];
    assert_eq!(face.id, 0);
    let BrepSurface::Nurbs {
        u_knots,
        v_knots,
        points: mapped,
    } = &face.surface
    else {
        unreachable!("filtered to the NURBS face")
    };
    // Bit-for-bit: the net is copied, not re-derived, and a `Spline` (no weights)
    // is exactly weight 1.
    assert_eq!(*u_knots, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    assert_eq!(*v_knots, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    assert_eq!(mapped.len(), points.len());
    for (row, expected_row) in mapped.iter().zip(&points) {
        assert_eq!(row.len(), expected_row.len());
        for (coefficient, expected) in row.iter().zip(expected_row) {
            assert_eq!(
                *coefficient,
                [expected[0], expected[1], expected[2], 1.0],
                "a Spline control point carries weight 1"
            );
        }
    }
    // The producer's box is the only source of `BrepFace::domain`.
    assert_eq!(face.domain, Some(domain));
    // The untransformed box faces keep their analytic label and still carry the
    // record's domain.
    for other in brep
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .filter(|face| face.id != 0)
    {
        assert!(matches!(other.surface, BrepSurface::Plane { .. }));
        assert!(other.domain.is_some(), "every mapped face carries the box");
    }
}

#[test]
fn spline_and_rational_spline_with_unit_weights_agree_and_weights_survive() {
    let SplineFace { record, points, .. } = spline_face_record();
    let from_spline = from_record(&record).expect("spline maps");
    let spline_surface = nurbs_faces(&from_spline)[0].surface.clone();

    // The same net as a `RationalSpline` with w = 1 everywhere.
    let homogeneous: Vec<Vec<[f64; 4]>> = points
        .iter()
        .map(|row| {
            row.iter()
                .map(|p| [p[0], p[1], p[2], 1.0])
                .collect::<Vec<[f64; 4]>>()
        })
        .collect();
    let mut rational_record = record.clone();
    rational_record.faces[0].surface = Surface::RationalSpline {
        u_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        v_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        homogeneous: homogeneous.clone(),
    };
    let from_rational = from_record(&rational_record).expect("rational spline maps");
    assert_eq!(
        nurbs_faces(&from_rational)[0].surface,
        spline_surface,
        "unit weights make the two labels the same geometry"
    );

    // Real weights: carried through the seam unchanged (bit-for-bit), not
    // normalised and not dehomogenised here.
    let weighted: Vec<Vec<[f64; 4]>> = points
        .iter()
        .enumerate()
        .map(|(i, row)| {
            row.iter()
                .enumerate()
                .map(|(j, p)| {
                    let w = 1.0 + i as f64 + 3.0 * j as f64;
                    [p[0] * w, p[1] * w, p[2] * w, w]
                })
                .collect::<Vec<[f64; 4]>>()
        })
        .collect();
    rational_record.faces[0].surface = Surface::RationalSpline {
        u_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        v_knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        homogeneous: weighted.clone(),
    };
    let mapped = from_record(&rational_record).expect("weighted rational spline maps");
    let BrepSurface::Nurbs { points: got, .. } = &nurbs_faces(&mapped)[0].surface else {
        unreachable!()
    };
    assert_eq!(*got, weighted);
}

#[test]
fn baking_a_similarity_moves_nurbs_control_points_and_keeps_knots_and_weights() {
    let mut points = two_by_three();
    points[1][0] = [3.0, 0.0, 3.0, 2.0]; // weight 2 at (1, 0): the point (1.5, 0, 1.5)
    let brep = nurbs_body(points.clone(), Some([[0.0, 1.0], [0.0, 1.0]]));
    // x' = 2x + 10, y' = 2y, z' = 2z.
    let baked = bake(
        &brep,
        BrepPlacement {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            scale: 2.0,
        },
    )
    .expect("a similarity bakes a NURBS net");
    let BrepSurface::Nurbs {
        u_knots,
        v_knots,
        points: got,
    } = &baked.shells[0].faces[0].surface
    else {
        panic!("the form surface stays a form surface");
    };
    // Knots are parameters: untouched.
    assert_eq!(*u_knots, vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    assert_eq!(*v_knots, vec![0.0, 0.0, 1.0, 1.0]);
    // Weights are a property of the control point: untouched.
    for (row, expected_row) in got.iter().zip(&points) {
        for (coefficient, expected) in row.iter().zip(expected_row) {
            assert_eq!(coefficient[3], expected[3]);
        }
    }
    // The dehomogenised point moved exactly like any other point.
    let weight = got[1][0][3];
    assert_eq!(weight, 2.0);
    let control = [
        got[1][0][0] / weight,
        got[1][0][1] / weight,
        got[1][0][2] / weight,
    ];
    // The stored coefficient was [3, 0, 3] with weight 2, i.e. the point (1.5, 0, 1.5).
    assert!((control[0] - 13.0).abs() < 1e-12, "{control:?}");
    assert!(control[1].abs() < 1e-12, "{control:?}");
    assert!((control[2] - 3.0).abs() < 1e-12, "{control:?}");
    // The (u, v) box is a parameter rectangle: a similarity does not touch it.
    assert_eq!(
        baked.shells[0].faces[0].domain,
        Some([[0.0, 1.0], [0.0, 1.0]])
    );
}

#[test]
fn a_nurbs_control_point_with_a_zero_or_nan_weight_is_refused_by_name() {
    for bad in [0.0, -1.0, f64::NAN] {
        let mut points = two_by_three();
        points[0][1][3] = bad;
        let brep = nurbs_body(points, Some([[0.0, 1.0], [0.0, 1.0]]));
        let refusal = bake(
            &brep,
            BrepPlacement {
                matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                translation: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                scale: 1.0,
            },
        )
        .expect_err("a weight that cannot be divided by is refused");
        assert_eq!(refusal.code, LIFT_UNSUPPORTED, "weight {bad}");
        assert!(
            refusal.detail.contains("weight") && refusal.detail.contains("(0, 1)"),
            "the refusal names the weight and the coefficient: {}",
            refusal.detail
        );
    }
}

/// The write side now writes a form surface (P2.1b part 2), but a boundary with
/// no exact pcurve in the net's own parameters is still refused by name — and a
/// form face that states no `(u, v)` box is refused rather than given one.
#[test]
fn a_nurbs_face_is_written_or_refused_by_name_and_planes_still_write() {
    let boxed = polyhedra::box_brep(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        1.0,
        2.0,
        3.0,
    );
    let record = to_record(&boxed, DEFAULT_TOLERANCE).expect("planes still write");
    assert_eq!(record.faces.len(), 6, "the planar path is unchanged");
    assert!(record.faces.iter().all(|face| face.nurbs.is_none()));

    // The net carries its box, so the writer reaches the pcurve step: the box's
    // straight edges are not the exact images of lines in this net's parameters,
    // and nothing is fitted. The code says exactly that.
    let with_domain = nurbs_body(two_by_three(), Some([[0.0, 1.0], [0.0, 1.0]]));
    let refusal = to_record(&with_domain, DEFAULT_TOLERANCE)
        .expect_err("a form face's boundary must have an exact pcurve");
    assert_eq!(refusal.code, crate::error::PCURVE_NOT_EXACT);
    assert!(
        refusal.detail.contains("exact pcurve"),
        "the refusal names the gap: {}",
        refusal.detail
    );

    let without_domain = nurbs_body(two_by_three(), None);
    let refusal = to_record(&without_domain, DEFAULT_TOLERANCE)
        .expect_err("a form face without its box is refused, never given one");
    assert_eq!(refusal.code, LIFT_UNSUPPORTED);
    assert!(
        refusal.detail.contains("domain"),
        "the refusal names the missing box: {}",
        refusal.detail
    );
}

/// An analytic label is the geometry, so a `nurbs` form attached to it is ignored
/// (whatever it says), and the face's box still crosses the seam.
#[test]
fn an_analytic_label_ignores_its_nurbs_form_and_carries_its_domain() {
    let mut record = to_record(
        &polyhedra::box_brep(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            1.0,
            2.0,
            3.0,
        ),
        DEFAULT_TOLERANCE,
    )
    .expect("box lifts");
    // A form that is not the plane's would be a lie: the label wins, as documented.
    record.faces[0].nurbs = Some(Surface::RationalSpline {
        u_knots: vec![0.0, 0.0, 1.0, 1.0],
        v_knots: vec![0.0, 0.0, 1.0, 1.0],
        homogeneous: vec![
            vec![[0.0, 0.0, 0.0, 1.0], [0.0, 2.0, 0.0, 1.0]],
            vec![[2.0, 0.0, 0.0, 1.0], [2.0, 2.0, 0.0, 1.0]],
        ],
    });
    record.faces[0].domain = [[0.0, 2.0], [0.0, 2.0]];
    let brep = from_record(&record).expect("the analytic label maps");
    assert!(
        nurbs_faces(&brep).is_empty(),
        "the form must not become the surface"
    );
    assert!(matches!(
        brep.shells[0].faces[0].surface,
        BrepSurface::Plane { .. }
    ));
    assert_eq!(
        brep.shells[0].faces[0].domain,
        Some([[0.0, 2.0], [0.0, 2.0]])
    );
}

/// A NURBS **surface** does not drag curve support in with it: a face whose loop
/// carries a spline edge still refuses at the curve, with the existing named code.
#[test]
fn a_nurbs_face_with_a_spline_boundary_still_refuses_at_the_curve() {
    let SplineFace { mut record, .. } = spline_face_record();
    record.edges[0].curve = solidcore_record::brep::Curve::Spline {
        knots: vec![0.0, 0.0, 1.0, 1.0],
        points: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
    };
    let refusal = from_record(&record).expect_err("a curved boundary is still not mapped");
    assert_eq!(refusal.code, LIFT_UNSUPPORTED);
    assert!(refusal.detail.contains("spline"), "{}", refusal.detail);
}

/// A `Nurbs` face keeps its loops literally (one curve per oriented edge): the
/// box's line loops survive the form surface.
#[test]
fn a_nurbs_face_keeps_its_line_loops() {
    let SplineFace { record, .. } = spline_face_record();
    let brep = from_record(&record).expect("form surface maps");
    let face = nurbs_faces(&brep)[0];
    assert_eq!(face.loops.len(), 1);
    let BrepLoop { edges } = &face.loops[0];
    assert_eq!(edges.len(), 4, "a box face has four line edges");
    assert!(edges
        .iter()
        .all(|curve| matches!(curve, cad_domain::BrepCurve::Line { .. })));
}
