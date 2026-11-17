//! Unit tests for `BrepData → record` (the writer).
//!
//! Split out of `to_record.rs` because the writer plus its tests crossed the
//! workspace's per-file line ceiling; the writer reads as production code and the
//! tests live here as a sibling.

#![cfg(test)]

use cad_domain::{BrepData, BrepSurface, Point3};
use solidcore_record::brep::{Brep as Record, Curve, Curve2, Loop, Surface, VERSION};

use crate::error::{LIFT_REQUIRES_PLANAR, LIFT_UNSUPPORTED, PLACEMENT_NOT_SUPPORTED};
use crate::polyhedra;
use crate::record::{dot, from_record, point, sub, to_record, DEFAULT_TOLERANCE};

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn unit_box() -> BrepData {
    polyhedra::box_brep(p(0.0, 0.0, 0.0), 1.0, 1.0, 1.0)
}

#[test]
fn a_planar_box_record_is_canonical_and_ordered() {
    let record = to_record(&unit_box(), DEFAULT_TOLERANCE).expect("record");
    assert_eq!(record.version, VERSION);
    assert_eq!(record.faces.len(), 6);
    assert_eq!(record.edges.len(), 12);
    assert_eq!(record.vertices.len(), 8);
    assert_eq!(record.regions.len(), 2);
    assert!(!record.regions[0].void && !record.regions[1].void);
    assert_eq!(record.regions[0].shells, vec![0]);
    assert_eq!(record.regions[1].shells, vec![0]);
    // Every edge is used twice, once per adjacent face, and its uses carry
    // the radial `upper = !forward` flag.
    for (index, edge) in record.edges.iter().enumerate() {
        assert_eq!(edge.uses.len(), 2, "closed shell edge used twice");
        assert_eq!(edge.interval, [0.0, 1.0]);
        for use_ in &edge.uses {
            let Loop::Edges { edges } =
                &record.faces[use_.face as usize].loops[use_.r#loop as usize]
            else {
                panic!("a vertex loop in an authored box");
            };
            let oriented = &edges[use_.position as usize];
            assert_eq!(oriented.edge as usize, index);
            assert_eq!(use_.upper, !oriented.forward);
        }
    }
    // No signed zeros survive: the record's exact-bit dedup would see two
    // vertices otherwise.
    for vertex in &record.vertices {
        for value in vertex.point {
            assert!(!value.is_sign_negative() || value != 0.0);
        }
    }
}

#[test]
fn pcurves_are_the_exact_uv_image_of_each_edge() {
    let record = to_record(&unit_box(), DEFAULT_TOLERANCE).expect("record");
    for face in &record.faces {
        let Surface::Plane {
            origin,
            u_axis,
            v_axis,
        } = &face.surface
        else {
            panic!("authored box face is a plane");
        };
        let (origin, u, v) = (point(*origin), point(*u_axis), point(*v_axis));
        for loop_ in &face.loops {
            let Loop::Edges { edges } = loop_ else {
                panic!("edge loop");
            };
            for use_ in edges {
                let edge = &record.edges[use_.edge as usize];
                let Curve::Line { start, end } = &edge.curve else {
                    panic!("authored box edge is a line");
                };
                let expected = [
                    [
                        dot(sub(point(*start), origin), u),
                        dot(sub(point(*start), origin), v),
                    ],
                    [
                        dot(sub(point(*end), origin), u),
                        dot(sub(point(*end), origin), v),
                    ],
                ];
                let Curve2::Spline { points, .. } = &use_.pcurve else {
                    panic!("a degree-1 spline pcurve");
                };
                for (got, want) in points.iter().zip(&expected) {
                    assert!((got[0] - want[0]).abs() < 1e-12);
                    assert!((got[1] - want[1]).abs() < 1e-12);
                }
            }
        }
    }
}

#[test]
fn a_round_trip_keeps_the_body() {
    let brep = unit_box();
    let record = to_record(&brep, DEFAULT_TOLERANCE).expect("record");
    let back = from_record(&record).expect("neutral body");
    assert_eq!(back.face_count(), brep.face_count());
    let again = to_record(&back, DEFAULT_TOLERANCE).expect("record again");
    assert_eq!(again.faces.len(), record.faces.len());
    assert_eq!(again.edges.len(), record.edges.len());
    assert_eq!(again.vertices.len(), record.vertices.len());
}

/// Inverted from `a_curved_face_is_refused_with_the_stable_planar_reason`:
/// a cylinder is now written with its exact form, its box and exact pcurves.
#[test]
fn a_curved_face_is_written_with_its_exact_form_and_box() {
    let body = solidcore_ops::make_cylinder([0.0, 0.0, 0.0], 1.0, 2.0).expect("cylinder");
    let (record, _) = body.to_record().expect("cylinder record");
    let brep = from_record(&record).expect("neutral cylinder");
    let lifted = to_record(&brep, DEFAULT_TOLERANCE).expect("curved lift writes");
    assert_eq!(lifted.faces.len(), 3);
    let lateral = &lifted.faces[0];
    // The label stays analytic and the form is the exact rational NURBS.
    let solidcore_record::brep::Surface::Cylinder { radius, .. } = &lateral.surface else {
        panic!(
            "the lateral face keeps its cylinder label: {:?}",
            lateral.surface
        );
    };
    assert!((radius - 1.0).abs() < 1e-12);
    let solidcore_record::brep::Surface::RationalSpline {
        u_knots, v_knots, ..
    } = lateral
        .nurbs
        .as_ref()
        .expect("a labelled curved face carries its form")
    else {
        panic!("the form is a NURBS: {:?}", lateral.nurbs);
    };
    // One turn of the form is 2π (`parameter_scale`), not one unit per quarter.
    assert!((u_knots[0] - 0.0).abs() < 1e-12);
    assert!((v_knots[0] - 0.0).abs() < 1e-12);
    // The box is the form's, around the whole face.
    assert!(
        (lateral.domain[0][1] - std::f64::consts::TAU).abs() < 1e-9,
        "{:?}",
        lateral.domain
    );
    assert!(
        (lateral.domain[1][1] - 2.0).abs() < 1e-9,
        "{:?}",
        lateral.domain
    );
    // Every use has a pcurve, and the record is structurally valid.
    for face in &lifted.faces {
        for loop_ in &face.loops {
            assert!(matches!(loop_, solidcore_record::brep::Loop::Edges { .. }));
        }
    }
    lifted.validate().expect("a valid record");
}

/// Every use's pcurve is the exact image of its edge **inside** the span.
///
/// `solidcore`'s own record check samples `t0`, the middle and `t1`. On a
/// quarter-aligned span the middle lands on a grid half-integer, where two
/// different parameterisations can agree by construction, so that check alone
/// cannot tell "the label shares the face's grid" from "it happens to meet at
/// three points". Sixteen interior samples per use can.
fn assert_pcurves_are_exact(record: &Record) {
    let mut uses = 0;
    for (face_index, face) in record.faces.iter().enumerate() {
        let kernel = solidcore_convert::surface(face.parameters())
            .unwrap_or_else(|error| panic!("face {face_index}: {error}"));
        let surface = solidcore::eval::SurfaceEval::new(&kernel)
            .unwrap_or_else(|error| panic!("face {face_index}: {error}"));
        for loop_ in &face.loops {
            let Loop::Edges { edges } = loop_ else {
                continue;
            };
            for use_ in edges {
                let edge = &record.edges[use_.edge as usize];
                let kcurve = solidcore_convert::curve(edge.parameters())
                    .unwrap_or_else(|error| panic!("edge {}: {error}", use_.edge));
                let curve = solidcore::eval::CurveEval::new(&kcurve)
                    .unwrap_or_else(|error| panic!("edge {}: {error}", use_.edge));
                let [t0, t1] = edge.interval;
                for step in 0..=16 {
                    let t = t0 + (t1 - t0) * f64::from(step) / 16.0;
                    let uv = solidcore_ops::pcurve::eval(&use_.pcurve, t)
                        .unwrap_or_else(|| panic!("face {face_index}: pcurve at {t}"));
                    let on_surface = surface
                        .point(uv)
                        .unwrap_or_else(|| panic!("face {face_index}: surface point at {uv:?}"));
                    let on_edge = curve
                        .point(t)
                        .unwrap_or_else(|| panic!("edge {} at {t}", use_.edge));
                    let gap = ((on_surface.x - on_edge.x).powi(2)
                        + (on_surface.y - on_edge.y).powi(2)
                        + (on_surface.z - on_edge.z).powi(2))
                    .sqrt();
                    let scale = 1.0 + on_edge.x.abs().max(on_edge.y.abs()).max(on_edge.z.abs());
                    assert!(
                        gap <= 1e-9 * scale,
                        "face {face_index} edge {} t={t}: the pcurve lifts {gap} off its \
                         edge (scale {scale})",
                        use_.edge
                    );
                }
                uses += 1;
            }
        }
    }
    assert!(uses >= 1, "the body carries no pcurved use");
}

fn relift(body: &BrepData) -> Record {
    to_record(body, DEFAULT_TOLERANCE).expect("the body lifts")
}

#[test]
fn pcurves_are_exact_at_sixteen_interior_samples() {
    // A ring band (circles + seam), a frustum (circles + a generator), a full
    // cone (a circle + the apex generator), a sphere's meridian bigon and a
    // torus's tube/equator square: the five parameterisations this build
    // writes.
    let cylinder = solidcore_ops::make_cylinder([0.0, 0.0, 0.0], 1.0, 2.0).expect("cylinder");
    let (record, _) = cylinder.to_record().expect("record");
    assert_pcurves_are_exact(&relift(&from_record(&record).expect("neutral")));

    let cone = solidcore_ops::make_cone([0.0, 0.0, 0.0], 1.0, 0.5, 2.0).expect("cone");
    let (record, _) = cone.to_record().expect("record");
    assert_pcurves_are_exact(&relift(&from_record(&record).expect("neutral")));

    let full = solidcore_ops::make_cone([0.0, 0.0, 0.0], 1.0, 0.0, 2.0).expect("full cone");
    let (record, _) = full.to_record().expect("record");
    assert_pcurves_are_exact(&relift(&from_record(&record).expect("neutral")));

    let sphere = solidcore_ops::make_sphere([0.0, 0.0, 0.0], 1.0).expect("sphere");
    let (record, _) = sphere.to_record().expect("record");
    assert_pcurves_are_exact(&relift(&from_record(&record).expect("neutral")));

    let torus = solidcore_ops::make_torus([0.0, 0.0, 0.0], 2.0, 0.5).expect("torus");
    let (record, _) = torus.to_record().expect("record");
    assert_pcurves_are_exact(&relift(&from_record(&record).expect("neutral")));
}

/// A rim circle labelled on another frame (what a SAT import keeps) is
/// relabelled onto the face's frame before the record is written, so the
/// kernel's `u0` offset is zero — and the pcurves stay exact.
#[test]
fn a_rim_circle_labelled_on_another_frame_is_relabelled_onto_the_face() {
    let cylinder = solidcore_ops::make_cylinder([10.3, -4.7, 2.9], 1.0, 2.0).expect("cyl");
    let (record, _) = cylinder.to_record().expect("record");
    let mut neutral = from_record(&record).expect("neutral");
    // Rotate each boundary circle's own `u_dir` by 0.3 rad about its normal:
    // the same point set, a different parameter zero.
    for face in &mut neutral.shells[0].faces {
        for loop_ in &mut face.loops {
            for curve in &mut loop_.edges {
                if let cad_domain::BrepCurve::Circle { normal, u_dir, .. } = curve {
                    let (s, c) = 0.3f64.sin_cos();
                    let n = *normal;
                    let u = *u_dir;
                    let cross = Point3 {
                        x: n.y * u.z - n.z * u.y,
                        y: n.z * u.x - n.x * u.z,
                        z: n.x * u.y - n.y * u.x,
                    };
                    *u_dir = Point3 {
                        x: u.x * c + cross.x * s,
                        y: u.y * c + cross.y * s,
                        z: u.z * c + cross.z * s,
                    };
                }
            }
        }
    }
    let lifted = relift(&neutral);
    // Every written circle now shares its face's `x` axis: the angle the
    // kernel adds (`u0`) is zero.
    let mut checked = 0;
    for face in &lifted.faces {
        let solidcore_record::brep::Surface::Cylinder { frame, .. } = &face.surface else {
            continue;
        };
        let x = point(frame.x_axis);
        for loop_ in &face.loops {
            let Loop::Edges { edges } = loop_ else {
                continue;
            };
            for use_ in edges {
                let solidcore_record::brep::Curve::Circle { frame, .. } =
                    &lifted.edges[use_.edge as usize].curve
                else {
                    continue;
                };
                let labelled = point(frame.x_axis);
                assert!(
                    dot(labelled, x) >= 1.0 - 1e-12,
                    "the rim's label moved onto the face's frame: {labelled:?} vs {x:?}"
                );
                checked += 1;
                break;
            }
        }
    }
    assert_eq!(checked, 1, "one lateral face, one rim");
    assert_pcurves_are_exact(&lifted);
}

/// An arc whose ends are **off** the shared quarter grid still lifts a plane
/// body: its form is built on its own knots, so the form's span *is* the trim
/// (the record requires a use's pcurve to span exactly the edge's interval).
/// On a curved face the same arc has no exact pcurve and is refused — that is
/// the documented boundary, not a silent approximation.
#[test]
fn an_arc_off_the_quarter_grid_still_lifts_a_plane_body() {
    // A half disc whose two half-arcs start at 0.3 rad instead of 0: the same
    // shape, a parameterisation off the grid `eval::form` works on.
    let arc = |start: f64, end: f64| cad_domain::BrepCurve::Arc {
        center: p(0.0, 0.0, 0.0),
        normal: p(0.0, 0.0, 1.0),
        u_dir: p(1.0, 0.0, 0.0),
        radius: 1.0,
        start: p(start.cos(), start.sin(), 0.0),
        end: p(end.cos(), end.sin(), 0.0),
    };
    let offset = 0.3;
    let upper = arc(offset, offset + std::f64::consts::PI);
    let lower = arc(offset + std::f64::consts::PI, offset);
    let loops = vec![cad_domain::BrepLoop {
        edges: vec![upper, lower],
    }];
    let face = |id: u32, reversed: bool| cad_domain::BrepFace {
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
    let body = BrepData {
        shells: vec![cad_domain::BrepShell {
            id: 0,
            faces: vec![face(0, false), face(1, true)],
        }],
        placement: None,
        tolerance: None,
    };
    let record = relift(&body);
    for edge in &record.edges {
        let span = edge.interval;
        let form = edge.nurbs.as_ref().expect("an arc carries its form");
        let (lo, hi) = match form {
            solidcore_record::brep::Curve::RationalSpline { knots, .. }
            | solidcore_record::brep::Curve::Spline { knots, .. } => {
                (knots[0], knots[knots.len() - 1])
            }
            other => panic!("not a spline: {other:?}"),
        };
        assert!(
            (span[0] - lo).abs() < 1e-12 && (span[1] - hi).abs() < 1e-12,
            "the form spans [{lo}, {hi}] but the trim is {span:?}"
        );
    }
    // The pcurve is the projection of that form, so it spans the same interval.
    assert_pcurves_are_exact(&record);
}

#[test]
fn a_circle_boundary_that_does_not_chain_is_structurally_refused() {
    // A full circle is now *writable* (P2.1b part 2), so the old "a circle
    // boundary is a curved-lift refusal" pin is gone. A box face whose first
    // side is a lone full circle is not a closed loop any more, and that is a
    // structural refusal with a structural code — not a curved-geometry one.
    let mut brep = unit_box();
    brep.shells[0].faces[0].loops[0].edges[0] = cad_domain::BrepCurve::Circle {
        center: p(0.5, 0.5, 0.0),
        normal: p(0.0, 0.0, 1.0),
        u_dir: p(1.0, 0.0, 0.0),
        radius: 0.5,
    };
    let refusal = to_record(&brep, DEFAULT_TOLERANCE).expect_err("circle does not chain");
    assert_eq!(refusal.code, LIFT_UNSUPPORTED);
    assert!(
        refusal.detail.contains("not chained")
            || refusal.detail.contains("open")
            || refusal.detail.contains("used 1 time"),
        "the refusal names the broken loop: {}",
        refusal.detail
    );
}

#[test]
fn a_second_shell_is_refused() {
    let mut brep = unit_box();
    let shell = brep.shells[0].clone();
    brep.shells.push(shell);
    let refusal = to_record(&brep, DEFAULT_TOLERANCE).expect_err("two shells refused");
    assert_eq!(refusal.code, LIFT_UNSUPPORTED);
    assert!(refusal.detail.contains("2 shells"), "{}", refusal.detail);
}

#[test]
fn a_placed_body_is_baked_into_the_record() {
    let mut brep = unit_box();
    brep.tolerance = Some(0.5);
    brep.placement = Some(cad_domain::BrepPlacement {
        matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: p(10.0, 0.0, 0.0),
        scale: 2.0,
    });
    let record = crate::ops::lift(&brep).expect("placed body lifts");
    // The geometry was doubled by the placement, so the length tolerance is
    // doubled with it.
    assert_eq!(record.tolerance, 1.0);
    assert_eq!(record.edges[0].tolerance, 1.0);
    let min = record
        .vertices
        .iter()
        .map(|vertex| vertex.point[0])
        .fold(f64::INFINITY, f64::min);
    let max = record
        .vertices
        .iter()
        .map(|vertex| vertex.point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    assert!((min - 10.0).abs() < 1e-12, "min {min}");
    assert!((max - 12.0).abs() < 1e-12, "max {max}");
}

#[test]
fn a_negative_tolerance_is_refused() {
    let refusal = to_record(&unit_box(), 0.0).expect_err("zero tolerance refused");
    assert_eq!(refusal.code, LIFT_UNSUPPORTED);
    assert!(refusal.detail.contains("tolerance"), "{}", refusal.detail);
}

#[test]
fn a_stated_face_tolerance_becomes_the_record_tolerance_or_is_refused_by_name() {
    // Body 'unspecified' + one stated face: the face's number is the record's
    // tolerance. Dropping it (and lifting at 1e-7) would silently widen a
    // producer's geometry checks.
    let mut brep = unit_box();
    brep.tolerance = None;
    brep.shells[0].faces[0].tolerance = Some(0.5);
    let record = crate::ops::lift(&brep).expect("a stated face tolerance is honoured");
    assert_eq!(record.tolerance, 0.5);
    assert_eq!(record.edges[0].tolerance, 0.5);

    // Several stated faces: the coarsest wins (one record tolerance must cover
    // them all).
    brep.shells[0].faces[1].tolerance = Some(0.2);
    brep.shells[0].faces[2].tolerance = Some(0.75);
    let record = crate::ops::lift(&brep).expect("the coarsest face tolerance wins");
    assert_eq!(record.tolerance, 0.75);

    // Nothing stated: our own default, not a fabricated producer number.
    brep.shells[0].faces[2].tolerance = None;
    brep.shells[0].faces[1].tolerance = None;
    brep.shells[0].faces[0].tolerance = None;
    let record = crate::ops::lift(&brep).expect("no stated tolerance");
    assert_eq!(record.tolerance, DEFAULT_TOLERANCE);

    // A body tolerance that a face agrees with is fine...
    brep.tolerance = Some(0.25);
    brep.shells[0].faces[0].tolerance = Some(0.25);
    let record = crate::ops::lift(&brep).expect("an agreeing face tolerance");
    assert_eq!(record.tolerance, 0.25);

    // ...but one that differs materially is refused by name, with both numbers
    // and the face id: the record cannot carry both, and picking one silently
    // would change a geometry check the producer stated.
    brep.shells[0].faces[3].tolerance = Some(0.5);
    let refusal = to_record(&brep, 0.25).expect_err("a divergent face tolerance");
    assert_eq!(refusal.code, LIFT_UNSUPPORTED);
    assert!(
        refusal.detail.contains("face 3")
            && refusal.detail.contains("0.5")
            && refusal.detail.contains("0.25"),
        "the refusal names the face and both numbers: {}",
        refusal.detail
    );

    // A nonsensical stated face tolerance is refused rather than ignored.
    let mut bad = unit_box();
    bad.tolerance = None;
    bad.shells[0].faces[0].tolerance = Some(0.0);
    let refusal = to_record(&bad, DEFAULT_TOLERANCE).expect_err("zero face tolerance");
    assert_eq!(refusal.code, LIFT_UNSUPPORTED);
    assert!(refusal.detail.contains("face 0"), "{}", refusal.detail);
}

#[test]
fn lift_takes_the_body_tolerance_and_none_uses_the_default() {
    let mut brep = unit_box();
    brep.tolerance = Some(0.25);
    let record = crate::ops::lift(&brep).expect("box lifts");
    assert_eq!(record.tolerance, 0.25);
    for edge in &record.edges {
        assert_eq!(edge.tolerance, 0.25);
    }
    for vertex in &record.vertices {
        assert_eq!(vertex.tolerance, 0.25);
    }

    brep.tolerance = None;
    let record = crate::ops::lift(&brep).expect("box lifts");
    assert_eq!(record.tolerance, DEFAULT_TOLERANCE);
}

/// One table: each refusal site, and the one code it must report. This is the
/// guard against code conflation — `solid.lift_requires_planar` is only for
/// curved geometry, structural problems are `solid.lift_unsupported`, and a
/// bad frame is `solid.placement_not_supported`.
#[test]
fn refusal_codes_are_not_conflated() {
    let mut cases: Vec<(&str, BrepData, &'static str)> = Vec::new();

    // (1) the one remaining `lift_requires_planar` site: a boundary curve kind
    // this build does not carry. A curved *surface* is no longer one (it is
    // written with its exact form), and neither is a full circle.
    let mut unsupported = unit_box();
    unsupported.shells[0].faces[0].loops[0].edges[0] = cad_domain::BrepCurve::Unsupported {
        type_key: "b-spline".into(),
    };
    cases.push((
        "unsupported boundary curve",
        unsupported,
        LIFT_REQUIRES_PLANAR,
    ));

    // (2) structural refusals.
    let mut circle = unit_box();
    circle.shells[0].faces[0].loops[0].edges[0] = cad_domain::BrepCurve::Circle {
        center: p(0.5, 0.5, 0.0),
        normal: p(0.0, 0.0, 1.0),
        u_dir: p(1.0, 0.0, 0.0),
        radius: 0.5,
    };
    cases.push(("a circle that does not chain", circle, LIFT_UNSUPPORTED));

    // A curved face that states no (u, v) domain: refused by name, not given
    // an invented box.
    let mut no_domain = unit_box();
    no_domain.shells[0].faces[0].surface = BrepSurface::Cylinder {
        origin: p(0.0, 0.0, 0.0),
        axis: p(0.0, 0.0, 1.0),
        ref_dir: p(1.0, 0.0, 0.0),
        radius: 1.0,
    };
    no_domain.shells[0].faces[0].domain = None;
    cases.push((
        "a curved face without a domain",
        no_domain,
        LIFT_UNSUPPORTED,
    ));

    // A surface kind this build cannot write.
    let mut unsupported_surface = unit_box();
    unsupported_surface.shells[0].faces[0].surface = BrepSurface::Unsupported {
        type_key: "blend".into(),
    };
    cases.push((
        "an unsupported surface",
        unsupported_surface,
        LIFT_UNSUPPORTED,
    ));

    let mut reversed = unit_box();
    reversed.shells[0].faces[0].reversed = true;
    cases.push(("reversed face", reversed, LIFT_UNSUPPORTED));

    let mut duplicated = unit_box();
    let duplicate = duplicated.shells[0].faces[0].clone();
    duplicated.shells[0].faces.push(duplicate);
    cases.push(("duplicated face", duplicated, LIFT_UNSUPPORTED));

    // A loop written backwards (order reversed *and* each segment flipped):
    // still a closed cycle, but it winds the other way round the face.
    let mut reversed_loop = unit_box();
    let edges = &mut reversed_loop.shells[0].faces[0].loops[0].edges;
    *edges = edges
        .iter()
        .rev()
        .map(|curve| match curve {
            cad_domain::BrepCurve::Line { start, end } => cad_domain::BrepCurve::Line {
                start: *end,
                end: *start,
            },
            other => other.clone(),
        })
        .collect();
    cases.push(("reversed loop", reversed_loop, LIFT_UNSUPPORTED));

    let mut two_shells = unit_box();
    let shell = two_shells.shells[0].clone();
    two_shells.shells.push(shell);
    cases.push(("two shells", two_shells, LIFT_UNSUPPORTED));

    let mut sheet = unit_box();
    sheet.shells[0].faces.truncate(1);
    cases.push(("open sheet", sheet, LIFT_UNSUPPORTED));

    // (3) frame refusals: a plane normal cannot be zero.
    let mut degenerate = unit_box();
    degenerate.shells[0].faces[0].surface = BrepSurface::Plane {
        origin: p(0.0, 0.0, 0.0),
        normal: p(0.0, 0.0, 0.0),
        u_dir: p(1.0, 0.0, 0.0),
    };
    cases.push(("degenerate frame", degenerate, PLACEMENT_NOT_SUPPORTED));

    for (name, body, expected) in cases {
        let refusal = to_record(&body, DEFAULT_TOLERANCE).expect_err(name);
        assert_eq!(refusal.code, expected, "{name}: {refusal:?}");
        assert!(!refusal.detail.is_empty(), "{name}: empty detail");
    }

    // Tolerance is structural, not curved.
    let refusal = to_record(&unit_box(), 0.0).expect_err("tolerance");
    assert_eq!(refusal.code, LIFT_UNSUPPORTED);
}
