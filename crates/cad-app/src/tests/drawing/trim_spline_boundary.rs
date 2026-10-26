//! TRIM/EXTEND contract tests for **SPLINE boundaries**.
//!
//! A SPLINE is accepted as a *sampled* TRIM/EXTEND boundary: `cad_geometry` has no
//! analytic arm for it, so the cut comes from the engine's flatten-and-intersect
//! fallback and every command reports a `drawing.trim`/`drawing.extend` note saying
//! the cut point is approximate. As a **target** it is supported by TRIM only (the
//! kept piece is an exact sub-curve, see `trim_spline_target`); EXTEND refuses it
//! because a NURBS curve has no continuation semantics.
//!
//! Sampled tolerance (documented in `app_drawing::trim_boundary`): the fallback
//! flattens each operand with chord tolerance
//! `extent × FALLBACK_CHORD_FRACTION (1e-4)`, capped at 256 segments, and the
//! refinement stops once the true parameter midpoint is within that chord of the
//! polyline. So a sampled cut point is within `extent × 1e-4` of the true curve —
//! `1e-3` world units for the 10-unit shapes below. The assertions use `2e-3`
//! (`SAMPLE_TOL`), i.e. twice the documented bound, never equality.
//!
//! The closed-spline case is deliberately **refused**: `SemanticGeometry::Spline`
//! carries no closed flag and no boundary plane, so the wholly-inside delete rule
//! cannot be soundly evaluated for it.

use super::*;

const PI: f64 = std::f64::consts::PI;

/// Twice the documented sampled-cut bound (`extent × 1e-4`) for the shapes here.
const SAMPLE_TOL: f64 = 2e-3;

fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-9
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

fn arc(id: u128, center: Point3, radius: f64, start: f64, sweep: f64, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Arc {
            center,
            normal: p3(0.0, 0.0, 1.0),
            radius,
            start,
            sweep,
        },
        order,
    )
}

/// A clamped quadratic B-spline (weights all 1, i.e. an exact quadratic Bézier).
fn quadratic_spline(id: u128, p0: Point3, p1: Point3, p2: Point3, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Spline {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![p0, p1, p2],
            weights: Vec::new(),
        },
        order,
    )
}

/// A closed degree-1 spline ring through `corners` (the last corner repeats the
/// first, so the sample set is a closed polygon).
fn closed_ring_spline(id: u128, corners: &[Point3], order: i64) -> DbEntity {
    let mut control = corners.to_vec();
    control.push(corners[0]);
    let n = control.len();
    let degree = 1usize;
    let mut knots = vec![0.0; n + degree + 1];
    // Clamped uniform: `degree + 1` leading zeros, inner spans 1..=n-degree-1,
    // and `degree + 1` trailing copies of the last inner value.
    let span = (n - degree) as f64;
    for (index, knot) in knots.iter_mut().enumerate().skip(degree + 1) {
        *knot = ((index - degree) as f64).min(span);
    }
    ent(
        id,
        SemanticGeometry::Spline {
            degree: degree as u32,
            knots,
            control_points: control,
            weights: Vec::new(),
        },
        order,
    )
}

/// Everything a refusal must leave byte-identical: revision, entity count and the
/// debug form of every entity.
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

fn expect_line(app: &Application, id: u128) -> (Point3, Point3) {
    match geometry_of(app, id) {
        SemanticGeometry::Line { start, end } => (start, end),
        other => panic!("expected a LINE, got {other:?}"),
    }
}

/// `(center, radius, start, sweep)` of an ARC.
fn expect_arc(app: &Application, id: u128) -> (Point3, f64, f64, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => (center, radius, start, sweep),
        other => panic!("expected an ARC, got {other:?}"),
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

#[test]
fn trim_line_by_a_spline_boundary_keeps_the_picked_piece_and_reports_the_sampling() {
    // The quadratic Bézier P0=(2,-2), P1=(5,4), P2=(8,-2) is the parabola
    // y = -2 + 12t - 12t² with x = 2 + 6t, so it crosses y = 0 at t = (1 ∓ 1/√3)/2,
    // i.e. exactly at x = 5 ∓ √3. Its x range is 2..8, so a target line
    // (0,0)->(10,0) is cut twice and the picked right piece starts at 5 + √3.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        quadratic_spline(2, point(2.0, -2.0), point(5.0, 4.0), point(8.0, -2.0), 1),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(8.0, 0.0)).unwrap();

    let expected_cut = 5.0 + 3.0f64.sqrt();
    let (start, end) = expect_line(&app, 1);
    assert!(
        (start.x - expected_cut).abs() <= SAMPLE_TOL,
        "the sampled cut is within {SAMPLE_TOL} of 5+√3 ≈ {expected_cut}: {start:?}"
    );
    assert!(near(start.y, 0.0), "{start:?}");
    assert!(near(end.x, 10.0), "{end:?}");

    // The cut is sampled, so the command must say so and name the spline pair.
    assert_eq!(outcome.diagnostics[0].code, "drawing.trim");
    assert_eq!(
        outcome.diagnostics.len(),
        2,
        "one success diagnostic plus one approximation note: {:?}",
        outcome.diagnostics
    );
    let note = &outcome.diagnostics[1];
    assert_eq!(note.code, "drawing.trim");
    assert!(
        note.message.contains("sampled intersection approximation"),
        "{:?}",
        note.message
    );
    assert!(
        note.message.contains("AcDbSpline") && note.message.contains("AcDbLine"),
        "the note names the sampled pair: {:?}",
        note.message
    );
    assert_eq!(undo_depth(&app), 1, "exactly one undo entry");
}

#[test]
fn trim_arc_by_a_spline_boundary_reports_the_angular_remainder_within_the_sampling_bound() {
    // The quadratic Bézier (-4,4), (0,-4), (4,4) is the parabola y = x²/4 (its x
    // coordinate is linear in t). It meets the circle x² + y² = 25 where
    // u = x² solves u² + 16u - 400 = 0, i.e. x = √(4√29 - 8) and y = √29 - 2, at
    // the polar angle θ = atan2(√29 - 2, √(4√29 - 8)) ≈ 0.7434 rad. Its mirror also
    // meets the upper half arc, at π - θ, so the arc is cut twice.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI, 0),
        quadratic_spline(2, point(-4.0, 4.0), point(0.0, -4.0), point(4.0, 4.0), 1),
    ]);
    let x = (4.0 * 29.0f64.sqrt() - 8.0).sqrt();
    let y = 29.0f64.sqrt() - 2.0;
    let theta = y.atan2(x);
    // A pick near angle 0.3 keeps the piece [0, θ].
    let pick = point(5.0 * 0.3f64.cos(), 5.0 * 0.3f64.sin());
    let outcome = trim(&mut app, 1, &[2], pick).unwrap();

    let (center, radius, start, sweep) = expect_arc(&app, 1);
    assert_eq!(center, point(0.0, 0.0));
    assert!(
        (radius - 5.0).abs() <= 1e-9,
        "the trim must not change the target's radius: {radius}"
    );
    assert!(start.abs() <= SAMPLE_TOL, "start {start} != 0");
    assert!(
        (sweep - theta).abs() <= SAMPLE_TOL,
        "sweep {sweep} is within {SAMPLE_TOL} rad of θ ≈ {theta}"
    );
    assert_eq!(
        outcome.diagnostics.len(),
        2,
        "the sampled spline cut is reported: {:?}",
        outcome.diagnostics
    );
    assert!(
        outcome.diagnostics[1].message.contains("AcDbSpline"),
        "{:?}",
        outcome.diagnostics[1].message
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn extend_line_to_a_spline_boundary_stops_at_the_near_crossing_ahead() {
    // Same parabola translated to x ∈ 10..16: P0=(10,-2), P1=(13,4), P2=(16,-2)
    // crosses y = 0 at x = 13 ∓ √3. The near crossing ahead of x = 10 is 13 - √3.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        quadratic_spline(2, point(10.0, -2.0), point(13.0, 4.0), point(16.0, -2.0), 1),
    ]);
    let outcome = extend(&mut app, 1, &[2], point(9.0, 0.0)).unwrap();

    let expected_end = 13.0 - 3.0f64.sqrt();
    let (start, end) = expect_line(&app, 1);
    assert!(start.x.abs() <= 1e-9, "{start:?}");
    assert!(
        (end.x - expected_end).abs() <= SAMPLE_TOL,
        "the extension stops within {SAMPLE_TOL} of 13-√3 ≈ {expected_end}: {end:?}"
    );
    assert_eq!(outcome.diagnostics[0].code, "drawing.extend");
    assert_eq!(
        outcome.diagnostics.len(),
        2,
        "the sampled spline cut is reported: {:?}",
        outcome.diagnostics
    );
    assert!(
        outcome.diagnostics[1].message.contains("AcDbSpline"),
        "{:?}",
        outcome.diagnostics[1].message
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn spline_targets_are_refused_where_the_command_has_no_semantics_for_them() {
    // TRIM *does* support a SPLINE target now (see `trim_spline_target`), so the
    // only refusal left here is the honest one: no intersection with the boundary.
    let target = quadratic_spline(1, point(2.0, -2.0), point(5.0, 4.0), point(8.0, -2.0), 0);
    let mut app = app_of(vec![
        target,
        line(2, point(0.0, -10.0), point(0.0, 10.0), 1),
    ]);
    let before = snapshot(&app);

    let err = trim(&mut app, 1, &[2], point(5.0, 1.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(
        err.to_string().contains("no intersection"),
        "the refusal names what is missing: {err:?}"
    );
    assert_eq!(snapshot(&app), before, "a refused target changes nothing");
    assert_eq!(undo_depth(&app), 0, "a refused target opens no transaction");

    // EXTEND still refuses a SPLINE target, now naming the reason: a NURBS curve
    // has no continuation semantics (see `trim_spline_target` for the full
    // message contract).
    let err = extend(&mut app, 1, &[2], point(5.0, 1.0)).unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    assert!(
        err.to_string().contains("AcDbSpline")
            && err.to_string().contains("no continuation semantics"),
        "the EXTEND refusal names the target kind and the reason: {err:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn a_spline_that_contributes_no_cut_does_not_add_an_approximation_note() {
    // The note must mean "this cut was sampled": a spline bounding far away from
    // the target contributes nothing, so only the analytic LINE cut is reported.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        line(2, point(2.0, -1.0), point(2.0, 1.0), 1),
        quadratic_spline(
            3,
            point(100.0, -2.0),
            point(103.0, 4.0),
            point(106.0, -2.0),
            2,
        ),
    ]);
    let outcome = trim(&mut app, 1, &[2, 3], point(8.0, 0.0)).unwrap();
    assert_eq!(
        outcome.diagnostics.len(),
        1,
        "no approximation note for a spline pair that cut nothing: {:?}",
        outcome.diagnostics
    );
    let (start, end) = expect_line(&app, 1);
    assert!((start.x - 2.0).abs() <= 1e-9, "{start:?}");
    assert!((end.x - 10.0).abs() <= 1e-9, "{end:?}");
}

#[test]
fn trim_refuses_closed_spline_containment_instead_of_deleting_the_target() {
    // A closed degree-1 spline ring around the target: the target has no cut and
    // looks "wholly inside", but a SPLINE has no closed flag and no boundary plane,
    // so the delete rule cannot be soundly evaluated. The combination is refused by
    // name (`Unsupported`) — never deleted, never a fabricated containment.
    let mut app = app_of(vec![
        line(1, point(2.0, 5.0), point(3.0, 5.0), 0),
        closed_ring_spline(
            2,
            &[
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            1,
        ),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], point(2.5, 5.0)).unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    assert!(
        err.to_string().contains("closed-SPLINE"),
        "the refusal names the unsupported rule: {err:?}"
    );
    assert_eq!(
        snapshot(&app),
        before,
        "a refused containment changes nothing"
    );
    assert_eq!(undo_depth(&app), 0);

    // The same ring *with* a cutting analytic boundary still trims normally: the
    // refusal is scoped to the containment question, not to every spline boundary.
    let mut app = app_of(vec![
        line(1, point(0.0, 5.0), point(10.0, 5.0), 0),
        closed_ring_spline(
            2,
            &[
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            1,
        ),
        line(3, point(2.0, 0.0), point(2.0, 10.0), 2),
    ]);
    let outcome = trim(&mut app, 1, &[2, 3], point(8.0, 5.0)).unwrap();
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 2.0), "{start:?}");
    assert!(near(end.x, 10.0), "{end:?}");
    // The ring is sampled too, but it only meets this target at the target's own
    // endpoints (x = 0 and x = 10), which are not interior cuts: the kept window is
    // driven by the analytic LINE boundary at x = 2 and no approximation note is
    // added. The refusal above is scoped to the containment question, not to every
    // spline boundary.
    assert_eq!(
        outcome.diagnostics.len(),
        1,
        "the spline adds no cut and therefore no note: {:?}",
        outcome.diagnostics
    );
}

#[test]
fn spline_boundary_keeps_a_full_turn_arc_target_periodic_semantics() {
    // A CIRCLE (periodic) target cut by the same parabola twice, at the polar
    // angles θ and π - θ (θ ≈ 0.7434 rad). The pick at 0.3 rad lies below θ, so the
    // kept window runs from π - θ around through 2π/0 to θ: its sweep is π + 2θ.
    // The tolerance applies to that angle, not to an end point, and the seam is
    // folded instead of being treated as an end.
    let mut app = app_of(vec![
        ent(
            1,
            SemanticGeometry::Circle {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                radius: 5.0,
            },
            0,
        ),
        quadratic_spline(2, point(-4.0, 4.0), point(0.0, -4.0), point(4.0, 4.0), 1),
    ]);
    let x = (4.0 * 29.0f64.sqrt() - 8.0).sqrt();
    let y = 29.0f64.sqrt() - 2.0;
    let theta = y.atan2(x);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(1),
            boundary: vec![ref_of(2)],
            pick_point: point(5.0 * 0.3f64.cos(), 5.0 * 0.3f64.sin()),
        },
    )
    .unwrap();
    let (_, radius, start, sweep) = expect_arc(&app, 1);
    assert!((radius - 5.0).abs() <= 1e-9);
    let expected_sweep = PI + 2.0 * theta;
    assert!(
        (sweep - expected_sweep).abs() <= SAMPLE_TOL,
        "the picked window is [π-θ, θ] the long way round: sweep {sweep} vs {expected_sweep}"
    );
    let expected_start = PI - theta;
    assert!(
        (start - expected_start).abs() <= SAMPLE_TOL,
        "start {start} vs π-θ {expected_start}"
    );
    assert_eq!(outcome.diagnostics.len(), 2, "{:?}", outcome.diagnostics);
}
