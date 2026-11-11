//! SPLINE create-command contract tests (ADR 0015 decision F, P3d).
//!
//! The captured fit points become **one `SemanticGeometry::Spline`** (degree 3,
//! clamped chord knots, unit weights) in one transaction and one undo step. The
//! interpolation itself is the pure geometry core
//! ([`cad_geometry::spline::fit_points_to_cubic_spline`]); these tests exercise it
//! **through the command** and pin the ADR-named contracts:
//! `spline_requires_at_least_four_fit_points_for_degree_three`,
//! `spline_knots_are_clamped_chord_and_match_n_plus_degree_plus_one`, the
//! interpolation actually passing through the fit points, and the geometry core's
//! own refusals propagating rather than being swallowed.

use super::*;
use cad_geometry::NurbsCurve;

/// Read the stored spline `(degree, knots, control_points, weights)` of an entity,
/// asserting it is on layer 0 in model space.
fn spline_parts(app: &Application, object: ObjectId) -> (u32, Vec<f64>, Vec<Point3>, Vec<f64>) {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the spline entity");
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbSpline");
    match &entity.geometry {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => (
            *degree,
            knots.clone(),
            control_points.clone(),
            weights.clone(),
        ),
        other => panic!("expected a spline, got {other:?}"),
    }
}

fn separation(a: Point3, b: Point3) -> f64 {
    (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z)
}

#[test]
fn create_spline_inserts_one_entity_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let fit = vec![
        point(0.0, 0.0),
        point(1.0, 2.0),
        point(2.5, 2.5),
        point(4.0, 1.0),
        point(5.5, -1.0),
    ];
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateSpline,
        CommandPayload::Points(fit.clone()),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one capture");
    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(cs.changes.len(), 1, "one transaction");

    let entity_id = EntityId(outcome.objects[0].0);
    let (degree, _knots, cps, weights) = spline_parts(&app, outcome.objects[0]);
    assert_eq!(degree, 3, "SPLINE is degree 3");
    assert_eq!(cps.len(), fit.len(), "one control point per fit point");
    assert_eq!(weights, vec![1.0; fit.len()], "non-rational: all weights 1");
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // Undo removes the spline exactly; redo restores it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .is_none());
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(entity_id)
        .is_some());
}

#[test]
fn spline_requires_at_least_four_fit_points_for_degree_three() {
    // The B1 decision: a degree-3 fit spline needs at least four control points,
    // and the construction uses one control point per fit point, so fewer than
    // four fit points is an explicit refusal naming the minimum — never a silent
    // approximation or a relaxed database bound.
    for n in 0..4usize {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let fit: Vec<Point3> = (0..n).map(|i| point(i as f64, (i % 2) as f64)).collect();
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateSpline,
            CommandPayload::Points(fit),
        )
        .unwrap_err();
        match err {
            CadError::InvalidInput(message) => {
                assert!(
                    message.contains("at least 4"),
                    "n = {n}: the refusal must name the minimum, got {message:?}"
                );
            }
            other => panic!("n = {n}: expected an InvalidInput refusal, got {other:?}"),
        }
        assert_eq!(revision(&app), rev, "n = {n}: a refusal must not write");
        assert!(!app.can_undo(&DocumentId(1)), "n = {n}");
    }
}

#[test]
fn spline_knots_are_clamped_chord_and_match_n_plus_degree_plus_one() {
    // Through the command: the stored knots are the clamped chord vector of the
    // importer's `Complete` shape, `knots.len() == n + degree + 1`, with end
    // multiplicity `degree + 1` and strictly increasing interior knots.
    for n in [4usize, 5, 8] {
        let (mut app, mut session) = drawing_app();
        let fit: Vec<Point3> = (0..n)
            .map(|i| point(i as f64, ((i * i) % 5) as f64))
            .collect();
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::CreateSpline,
            CommandPayload::Points(fit.clone()),
        )
        .unwrap();
        let (degree, knots, cps, _weights) = spline_parts(&app, outcome.objects[0]);
        assert_eq!(degree, 3);
        assert_eq!(cps.len(), n);
        assert_eq!(knots.len(), cps.len() + degree as usize + 1, "n = {n}");
        assert!(
            knots[..=degree as usize].iter().all(|k| *k == 0.0),
            "n = {n}: clamped start"
        );
        assert!(knots[n..].iter().all(|k| *k == 1.0), "n = {n}: clamped end");
        assert!(
            knots.windows(2).all(|w| w[1] >= w[0]),
            "n = {n}: non-decreasing"
        );
        assert!(
            knots[degree as usize..=n].windows(2).all(|w| w[1] > w[0]),
            "n = {n}: strictly increasing interior knots"
        );
        // Chord (not uniform) parameters: the first interior knot (present only
        // when `n >= degree + 2`) equals the average of the first `degree` chord
        // parameters, which for uneven spacing is not a uniform step.
        if n >= degree as usize + 2 {
            let params = cad_geometry::chord_parameters(&fit).unwrap();
            let expected = (params[1..=degree as usize].iter().sum::<f64>()) / degree as f64;
            assert!(
                (knots[degree as usize + 1] - expected).abs() < 1e-12,
                "n = {n}: interior knot must be the averaged chord parameter"
            );
        }
    }
}

#[test]
fn spline_interpolates_the_captured_fit_points() {
    // The stored control points/knots evaluate back to the fit points at their
    // chord parameters: the curve passes through every input (the interpolating
    // property the geometry core guarantees), asserted through the command.
    let (mut app, mut session) = drawing_app();
    let fit = vec![
        point(0.0, 0.0),
        point(1.0, 2.0),
        point(2.5, 2.5),
        point(4.0, 1.0),
        point(5.5, -1.0),
        point(6.0, 0.5),
    ];
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateSpline,
        CommandPayload::Points(fit.clone()),
    )
    .unwrap();
    let (degree, knots, cps, weights) = spline_parts(&app, outcome.objects[0]);
    let curve = NurbsCurve::new(degree as usize, cps, knots, weights).expect("valid stored spline");
    let params = cad_geometry::chord_parameters(&fit).unwrap();
    for (i, q) in fit.iter().enumerate() {
        let got = curve.evaluate(params[i]);
        assert!(
            separation(got, *q) < 1e-9,
            "fit point {i}: got {got:?} want {q:?}"
        );
    }
}

#[test]
fn spline_refuses_non_finite_non_coplanar_and_wrong_payload_without_change() {
    let cases: &[(&str, Vec<Point3>)] = &[
        (
            "non-finite fit point",
            vec![
                point(0.0, 0.0),
                point(1.0, 1.0),
                point(f64::NAN, 2.0),
                point(3.0, 0.0),
            ],
        ),
        (
            "non-coplanar +Z fit point",
            vec![
                point(0.0, 0.0),
                point(1.0, 1.0),
                point(2.0, 2.0),
                p3(3.0, 0.0, 5.0),
            ],
        ),
    ];
    for (label, fit) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateSpline,
            CommandPayload::Points(fit.clone()),
        )
        .unwrap_err();
        assert!(
            matches!(err, CadError::InvalidInput(_)),
            "{label}: expected an InvalidInput refusal, got {err:?}"
        );
        assert_eq!(revision(&app), rev, "{label}: refusal must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{label}");
    }

    // A pre-built geometry of another kind is refused: the fit-point arm takes raw
    // fit points, and a mismatched geometry is never coerced into a spline.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateSpline,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        })),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn spline_propagates_the_geometry_core_degenerate_chord_refusal() {
    // A duplicate consecutive fit point makes the chord parameterisation undefined;
    // the geometry core refuses it explicitly and the command must surface that
    // error (never swallow it into an approximation or a dropped entity).
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let fit = vec![
        point(0.0, 0.0),
        point(1.0, 0.0),
        point(1.0, 0.0),
        point(2.0, 0.0),
    ];
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateSpline,
        CommandPayload::Points(fit),
    )
    .unwrap_err();
    match err {
        CadError::InvalidInput(message) => {
            assert!(message.contains("degenerate chord"), "{message}")
        }
        other => panic!("expected the core's degenerate-chord refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_spline_on_a_missing_layer_is_refused_by_the_shared_funnel() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateSpline,
        CommandPayload::Points(vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(2.0, 0.0),
            point(3.0, 1.0),
        ]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

// ---------------------------------------------------------------------------
// The CV (control vertex) method: a programmatic `Geometry(Spline)` payload
// stores the picked control points verbatim with an explicit degree 2 or 3.
// ---------------------------------------------------------------------------

/// The stored `(degree, knots, control_points, weights)` of a created spline.
type StoredSpline = (u32, Vec<f64>, Vec<Point3>, Vec<f64>);

/// Execute one `Geometry(Spline)` payload and return the stored spline.
fn execute_cv_spline(
    app: &mut Application,
    session: &mut SessionState,
    degree: u32,
    knots: Vec<f64>,
    control_points: Vec<Point3>,
    weights: Vec<f64>,
) -> CadResult<StoredSpline> {
    let outcome = execute(
        app,
        session,
        CommandId::CreateSpline,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        })),
    )?;
    Ok(spline_parts(app, outcome.objects[0]))
}

#[test]
fn create_spline_cv_method_stores_the_control_points_verbatim() {
    // The CV method is the other half of AutoCAD's SPLINE: the picked points are
    // CONTROL points, not fit points, so they are stored exactly as given (the curve
    // does not interpolate them). One entity, one transaction, one undo step.
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let control_points = vec![
        point(0.0, 0.0),
        point(1.0, 3.0),
        point(3.0, 3.0),
        point(4.0, 0.0),
    ];
    let (degree, knots, stored, weights) = execute_cv_spline(
        &mut app,
        &mut session,
        3,
        Vec::new(),
        control_points.clone(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(degree, 3);
    assert_eq!(stored, control_points, "control points are stored verbatim");
    assert!(knots.is_empty(), "the clamped-uniform shorthand is kept");
    assert!(weights.is_empty(), "the all-ones shorthand is kept");
    let cs = &app.history[&DocumentId(1)];
    assert_eq!(cs.undo_depth(), 1, "one undo step");
    assert!(revision(&app).0 > before.0, "one revision bump");

    // One undo removes it exactly.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entities()
            .filter(|entity| entity.object.type_key == "AcDbSpline")
            .count(),
        0
    );
}

#[test]
fn create_spline_cv_method_degree_three_respects_an_explicit_knot_vector() {
    // An explicit (clamped) knot vector is stored as given and evaluates back to the
    // end control points at t = 0 / t = 1 — the standard B-spline endpoint
    // property, which proves the command stored a real NURBS definition rather than
    // re-interpolating the points.
    let (mut app, mut session) = drawing_app();
    let control_points = vec![
        point(0.0, 0.0),
        point(1.0, 3.0),
        point(3.0, 3.0),
        point(4.0, 1.0),
        point(5.0, 0.0),
    ];
    let knots = vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0];
    let (degree, stored_knots, stored, weights) = execute_cv_spline(
        &mut app,
        &mut session,
        3,
        knots.clone(),
        control_points.clone(),
        vec![1.0; control_points.len()],
    )
    .unwrap();
    assert_eq!(degree, 3);
    assert_eq!(stored_knots, knots, "the knot vector is stored as given");
    assert_eq!(stored, control_points);
    assert_eq!(weights, vec![1.0; control_points.len()]);

    let curve =
        NurbsCurve::new(3, stored.clone(), stored_knots, weights).expect("a valid stored spline");
    assert!(separation(curve.evaluate(0.0), control_points[0]) < 1e-9);
    assert!(separation(curve.evaluate(1.0), control_points[4]) < 1e-9);
    // A CV spline does NOT interpolate its interior control points (unlike the
    // fit-point method): the curve at a mid knot is elsewhere.
    let mid = curve.evaluate(0.5);
    assert!(
        control_points.iter().all(|cp| separation(mid, *cp) > 1e-6),
        "a CV spline does not pass through its control polygon: {mid:?}"
    );
}

#[test]
fn create_spline_cv_method_accepts_degree_two() {
    // Degree 2 is the other value AutoCAD 2026's Degree prompt offers.
    let (mut app, mut session) = drawing_app();
    let control_points = vec![point(0.0, 0.0), point(2.0, 2.0), point(4.0, 0.0)];
    let (degree, knots, stored, _weights) = execute_cv_spline(
        &mut app,
        &mut session,
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        control_points.clone(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(degree, 2);
    assert_eq!(stored, control_points);
    assert_eq!(knots.len(), control_points.len() + 3);
}

#[test]
fn create_spline_cv_method_refuses_unsupported_degrees_and_short_lists() {
    // Stable `spline.<reason>` refusals, none of which writes.
    let cases: &[(&str, &str, u32, Vec<Point3>)] = &[
        (
            "degree 1 (a polyline's job)",
            "spline.degree",
            1,
            vec![point(0.0, 0.0), point(1.0, 0.0)],
        ),
        (
            "degree 4 (no prompt in the modeled domain)",
            "spline.degree",
            4,
            vec![
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(2.0, 0.0),
                point(3.0, 0.0),
                point(4.0, 0.0),
            ],
        ),
        (
            "degree 3 with three control points",
            "spline.control_points",
            3,
            vec![point(0.0, 0.0), point(1.0, 0.0), point(2.0, 0.0)],
        ),
        (
            "non-finite control point",
            "spline.invalid_point",
            2,
            vec![point(0.0, 0.0), point(f64::NAN, 1.0), point(2.0, 0.0)],
        ),
    ];
    for (label, code, degree, control_points) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateSpline,
            CommandPayload::Geometry(Box::new(SemanticGeometry::Spline {
                degree: *degree,
                knots: Vec::new(),
                control_points: control_points.clone(),
                weights: Vec::new(),
            })),
        )
        .unwrap_err();
        match &err {
            CadError::InvalidInput(message) => assert!(
                message.starts_with(code),
                "{label}: expected {code}, got {message}"
            ),
            other => panic!("{label}: expected an InvalidInput refusal, got {other:?}"),
        }
        assert_eq!(revision(&app), rev, "{label}: refusal must not write");
        assert!(!app.can_undo(&DocumentId(1)), "{label}");
    }
}

#[test]
fn create_spline_cv_method_surfaces_the_database_knot_relation_refusal() {
    // The structural NURBS relations (`knots.len() == n + degree + 1`,
    // `weights.len() == n`) stay the database write validator's single rule; the
    // command never pads or repairs them, so the refusal propagates and nothing is
    // written.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let control_points = vec![
        point(0.0, 0.0),
        point(1.0, 1.0),
        point(2.0, 0.0),
        point(3.0, 1.0),
    ];
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateSpline,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Spline {
            degree: 3,
            // A degree-3 spline with 4 control points needs 8 knots, not 5.
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0],
            control_points,
            weights: vec![1.0; 4],
        })),
    )
    .unwrap_err();
    let message = match &err {
        CadError::InvalidInput(message) => message.clone(),
        CadError::Invariant(message) => message.clone(),
        other => panic!("expected the database knot relation refusal, got {other:?}"),
    };
    assert!(
        message.contains("spline.knot_count"),
        "the stable database code must survive: {message}"
    );
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}
