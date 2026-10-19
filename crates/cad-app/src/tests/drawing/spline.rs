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

    // A pre-built `Spline` geometry payload is refused: SPLINE's payload is the
    // raw fit points, never a second source of truth.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateSpline,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(2.0, 0.0),
                point(3.0, 0.0),
            ],
            weights: vec![1.0; 4],
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
