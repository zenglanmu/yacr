//! TRIM with a **SPLINE target**: the result must be an exact B-spline.
//!
//! A SPLINE target is cut at its intersections with the boundary and replaced in
//! place by the **exact sub-curve** of itself on the kept parameter span
//! (`cad_geometry::sub_curve`: Boehm knot insertion on the homogeneous control
//! points, both ends clamped). The two precisions are separate and both are
//! asserted here:
//!
//! * the **cut parameter** comes from the engine's sampled fallback (a SPLINE has
//!   no analytic intersection arm), so a new end lies within the documented
//!   `extent × 1e-4` chord of the true crossing — asserted with the `2e-3` margin
//!   (`SAMPLE_TOL`) the SPLINE-boundary tests already use for the 10-unit shapes;
//! * the **curve itself** is exact, so a rational quarter circle stays exactly a
//!   quarter circle (radius to `1e-12` at 201 parameters), which is the property
//!   that catches a mistake in the homogeneous surgery.
//!
//! Everything else is the existing TRIM contract: the kept piece is the one
//! containing the projected pick (never a side assumption), the entity keeps its
//! id/layer/space, no second entity appears, and the whole command is one undo
//! step that restores the source bit-exactly.

use super::*;

/// Twice the documented sampled-cut bound (`extent × 1e-4`) for the shapes here.
const SAMPLE_TOL: f64 = 2e-3;

fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-9
}

fn distance(a: Point3, b: Point3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

/// A one-layer model-space document holding exactly `entities`.
fn app_of(mut entities: Vec<DbEntity>) -> Application {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for entity in entities.drain(..) {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    app
}

fn line(id: u128, start: Point3, end: Point3, order: i64) -> DbEntity {
    ent(id, SemanticGeometry::Line { start, end }, order)
}

/// The clamped quadratic Bézier P0=(2,-2), P1=(5,4), P2=(8,-2): the parabola
/// `y = -2 + 12t - 12t²` with `x = 2 + 6t`. It passes through (5,1) at `t = 1/2`.
fn quadratic_spline(id: u128, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Spline {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![point(2.0, -2.0), point(5.0, 4.0), point(8.0, -2.0)],
            weights: Vec::new(),
        },
        order,
    )
}

/// The exact rational quadratic quarter circle `x² + y² = 1` in the first
/// quadrant: control points (1,0), (1,1), (0,1), middle weight `1/√2`.
fn quarter_circle_spline(id: u128, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Spline {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![point(1.0, 0.0), point(1.0, 1.0), point(0.0, 1.0)],
            weights: vec![1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0],
        },
        order,
    )
}

/// A degree-2 spline that stores **both** shorthands (`knots: []`, `weights: []`)
/// over four control points. The clamped-uniform expansion is
/// `[0, 0, 0, 0.5, 1, 1, 1]`, i.e. two quadratic segments meeting at `t = 1/2`.
fn shorthand_spline(id: u128, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Spline {
            degree: 2,
            knots: Vec::new(),
            control_points: vec![
                point(0.0, 0.0),
                point(3.0, 4.0),
                point(6.0, -4.0),
                point(9.0, 0.0),
            ],
            weights: Vec::new(),
        },
        order,
    )
}

/// Everything a refusal must leave byte-identical.
fn snapshot(app: &Application) -> (Revision, usize, String) {
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    (
        drawing.revision(),
        drawing.entity_count(),
        format!("{:?}", drawing.entities().collect::<Vec<_>>()),
    )
}

fn geometry_of(app: &Application, id: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .unwrap_or_else(|| panic!("entity {id} must exist"))
        .geometry
        .clone()
}

/// `(degree, knots, control_points, weights)` of a SPLINE entity.
fn expect_spline(app: &Application, id: u128) -> (u32, Vec<f64>, Vec<Point3>, Vec<f64>) {
    match geometry_of(app, id) {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => (degree, knots, control_points, weights),
        other => panic!("the trimmed target must stay a SPLINE, got {other:?}"),
    }
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn trim(
    app: &mut Application,
    target: u128,
    boundaries: &[u128],
    pick: Point3,
) -> CadResult<CommandOutcome> {
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(target),
            boundary: boundaries.iter().map(|id| ref_of(*id)).collect(),
            pick_point: pick,
        },
    )
}

fn extend(
    app: &mut Application,
    target: u128,
    boundaries: &[u128],
    pick: Point3,
) -> CadResult<CommandOutcome> {
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(target),
            boundary: boundaries.iter().map(|id| ref_of(*id)).collect(),
            pick_point: pick,
        },
    )
}

/// The `drawing.trim` notes of an outcome, in order.
fn notes(outcome: &CommandOutcome) -> Vec<&str> {
    outcome
        .diagnostics
        .iter()
        .skip(1)
        .map(|diagnostic| {
            assert_eq!(diagnostic.code, "drawing.trim");
            diagnostic.message.as_str()
        })
        .collect()
}

#[test]
fn trim_spline_target_by_a_line_keeps_an_exact_clamped_spline_of_the_same_degree() {
    // The vertical boundary x = 5 meets the parabola exactly once, at (5,1) on
    // t = 1/2 (the parabola's x is 2 + 6t). The pick is on the right half.
    let mut app = app_of(vec![
        quadratic_spline(1, 0),
        line(2, point(5.0, -10.0), point(5.0, 10.0), 1),
    ]);
    let source = geometry_of(&app, 1);
    let before_count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();

    let pick = point(6.8, -0.08); // the curve point at t = 0.8
    let outcome = trim(&mut app, 1, &[2], pick).unwrap();

    let (degree, knots, control_points, weights) = expect_spline(&app, 1);
    assert_eq!(degree, 2, "the trim must not change the degree");
    assert_eq!(control_points.len(), 3);
    assert_eq!(
        knots.len(),
        control_points.len() + degree as usize + 1,
        "the stored knot relation must hold: {knots:?}"
    );
    // Clamped at both ends, no interior knot: a quadratic sub-curve of a one-span
    // quadratic is again a one-span quadratic.
    assert!(
        knots[0] == knots[1] && knots[1] == knots[2],
        "start must have degree+1 = 3 copies: {knots:?}"
    );
    assert!(
        knots[3] == knots[4] && knots[4] == knots[5],
        "end must have degree+1 = 3 copies: {knots:?}"
    );
    assert!(
        (knots[0] - 0.5).abs() <= SAMPLE_TOL,
        "the sampled cut is at t ≈ 0.5: {knots:?}"
    );
    assert!(
        knots[5] == 1.0,
        "the far end is the source's own end: {knots:?}"
    );
    // A polynomial source stays polynomial: the weights are exactly one.
    assert!(weights.iter().all(|weight| *weight == 1.0), "{weights:?}");

    // The kept end points are exact curve points: the cut end is within the
    // sampled-cut bound of (5,1), the source end is exactly (8,-2).
    assert!(
        distance(control_points[0], point(5.0, 1.0)) <= SAMPLE_TOL,
        "the new start is the exact curve point nearest the sampled cut: {:?}",
        control_points[0]
    );
    assert!(
        distance(control_points[2], point(8.0, -2.0)) <= 1e-12,
        "the untouched end is bit-exact: {:?}",
        control_points[2]
    );
    // The discarded (left) piece is gone: the kept curve starts at x ≈ 5.
    assert!(control_points[0].x > 4.9, "{:?}", control_points[0]);

    // In place: same id, no second entity, one undo step.
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before_count
    );
    assert_eq!(undo_depth(&app), 1);

    // Both precisions are reported: the sampled contributing pair, then the
    // target-side note naming the exact split.
    let reported = notes(&outcome);
    assert_eq!(reported.len(), 2, "{reported:?}");
    assert!(
        reported[0].contains("sampled intersection approximation")
            && reported[0].contains("AcDbSpline")
            && reported[0].contains("AcDbLine"),
        "{reported:?}"
    );
    assert!(
        reported[1].contains("exact B-spline sub-curve")
            && reported[1].contains("inverting sampled intersection points")
            && reported[1].contains("never refitted"),
        "the target-side note must name the split: {reported:?}"
    );

    // UNDO restores the source bit-exactly.
    execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        format!("{:?}", geometry_of(&app, 1)),
        format!("{source:?}"),
        "undo must restore the source spline exactly"
    );
}

#[test]
fn trim_spline_target_keeps_the_complementary_span_when_the_far_side_is_picked() {
    // Same geometry, pick on the LEFT half: the complementary window [0, t] is
    // kept. This is the "no silent side assumption" check — both pieces are
    // reachable and only the pick decides.
    let mut app = app_of(vec![
        quadratic_spline(1, 0),
        line(2, point(5.0, -10.0), point(5.0, 10.0), 1),
    ]);
    let pick = point(3.2, -0.08); // the curve point at t = 0.2
    trim(&mut app, 1, &[2], pick).unwrap();

    let (degree, knots, control_points, _) = expect_spline(&app, 1);
    assert_eq!(degree, 2);
    assert_eq!(control_points.len(), 3);
    assert_eq!(knots.len(), 6);
    // The domain start is the source's own start: bit-exact.
    assert!(
        knots[0] == 0.0 && knots[1] == 0.0 && knots[2] == 0.0,
        "{knots:?}"
    );
    assert!(
        control_points[0] == point(2.0, -2.0),
        "the untouched start is bit-exact: {:?}",
        control_points[0]
    );
    assert!(
        distance(control_points[2], point(5.0, 1.0)) <= SAMPLE_TOL,
        "the cut end is within the sampled bound of (5,1): {:?}",
        control_points[2]
    );
    assert!(control_points[2].x < 5.1, "{:?}", control_points[2]);
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn trim_rational_spline_target_stays_an_exact_circle_arc() {
    // The line y = x meets the quarter circle at (1/√2, 1/√2) on t = 1/2. A pick
    // near the start keeps [0, t]; the kept piece must still be the exact circle.
    let mut app = app_of(vec![
        quarter_circle_spline(1, 0),
        line(2, point(0.0, 0.0), point(2.0, 2.0), 1),
    ]);
    let pick = point(0.95, 0.30); // just outside the arc near its start
    let outcome = trim(&mut app, 1, &[2], pick).unwrap();

    let (degree, knots, control_points, weights) = expect_spline(&app, 1);
    assert_eq!(degree, 2);
    assert_eq!(knots.len(), control_points.len() + degree as usize + 1);
    assert!(
        weights
            .iter()
            .all(|weight| weight.is_finite() && *weight > 0.0),
        "the rational weights must stay positive: {weights:?}"
    );
    // The start is the source start (1,0); the cut end is the sampled crossing.
    let start = control_points[0];
    assert!(
        distance(start, point(1.0, 0.0)) <= 1e-12,
        "the untouched start is bit-exact: {start:?}"
    );
    let end = *control_points.last().unwrap();
    let expected = point(
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    );
    assert!(
        distance(end, expected) <= SAMPLE_TOL,
        "the cut end is within the sampled bound of (1/√2, 1/√2): {end:?}"
    );

    // The exactness test: the kept sub-curve is still a rational quadratic on the
    // unit circle. A surgery mistake (weights interpolated as points, a shifted
    // control point) shows up here at once.
    let kept =
        cad_geometry::NurbsCurve::new(degree as usize, control_points, knots, weights).unwrap();
    let (t0, t1) = kept.domain();
    let mut worst = 0.0f64;
    for index in 0..=200 {
        let parameter = t0 + (t1 - t0) * index as f64 / 200.0;
        let point = kept.evaluate(parameter);
        worst = worst.max((point.x.hypot(point.y) - 1.0).abs());
        assert!(
            point.x >= -1e-12 && point.y >= -1e-12,
            "the kept piece stays in the first quadrant: {point:?}"
        );
    }
    assert!(
        worst < 1e-12,
        "the kept sub-curve must stay on the unit circle: worst radius error {worst:e}"
    );

    let reported = notes(&outcome);
    assert!(
        reported
            .iter()
            .any(|note| note.contains("exact B-spline sub-curve")),
        "{reported:?}"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn trim_spline_target_expands_the_stored_shorthands_and_keeps_the_interior_knot() {
    // `knots: []` is the clamped-uniform shorthand `[0,0,0,0.5,1,1,1]`; the
    // boundary x = 4 cuts the first segment, so the kept piece [t, 1] must still
    // carry the source's interior knot 0.5 — a refit would not.
    let mut app = app_of(vec![
        shorthand_spline(1, 0),
        line(2, point(4.0, -10.0), point(4.0, 10.0), 1),
    ]);
    trim(&mut app, 1, &[2], point(9.0, 0.0)).unwrap();

    let (degree, knots, control_points, weights) = expect_spline(&app, 1);
    assert_eq!(degree, 2);
    assert_eq!(
        control_points.len(),
        4,
        "one interior knot remains: {knots:?}"
    );
    assert_eq!(knots.len(), 7);
    assert!(
        knots[0] == knots[1] && knots[1] == knots[2],
        "clamped start: {knots:?}"
    );
    assert!(
        knots[4] == 1.0 && knots[5] == 1.0 && knots[6] == 1.0,
        "clamped end: {knots:?}"
    );
    assert!(
        (knots[3] - 0.5).abs() <= 1e-15,
        "the source's interior knot is preserved exactly: {knots:?}"
    );
    assert!(weights.iter().all(|weight| *weight == 1.0), "{weights:?}");
    assert!(
        (control_points[0].x - 4.0).abs() <= SAMPLE_TOL,
        "the kept start is the sampled cut at x = 4: {:?}",
        control_points[0]
    );
    assert!(
        control_points[3] == point(9.0, 0.0),
        "the untouched end is bit-exact: {:?}",
        control_points[3]
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn trim_spline_target_without_a_cut_is_refused_without_change() {
    let mut app = app_of(vec![
        quadratic_spline(1, 0),
        line(2, point(0.0, -10.0), point(0.0, 10.0), 1),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], point(5.0, 1.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(
        err.to_string().contains("no intersection"),
        "the refusal names the missing intersection: {err:?}"
    );
    assert_eq!(snapshot(&app), before, "a refused trim changes nothing");
    assert_eq!(undo_depth(&app), 0, "a refused trim opens no transaction");
}

#[test]
fn extend_spline_target_is_refused_with_the_continuation_reason() {
    // EXTEND keeps refusing a SPLINE target, but now says *why*: no continuation
    // semantics (a knot vector, hence a parameterisation, would change) and no
    // extrapolation policy.
    let mut app = app_of(vec![
        quadratic_spline(1, 0),
        line(2, point(5.0, -10.0), point(5.0, 10.0), 1),
    ]);
    let before = snapshot(&app);
    let err = extend(&mut app, 1, &[2], point(8.0, -2.0)).unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    let message = err.to_string();
    for expected in [
        "AcDbSpline",
        "no continuation semantics",
        "knot vector",
        "parameterisation",
        "extrapolation policy",
    ] {
        assert!(
            message.contains(expected),
            "the EXTEND refusal must name {expected:?}: {message}"
        );
    }
    assert_eq!(snapshot(&app), before, "a refused extend changes nothing");
    assert_eq!(undo_depth(&app), 0);

    // Mirror check: EXTEND's LINE behaviour is untouched by the new refusal path.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(4.0, 0.0), 0),
        line(2, point(7.0, -5.0), point(7.0, 5.0), 1),
    ]);
    extend(&mut app, 1, &[2], point(4.0, 0.0)).unwrap();
    match geometry_of(&app, 1) {
        SemanticGeometry::Line { start, end } => {
            assert!(near(start.x, 0.0), "{start:?}");
            assert!(near(end.x, 7.0), "{end:?}");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn trim_spline_target_survives_a_second_trim_and_stays_a_spline() {
    // The result of a trim is itself a valid target: trimming the kept piece again
    // must not degrade it to a polyline or change its degree.
    let mut app = app_of(vec![
        quadratic_spline(1, 0),
        line(2, point(5.0, -10.0), point(5.0, 10.0), 1),
        line(3, point(7.0, -10.0), point(7.0, 10.0), 2),
    ]);
    trim(&mut app, 1, &[2], point(6.8, -0.08)).unwrap();
    let (degree, _, _, _) = expect_spline(&app, 1);
    assert_eq!(degree, 2);
    // Trim again with the second boundary (x = 7, on the kept right piece).
    let pick = point(8.0, -2.0);
    trim(&mut app, 1, &[3], pick).unwrap();
    let (degree, knots, control_points, weights) = expect_spline(&app, 1);
    assert_eq!(degree, 2);
    assert_eq!(knots.len(), control_points.len() + degree as usize + 1);
    assert!(weights.iter().all(|weight| *weight == 1.0));
    assert!(
        control_points[0].x > 6.9,
        "the second trim cut near x = 7: {:?}",
        control_points[0]
    );
    assert_eq!(undo_depth(&app), 2, "each trim is its own undo step");
}

#[test]
fn trim_spline_target_between_two_cuts_truncates_both_ends() {
    // The horizontal line y = 0 meets the parabola twice, at t = (1 ∓ 1/√3)/2,
    // i.e. x = 5 ∓ √3. Picking the apex keeps the window between them, so BOTH
    // ends of the result are cut ends of one surgery.
    let mut app = app_of(vec![
        quadratic_spline(1, 0),
        line(2, point(0.0, 0.0), point(10.0, 0.0), 1),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(5.0, 1.0)).unwrap();

    let (degree, knots, control_points, weights) = expect_spline(&app, 1);
    assert_eq!(degree, 2);
    assert_eq!(control_points.len(), 3);
    assert_eq!(knots.len(), 6);
    assert!(
        knots[0] == knots[1] && knots[1] == knots[2],
        "clamped start: {knots:?}"
    );
    assert!(
        knots[3] == knots[4] && knots[4] == knots[5],
        "clamped end: {knots:?}"
    );
    assert!(weights.iter().all(|weight| *weight == 1.0));
    let sqrt3 = 3.0f64.sqrt();
    assert!(
        distance(control_points[0], point(5.0 - sqrt3, 0.0)) <= SAMPLE_TOL,
        "the left cut end is within the sampled bound of 5-√3: {:?}",
        control_points[0]
    );
    assert!(
        distance(control_points[2], point(5.0 + sqrt3, 0.0)) <= SAMPLE_TOL,
        "the right cut end is within the sampled bound of 5+√3: {:?}",
        control_points[2]
    );
    // The kept piece is the middle one: its own apex is still above y = 0.
    let kept =
        cad_geometry::NurbsCurve::new(degree as usize, control_points, knots, weights).unwrap();
    let (t0, t1) = kept.domain();
    assert!(
        kept.evaluate(0.5 * (t0 + t1)).y > 0.9,
        "the apex of the kept window survives"
    );
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(notes(&outcome).len(), 2, "pair note + target note");
}

/// A tiny guard against an accidental `Polyline` fallback anywhere in this module:
/// every fixture above is a SPLINE and no test may observe a POLYLINE.
#[test]
fn no_spline_trim_fixture_produces_a_polyline() {
    let mut app = app_of(vec![
        quadratic_spline(1, 0),
        line(2, point(5.0, -10.0), point(5.0, 10.0), 1),
    ]);
    trim(&mut app, 1, &[2], point(6.8, -0.08)).unwrap();
    assert!(!matches!(
        geometry_of(&app, 1),
        SemanticGeometry::Polyline { .. }
    ));
}
