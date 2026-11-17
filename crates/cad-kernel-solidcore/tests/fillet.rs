//! Contract test for a **kernel-produced round fillet** in this crate's neutral
//! B-rep (ADR-0036 P2.1c-2, first slice).
//!
//! The pinned kernel records a round as `Surface::Cone { half_angle: 0.0 }` plus a
//! rational NURBS net, which is a cylinder. `record::surface` maps that label to
//! [`BrepSurface::Cylinder`] (an exact-zero test, unit-tested in `src`), so a round
//! the kernel produced lifts, measures and booleans through the existing
//! trimmed-side path. This file pins, against the real kernel:
//!
//! 1. the blend's own record: face count, **label census** (`cone`, never `blend`
//!    at this pin), the presence and shape of `Face.nurbs` (3×2, knots, degrees,
//!    weights `[1, 1, √2⁄2, √2⁄2, 1, 1]`) and the loop families;
//! 2. `Body::mass_properties(1e-6)` against the closed form
//!    `V = 1 − (1 − π/4)·r²` for one full-length edge of a unit cube, at
//!    r = 0.1 / 0.25 / 0.49;
//! 3. the neutral model's behaviour on that body: mass properties and a boolean
//!    through [`SolidModeler`], including the cylinder face the four faces of the
//!    round become, and the volume of the result;
//! 4. the two mappings that stay refused — the round's **net** as
//!    [`BrepSurface::Nurbs`] (no exact pcurve of a circle on a spline surface) and
//!    a neutral **cone with no slope** written back out (the record writer needs a
//!    slope) — which is why the reclassification happens on the way in.
//!
//! # What this file cannot do
//!
//! `record::from_record` is crate-private, so the reader is reached the only way a
//! caller reaches it: through an operation that returns a body. The walk below
//! (`record` → neutral body) therefore restates the reader — bounded region, face
//! shells, oriented faces, literal loops — with the surface mapping the reader
//! performs, and the boolean at the end is the **real** reader's output, asserted
//! face-for-face. The mapping itself (`Cone { half_angle: 0.0 }` → `Cylinder`) is
//! pinned by unit tests next to it in `src/record/surface.rs`.
//!
//! # Evidence boundary
//!
//! Linux x86-64, the pinned kernel (`solidcore` `55ae49e`), no mock. No GPU, no
//! pixels, no platform other than Linux (ADR-0036). The volume tolerances are the
//! relative accuracy the mass-properties operation is *asked* for (`1e-6`), which
//! the kernel estimates rather than bounds; the pinned absolute values below are
//! measurements of this pin.

use cad_domain::{
    BrepCurve, BrepData, BrepFace, BrepLoop, BrepShell, BrepSurface, DocumentId, Point3, TaskStamp,
};
use cad_kernel_adapter::{
    execute_solid, BooleanOp, SolidEdgeSelector, SolidOperation, SolidOutcome, SolidRequest,
};
use cad_kernel_solidcore::{box_brep, SolidCoreModeler, LIFT_UNSUPPORTED, PCURVE_NOT_EXACT};
use solidcore_ops::{blend_ordered, make_box, Blend, BlendOptions, Body, Corners};
use solidcore_record::brep::{Brep as Record, Curve, Loop, OrientedEdge, Shell, Surface};

/// The round radius the test asks for (a tenth of the cube's side).
const R: f64 = 0.1;

/// The cube's side, so `V = SIDE³` and the removed prism is `(1 − π/4)·r²·SIDE`.
const SIDE: f64 = 1.0;

/// The relative accuracy the mass-properties operation is asked for.
const ACCURACY: f64 = 1e-6;

/// The volume of a unit cube with one full-length edge rounded by `radius`.
fn closed_form(radius: f64) -> f64 {
    SIDE * SIDE * SIDE - (1.0 - std::f64::consts::FRAC_PI_4) * radius * radius
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn cross(a: Point3, b: Point3) -> Point3 {
    p(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn unit(v: Point3) -> Point3 {
    let length = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    p(v.x / length, v.y / length, v.z / length)
}

fn point_of(values: [f64; 3]) -> Point3 {
    p(values[0], values[1], values[2])
}

/// The record's own name for a surface, for the label census.
fn label(surface: &Surface) -> &'static str {
    match surface {
        Surface::Plane { .. } => "plane",
        Surface::PlaneThrough { .. } => "plane-through",
        Surface::Cylinder { .. } => "cylinder",
        Surface::Cone { .. } => "cone",
        Surface::Sphere { .. } => "sphere",
        Surface::Torus { .. } => "torus",
        Surface::BallPatch { .. } => "ball-patch",
        Surface::Spline { .. } => "spline",
        Surface::RationalSpline { .. } => "rational-spline",
        Surface::PlacedRevolution { .. } => "placed-revolution",
        Surface::Blend { .. } => "blend",
    }
}

fn curve_label(curve: &Curve) -> &'static str {
    match curve {
        Curve::Line { .. } => "line",
        Curve::Circle { .. } => "circle",
        Curve::Ellipse { .. } => "ellipse",
        Curve::Polyline { .. } => "polyline",
        Curve::Spline { .. } => "spline",
        Curve::RationalSpline { .. } => "rational-spline",
        Curve::SurfaceIntersection { .. } => "surface-intersection",
    }
}

/// A blended body: the kernel `Body`, its record, and which edge was rounded.
struct Blended {
    body: Body,
    record: Record,
    made: Vec<usize>,
    edge: usize,
}

/// A unit cube, one straight edge rounded by `kind` at `radius`.
///
/// The edge is the first shared `Line` of the box record: the record's canonical
/// order makes that deterministic, and a full-length edge of a 1×1×1 cube is what
/// the closed form above describes.
fn rounded_cube(kind: Blend, radius: f64) -> Blended {
    let seed = match make_box([0.0, 0.0, 0.0], [SIDE, SIDE, SIDE]) {
        Ok(seed) => seed,
        Err(error) => panic!("the pinned kernel refused a unit box: {error:?}"),
    };
    let (record, _order) = match seed.to_record() {
        Ok(record) => record,
        Err(error) => panic!("the unit box has no record: {error:?}"),
    };
    let (body, order) = match Body::from_record(&record, false) {
        Ok(body) => body,
        Err(error) => panic!("the unit box record does not read back: {error:?}"),
    };
    let edge = record
        .edges
        .iter()
        .position(|edge| matches!(edge.curve, Curve::Line { .. }) && edge.uses.len() == 2)
        .expect("a box has shared straight edges");
    let blended = match blend_ordered(
        &body,
        &order,
        kind,
        &[edge],
        &[radius],
        &[],
        Corners::Filled,
        BlendOptions::default(),
    ) {
        Ok(blended) => blended,
        Err(error) => panic!("the pinned kernel refused {kind:?} at {radius}: {error:?}"),
    };
    let (after, _order) = match body.to_record() {
        Ok(record) => record,
        Err(error) => panic!("the blended body has no record: {error:?}"),
    };
    Blended {
        body,
        record: after,
        made: blended.made,
        edge,
    }
}

/// The label census of a record, in face order.
fn census(record: &Record) -> Vec<&'static str> {
    record
        .faces
        .iter()
        .map(|face| label(&face.surface))
        .collect()
}

/// The loop families of one face, in loop order (each use tagged with its curve).
fn loop_families(record: &Record, face: &solidcore_record::brep::Face) -> Vec<Vec<&'static str>> {
    face.loops
        .iter()
        .map(|loop_| match loop_ {
            Loop::Edges { edges } => edges
                .iter()
                .map(|use_| curve_label(&record.edges[use_.edge as usize].curve))
                .collect(),
            Loop::Vertex { .. } => vec!["vertex"],
        })
        .collect()
}

/// The face's `Face.nurbs` net as a neutral NURBS surface.
fn net(surface: &Surface) -> BrepSurface {
    match surface {
        Surface::Spline {
            u_knots,
            v_knots,
            points,
        } => BrepSurface::Nurbs {
            u_knots: u_knots.clone(),
            v_knots: v_knots.clone(),
            // A `Spline` carries no weights: every one is exactly 1.
            points: points
                .iter()
                .map(|row| row.iter().map(|q| [q[0], q[1], q[2], 1.0]).collect())
                .collect(),
        },
        Surface::RationalSpline {
            u_knots,
            v_knots,
            homogeneous,
        } => BrepSurface::Nurbs {
            u_knots: u_knots.clone(),
            v_knots: v_knots.clone(),
            points: homogeneous.clone(),
        },
        other => panic!("the face's nurbs is not a net but a '{}'", label(other)),
    }
}

/// One record face's neutral surface, **as the reader maps it** (`record::surface`):
/// planes analytic, and a cone of exactly zero slope as the cylinder it is. Any
/// other label is a measurement this file does not expect.
fn reader_surface(face: &solidcore_record::brep::Face) -> BrepSurface {
    match &face.surface {
        Surface::Plane {
            origin,
            u_axis,
            v_axis,
        } => {
            let u = point_of(*u_axis);
            BrepSurface::Plane {
                origin: point_of(*origin),
                normal: unit(cross(u, point_of(*v_axis))),
                u_dir: unit(u),
            }
        }
        // The only non-planar label a one-edge round of a box produces, and the
        // one the reader reclassifies: `half_angle == 0.0` is a cylinder.
        Surface::Cone {
            frame,
            radius_at_origin,
            half_angle,
        } => {
            assert_eq!(
                *half_angle, 0.0,
                "the round's label is a zero-slope cone; a real cone is a different face"
            );
            BrepSurface::Cylinder {
                origin: point_of(frame.origin),
                axis: unit(cross(
                    unit(point_of(frame.x_axis)),
                    unit(point_of(frame.y_axis)),
                )),
                ref_dir: unit(point_of(frame.x_axis)),
                radius: *radius_at_origin,
            }
        }
        other => panic!("an unexpected face label '{}'", label(other)),
    }
}

/// One record use as this crate's neutral curve (the reader's own orientation
/// rule, restated here because `record::curve` is private).
fn neutral_curve(record: &Record, use_: &OrientedEdge) -> BrepCurve {
    let edge = &record.edges[use_.edge as usize];
    let vertex = |index: usize| point_of(record.vertices[edge.vertices[index] as usize].point);
    match &edge.curve {
        Curve::Line { .. } => BrepCurve::Line {
            start: vertex(if use_.forward { 0 } else { 1 }),
            end: vertex(if use_.forward { 1 } else { 0 }),
        },
        Curve::Circle { frame, radius } => {
            let center = point_of(frame.origin);
            let x = unit(point_of(frame.x_axis));
            let normal = cross(x, point_of(frame.y_axis));
            if edge.vertices[0] == edge.vertices[1] {
                BrepCurve::Circle {
                    center,
                    normal: unit(normal),
                    u_dir: x,
                    radius: *radius,
                }
            } else if use_.forward {
                BrepCurve::Arc {
                    center,
                    normal: unit(normal),
                    u_dir: x,
                    radius: *radius,
                    start: vertex(0),
                    end: vertex(1),
                }
            } else {
                BrepCurve::Arc {
                    center,
                    normal: unit(p(-normal.x, -normal.y, -normal.z)),
                    u_dir: x,
                    radius: *radius,
                    start: vertex(1),
                    end: vertex(0),
                }
            }
        }
        other => panic!(
            "an unexpected boundary curve '{}' (the neutral model has no variant for it)",
            curve_label(other)
        ),
    }
}

/// The blend record as a neutral body: the reader's walk (bounded regions → face
/// shells → oriented faces → literal loops) with [`reader_surface`]'s mapping.
fn neutral(record: &Record) -> BrepData {
    let mut faces = Vec::new();
    for region in record.regions.iter().skip(1) {
        for &shell_index in &region.shells {
            let Shell::Faces { faces: uses, .. } = &record.shells[shell_index as usize] else {
                panic!("the blended body is not a face shell");
            };
            for use_ in uses {
                let face = &record.faces[use_.face as usize];
                let loops = face
                    .loops
                    .iter()
                    .map(|loop_| {
                        let Loop::Edges { edges } = loop_ else {
                            panic!("the blended body has a vertex loop");
                        };
                        BrepLoop {
                            edges: edges
                                .iter()
                                .map(|use_| neutral_curve(record, use_))
                                .collect(),
                        }
                    })
                    .collect();
                faces.push(BrepFace {
                    id: use_.face,
                    surface: reader_surface(face),
                    reversed: !use_.forward,
                    loops,
                    // The record's `(u, v)` box, verbatim: the only source of it.
                    domain: Some(face.domain),
                    tolerance: None,
                });
            }
        }
    }
    BrepData {
        shells: vec![BrepShell { id: 0, faces }],
        placement: None,
        tolerance: Some(record.tolerance),
    }
}

/// The same body with its round face carried as the record's **net** instead of its
/// label: the "map a blend to a NURBS form" hypothesis, kept as a refusal contract.
fn with_round_as_net(record: &Record) -> BrepData {
    let index = record
        .faces
        .iter()
        .position(|face| !matches!(face.surface, Surface::Plane { .. }))
        .expect("the blended body has one non-planar face: the round");
    let source = match record.faces[index].nurbs.as_ref() {
        Some(source) => net(source),
        None => panic!("the round face carries no net"),
    };
    let mut brep = neutral(record);
    // A neutral face's id is its record face index.
    for face in brep.shells.iter_mut().flat_map(|shell| &mut shell.faces) {
        if face.id == index as u32 {
            face.surface = source.clone();
        }
    }
    brep
}

/// Run one operation through the public modeler seam (the only public route to
/// this crate's `lift`/`from_record`).
fn run(operation: SolidOperation<'_>) -> SolidOutcome {
    let modeler = SolidCoreModeler::new();
    let request = SolidRequest {
        stamp: TaskStamp::new(DocumentId(1), 1),
        operation,
    };
    let result = execute_solid(&modeler, &request, &|| false)
        .expect("the request is well formed and nothing is cancelled");
    result.outcome
}

/// The mass properties of `brep`, asserting the seam produced them.
fn volume(brep: &BrepData) -> f64 {
    let outcome = run(SolidOperation::MassProperties { brep, density: 1.0 });
    match &outcome {
        SolidOutcome::Success(product) => match product.mass() {
            Some(mass) => mass.volume,
            None => panic!("mass properties returned no mass: {outcome:?}"),
        },
        other => panic!("mass properties were refused: {other:?}"),
    }
}

/// The stable code and verbatim detail of an `Unsupported` outcome.
fn refusal(outcome: &SolidOutcome) -> (&'static str, &str) {
    match outcome {
        SolidOutcome::Unsupported { reason } => (reason.code, reason.detail.as_str()),
        other => panic!("expected an Unsupported outcome, got {other:?}"),
    }
}

/// Assert a measured volume against the closed form at the requested relative
/// accuracy, naming both numbers when it misses.
fn assert_closed_form(measured: f64, expected: f64, what: &str) {
    let relative = (measured - expected).abs() / expected.abs();
    assert!(
        relative <= ACCURACY,
        "{what}: measured volume {measured:.15} against the closed form {expected:.15} \
         (relative {relative:.3e} > {ACCURACY:.0e}); this is a measurement, not a loose bound"
    );
}

/// Assert a measured volume bit-for-bit at the precision this pin reproduces it
/// (`1e-9` absolute), so a change in the kernel or the seam is not absorbed by the
/// `1e-6` closed-form tolerance above.
fn assert_pinned(measured: f64, pinned: f64, what: &str) {
    assert!(
        (measured - pinned).abs() <= 1e-9,
        "{what}: measured volume {measured:.15} is not the pinned {pinned:.15} (this pin's \
         measurement, not a tolerance)"
    );
}

#[test]
fn a_round_on_one_unit_cube_edge_is_a_zero_slope_cone_with_a_rational_net() {
    let blended = rounded_cube(Blend::Round, R);
    let record = &blended.record;

    // The edge is the record's first shared straight edge, and the kernel accepted
    // the round: 12 edges → 15 (the rounded edge's 4 new ones less the old one),
    // 6 faces → 7.
    assert_eq!(blended.edge, 0, "the box record's first shared Line edge");
    assert_eq!(record.faces.len(), 7, "six box faces plus the round");
    assert_eq!(blended.made, vec![6], "the kernel's own list of made faces");
    assert_eq!(record.edges.len(), 15);
    assert!(
        record.regions.iter().skip(1).count() == 1
            && record.regions[1].shells.len() == 1
            && !record.regions[1].void,
        "one bounded region, one shell, no void"
    );

    // The label census: the round face is **not** a `Surface::Blend` at this pin.
    assert_eq!(
        census(record),
        vec!["plane", "plane", "plane", "plane", "plane", "plane", "cone"],
        "the round face's analytic label"
    );
    let round = &record.faces[6];
    let Surface::Cone {
        frame,
        radius_at_origin,
        half_angle,
    } = &round.surface
    else {
        panic!("the round face is not a cone: {}", label(&round.surface))
    };
    // A cone of zero half angle is a cylinder — the label the reader reclassifies.
    assert_eq!(*half_angle, 0.0, "the round's cone has no slope");
    assert!(
        (radius_at_origin - R).abs() < 1e-12,
        "the label's radius is the round's: {radius_at_origin}"
    );
    // The rounded edge is the cube's at x = 0, y = 1: its axis sits at (r, 1 − r).
    let axis = unit(cross(
        unit(point_of(frame.x_axis)),
        unit(point_of(frame.y_axis)),
    ));
    assert!(
        (axis.x).abs() < 1e-12 && (axis.y).abs() < 1e-12 && (axis.z - 1.0).abs() < 1e-12,
        "the round's axis runs along the box edge (+z): {axis:?}"
    );
    let origin = point_of(frame.origin);
    assert!(
        (origin.x - R).abs() < 1e-9 && (origin.y - (SIDE - R)).abs() < 1e-9,
        "the round's axis is at (r, 1 − r): {origin:?}"
    );

    // `Face.nurbs`: the net is a **rational** 3×2 grid of degree (2, 1).
    let Some(net) = round.nurbs.as_ref() else {
        panic!("the round face carries no net");
    };
    let Surface::RationalSpline {
        u_knots,
        v_knots,
        homogeneous,
    } = net
    else {
        panic!(
            "the round's net is a '{}', not a rational spline",
            label(net)
        )
    };
    assert_eq!(homogeneous.len(), 3, "3 control rows along u");
    assert_eq!(homogeneous[0].len(), 2, "2 control columns along v");
    assert!(
        homogeneous.iter().all(|row| row.len() == 2),
        "a rectangular control grid"
    );
    assert_eq!(
        u_knots,
        &vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        "6 u knots: degree 2 over [0, 1]"
    );
    assert_eq!(v_knots.len(), 4, "4 v knots: degree 1");
    for (knot, expected) in v_knots.iter().zip([0.25, 0.25, 0.75, 0.75]) {
        assert!(
            (knot - expected).abs() < 1e-12,
            "the v knots are the box's quarter lines: {v_knots:?}"
        );
    }
    assert_eq!(
        u_knots.len() - homogeneous.len() - 1,
        2,
        "u degree is implicit"
    );
    assert_eq!(
        v_knots.len() - homogeneous[0].len() - 1,
        1,
        "v degree is implicit"
    );

    // The weights, row by row: 1, 1 / √2⁄2, √2⁄2 / 1, 1. The net is genuinely
    // rational — the middle row's control points carry 1/√2 — so the "is the round
    // rational NURBS?" question is answered with numbers, not by symmetry.
    let weights: Vec<f64> = homogeneous
        .iter()
        .flat_map(|row| row.iter().map(|q| q[3]))
        .collect();
    let half = std::f64::consts::FRAC_1_SQRT_2;
    for (measured, expected) in weights.iter().zip([1.0, 1.0, half, half, 1.0, 1.0]) {
        assert!(
            (measured - expected).abs() < 1e-12,
            "the weight profile is 1, √2⁄2, 1: {weights:?}"
        );
    }
    assert!(
        weights.iter().any(|weight| (weight - 1.0).abs() > 1e-6),
        "at least one weight is not 1: {weights:?}"
    );
    // Homogeneous, not Cartesian: the coefficients are `[x·w, y·w, z·w, w]`. The
    // middle row carries w = 1/√2, so its stored numbers are not its coordinates;
    // dehomogenised, that control point sits on the corner of the two tangency
    // planes (x = 0, y = 1) — one radius out at 45°, which is why it needs a weight.
    let corner = homogeneous[1][0];
    assert!(
        (corner[1] - corner[1] / corner[3]).abs() > 1e-3,
        "the stored coefficients are premultiplied by the weight: {corner:?}"
    );
    assert!(
        (corner[0] / corner[3]).abs() < 1e-9 && (corner[1] / corner[3] - SIDE).abs() < 1e-9,
        "the weighted control point is the corner of the tangency planes: {corner:?}"
    );

    // The round's loop: two tangent **lines** and two **arcs** on the box's end
    // faces (an open circle is an arc in this record).
    assert_eq!(
        loop_families(record, round),
        vec![vec!["line", "circle", "line", "circle"]],
        "the round is bounded by two tangency lines and two end arcs"
    );
    // ... and the two end faces each gained one arc (4 → 5 uses); the two tangent
    // faces keep four lines (the rounded edge is replaced by the tangency line).
    let families = loop_families(record, &record.faces[4]);
    assert_eq!(families[0].len(), 5);
    assert!(families[0].contains(&"circle"));
    assert_eq!(loop_families(record, &record.faces[0])[0].len(), 4);

    // The `(u, v)` box the producer states is a positive finite box.
    let domain = round.domain;
    assert!(
        domain.iter().flatten().all(|value| value.is_finite())
            && domain[0][1] > domain[0][0]
            && domain[1][1] > domain[1][0],
        "the round states a real (u, v) box: {domain:?}"
    );

    // The kernel proves its own record again (topology, loops and agreement).
    match Body::from_record(record, false) {
        Ok((_body, _order)) => {}
        Err(error) => panic!("the blended record does not read back: {error:?}"),
    }

    // Volume against the closed form, at the accuracy the operation is asked for.
    // The measured value is 0.997853981730947 for r = 0.1, i.e. 9.7e-11 above the
    // closed form — the integrator's own error, not a modelling difference.
    let measured = match blended.body.mass_properties(ACCURACY) {
        Ok(estimate) => estimate.volume,
        Err(error) => panic!("the round's volume could not be integrated: {error:?}"),
    };
    assert_closed_form(measured, closed_form(R), "one round r = 0.1");
    assert_pinned(measured, 0.997_853_981_730_947, "one round r = 0.1");
}

#[test]
fn other_round_sizes_build_and_match_the_same_closed_form() {
    // 0.25 (a quarter of the side) and 0.49 (the largest that still leaves room on
    // both tangent faces) build, keep the same census, and remove the same prism.
    for radius in [0.25, 0.49] {
        let blended = rounded_cube(Blend::Round, radius);
        assert_eq!(blended.record.faces.len(), 7, "r = {radius}");
        assert_eq!(
            census(&blended.record),
            vec!["plane", "plane", "plane", "plane", "plane", "plane", "cone"],
            "r = {radius}"
        );
        let measured = match blended.body.mass_properties(ACCURACY) {
            Ok(estimate) => estimate.volume,
            Err(error) => panic!("r = {radius} has no volume: {error:?}"),
        };
        assert_closed_form(
            measured,
            closed_form(radius),
            &format!("one round r = {radius}"),
        );
    }
}

#[test]
fn a_curvature_round_is_a_bare_rational_net_with_spline_boundaries() {
    // The G² round is the contrast: it has no analytic label to fall back on, so the
    // face *is* its net and its boundary curves are splines, not lines and arcs.
    // Nothing here is mapped (there is no label), and the neutral model has no
    // spline *curve* either — this stays P2.1c-2's remaining gap.
    let blended = rounded_cube(Blend::Curvature, R);
    let record = &blended.record;
    assert_eq!(record.faces.len(), 7);
    assert_eq!(blended.made, vec![6]);
    assert_eq!(
        census(record),
        vec![
            "plane",
            "plane",
            "plane",
            "plane",
            "plane",
            "plane",
            "rational-spline"
        ],
        "the G² round's label is its own net"
    );
    let round = &record.faces[6];
    assert!(
        round.nurbs.is_none(),
        "a form-labelled face carries no separate nurbs"
    );
    let Surface::RationalSpline {
        u_knots,
        v_knots,
        homogeneous,
    } = &round.surface
    else {
        panic!("the G² round is not a rational net");
    };
    assert_eq!(homogeneous.len(), 4, "4 rows along u");
    assert_eq!(homogeneous[0].len(), 8, "8 columns along v");
    assert_eq!(u_knots.len(), 8, "8 u knots: degree 3");
    assert_eq!(v_knots.len(), 16, "16 v knots: degree 7");
    let weights: Vec<f64> = homogeneous
        .iter()
        .flat_map(|row| row.iter().map(|q| q[3]))
        .collect();
    assert!(
        weights.iter().any(|weight| (weight - 1.0).abs() > 1e-6),
        "the G² net is rational: {weights:?}"
    );
    let families = loop_families(record, round);
    for family in &families {
        assert!(
            family.iter().all(|kind| *kind == "spline"),
            "the G² round's boundary is splines: {families:?}"
        );
    }
    // The round itself is exact: an equal-area G² section removes the same prism.
    let measured = match blended.body.mass_properties(ACCURACY) {
        Ok(estimate) => estimate.volume,
        Err(error) => panic!("the G² round has no volume: {error:?}"),
    };
    assert_closed_form(measured, closed_form(R), "one curvature round r = 0.1");
}

#[test]
fn a_round_never_carries_a_blend_label_at_this_pin() {
    // Does the round face carry `Surface::Blend { definition }` with `Face.nurbs` as
    // its net? No — for `Blend::Round` the label is the analytic `Cone` (half angle
    // zero) and for `Blend::Curvature` it is the `RationalSpline` net itself.
    // `Surface::Blend` never appears, so `record::surface`'s "blend" refusal is not
    // the gate a fillet meets here.
    for kind in [Blend::Round, Blend::Curvature, Blend::Chord] {
        let blended = rounded_cube(kind, R);
        let labels = census(&blended.record);
        assert!(
            !labels.contains(&"blend"),
            "{kind:?} carried a Blend label: {labels:?}"
        );
    }
}

#[test]
fn a_kernel_round_lifts_measures_and_booleans_through_the_neutral_model() {
    let blended = rounded_cube(Blend::Round, R);
    let brep = neutral(&blended.record);

    // The neutral body the reader hands out: seven faces, and the round is the
    // cylinder its zero-slope cone label describes (`src/record/surface.rs`).
    assert_eq!(brep.shells.len(), 1);
    assert_eq!(brep.face_count(), 7);
    let rounds: Vec<&BrepFace> = brep
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .filter(|face| matches!(face.surface, BrepSurface::Cylinder { .. }))
        .collect();
    assert_eq!(rounds.len(), 1, "exactly the round is a cylinder");
    let BrepSurface::Cylinder { radius, .. } = rounds[0].surface else {
        unreachable!("filtered to a cylinder")
    };
    assert!(
        (radius - R).abs() < 1e-9,
        "the round's radius survives the label: {radius}"
    );

    // Mass properties: the lift writes the trimmed cylinder back (its exact form and
    // exact pcurves), the kernel integrates it, and the closed form comes back out.
    let measured = volume(&brep);
    assert_closed_form(measured, closed_form(R), "re-lifted round");
    assert_pinned(measured, 0.997_853_981_730_947, "re-lifted round");

    // A boolean proves the *real* reader: the result is `unlift`'s output, so its
    // round face is what `from_record` makes of the kernel's rebuilt record. The
    // notch removes 0.4 × 0.4 × 1 of the unit cube.
    let tool = box_brep(p(0.6, -0.1, -0.1), 0.5, 0.5, 1.2);
    let outcome = run(SolidOperation::Boolean {
        op: BooleanOp::Subtract,
        left: &brep,
        right: &tool,
    });
    let Some(cut) = outcome.body() else {
        panic!("the boolean over the re-lifted round was refused: {outcome:?}");
    };
    assert_eq!(cut.face_count(), 9, "seven faces plus the notch's two");
    let cylinders: Vec<&BrepFace> = cut
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .filter(|face| matches!(face.surface, BrepSurface::Cylinder { .. }))
        .collect();
    assert_eq!(cylinders.len(), 1, "the round comes back as one cylinder");
    let BrepSurface::Cylinder {
        origin,
        axis,
        radius,
        ..
    } = cylinders[0].surface
    else {
        unreachable!("filtered to a cylinder")
    };
    assert!(
        (radius - R).abs() < 1e-9,
        "the round's radius survives the boolean: {radius}"
    );
    assert!(
        (axis.z - 1.0).abs() < 1e-9,
        "the round's axis survives the boolean: {axis:?}"
    );
    assert!(
        (origin.x - R).abs() < 1e-9 && (origin.y - (SIDE - R)).abs() < 1e-9,
        "the round's axis is still at (r, 1 − r): {origin:?}"
    );
    // The round is carried as one trimmed band: a single loop of the two tangency
    // lines and the two end arcs. That is the P2.1b "trimmed side face" shape, so
    // its *display* is the partial one that path already documents (not measured
    // here: this test has no pixels).
    let kinds: Vec<&str> = cylinders[0]
        .loops
        .iter()
        .flat_map(|loop_| &loop_.edges)
        .map(|curve| match curve {
            BrepCurve::Line { .. } => "line",
            BrepCurve::Circle { .. } => "circle",
            BrepCurve::Arc { .. } => "arc",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["line", "arc", "line", "arc"]);
    assert_eq!(cylinders[0].loops.len(), 1);

    let notch = (SIDE - 0.6) * 0.4 * SIDE;
    let cut_volume = volume(cut);
    assert_closed_form(
        cut_volume,
        closed_form(R) - notch,
        "rounded cube less a notch",
    );
    assert_pinned(
        cut_volume,
        0.837_853_981_730_947,
        "rounded cube less a notch",
    );
}

#[test]
fn carrying_the_round_as_its_net_is_still_refused() {
    // Why the label is mapped and the net is not: our writer gives the kernel no
    // pcurve (it computes them), and the kernel's exact-pcurve call has no closed
    // form for an arc lying on a spline surface. The refusal is the lift gate, so it
    // is the same for every operation.
    let blended = rounded_cube(Blend::Round, R);
    let as_net = with_round_as_net(&blended.record);
    let outcome = run(SolidOperation::MassProperties {
        brep: &as_net,
        density: 1.0,
    });
    let (code, detail) = refusal(&outcome);
    assert_eq!(code, PCURVE_NOT_EXACT);
    assert_eq!(
        detail,
        "no exact pcurve for this face's boundary in its surface: pcurve of loop 0 use 1 \
         (Some([2.7755575615628914e-17, 0.9000000000000002, 0.0]) to Some([0.1, 1.0, 0.0])): \
         not supported yet: no exact pcurve of a circle on a spline surface",
        "a verbatim measurement of this pin's refusal: the arc ends are the round's"
    );
}

#[test]
fn a_neutral_cone_with_no_slope_cannot_be_written_back() {
    // The other half of the reason for the reader's reclassification: a neutral
    // `BrepSurface::Cone` with sin = 0 is written as `Surface::Cone { half_angle: 0 }`,
    // whose form the writer cannot build, so such a body is refused at every
    // operation. Mapping the record's zero-slope cone to *that* variant would
    // therefore strand the round instead of carrying it.
    let blended = rounded_cube(Blend::Round, R);
    let mut as_cone = neutral(&blended.record);
    for face in as_cone.shells.iter_mut().flat_map(|shell| &mut shell.faces) {
        if let BrepSurface::Cylinder {
            origin,
            axis,
            ref_dir,
            radius,
        } = face.surface
        {
            face.surface = BrepSurface::Cone {
                origin,
                axis,
                ref_dir,
                radius,
                sin_half_angle: 0.0,
                cos_half_angle: 1.0,
            };
        }
    }
    for (what, operation) in [
        (
            "mass",
            SolidOperation::MassProperties {
                brep: &as_cone,
                density: 1.0,
            },
        ),
        (
            "chamfer",
            SolidOperation::Chamfer {
                brep: &as_cone,
                selector: SolidEdgeSelector::NearestToPoint(p(0.5, SIDE, 0.0)),
                distance: 0.05,
                second: None,
            },
        ),
    ] {
        let outcome = run(operation);
        let (code, detail) = refusal(&outcome);
        assert_eq!(code, PCURVE_NOT_EXACT, "{what}: {detail}");
        assert_eq!(
            detail,
            "no exact pcurve for this face's boundary in its surface: portable B-rep: cone \
             dimensions",
            "{what}: a verbatim measurement of this pin's refusal"
        );
    }

    // A body whose round face the neutral model has no placeholder for at all is
    // refused by name rather than guessed: the writer names the key.
    let mut unlabelled = neutral(&blended.record);
    for face in unlabelled
        .shells
        .iter_mut()
        .flat_map(|shell| &mut shell.faces)
    {
        if matches!(face.surface, BrepSurface::Cylinder { .. }) {
            face.surface = BrepSurface::Unsupported {
                type_key: "blend".into(),
            };
        }
    }
    let outcome = run(SolidOperation::MassProperties {
        brep: &unlabelled,
        density: 1.0,
    });
    let (code, detail) = refusal(&outcome);
    assert_eq!(code, LIFT_UNSUPPORTED);
    assert!(
        detail.contains("unsupported surface 'blend'"),
        "the refusal names 'blend': {detail}"
    );
}
