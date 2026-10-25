//! TRIM/EXTEND contract tests for the widened (curved) geometry subset.
//!
//! The supported targets are LINE, ARC and CIRCLE; the supported boundaries are
//! LINE, ARC, CIRCLE, ELLIPSE, LWPOLYLINE (straight or bulged) and SPLINE, in any
//! mix. The SPLINE boundary cases (sampled cuts, their approximation notes and the
//! refused closed-spline containment) live in `trim_spline_boundary.rs`.
//! INSERT/Opaque targets and boundaries stay explicit refusals that change
//! nothing, and every success is one transaction / one undo step.

use std::f64::consts::PI;

use super::*;

const TAU: f64 = std::f64::consts::TAU;

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

fn circle(id: u128, center: Point3, radius: f64, order: i64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Circle {
            center,
            normal: p3(0.0, 0.0, 1.0),
            radius,
        },
        order,
    )
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

fn line(id: u128, start: Point3, end: Point3, order: i64) -> DbEntity {
    ent(id, SemanticGeometry::Line { start, end }, order)
}

/// Everything a refusal must leave byte-identical: the revision, the entity count
/// and the debug form (all stored fields, including geometry) of every entity.
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

fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1e-9
}

/// Angles are equal modulo `TAU` (the arc parameter is only defined up to a full
/// turn, and TRIM re-normalises `start`).
fn angle_near(actual: f64, expected: f64) -> bool {
    let delta = (actual - expected).rem_euclid(TAU);
    delta.min(TAU - delta) <= 1e-9
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
fn trim_line_by_a_circle_keeps_the_picked_half_in_one_undo_step() {
    // Target (0,0)->(10,0); the circle centre (5,0) r 2 cuts it at x = 3 and 7.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        circle(2, point(5.0, 0.0), 2.0, 1),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(8.0, 0.0)).unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.trim");

    // The picked (right) half survives; the left half is gone because the entity
    // was replaced in place, not split.
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 7.0), "{start:?}");
    assert!(near(end.x, 10.0), "{end:?}");
    assert_eq!(undo_depth(&app), 1, "exactly one undo entry");

    execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 0.0) && near(end.x, 10.0), "{start:?} {end:?}");
}

#[test]
fn trim_line_by_an_arc_honours_only_the_cut_on_the_arc_sweep() {
    // The arc is the LEFT half of the circle centre (5,0) r 2 (start PI/2, sweep
    // PI), so it meets the target only at (3,0); the other circle crossing (7,0)
    // lies outside the sweep and must not cut.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        arc(2, point(5.0, 0.0), 2.0, PI / 2.0, PI, 1),
    ]);
    trim(&mut app, 1, &[2], point(8.0, 0.0)).unwrap();
    let (start, end) = expect_line(&app, 1);
    assert!(
        near(start.x, 3.0),
        "the single cut at x=3 is the only one: {start:?}"
    );
    assert!(near(end.x, 10.0), "{end:?}");
}

#[test]
fn trim_arc_by_a_line_reports_the_angular_remainder_for_both_sweep_signs() {
    // Upper half circle centre (0,0) r 5 (start 0, sweep +PI); the vertical
    // boundary through the centre cuts it at angle PI/2.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI, 0),
        line(2, point(0.0, -10.0), point(0.0, 10.0), 1),
    ]);
    let pick = point(5.0 * (PI / 4.0).cos(), 5.0 * (PI / 4.0).sin());
    trim(&mut app, 1, &[2], pick).unwrap();
    let (center, radius, start, sweep) = expect_arc(&app, 1);
    assert_eq!(center, point(0.0, 0.0));
    assert!(near(radius, 5.0));
    assert!(angle_near(start, 0.0), "start {start}");
    assert!(near(sweep, PI / 2.0), "sweep {sweep}");

    // The mirror case: the same cut on a CLOCKWISE (negative sweep) lower half.
    // The kept piece is [PI/2 .. PI] measured backwards, i.e. start -PI/2 with a
    // negative half sweep — the sign convention must be preserved.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, -PI, 0),
        line(2, point(0.0, -10.0), point(0.0, 10.0), 1),
    ]);
    let pick = point(-5.0 * (PI / 4.0).cos(), -5.0 * (PI / 4.0).sin());
    trim(&mut app, 1, &[2], pick).unwrap();
    let (_, radius, start, sweep) = expect_arc(&app, 1);
    assert!(near(radius, 5.0));
    assert!(angle_near(start, -PI / 2.0), "start {start}");
    assert!(near(sweep, -PI / 2.0), "sweep {sweep}");
}

#[test]
fn trim_arc_refuses_a_pick_that_projects_outside_the_sweep() {
    // The pick is on the opposite half of the circle, so its projection is the
    // parameter 2.0 of a quarter-arc: outside the target's own span. That is the
    // one genuinely ambiguous pick, so it is refused by name, never clamped.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI / 2.0, 0),
        line(2, point(0.0, -10.0), point(0.0, 10.0), 1),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], point(-5.0, 0.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(
        err.to_string().contains("projects outside the target arc"),
        "the refusal names the projection: {err:?}"
    );
    assert_eq!(snapshot(&app), before, "a refused pick changes nothing");
    assert_eq!(undo_depth(&app), 0, "a refused pick opens no transaction");

    // The same rule for a LINE: a click beyond an end projects outside `[0, 1]`
    // and is refused, never clamped onto that end.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        circle(2, point(5.0, 0.0), 2.0, 1),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], point(30.0, 4.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(
        err.to_string().contains("projects outside the target line"),
        "{err:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_refuses_a_pick_on_the_circle_axis() {
    // Exactly at the centre: the radial direction is zero, so there is no angle to
    // project onto. Refused explicitly instead of silently picking a piece.
    let mut app = app_of(vec![
        circle(1, point(0.0, 0.0), 5.0, 0),
        line(2, point(0.0, 0.0), point(0.0, 10.0), 1),
        line(3, point(0.0, 0.0), point(10.0, 0.0), 2),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2, 3], point(0.0, 0.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(
        err.to_string().contains("no radial direction"),
        "the refusal names the degenerate pick: {err:?}"
    );
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // Off the target's plane but on its axis is degenerate too: only the in-plane
    // part of `pick − center` defines the radial direction.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI, 0),
        line(2, point(0.0, -10.0), point(0.0, 10.0), 1),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], p3(0.0, 0.0, 7.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(err.to_string().contains("no radial direction"), "{err:?}");
    assert_eq!(snapshot(&app), before);
}

#[test]
fn trim_circle_by_two_lines_leaves_the_picked_piece_as_an_arc() {
    // The circle centre (0,0) r 5 with non-default display attributes; the two
    // half-lines each cut it once, at angle 0 and at angle PI/2.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(circle(1, point(0.0, 0.0), 5.0, 0)).unwrap();
    b.insert_entity(line(2, point(0.0, 0.0), point(0.0, 10.0), 1))
        .unwrap();
    b.insert_entity(line(3, point(0.0, 0.0), point(10.0, 0.0), 2))
        .unwrap();
    let attributes = non_default_attributes();
    b.set_entity_render_attributes(EntityId(1), attributes.clone())
        .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = app_of(Vec::new());
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);

    let pick = point(5.0 * (PI / 4.0).cos(), 5.0 * (PI / 4.0).sin());
    trim(&mut app, 1, &[2, 3], pick).unwrap();

    // A trimmed circle becomes exactly one ARC: the quarter between the cuts that
    // brackets the pick.
    let (center, radius, start, sweep) = expect_arc(&app, 1);
    assert_eq!(center, point(0.0, 0.0));
    assert!(near(radius, 5.0), "{radius}");
    assert!(angle_near(start, 0.0), "start {start}");
    assert!(near(sweep, PI / 2.0), "sweep {sweep}");

    // Id, layer, space and display state survive the in-place replacement.
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap();
    assert_eq!(entity.id, EntityId(1));
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(EntityId(1)),
        attributes
    );
    assert_eq!(undo_depth(&app), 1);

    // One undo restores the CIRCLE with its attributes.
    execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    match geometry_of(&app, 1) {
        SemanticGeometry::Circle { radius, .. } => assert!(near(radius, 5.0)),
        other => panic!("undo must restore the CIRCLE, got {other:?}"),
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(EntityId(1)),
        attributes,
        "undo must not lose the render attributes"
    );
}

#[test]
fn trim_circle_with_fewer_than_two_cuts_is_refused_without_change() {
    // One cut only: the half-line from the centre outwards meets the circle once.
    let mut app = app_of(vec![
        circle(1, point(0.0, 0.0), 5.0, 0),
        line(2, point(0.0, 0.0), point(0.0, 10.0), 1),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], point(4.0, 3.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(err.to_string().contains("at least two"), "{err:?}");
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // No cut at all: the boundary is nowhere near the circle. The containment
    // question is answered before the "needs two cuts" rule, so this is the
    // no-intersection refusal rather than a cut-count one.
    let mut app = app_of(vec![
        circle(1, point(0.0, 0.0), 5.0, 0),
        line(2, point(20.0, -5.0), point(20.0, 5.0), 1),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], point(0.0, 5.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(err.to_string().contains("no intersection"), "{err:?}");
    assert_eq!(snapshot(&app), before);
}

#[test]
fn trim_accepts_a_mixed_line_arc_and_bulged_polyline_boundary_set() {
    // Cuts on the target at x = 2 (LINE), 4 and 6 (the ARC's two ends) and 8.5
    // (the bulge arc's apex); the pick keeps only (8.5,0)->(10,0).
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        line(2, point(2.0, -1.0), point(2.0, 1.0), 1),
        arc(3, point(5.0, 0.0), 1.0, 0.0, PI, 2),
        ent(
            4,
            SemanticGeometry::Polyline {
                points: vec![point(8.0, -1.0), point(8.0, 1.0)],
                bulges: vec![0.5],
                closed: false,
            },
            3,
        ),
    ]);
    trim(&mut app, 1, &[2, 3, 4], point(9.0, 0.0)).unwrap();
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 8.5), "the bulge arc cuts at x=8.5: {start:?}");
    assert!(near(end.x, 10.0), "{end:?}");
}

#[test]
fn trim_reports_no_approximation_note_when_every_cut_is_analytic() {
    // LINE, ARC and bulged-LWPOLYLINE boundaries are all solved analytically, so
    // no approximation note may appear (the note must mean something).
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        line(2, point(2.0, -1.0), point(2.0, 1.0), 1),
        ent(
            3,
            SemanticGeometry::Polyline {
                points: vec![point(8.0, -1.0), point(8.0, 1.0)],
                bulges: vec![0.5],
                closed: false,
            },
            2,
        ),
    ]);
    let outcome = trim(&mut app, 1, &[2, 3], point(9.0, 0.0)).unwrap();
    assert_eq!(
        outcome.diagnostics.len(),
        1,
        "only the success diagnostic: {:?}",
        outcome.diagnostics
    );
}

#[test]
fn trim_line_by_a_closed_ellipse_boundary() {
    // Ellipse centre (5,0), major radius 2 along +x, ratio 0.5: it cuts y = 0 at
    // x = 3 and x = 7, so the picked right half is kept.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        ent(
            2,
            SemanticGeometry::Ellipse {
                center: point(5.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: p3(2.0, 0.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: TAU,
            },
            1,
        ),
    ]);
    trim(&mut app, 1, &[2], point(8.0, 0.0)).unwrap();
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 7.0), "{start:?}");
    assert!(near(end.x, 10.0), "{end:?}");
}

#[test]
fn extend_line_uses_the_pick_projection_to_choose_the_end() {
    // (9, 5) is five units off the line, so only its projection can say which end
    // was picked: the right end still grows to the near crossing at x = 15.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        circle(2, point(20.0, 0.0), 5.0, 1),
    ]);
    extend(&mut app, 1, &[2], point(9.0, 5.0)).unwrap();
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 0.0), "{start:?}");
    assert!(near(end.x, 15.0), "{end:?}");

    // Below the line near the start the projection chooses the *left* end, and
    // nothing lies behind it: the refusal proves which end the projection picked.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        circle(2, point(20.0, 0.0), 5.0, 1),
    ]);
    let before = snapshot(&app);
    let err = extend(&mut app, 1, &[2], point(1.0, -5.0)).unwrap_err();
    assert!(err.to_string().contains("no intersection"), "{err:?}");
    assert_eq!(snapshot(&app), before);
}

#[test]
fn extend_line_to_a_circle_ahead_stops_at_the_near_crossing() {
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        circle(2, point(20.0, 0.0), 5.0, 1),
    ]);
    let outcome = extend(&mut app, 1, &[2], point(9.0, 0.0)).unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.extend");
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 0.0), "{start:?}");
    assert!(near(end.x, 15.0), "the near crossing at x=15 wins: {end:?}");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn extend_line_refuses_when_nothing_lies_ahead_of_the_picked_end() {
    // The boundary is behind the picked (right) end: the line must not shrink.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        line(2, point(-10.0, -5.0), point(-10.0, 5.0), 1),
    ]);
    let before = snapshot(&app);
    let err = extend(&mut app, 1, &[2], point(9.0, 0.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(err.to_string().contains("intersection"), "{err:?}");
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // Nothing is behind the picked (left) end either: the only boundary sits
    // ahead of the right end.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        line(2, point(20.0, -5.0), point(20.0, 5.0), 1),
    ]);
    let before = snapshot(&app);
    let err = extend(&mut app, 1, &[2], point(1.0, 0.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(snapshot(&app), before);
}

#[test]
fn extend_arc_to_a_line_ahead_keeps_the_sweep_sign() {
    // Quarter arc centre (0,0) r 5 from angle 0 to PI/2; the vertical line at
    // x = -3 meets the circle at angle acos(-3/5) = 2.214 rad, which is the
    // nearest crossing ahead of the arc's end in the positive sweep direction.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI / 2.0, 0),
        line(2, point(-3.0, -10.0), point(-3.0, 10.0), 1),
    ]);
    let pick = point(5.0 * 1.2f64.cos(), 5.0 * 1.2f64.sin());
    extend(&mut app, 1, &[2], pick).unwrap();
    let (center, radius, start, sweep) = expect_arc(&app, 1);
    assert_eq!(center, point(0.0, 0.0));
    assert!(near(radius, 5.0));
    assert!(angle_near(start, 0.0), "start {start}");
    let expected = (-3.0f64 / 5.0).acos();
    assert!(near(sweep, expected), "sweep {sweep} != {expected}");
}

#[test]
fn extend_arc_keeps_the_sweep_sign_when_extending_the_start() {
    // Quarter arc centre (0,0) r 5 from angle 0 to PI/2. The short boundary
    // segment crosses the circle exactly at angle -0.1, which is behind the arc's
    // start; picking near the start grows it backwards and the sweep stays
    // positive (it only gets longer).
    let angle = -0.1f64;
    let x = 5.0 * angle.cos();
    let y = 5.0 * angle.sin();
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI / 2.0, 0),
        line(2, point(x - 0.2, y - 0.2), point(x + 0.2, y + 0.2), 1),
    ]);
    // Pick near the arc's start (u < 0.5 grows the start).
    let pick = point(5.0 * 0.2f64.cos(), 5.0 * 0.2f64.sin());
    extend(&mut app, 1, &[2], pick).unwrap();
    let (_, _, start, sweep) = expect_arc(&app, 1);
    assert!(angle_near(start, -0.1), "start {start}");
    assert!(near(sweep, PI / 2.0 + 0.1), "sweep {sweep}");
}

#[test]
fn extend_arc_refuses_when_nothing_lies_ahead_of_the_picked_end() {
    // A boundary that never meets the target's own circle: nothing to extend to.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI / 2.0, 0),
        line(2, point(20.0, -5.0), point(20.0, 5.0), 1),
    ]);
    let before = snapshot(&app);
    let pick = point(5.0 * 1.2f64.cos(), 5.0 * 1.2f64.sin());
    let err = extend(&mut app, 1, &[2], pick).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(err.to_string().contains("intersection"), "{err:?}");
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);

    // A boundary that already crosses the arc is not extendable either.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI / 2.0, 0),
        line(2, point(3.0, -10.0), point(3.0, 10.0), 1),
    ]);
    let before = snapshot(&app);
    let err = extend(&mut app, 1, &[2], pick).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(err.to_string().contains("already reaches"), "{err:?}");
    assert_eq!(snapshot(&app), before);
}

#[test]
fn extend_accepts_a_bulged_polyline_boundary() {
    // The bulge arc from (20,-5) to (20,5) with bulge 0.5 has its centre at
    // (16.25,0) and radius 6.25, so its apex (the bow of a positive bulge) is at
    // x = 22.5, and the other circle crossing (x = 10) is outside the sweep.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        ent(
            2,
            SemanticGeometry::Polyline {
                points: vec![point(20.0, -5.0), point(20.0, 5.0)],
                bulges: vec![0.5],
                closed: false,
            },
            1,
        ),
    ]);
    extend(&mut app, 1, &[2], point(9.0, 0.0)).unwrap();
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 0.0), "{start:?}");
    assert!(
        near(end.x, 22.5),
        "the extension must stop on the bulge apex: {end:?}"
    );
}

/// The point at `angle` on a circle of `radius` about the origin.
fn at(radius: f64, angle: f64) -> Point3 {
    point(radius * angle.cos(), radius * angle.sin())
}

#[test]
fn extend_arc_negative_sweep_grows_the_start_clockwise() {
    // Clockwise quarter arc: start angle 0 down to -PI/2. The boundary crosses the
    // circle at angle +0.1 rad, i.e. counter-clockwise of the start — exactly the
    // direction the start end grows in. Before the unsigned period fix the
    // period-shifted candidate was more positive than `u`, so this end could never
    // grow at all.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, -PI / 2.0, 0),
        line(2, at(4.5, 0.1), at(5.5, 0.1), 1),
    ]);
    // Pick near the arc's start (u < 0.5 grows the start end).
    extend(&mut app, 1, &[2], at(5.0, -0.2)).unwrap();
    let (center, radius, start, sweep) = expect_arc(&app, 1);
    assert_eq!(center, point(0.0, 0.0));
    assert!(near(radius, 5.0), "{radius}");
    assert!(angle_near(start, 0.1), "start {start}");
    assert!(near(sweep, -(PI / 2.0 + 0.1)), "sweep {sweep}");
}

#[test]
fn extend_arc_refuses_a_boundary_only_reachable_the_long_way_round() {
    // The crossing sits just *behind* the start (angle -0.1 rad). The end can only
    // reach it by sweeping 354 degrees, so the half-turn bound refuses instead of
    // unfolding the arc into almost a full circle.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI / 2.0, 0),
        line(2, at(4.5, -0.1), at(5.5, -0.1), 1),
    ]);
    let before = snapshot(&app);
    let err = extend(&mut app, 1, &[2], at(5.0, 1.2)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(err.to_string().contains("half a turn"), "{err:?}");
    assert_eq!(snapshot(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn trim_refuses_an_arc_target_whose_sweep_exceeds_a_full_turn() {
    // More than one turn has no single periodic period, so its cuts would fold at
    // the wrong place: an explicit refusal, never a guess.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, TAU + 0.5, 0),
        line(2, at(4.5, 0.0), at(5.5, 0.0), 1),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], at(5.0, 0.1)).unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    assert!(err.to_string().contains("one full turn"), "{err:?}");
    assert_eq!(snapshot(&app), before);
}

#[test]
fn trim_deletes_a_circle_wholly_inside_a_closed_boundary() {
    // A CIRCLE has no ends, so the containment rule must be answered before the
    // "a periodic target needs two cuts" rule; otherwise a fully contained circle
    // would be refused instead of removed.
    let mut app = app_of(vec![
        circle(1, point(5.0, 5.0), 1.0, 0),
        ent(
            2,
            SemanticGeometry::Polyline {
                points: vec![
                    point(0.0, 0.0),
                    point(10.0, 0.0),
                    point(10.0, 10.0),
                    point(0.0, 10.0),
                ],
                bulges: vec![0.0, 0.0, 0.0, 0.0],
                closed: true,
            },
            1,
        ),
    ]);
    trim(&mut app, 1, &[2], point(6.0, 5.0)).unwrap();
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_none(),
        "a circle wholly inside the boundary is removed"
    );
    assert_eq!(undo_depth(&app), 1);

    execute(
        &mut app,
        &mut SessionState::new(DocumentId(1), AppMode::Work),
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    match geometry_of(&app, 1) {
        SemanticGeometry::Circle { radius, .. } => assert!(near(radius, 1.0)),
        other => panic!("undo must restore the CIRCLE, got {other:?}"),
    }
}

#[test]
fn trim_delete_rule_covers_circle_ellipse_and_full_arc_boundaries() {
    // One target LINE wholly inside each closed analytic boundary kind: the
    // `closed_curves` inside test, not just the polyline outline.
    let boundaries = [
        circle(2, point(5.0, 5.0), 4.0, 1),
        ent(
            2,
            SemanticGeometry::Ellipse {
                center: point(5.0, 5.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: p3(4.0, 0.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: TAU,
            },
            1,
        ),
        arc(2, point(5.0, 5.0), 4.0, 0.0, TAU, 1),
    ];
    for boundary in boundaries {
        let mut app = app_of(vec![line(1, point(5.0, 5.0), point(6.0, 5.0), 0), boundary]);
        trim(&mut app, 1, &[2], point(5.5, 5.0)).unwrap();
        assert!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(EntityId(1))
                .is_none(),
            "the contained target is removed"
        );
        assert_eq!(undo_depth(&app), 1);
    }
}

#[test]
fn trim_delete_rule_covers_a_closed_bulged_polyline() {
    // The outline is the sampled ring of a closed polyline whose right side bulges
    // out, so the inside test runs over the bulge geometry rather than the chords.
    let mut app = app_of(vec![
        line(1, point(2.0, 5.0), point(3.0, 5.0), 0),
        ent(
            2,
            SemanticGeometry::Polyline {
                points: vec![
                    point(0.0, 0.0),
                    point(10.0, 0.0),
                    point(10.0, 10.0),
                    point(0.0, 10.0),
                ],
                bulges: vec![0.0, 0.5, 0.0, 0.0],
                closed: true,
            },
            1,
        ),
    ]);
    trim(&mut app, 1, &[2], point(2.5, 5.0)).unwrap();
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_none(),
        "the contained target is removed"
    );
}

#[test]
fn trim_delete_rule_respects_the_boundary_plane() {
    // A closed square lifted 45 degrees about +x. Its XY projection is the
    // rectangle 0..10 x 0..7.07, so a flat XY ray cast would believe a line at
    // z = 0 / y = 2 is inside; it is 1.41 world units off the boundary plane and
    // never touches it, so it must be refused instead of deleted.
    let tilted = || SemanticGeometry::Polyline {
        points: vec![
            p3(0.0, 0.0, 0.0),
            p3(10.0, 0.0, 0.0),
            p3(10.0, 7.071_067_8, 7.071_067_8),
            p3(0.0, 7.071_067_8, 7.071_067_8),
        ],
        bulges: vec![0.0, 0.0, 0.0, 0.0],
        closed: true,
    };

    let mut app = app_of(vec![
        line(1, p3(3.0, 2.0, 0.0), p3(4.0, 2.0, 0.0), 0),
        ent(2, tilted(), 1),
    ]);
    let before = snapshot(&app);
    let err = trim(&mut app, 1, &[2], p3(3.5, 2.0, 0.0)).unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(err.to_string().contains("no intersection"), "{err:?}");
    assert_eq!(
        snapshot(&app),
        before,
        "the off-plane target must not be deleted"
    );

    // In the boundary's own plane (y = z) the same projected position *is* inside,
    // so the delete rule still fires: the test is the plane, not a blanket refusal
    // of tilted boundaries.
    let mut app = app_of(vec![
        line(1, p3(3.0, 2.0, 2.0), p3(4.0, 2.0, 2.0), 0),
        ent(2, tilted(), 1),
    ]);
    trim(&mut app, 1, &[2], p3(3.5, 2.0, 2.0)).unwrap();
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_none(),
        "an in-plane contained target is removed"
    );
}

#[test]
fn trim_line_uses_the_pick_projection_well_beyond_the_world_tolerance() {
    // The host resolves the target with its pickbox and passes the raw click, so a
    // click 5 units off the line (orders of magnitude beyond the world tolerance)
    // is still a pick of it: the perpendicular projection decides which half.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        circle(2, point(5.0, 0.0), 2.0, 1),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(8.0, 5.0)).unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.trim");
    let (start, end) = expect_line(&app, 1);
    assert!(
        near(start.x, 7.0),
        "the picked (right) half survives: {start:?}"
    );
    assert!(near(end.x, 10.0), "{end:?}");
    assert_eq!(undo_depth(&app), 1);

    // The mirror pick below the line keeps the left half: the projection, not the
    // click's distance, decides.
    let mut app = app_of(vec![
        line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
        circle(2, point(5.0, 0.0), 2.0, 1),
    ]);
    trim(&mut app, 1, &[2], point(2.0, -5.0)).unwrap();
    let (start, end) = expect_line(&app, 1);
    assert!(near(start.x, 0.0), "{start:?}");
    assert!(near(end.x, 3.0), "the picked (left) half survives: {end:?}");
}

#[test]
fn trim_arc_uses_the_pick_radial_projection_at_a_wrong_radius() {
    // Upper half circle centre (0,0) r 5 cut at angle PI/2 by the vertical line.
    // The pick sits at radius 13 (a radius 8 off the target), so only the radial
    // projection can place it: each side of the cut keeps its own piece.
    for (pick_angle, expected_start) in [(PI / 4.0, 0.0), (3.0 * PI / 4.0, PI / 2.0)] {
        let mut app = app_of(vec![
            arc(1, point(0.0, 0.0), 5.0, 0.0, PI, 0),
            line(2, point(0.0, -10.0), point(0.0, 10.0), 1),
        ]);
        trim(&mut app, 1, &[2], at(13.0, pick_angle)).unwrap();
        let (center, radius, start, sweep) = expect_arc(&app, 1);
        assert_eq!(center, point(0.0, 0.0));
        assert!(near(radius, 5.0), "the radius must stay 5: {radius}");
        assert!(angle_near(start, expected_start), "start {start}");
        assert!(near(sweep, PI / 2.0), "sweep {sweep}");
    }
}

#[test]
fn trim_reports_the_sampled_approximation_for_a_non_analytic_pair() {
    // ARC x ELLIPSE has no analytic arm in the engine, so the cut point comes from
    // its sampled fallback. The command must say so instead of implying an exact
    // cut; the numbers below are deliberately loose because the cut is an estimate.
    let mut app = app_of(vec![
        arc(1, point(0.0, 0.0), 5.0, 0.0, PI, 0),
        ent(
            2,
            SemanticGeometry::Ellipse {
                center: point(0.0, 0.0),
                normal: p3(0.0, 0.0, 1.0),
                major_axis: p3(8.0, 0.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: TAU,
            },
            1,
        ),
    ]);
    let outcome = trim(&mut app, 1, &[2], point(0.0, 5.0)).unwrap();
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
        note.message.contains("AcDbEllipse"),
        "the note names the approximated boundary: {:?}",
        note.message
    );
    let (_, _, start, sweep) = expect_arc(&app, 1);
    assert!(start > 0.7 && start < 0.9, "approximate start {start}");
    assert!(sweep > 1.35 && sweep < 1.70, "approximate sweep {sweep}");
}
