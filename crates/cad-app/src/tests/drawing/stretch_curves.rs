//! STRETCH contract tests for the two **curve** kinds (ADR 0016 decision E):
//! `SPLINE` (stored control points) and `ARC` (stored start/end point on the
//! unchanged circle).
//!
//! These pin the declared per-kind rules documented in
//! `crate::app_drawing::stretch` and `docs/compatibility.md`:
//!
//! * SPLINE: a partial crossing window moves **exactly** the control points
//!   inside the window by the drag delta, every other control point stays
//!   bit-identical, and `degree`/`knots`/`weights` are copied verbatim, so the
//!   committed entity still passes `cad_db::validate_geometry`. A fully enclosed
//!   spline moves whole; a window with no control point inside writes nothing.
//! * ARC: a partial crossing window moves **only the in-window endpoints** onto
//!   new in-plane angles while `center`/`radius`/`normal` stay bit-identical and
//!   the sweep is recomputed in the stored direction; the three refusals
//!   (`arc_no_direction`, `arc_degenerate_sweep`, `arc_unchanged`) write nothing
//!   and name their reason.
//!
//! Every case asserts the transaction/undo shape: one transaction for the whole
//! window, one `update_entity` per changed entity, one undo entry, and nothing
//! written when nothing moves.

use super::*;

/// Build a drawing with layer 0 plus the given entities.
fn build(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// The two opposite corners of a crossing window, as the **canvas logical**
/// corners the payload expects, from the two world corners a test thinks in.
fn window(a: Point3, b: Point3) -> [[f64; 2]; 2] {
    let vp = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    let size = vp.logical_size;
    let to_logical = |world: Point3| {
        vp.camera
            .world_to_screen(world, size)
            .expect("finite world point")
    };
    [to_logical(a), to_logical(b)]
}

fn stretch(
    app: &mut Application,
    session: &mut SessionState,
    rect: [[f64; 2]; 2],
    base: Point3,
    delta: Point3,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::StretchEntities,
        CommandPayload::Stretch {
            rect_logical: rect,
            base,
            delta,
        },
    )
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn geometry_of(app: &Application, id: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .unwrap()
        .geometry
        .clone()
}

fn has_code(outcome: &CommandOutcome, code: &str) -> bool {
    outcome.diagnostics.iter().any(|d| d.code == code)
}

fn linear_of(app: &Application, id: u128) -> (Point3, Point3) {
    match geometry_of(app, id) {
        SemanticGeometry::Line { start, end } => (start, end),
        other => panic!("expected a line, got {other:?}"),
    }
}

fn polyline_of(app: &Application, id: u128) -> (Vec<Point3>, Vec<f64>, bool) {
    match geometry_of(app, id) {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points, bulges, closed),
        other => panic!("expected a polyline, got {other:?}"),
    }
}

fn circle_of(app: &Application, id: u128) -> (Point3, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Circle { center, radius, .. } => (center, radius),
        other => panic!("expected a circle, got {other:?}"),
    }
}

/// The full stored spline definition, cloned out for bit-exact comparison.
fn spline_of(app: &Application, id: u128) -> (u32, Vec<f64>, Vec<Point3>, Vec<f64>) {
    match geometry_of(app, id) {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => (degree, knots, control_points, weights),
        other => panic!("expected a spline, got {other:?}"),
    }
}

/// The full stored arc definition, cloned out for bit-exact comparison.
fn arc_of(app: &Application, id: u128) -> (Point3, Point3, f64, f64, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => (center, normal, radius, start, sweep),
        other => panic!("expected an arc, got {other:?}"),
    }
}

/// Degree-2 clamped spline along +X: control points `(0,0) (2,0) (4,0) (6,0)`,
/// so both a "contains only some control points" window and a "crosses the curve
/// with no control point inside" window are easy to express.
fn spline_geometry() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0],
        control_points: vec![
            point(0.0, 0.0),
            point(2.0, 0.0),
            point(4.0, 0.0),
            point(6.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0, 1.0],
    }
}

/// A `+Z` arc centred at `(5, 5)`, radius `3`, `start = 0`, `sweep = π/2`, so its
/// start point is exactly `(8, 5)` and its end point exactly `(5, 8)`
/// (`arbitrary_axis(+Z)` gives `ax = +X`, `ay = +Y`).
fn arc_geometry(sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center: point(5.0, 5.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 3.0,
        start: 0.0,
        sweep,
    }
}

// ---------------------------------------------------------------------------
// SPLINE
// ---------------------------------------------------------------------------

#[test]
fn stretch_spline_partial_moves_only_the_control_points_inside_the_window() {
    let original = spline_geometry();
    let (mut app, mut session) = build(vec![ent(1, original.clone(), 0)]);
    let before = revision(&app);

    // Contains (0,0) and (2,0); the curve passes through the window, so the
    // enclosure is Partial. The two out-of-window control points stay put.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(-0.5, -1.0), point(3.0, 1.0)),
        point(0.0, 0.0),
        point(1.0, 2.0),
    )
    .unwrap();

    let (degree, knots, control_points, weights) = spline_of(&app, 1);
    assert_eq!(degree, 2, "the degree is copied verbatim");
    assert_eq!(
        knots,
        vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0],
        "the knot vector is copied verbatim"
    );
    assert_eq!(
        weights,
        vec![1.0, 1.0, 1.0, 1.0],
        "the weights are copied verbatim"
    );
    // Each in-window control point moved by exactly the delta, bit for bit.
    assert_eq!(control_points[0], point(1.0, 2.0));
    assert_eq!(control_points[1], point(3.0, 2.0));
    // Each out-of-window control point is the stored point, bit for bit.
    assert_eq!(control_points[2], point(4.0, 0.0));
    assert_eq!(control_points[3], point(6.0, 0.0));

    // The moved definition is still a valid *write*: the knot/weight relations
    // `cad-db` enforces still hold, so a partial stretch can never store a spline
    // the database would refuse.
    cad_db::validate_geometry(&geometry_of(&app, 1))
        .expect("the stretched spline is still a valid stored definition");

    assert_eq!(outcome.objects, vec![ObjectId(1)]);
    assert!(outcome.changes.is_some(), "one committed transaction");
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo entry for one entity");
    assert!(
        has_code(&outcome, "drawing.stretch"),
        "{:?}",
        outcome.diagnostics
    );

    // The single undo entry restores the whole stored definition bit-exactly.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(geometry_of(&app, 1), original);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn stretch_spline_fully_enclosed_moves_every_control_point_whole() {
    let (mut app, mut session) = build(vec![ent(1, spline_geometry(), 0)]);
    // The window contains the whole control hull (the curve lies inside it).
    stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -1.0), point(7.0, 1.0)),
        point(0.0, 0.0),
        point(1.0, 2.0),
    )
    .unwrap();
    let (degree, knots, control_points, weights) = spline_of(&app, 1);
    assert_eq!(degree, 2);
    assert_eq!(knots, vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0]);
    assert_eq!(weights, vec![1.0, 1.0, 1.0, 1.0]);
    assert_eq!(
        control_points,
        vec![
            point(1.0, 2.0),
            point(3.0, 2.0),
            point(5.0, 2.0),
            point(7.0, 2.0)
        ],
        "a fully enclosed spline is translated as a whole"
    );
}

#[test]
fn stretch_spline_window_with_no_control_point_moves_nothing_and_writes_nothing() {
    let original = spline_geometry();
    let (mut app, mut session) = build(vec![ent(1, original.clone(), 0)]);
    let before = revision(&app);
    // The window crosses the curve around x = 3 but contains no control point
    // (they sit at x = 0, 2, 4, 6): a control point is the addressable vertex, so
    // a curve-only crossing is explicitly unchanged rather than guessed.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(2.5, -1.0), point(3.5, 1.0)),
        point(0.0, 0.0),
        point(1.0, 2.0),
    )
    .unwrap();
    assert!(
        outcome.changes.is_none(),
        "no transaction when nothing moves"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(geometry_of(&app, 1), original);
    assert!(
        has_code(&outcome, "drawing.stretch.no_vertex"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_spline_with_an_unindexable_definition_is_refused_by_name() {
    // `weights` has one entry fewer than the control points: nothing can index
    // the definition together with its weights, so editing it would be a guess.
    // The import builder is permissive, so such an entity can exist in a drawing.
    let original = SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0],
        control_points: vec![
            point(0.0, 0.0),
            point(2.0, 0.0),
            point(4.0, 0.0),
            point(6.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0],
    };
    let (mut app, mut session) = build(vec![ent(1, original.clone(), 0)]);
    let before = revision(&app);
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(-0.5, -1.0), point(3.0, 1.0)),
        point(0.0, 0.0),
        point(1.0, 2.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(geometry_of(&app, 1), original, "nothing is written");
    assert!(
        has_code(&outcome, "drawing.stretch.spline_inconsistent"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_spline_and_line_in_one_window_are_one_transaction_and_one_undo() {
    // Command-level integration: a real crossing window with two per-vertex kinds.
    // The spline keeps its definition, the line stretches, and both land in one
    // transaction / one undo entry (one `update_entity` per changed entity).
    let spline = spline_geometry();
    let line = SemanticGeometry::Line {
        start: point(0.0, 5.0),
        end: point(6.0, 5.0),
    };
    let (mut app, mut session) = build(vec![ent(1, spline.clone(), 0), ent(2, line.clone(), 1)]);
    let before = revision(&app);
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(-0.5, -1.0), point(3.0, 6.0)),
        point(0.0, 0.0),
        point(1.0, 2.0),
    )
    .unwrap();
    let mut changed = outcome.objects.clone();
    changed.sort();
    assert_eq!(changed, vec![ObjectId(1), ObjectId(2)]);
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo entry");

    let (_, knots, control_points, _) = spline_of(&app, 1);
    assert_eq!(knots.len(), 7);
    assert_eq!(control_points[1], point(3.0, 2.0));
    assert_eq!(control_points[2], point(4.0, 0.0));
    let (start, end) = linear_of(&app, 2);
    assert_eq!(start, point(1.0, 7.0));
    assert_eq!(end, point(6.0, 5.0));

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(geometry_of(&app, 1), spline);
    assert_eq!(geometry_of(&app, 2), line);
    assert_eq!(undo_depth(&app), 0);
}

// ---------------------------------------------------------------------------
// ARC
// ---------------------------------------------------------------------------

#[test]
fn stretch_arc_moves_only_the_in_window_start_point_on_the_same_circle() {
    let (mut app, mut session) = build(vec![ent(1, arc_geometry(std::f64::consts::FRAC_PI_2), 0)]);
    let before = revision(&app);
    // Contains the start point (8,5), not the end point (5,8). Moving the start
    // point to (9,6) puts it at angle `atan2(1, 4)` on the same circle.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(6.5, 4.5), point(9.5, 6.0)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();
    let (center, normal, radius, start, sweep) = arc_of(&app, 1);
    assert_eq!(center, point(5.0, 5.0), "the centre is bit-identical");
    assert_eq!(normal, p3(0.0, 0.0, 1.0), "the normal is bit-identical");
    assert_eq!(radius, 3.0, "the radius is bit-identical");
    assert_eq!(
        start,
        1.0f64.atan2(4.0),
        "the start angle is the moved direction"
    );
    let expected_sweep = std::f64::consts::FRAC_PI_2 - 1.0f64.atan2(4.0);
    assert!(
        (sweep - expected_sweep).abs() < 1e-12,
        "sweep {sweep} != {expected_sweep}"
    );
    assert!(sweep > 0.0, "the stored (positive) direction is preserved");
    assert_eq!(revision(&app).0, before.0 + 1);
    assert_eq!(undo_depth(&app), 1);
    assert!(
        has_code(&outcome, "drawing.stretch"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_arc_moves_only_the_in_window_end_point_and_keeps_start_bit_identical() {
    let (mut app, mut session) = build(vec![ent(1, arc_geometry(std::f64::consts::FRAC_PI_2), 0)]);
    // Contains the end point (5,8), not the start point (8,5).
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(4.5, 6.5), point(5.5, 9.0)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();
    assert!(
        has_code(&outcome, "drawing.stretch"),
        "{:?}",
        outcome.diagnostics
    );
    let (center, normal, radius, start, sweep) = arc_of(&app, 1);
    assert_eq!(center, point(5.0, 5.0));
    assert_eq!(normal, p3(0.0, 0.0, 1.0));
    assert_eq!(radius, 3.0);
    assert_eq!(start, 0.0, "the out-of-window start angle is bit-identical");
    // The end point moved to (6,9): direction `atan2(4, 1)` from the centre.
    let expected_sweep = 4.0f64.atan2(1.0);
    assert!(
        (sweep - expected_sweep).abs() < 1e-12,
        "sweep {sweep} != {expected_sweep}"
    );
}

#[test]
fn stretch_arc_keeps_a_clockwise_arc_clockwise() {
    // `start = 0`, `sweep = -π/2`: the arc runs clockwise from the start point
    // (8,5) to the end point (5,2). The window contains only the end point.
    let (mut app, mut session) = build(vec![ent(1, arc_geometry(-std::f64::consts::FRAC_PI_2), 0)]);
    stretch(
        &mut app,
        &mut session,
        window(point(4.5, 1.0), point(5.5, 3.5)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();
    let (center, _, radius, start, sweep) = arc_of(&app, 1);
    assert_eq!(center, point(5.0, 5.0));
    assert_eq!(radius, 3.0);
    assert_eq!(start, 0.0, "the out-of-window start angle is bit-identical");
    // The end point (5,2) moved to (6,3): direction `atan2(-2, 1)` from the
    // centre, and the stored negative direction keeps the sign.
    let expected_sweep = (-2.0f64).atan2(1.0);
    assert!(sweep < 0.0, "the clockwise direction is preserved: {sweep}");
    assert!(
        (sweep - expected_sweep).abs() < 1e-12,
        "sweep {sweep} != {expected_sweep}"
    );
}

#[test]
fn stretch_arc_crossing_with_no_endpoint_inside_changes_nothing_with_diagnostic() {
    let original = arc_geometry(std::f64::consts::FRAC_PI_2);
    let (mut app, mut session) = build(vec![ent(1, original.clone(), 0)]);
    let before = revision(&app);
    // Crosses the middle of the arc (around (7.1, 7.1)); neither endpoint is in.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(6.8, 6.8), point(7.5, 7.5)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(geometry_of(&app, 1), original);
    assert!(
        has_code(&outcome, "drawing.stretch.no_vertex"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_arc_refuses_a_degenerate_recomputed_sweep_by_name() {
    let original = arc_geometry(std::f64::consts::FRAC_PI_2);
    let (mut app, mut session) = build(vec![ent(1, original.clone(), 0)]);
    let before = revision(&app);
    // The start point (8,5) is dragged exactly onto the end point (5,8): the
    // stored direction then runs a full turn, which is not a storable arc.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(6.5, 4.5), point(9.5, 6.0)),
        point(0.0, 0.0),
        point(-3.0, 3.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none(), "nothing is written");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(geometry_of(&app, 1), original, "the arc is bit-identical");
    assert!(
        has_code(&outcome, "drawing.stretch.arc_degenerate_sweep"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_arc_refuses_an_endpoint_with_no_in_plane_direction_by_name() {
    let original = arc_geometry(std::f64::consts::FRAC_PI_2);
    let (mut app, mut session) = build(vec![ent(1, original.clone(), 0)]);
    let before = revision(&app);
    // The start point (8,5) is dragged onto the centre (5,5): no in-plane
    // direction, so no endpoint angle exists.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(6.5, 4.5), point(9.5, 6.0)),
        point(0.0, 0.0),
        point(-3.0, 0.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none(), "nothing is written");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(geometry_of(&app, 1), original);
    assert!(
        has_code(&outcome, "drawing.stretch.arc_no_direction"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_arc_with_a_purely_radial_drag_is_unchanged_by_name() {
    let original = arc_geometry(std::f64::consts::FRAC_PI_2);
    let (mut app, mut session) = build(vec![ent(1, original.clone(), 0)]);
    let before = revision(&app);
    // A purely radial drag of the start point (8,5) keeps its angle: an arc that
    // must stay on the same circle has nothing to move.
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(6.5, 4.5), point(9.5, 6.0)),
        point(0.0, 0.0),
        point(1.0, 0.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none(), "nothing is written");
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(geometry_of(&app, 1), original);
    assert!(
        has_code(&outcome, "drawing.stretch.arc_unchanged"),
        "{:?}",
        outcome.diagnostics
    );
}

// ---------------------------------------------------------------------------
// Regressions: the non-curve kinds keep their pre-existing behaviour
// ---------------------------------------------------------------------------

#[test]
fn stretch_regression_line_polyline_and_circle_behaviour_is_unchanged() {
    // LINE: only the in-window endpoint moves.
    let (mut app, mut session) = build(vec![ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    )]);
    stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -1.0), point(2.0, 1.0)),
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap();
    let (start, end) = linear_of(&app, 1);
    assert_eq!(start, point(0.0, 5.0));
    assert_eq!(end, point(10.0, 0.0), "the outside endpoint is put");

    // Bulge POLYLINE: in-window vertices move, the stored bulges are preserved.
    let (mut app, mut session) = build(vec![ent(
        1,
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 3.0), point(4.0, 3.0), point(8.0, 0.0)],
            bulges: vec![0.5, 0.0, 0.0],
            closed: false,
        },
        0,
    )]);
    stretch(
        &mut app,
        &mut session,
        window(point(-1.0, 2.0), point(5.0, 4.0)),
        point(0.0, 0.0),
        point(0.0, -3.0),
    )
    .unwrap();
    let (points, bulges, closed) = polyline_of(&app, 1);
    assert_eq!(
        points,
        vec![point(0.0, 0.0), point(4.0, 0.0), point(8.0, 0.0)]
    );
    assert_eq!(bulges, vec![0.5, 0.0, 0.0]);
    assert!(!closed);

    // CIRCLE: still move-only, so a partial enclosure writes nothing and reports
    // the move-only diagnostic.
    let circle = SemanticGeometry::Circle {
        center: point(5.0, 5.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 3.0,
    };
    let (mut app, mut session) = build(vec![ent(1, circle.clone(), 0)]);
    let before = revision(&app);
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(0.0, 0.0), point(6.0, 6.0)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app), before);
    let (center, radius) = circle_of(&app, 1);
    assert_eq!(center, point(5.0, 5.0));
    assert_eq!(radius, 3.0);
    assert_eq!(geometry_of(&app, 1), circle);
    assert!(
        has_code(&outcome, "drawing.stretch.not_stretchable"),
        "{:?}",
        outcome.diagnostics
    );
}
