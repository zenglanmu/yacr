//! ACIS circular arcs: an `ellipse-curve` whose `ratio` is 1 on an **open** edge
//! (a proper sub-span of a circle) must lift into a neutral `BrepCurve::Arc`,
//! tessellate, and refuse (by name) anything that is not really that arc.
//!
//! Evidence boundary: the synthetic SAT documents built here and the software
//! tessellator only — no third-party CAD file, no GPU, no other platform.

use super::acis::builder;
use super::*;

use cad_kernel_adapter::{
    BrepTessellator, GeometryHandle, SolidExchange, SolidTessellator, TessellationBudget,
    TessellationOutcome, TessellationRequest, TessellationTolerance,
};

// ---------------------------------------------------------------------------
// Circular arcs (`ellipse-curve` with `ratio == 1` on an open edge)
// ---------------------------------------------------------------------------

/// A one-face quarter disc in `z = 0`: a quarter arc from `(1, 0)` to
/// `(0, 1)` plus the two radii, as a SAT sheet.
///
/// `arc_end` lets a test move the arc's far vertex off the circle, which must stay
/// a named refusal instead of a re-projected arc; `arc_start`/`arc_end_param` are
/// the edge's recorded parameters, which is how the importer derives the arc's
/// direction (the same endpoints describe two different arcs).
fn quarter_disc_sat(
    arc_end: [f64; 3],
    arc_start: f64,
    arc_end_param: f64,
) -> acadrust::entities::acis::SatDocument {
    use acadrust::entities::acis::{SatDocument, SatPointer, Sense, Sidedness};
    const NULL: SatPointer = SatPointer::NULL;
    fn ptr(index: i32) -> SatPointer {
        SatPointer::new(index)
    }

    let mut sat = SatDocument::new_body();
    let body = SatPointer::new(0);
    let centre = [0.0, 0.0, 0.0];
    let start = [1.0, 0.0, 0.0];

    let p_start = sat.add_point(start[0], start[1], start[2]);
    let p_end = sat.add_point(arc_end[0], arc_end[1], arc_end[2]);
    let p_centre = sat.add_point(centre[0], centre[1], centre[2]);
    let plane = sat.add_plane_surface(centre, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
    let arc_curve = sat.add_ellipse_curve(centre, [0.0, 0.0, 1.0], start, 1.0);
    let back_curve = sat.add_straight_curve(
        arc_end,
        [
            centre[0] - arc_end[0],
            centre[1] - arc_end[1],
            centre[2] - arc_end[2],
        ],
    );
    let home_curve = sat.add_straight_curve(centre, start);

    // Each edge carries its own vertex records; the coedges mate them.
    let v_arc_start = sat.add_vertex(NULL, ptr(p_start));
    let v_arc_end = sat.add_vertex(NULL, ptr(p_end));
    let v_back_start = sat.add_vertex(NULL, ptr(p_end));
    let v_back_end = sat.add_vertex(NULL, ptr(p_centre));
    let v_home_start = sat.add_vertex(NULL, ptr(p_centre));
    let v_home_end = sat.add_vertex(NULL, ptr(p_start));

    let e_arc = sat.add_edge(
        ptr(v_arc_start),
        arc_start,
        ptr(v_arc_end),
        arc_end_param,
        NULL,
        ptr(arc_curve),
        Sense::Forward,
    );
    let e_back = sat.add_edge(
        ptr(v_back_start),
        0.0,
        ptr(v_back_end),
        1.0,
        NULL,
        ptr(back_curve),
        Sense::Forward,
    );
    let e_home = sat.add_edge(
        ptr(v_home_start),
        0.0,
        ptr(v_home_end),
        1.0,
        NULL,
        ptr(home_curve),
        Sense::Forward,
    );

    let base = sat.records.len() as i32;
    let co = |offset: i32| ptr(base + offset);
    let loop_index = ptr(base + 3);
    let face_index = ptr(base + 4);
    let shell_index = ptr(base + 5);
    let lump_index = ptr(base + 6);
    sat.add_coedge(co(1), co(2), NULL, ptr(e_arc), Sense::Forward, loop_index);
    sat.add_coedge(co(2), co(0), NULL, ptr(e_back), Sense::Forward, loop_index);
    sat.add_coedge(co(0), co(1), NULL, ptr(e_home), Sense::Forward, loop_index);
    sat.add_loop(NULL, co(0), face_index);
    sat.add_face(
        NULL,
        loop_index,
        shell_index,
        ptr(plane),
        Sense::Forward,
        Sidedness::Single,
    );
    sat.add_shell(face_index, lump_index);
    sat.add_lump(shell_index, body);
    sat
}

#[test]
fn an_arc_bounded_planar_face_lifts_to_an_arc_not_a_refusal() {
    use cad_kernel_adapter::BrepCurve;

    let brep = sat_to_brep(&quarter_disc_sat(
        [0.0, 1.0, 0.0],
        0.0,
        std::f64::consts::FRAC_PI_2,
    ));
    assert_eq!(brep.face_count(), 1);
    // The SAT record set states no geometry tolerance: "unspecified", not a guess.
    assert_eq!(brep.tolerance, None);

    let face = &brep.shells[0].faces[0];
    assert!(matches!(face.surface, BrepSurface::Plane { .. }));
    assert_eq!(face.tolerance, None, "no per-face tolerance in SAT");

    let mut arcs = 0;
    let mut lines = 0;
    for curve in &face.loops[0].edges {
        match curve {
            BrepCurve::Arc {
                center,
                normal,
                radius,
                start,
                end,
                ..
            } => {
                arcs += 1;
                assert!((radius - 1.0).abs() < 1e-12, "radius {radius}");
                assert!(center.x.abs() < 1e-12 && center.y.abs() < 1e-12 && center.z.abs() < 1e-12);
                assert!(normal.z > 0.5, "the arc turns about +z: {normal:?}");
                assert!((start.x - 1.0).abs() < 1e-12 && start.y.abs() < 1e-12);
                assert!(end.x.abs() < 1e-12 && (end.y - 1.0).abs() < 1e-12);
            }
            BrepCurve::Line { .. } => lines += 1,
            other => panic!("an arc-bounded face must carry no unsupported curve: {other:?}"),
        }
    }
    assert_eq!((arcs, lines), (1, 2), "one arc plus the two radii");
}

#[test]
fn the_arc_bounded_face_tessellates_to_its_closed_form_area() {
    let brep = sat_to_brep(&quarter_disc_sat(
        [0.0, 1.0, 0.0],
        0.0,
        std::f64::consts::FRAC_PI_2,
    ));
    // A chord budget tight enough that the *area* is inside 0.1 % of the closed
    // form: a chord polyline undershoots a circle's area, so the budget (not the
    // arc) sets what the mesh can claim.
    let request = TessellationRequest {
        geometry: GeometryHandle::Resolved(ObjectId(1)),
        exchange: SolidExchange::Brep(brep),
        tolerance: TessellationTolerance {
            linear_deflection: 1e-5,
            ..TessellationTolerance::default()
        },
        budget: TessellationBudget::default(),
        stamp: TaskStamp::new(DocumentId(1), 0),
    };
    let result = BrepTessellator.tessellate(&request, &|| false).unwrap();
    let geometry = match result.outcome {
        TessellationOutcome::Success { geometry, .. }
        | TessellationOutcome::Partial { geometry, .. } => geometry,
        other => panic!("the quarter disc must tessellate, got {other:?}"),
    };
    let area = cad_kernel_adapter::brep::mesh_area(&geometry.mesh);
    let expected = std::f64::consts::FRAC_PI_4;
    assert!(
        (area - expected).abs() / expected < 1e-3,
        "quarter-disc area {area} vs {expected} (0.1 % of the closed form)"
    );
    // The arc is sampled, so the face is an approximation with a chordal bound —
    // never claimed as analytic.
    assert!(matches!(
        geometry.precision,
        Precision::Approximate {
            error_bound: Some(bound)
        } if bound.is_finite() && bound > 0.0
    ));
}

#[test]
fn an_arc_whose_vertex_left_the_circle_stays_a_named_refusal() {
    use cad_kernel_adapter::BrepCurve;
    // 1.2 instead of 1.0 on the circle: a different curve, not this arc.
    let brep = sat_to_brep(&quarter_disc_sat(
        [0.0, 1.2, 0.0],
        0.0,
        std::f64::consts::FRAC_PI_2,
    ));
    let curves: Vec<&BrepCurve> = brep
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .flat_map(|face| &face.loops)
        .flat_map(|loop_| &loop_.edges)
        .collect();
    assert!(
        curves.iter().any(|curve| matches!(
            curve,
            BrepCurve::Unsupported { type_key } if type_key == "partial-circle"
        )),
        "an off-circle arc endpoint must stay an explicit refusal: {curves:?}"
    );
    assert!(
        !curves
            .iter()
            .any(|curve| matches!(curve, BrepCurve::Arc { .. })),
        "nothing may be re-projected onto the circle"
    );
}

#[test]
fn a_solid3d_carrying_an_arc_bounded_face_imports_as_solid_and_exact() {
    use cad_kernel_adapter::BrepCurve;

    // The payload is one planar face bounded by an arc (a sheet's face subset);
    // the topology class comes from the entity, exactly as `acis_body_kind`
    // documents. What the test pins is that the *arc* lifts into an exact body
    // instead of being refused as a partial circle.
    let sat = quarter_disc_sat([0.0, 1.0, 0.0], 0.0, std::f64::consts::FRAC_PI_2).to_sat_string();
    let acad = acadrust::CadDocument::new();
    let req = request(Vec::new());
    let mut builder = builder(&req, &acad);
    let entity = EntityType::Solid3D(acadrust::entities::Solid3D::from_sat(&sat));
    let common = entity.common();
    let (geometry, _) = builder.convert(&entity, common);
    let SemanticGeometry::Body3d(body) = geometry else {
        panic!("an arc-bounded 3DSOLID must become a Body3d, got {geometry:?}");
    };
    assert_eq!(body.kind, BodyKind::Solid);
    let brep = body.exact_brep().expect("the arc B-rep is exact geometry");
    assert_eq!(brep.face_count(), 1);
    let curves: Vec<&BrepCurve> = brep
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .flat_map(|face| &face.loops)
        .flat_map(|loop_| &loop_.edges)
        .collect();
    assert!(
        curves
            .iter()
            .any(|curve| matches!(curve, BrepCurve::Arc { .. })),
        "the face's arc must survive the lift: {curves:?}"
    );
    assert!(
        !curves
            .iter()
            .any(|curve| matches!(curve, BrepCurve::Unsupported { .. })),
        "no curve may stay unsupported: {curves:?}"
    );
}

#[test]
fn the_arcs_direction_comes_from_the_recorded_span_not_from_an_assumption() {
    use cad_kernel_adapter::BrepCurve;

    // The same two vertices, but the edge records a three-quarter span: the arc
    // from `(1, 0)` to `(0, 1)` must then be the *complementary* arc, i.e. the
    // traversal runs counter-clockwise about `-z`. Deriving the direction from
    // "the endpoints only" would silently mirror the arc onto its complement.
    let long = sat_to_brep(&quarter_disc_sat(
        [0.0, 1.0, 0.0],
        0.0,
        3.0 * std::f64::consts::FRAC_PI_2,
    ));
    let arcs: Vec<&BrepCurve> = long
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .flat_map(|face| &face.loops)
        .flat_map(|loop_| &loop_.edges)
        .collect();
    let BrepCurve::Arc { normal, .. } = arcs[0] else {
        panic!("expected one arc, got {arcs:?}");
    };
    assert!(
        normal.z < -0.5,
        "a 270-degree span about +z is the complementary arc about -z: {normal:?}"
    );

    // And the quarter span still gives the quarter arc about +z.
    let short = sat_to_brep(&quarter_disc_sat(
        [0.0, 1.0, 0.0],
        0.0,
        std::f64::consts::FRAC_PI_2,
    ));
    let arcs: Vec<&BrepCurve> = short
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .flat_map(|face| &face.loops)
        .flat_map(|loop_| &loop_.edges)
        .collect();
    let BrepCurve::Arc { normal, .. } = arcs[0] else {
        panic!("expected one arc, got {arcs:?}");
    };
    assert!(normal.z > 0.5, "{normal:?}");
}

#[test]
fn a_span_that_wraps_past_a_full_turn_is_still_the_quarter_arc() {
    use cad_kernel_adapter::BrepCurve;

    // The same quarter disc, with the edge's parameters straddling 2π: the record
    // says `6.0 → 6.0 + π/2 − 2π`, i.e. a **negative** difference whose remainder is
    // still π/2. Reading `|end − start|` instead would give 4.712 rad — exactly
    // `3π/2`, the complementary arc — and silently mirror the geometry onto the
    // three-quarter arc instead of refusing.
    let wrapped = sat_to_brep(&quarter_disc_sat(
        [0.0, 1.0, 0.0],
        6.0,
        6.0 + std::f64::consts::FRAC_PI_2 - std::f64::consts::TAU,
    ));
    let arcs: Vec<&BrepCurve> = wrapped
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .flat_map(|face| &face.loops)
        .flat_map(|loop_| &loop_.edges)
        .collect();
    let BrepCurve::Arc {
        normal, start, end, ..
    } = arcs[0]
    else {
        panic!("expected one arc, got {arcs:?}");
    };
    assert!(
        normal.z > 0.5,
        "a wrapped quarter span is still the quarter arc about +z: {normal:?}"
    );
    // The arc's own ends are the vertices, in loop order.
    assert!((start.x - 1.0).abs() < 1e-12 && start.y.abs() < 1e-12);
    assert!(end.x.abs() < 1e-12 && (end.y - 1.0).abs() < 1e-12);
}

#[test]
fn an_arc_without_a_recorded_span_is_named_not_called_a_partial_circle() {
    use cad_kernel_adapter::BrepCurve;

    // acadrust's v400 up-converter pads edge parameters with `Float(0.0)`, so every
    // v400 arc arrives here with a zero span. That is "no span in the record", not a
    // mysterious partial circle: it gets its own name.
    let v400_shaped = sat_to_brep(&quarter_disc_sat([0.0, 1.0, 0.0], 0.0, 0.0));
    let curves: Vec<&BrepCurve> = v400_shaped
        .shells
        .iter()
        .flat_map(|shell| &shell.faces)
        .flat_map(|face| &face.loops)
        .flat_map(|loop_| &loop_.edges)
        .collect();
    assert!(
        curves.iter().any(|curve| matches!(
            curve,
            BrepCurve::Unsupported { type_key } if type_key == "arc-without-span"
        )),
        "a zero-span arc must be named: {curves:?}"
    );
    assert!(
        !curves
            .iter()
            .any(|curve| matches!(curve, BrepCurve::Arc { .. })),
        "a zero-span arc is not guessed into an arc: {curves:?}"
    );
}
