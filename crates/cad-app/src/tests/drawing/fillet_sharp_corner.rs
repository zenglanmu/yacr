//! FILLET `R = 0` **sharp corner** contracts for the non-spline planar pairs.
//!
//! `R = 0` has no arc: the picked pair is trimmed to the intersection nearest the
//! two picks, each object on the pick side, and nothing is inserted. Every planar
//! pair this build can intersect does this — LINE/LINE (its own file), LINE/CIRCLE,
//! LINE/ARC, LINE/ELLIPSE, LINE/SPLINE, CIRCLE/ARC, CIRCLE/ELLIPSE, ARC/ARC,
//! ARC/ELLIPSE, ELLIPSE/ELLIPSE and SPLINE pairs (see `fillet_spline.rs`) — with
//! one stated exception: a **CIRCLE** is a closed curve this command never cuts (a
//! single cut point leaves the whole circle), so CIRCLE ↔ CIRCLE has no corner to
//! write and is refused by name.
//!
//! The intersections come from the same machinery TRIM/EXTEND use:
//! `cad_geometry`'s geometry engine (analytic conic/conic and conic/segment
//! solutions) for LINE/CIRCLE/ARC pairs, and the numeric residual scan
//! (`corner_roots`) for pairs with an ellipse or spline.

use super::*;

/// The stored entity of one id.
fn db_entity(app: &Application, entity: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap()
        .clone()
}

/// The stored geometry of one entity.
fn stored_geometry(app: &Application, entity: u128) -> SemanticGeometry {
    db_entity(app, entity).geometry
}

/// Wrap a finished drawing in the standard application/session pair.
fn app_from(drawing: cad_db::DrawingDatabase) -> (Application, SessionState) {
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// A drawing with two curve entities (ids 1 and 2) on layer 0.
fn curve_app(first: SemanticGeometry, second: SemanticGeometry) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(1, first, 0)).unwrap();
    b.insert_entity(ent(2, second, 1)).unwrap();
    app_from(b.finish().unwrap())
}

fn line(start: Point3, end: Point3) -> SemanticGeometry {
    SemanticGeometry::Line { start, end }
}

fn circle(center: Point3, radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
    }
}

fn arc(center: Point3, radius: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
        start,
        sweep,
    }
}

/// A full ELLIPSE with the major axis along `+X` (`a`, `b = a · ratio`).
fn ellipse(center: Point3, a: f64, ratio: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Ellipse {
        center,
        normal: p3(0.0, 0.0, 1.0),
        major_axis: p3(a, 0.0, 0.0),
        ratio,
        start,
        sweep,
    }
}

fn fillet(first: u128, second: u128, pick_first: Point3, pick_second: Point3) -> CommandPayload {
    CommandPayload::Fillet(Box::new(TwoLineCommand {
        first: ref_of(first),
        second: ref_of(second),
        pick_first,
        pick_second,
    }))
}

fn set_number(session: &mut SessionState, parameter: ToolParameter, value: f64) {
    session
        .set_tool_parameter(parameter, ToolValue::Number(value))
        .unwrap_or_else(|error| panic!("{parameter:?} = {value} refused: {error}"));
}

fn run_fillet(
    app: &mut Application,
    session: &mut SessionState,
    payload: CommandPayload,
) -> CommandOutcome {
    execute(app, session, CommandId::FilletLines, payload)
        .expect("the R=0 sharp corner is supported")
}

/// `(start, end)` of a stored LINE.
fn line_ends(app: &Application, entity: u128) -> (Point3, Point3) {
    match stored_geometry(app, entity) {
        SemanticGeometry::Line { start, end } => (start, end),
        other => panic!("expected a LINE, got {other:?}"),
    }
}

/// `(center, radius, start, sweep)` of a stored ARC.
fn arc_data(app: &Application, entity: u128) -> (Point3, f64, f64, f64) {
    match stored_geometry(app, entity) {
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

/// `(start, sweep)` of a stored ELLIPSE.
fn ellipse_span(app: &Application, entity: u128) -> (f64, f64) {
    match stored_geometry(app, entity) {
        SemanticGeometry::Ellipse { start, sweep, .. } => (start, sweep),
        other => panic!("expected an ELLIPSE, got {other:?}"),
    }
}

fn point_at(center: Point3, radius: f64, angle: f64) -> Point3 {
    point(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    )
}

fn assert_point_near(actual: Point3, expected: Point3) {
    assert!(
        (actual.x - expected.x).abs() < 1e-9
            && (actual.y - expected.y).abs() < 1e-9
            && (actual.z - expected.z).abs() < 1e-9,
        "point ({}, {}, {}) != ({}, {}, {})",
        actual.x,
        actual.y,
        actual.z,
        expected.x,
        expected.y,
        expected.z
    );
}

fn assert_angle_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "angle {actual} != {expected}"
    );
}

/// Whether `p` lies on the circle `center`/`radius` (2D).
fn on_circle(p: Point3, center: Point3, radius: f64) -> bool {
    let distance = ((p.x - center.x).powi(2) + (p.y - center.y).powi(2)).sqrt();
    (distance - radius).abs() < 1e-9
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
}

/// Run a FILLET payload that must be refused with a named code and no state change.
fn assert_refused(
    app: &mut Application,
    session: &mut SessionState,
    payload: CommandPayload,
    expected_code: &str,
) {
    let rev = revision(app);
    let depth = app
        .history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0);
    let error = execute(app, session, CommandId::FilletLines, payload).unwrap_err();
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(
        error.to_string().contains(expected_code),
        "{error} must name {expected_code}"
    );
    assert_eq!(revision(app), rev, "a refused edit changes nothing");
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        depth,
        "a refused edit records no undo step"
    );
}

#[test]
fn fillet_zero_radius_line_circle_extends_the_line_and_keeps_the_circle_whole() {
    // The line's segment lies wholly inside the radius-5 circle at the origin, so
    // the corner is the near crossing of its **infinite** line: (-5, 0). R = 0
    // extends the line to it and inserts nothing; the CIRCLE is never trimmed.
    let (mut app, mut session) = curve_app(
        line(point(-2.0, 0.0), point(2.0, 0.0)),
        circle(point(0.0, 0.0), 5.0),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let circle_before = db_entity(&app, 2);
    let rev = revision(&app);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        fillet(1, 2, point(0.0, 0.0), point(-3.5, 3.5)),
    );
    assert_eq!(outcome.objects.len(), 1, "only the LINE is trimmed");
    assert_eq!(revision(&app).0, rev.0 + 1, "exactly one revision bump");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (start, end) = line_ends(&app, 1);
    assert_point_near(start, point(-5.0, 0.0));
    assert_point_near(end, point(2.0, 0.0));
    let circle_after = db_entity(&app, 2);
    assert_eq!(
        circle_after.geometry, circle_before.geometry,
        "a CIRCLE is never trimmed"
    );
    assert_eq!(circle_after.object.revision, circle_before.object.revision);
}

#[test]
fn fillet_zero_radius_line_arc_trims_both_objects() {
    // The upper half circle (−Z span 90°..270°) crosses y = 0 at (−5, 0) only
    // inside its span. Both the line and the arc are trimmed to it.
    let (mut app, mut session) = curve_app(
        line(point(-8.0, 0.0), point(8.0, 0.0)),
        arc(
            point(0.0, 0.0),
            5.0,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        ),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let rev = revision(&app);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        // The line pick is on the right, the arc pick at 120° (before the corner).
        fillet(
            1,
            2,
            point(8.0, 0.0),
            point_at(point(0.0, 0.0), 5.0, 2.0 * std::f64::consts::FRAC_PI_3),
        ),
    );
    assert_eq!(outcome.objects.len(), 2, "both objects are trimmed");
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (start, end) = line_ends(&app, 1);
    assert_point_near(start, point(-5.0, 0.0));
    assert_point_near(end, point(8.0, 0.0));

    // The arc keeps the pick side: 90° → 180°.
    let (center, radius, arc_start, arc_sweep) = arc_data(&app, 2);
    assert_point_near(center, point(0.0, 0.0));
    assert!((radius - 5.0).abs() < 1e-9);
    assert_angle_near(arc_start, std::f64::consts::FRAC_PI_2);
    assert_angle_near(arc_start + arc_sweep, std::f64::consts::PI);
}

#[test]
fn fillet_zero_radius_line_ellipse_trims_the_line_and_the_ellipse() {
    // Ellipse a = 5, b = 2.5 about the origin meets y = 2 at x = ±3; the picks
    // select the right corner (3, 2) at eccentric angle atan2(0.8, 0.6).
    let (mut app, mut session) = curve_app(
        line(point(-10.0, 2.0), point(10.0, 2.0)),
        ellipse(point(0.0, 0.0), 5.0, 0.5, 0.0, std::f64::consts::TAU),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        fillet(1, 2, point(8.0, 2.0), point(3.535_533_9, 1.767_767)),
    );
    assert_eq!(outcome.objects.len(), 2, "both objects are trimmed");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (start, end) = line_ends(&app, 1);
    assert_point_near(start, point(3.0, 2.0));
    assert_point_near(end, point(10.0, 2.0));

    // The ellipse is cut at theta = atan2(2/2.5, 3/5); the pick at 45° is before
    // it, so the kept span is [0, theta] (an elliptical ARC, never re-approximated).
    let (ellipse_start, ellipse_sweep) = ellipse_span(&app, 2);
    assert_angle_near(ellipse_start, 0.0);
    assert_angle_near(ellipse_sweep, (0.8_f64).atan2(0.6));
}

#[test]
fn fillet_zero_radius_line_elliptical_arc_trims_inside_the_stored_span() {
    // The same ellipse stored as a 0°..180° elliptical arc: the corner (3, 2) is
    // inside the span, so both sides trim exactly like the full ellipse.
    let (mut app, mut session) = curve_app(
        line(point(-10.0, 2.0), point(10.0, 2.0)),
        ellipse(point(0.0, 0.0), 5.0, 0.5, 0.0, std::f64::consts::PI),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        fillet(1, 2, point(8.0, 2.0), point(3.535_533_9, 1.767_767)),
    );
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    let (start, end) = line_ends(&app, 1);
    assert_point_near(start, point(3.0, 2.0));
    assert_point_near(end, point(10.0, 2.0));
    let (ellipse_start, ellipse_sweep) = ellipse_span(&app, 2);
    assert_angle_near(ellipse_start, 0.0);
    assert_angle_near(ellipse_sweep, (0.8_f64).atan2(0.6));
}

#[test]
fn fillet_zero_radius_arc_arc_trims_both_at_the_shared_crossing() {
    // Radius-3 upper-half arcs centred (−2, 0) and (2, 0) meet at (0, √5).
    let (mut app, mut session) = curve_app(
        arc(point(-2.0, 0.0), 3.0, 0.0, std::f64::consts::PI),
        arc(point(2.0, 0.0), 3.0, 0.0, std::f64::consts::PI),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        fillet(
            1,
            2,
            point_at(point(-2.0, 0.0), 3.0, 0.9 * std::f64::consts::FRAC_PI_4),
            point_at(point(2.0, 0.0), 3.0, 2.6 * std::f64::consts::FRAC_PI_4),
        ),
    );
    assert_eq!(outcome.objects.len(), 2, "both arcs are trimmed");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let corner = point(0.0, 5.0_f64.sqrt());
    let (center, radius, start, sweep) = arc_data(&app, 1);
    assert_point_near(center, point(-2.0, 0.0));
    assert!((radius - 3.0).abs() < 1e-9);
    assert_point_near(point_at(center, radius, start), point(1.0, 0.0));
    assert_point_near(point_at(center, radius, start + sweep), corner);

    // The second arc's pick (117°) is before the corner (131.8°), so the kept
    // span runs from 0° to the corner.
    let (center, radius, start, sweep) = arc_data(&app, 2);
    assert_point_near(center, point(2.0, 0.0));
    assert!((radius - 3.0).abs() < 1e-9);
    assert_point_near(point_at(center, radius, start), point(5.0, 0.0));
    assert_point_near(point_at(center, radius, start + sweep), corner);
}

#[test]
fn fillet_zero_radius_circle_arc_trims_only_the_arc() {
    // Circle (0,0) r=5 and the left-half arc centred (6, 0) r = 4 meet where
    // x² + y² = 25 and (x − 6)² + y² = 16, i.e. x = 3.75, y = ±√10.9375. The upper
    // crossing lies on the picked 90°..270° span and is the one nearest the picks.
    let (mut app, mut session) = curve_app(
        circle(point(0.0, 0.0), 5.0),
        arc(
            point(6.0, 0.0),
            4.0,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        ),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let circle_before = db_entity(&app, 1);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        fillet(
            1,
            2,
            point(-3.5, 3.5),
            point_at(point(6.0, 0.0), 4.0, 2.0 * std::f64::consts::FRAC_PI_3),
        ),
    );
    assert_eq!(outcome.objects.len(), 1, "only the ARC is trimmed");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    let circle_after = db_entity(&app, 1);
    assert_eq!(circle_after.geometry, circle_before.geometry);

    // The arc keeps its pick side: the pick at 120° is before the corner (124.2°),
    // so the kept span runs from 90° to the corner, where the arc now ends.
    let (center, radius, start, sweep) = arc_data(&app, 2);
    assert_point_near(center, point(6.0, 0.0));
    assert!((radius - 4.0).abs() < 1e-9);
    assert_point_near(point_at(center, radius, start), point(6.0, 4.0));
    let corner = point(3.75, 10.9375_f64.sqrt());
    assert_point_near(point_at(center, radius, start + sweep), corner);
    assert!(on_circle(corner, point(0.0, 0.0), 5.0));
}

#[test]
fn fillet_zero_radius_ellipse_ellipse_trims_both_parametric_spans() {
    // Two a = 5, b = 2.5 ellipses centred (0,0) and (6,0) cross at (3, ±2); the
    // picks select (3, 2).
    let (mut app, mut session) = curve_app(
        ellipse(point(0.0, 0.0), 5.0, 0.5, 0.0, std::f64::consts::TAU),
        ellipse(point(6.0, 0.0), 5.0, 0.5, 0.0, std::f64::consts::TAU),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        fillet(1, 2, point(2.5, 2.165_063_8), point(3.5, 2.165_063_8)),
    );
    assert_eq!(outcome.objects.len(), 2, "both ellipses are trimmed");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // First ellipse: pick at 60° is after the corner (53.13°), so the kept span is
    // [corner, start + 2π].
    let (start, sweep) = ellipse_span(&app, 1);
    assert_angle_near(start, (0.8_f64).atan2(0.6));
    assert_angle_near(sweep, std::f64::consts::TAU - (0.8_f64).atan2(0.6));

    // Second ellipse: the corner is at 126.87°; the pick at 120° is before it, so
    // the kept span is [0, corner].
    let (start, sweep) = ellipse_span(&app, 2);
    assert_angle_near(start, 0.0);
    assert_close(sweep, std::f64::consts::PI - (0.8_f64).atan2(0.6));
}

#[test]
fn fillet_zero_radius_circle_ellipse_trims_the_ellipse_and_keeps_the_circle() {
    // Ellipse a = 5, b = 2.5 about the origin and circle (7, 0) r = 2.5 meet at
    // x = 5 (the ellipse's right vertex, theta = 0).
    let (mut app, mut session) = curve_app(
        ellipse(point(0.0, 0.0), 5.0, 0.5, 0.0, std::f64::consts::TAU),
        circle(point(7.0, 0.0), 2.5),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let circle_before = db_entity(&app, 2);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        // The ELLIPSE drives the scan; its pick at 30° is *after* the corner, so
        // the kept span runs from the corner round to the stored start.
        fillet(1, 2, point(4.330_127, 1.25), point(5.0, 2.5)),
    );
    assert_eq!(outcome.objects.len(), 1, "only the ELLIPSE is trimmed");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    let circle_after = db_entity(&app, 2);
    assert_eq!(
        circle_after.geometry, circle_before.geometry,
        "a CIRCLE is never trimmed"
    );

    // The ellipse and the circle meet at x = 14/3, y = ±√(6.25 − 49/9); the corner
    // is the upper one, at eccentric angle atan2(y/2.5, (14/3)/5).
    let corner_y = (6.25_f64 - (14.0_f64 / 3.0 - 7.0).powi(2)).sqrt();
    let corner_theta = (corner_y / 2.5).atan2((14.0_f64 / 3.0) / 5.0);
    let (start, sweep) = ellipse_span(&app, 1);
    assert_close(start, corner_theta);
    assert_close(sweep, std::f64::consts::TAU - corner_theta);
}

#[test]
fn fillet_zero_radius_arc_ellipse_trims_both() {
    // Ellipse a = 5, b = 2.5 about the origin and the left-half arc centred (6, 0)
    // with r = 4 meet at x = 2·(8 − √31)/… — solved here as 0.75x² − 12x + 26.25 = 0,
    // i.e. x = (16 − √116)/2 ≈ 2.615 with y = ±2.131. Both crossings are on the
    // picked 90°..270° span; the picks select the upper one.
    let (mut app, mut session) = curve_app(
        arc(
            point(6.0, 0.0),
            4.0,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        ),
        ellipse(point(0.0, 0.0), 5.0, 0.5, 0.0, std::f64::consts::TAU),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let outcome = run_fillet(
        &mut app,
        &mut session,
        fillet(
            1,
            2,
            point_at(point(6.0, 0.0), 4.0, 8.0 * std::f64::consts::PI / 9.0),
            point(4.330_127, 1.25),
        ),
    );
    assert_eq!(outcome.objects.len(), 2, "both objects are trimmed");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // The corner: x = 8 − √29 (the root of 0.75x² − 12x + 26.25 = 0), y = +2.131…
    let corner_x = 8.0 - 29.0_f64.sqrt();
    let corner_y = (6.25 - 0.25 * corner_x * corner_x).sqrt();
    let corner = point(corner_x, corner_y);
    let corner_theta = (corner_y / 2.5).atan2(corner_x / 5.0);

    // The ellipse pick (30°) is before the corner, so the kept span is [0, corner].
    let (start, sweep) = ellipse_span(&app, 2);
    assert_angle_near(start, 0.0);
    assert_close(sweep, corner_theta);

    // The arc pick (160°) is past the corner, so the kept span is [corner, 270°].
    let (center, radius, arc_start, arc_sweep) = arc_data(&app, 1);
    assert_point_near(center, point(6.0, 0.0));
    assert!((radius - 4.0).abs() < 1e-9);
    assert_point_near(point_at(center, radius, arc_start), corner);
    assert_angle_near(arc_start + arc_sweep, 1.5 * std::f64::consts::PI);
}

#[test]
fn fillet_zero_radius_circle_circle_cuts_both_into_pick_side_arcs() {
    // Circles (0,0) r=5 and (6,0) r=5 cross at (3, ±4). Two crossings per circle is
    // the two-cut case TRIM resolves for a CIRCLE target, so R = 0 rewrites **both**
    // circles as the pick-side arc between those crossings — in place, one
    // transaction, nothing inserted.
    let (mut app, mut session) =
        curve_app(circle(point(0.0, 0.0), 5.0), circle(point(6.0, 0.0), 5.0));
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let first_before = db_entity(&app, 1);
    let second_before = db_entity(&app, 2);
    let rev = revision(&app);
    // Pick A on its right side (36.9°), pick B on its left side (143.1°).
    let outcome = run_fillet(
        &mut app,
        &mut session,
        fillet(1, 2, point(4.0, 3.0), point(2.0, 3.0)),
    );
    assert_eq!(
        outcome.objects.len(),
        2,
        "both circles are cut, nothing inserted"
    );
    assert_eq!(
        outcome.changes.as_ref().unwrap().changes.len(),
        2,
        "two updates, no insert"
    );
    assert_eq!(revision(&app).0, rev.0 + 1, "exactly one revision bump");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // Arc A: centre (0,0), radius 5, from (3,−4) through (5,0) to (3,4).
    let (center, radius, start, sweep) = arc_data(&app, 1);
    assert_point_near(center, point(0.0, 0.0));
    assert!((radius - 5.0).abs() < 1e-9);
    assert_point_near(point_at(center, radius, start), point(3.0, -4.0));
    assert_point_near(point_at(center, radius, start + sweep), point(3.0, 4.0));
    assert_close(sweep, 2.0 * (4.0_f64).atan2(3.0));

    // Arc B: centre (6,0), radius 5, from (3,4) through (1,0) to (3,−4).
    let (center, radius, start, sweep) = arc_data(&app, 2);
    assert_point_near(center, point(6.0, 0.0));
    assert!((radius - 5.0).abs() < 1e-9);
    assert_point_near(point_at(center, radius, start), point(3.0, 4.0));
    assert_point_near(point_at(center, radius, start + sweep), point(3.0, -4.0));
    assert_close(sweep, 2.0 * (4.0_f64).atan2(3.0));

    // The kind change is in place: identity, layer, space, draw order and the
    // source type key are all the CIRCLE's own (exactly what TRIM keeps).
    for (id, before) in [(1u128, &first_before), (2u128, &second_before)] {
        let after = db_entity(&app, id);
        assert_eq!(after.id, before.id, "same entity id");
        assert_eq!(after.layer, before.layer, "same layer");
        assert_eq!(after.space, before.space, "same space");
        assert_eq!(after.draw_order, before.draw_order, "same draw order");
        assert_eq!(
            after.object.type_key, before.object.type_key,
            "the imported/source type key is kept, like TRIM"
        );
    }

    // One undo restores both CIRCLEs; REDO returns both ARCs.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(stored_geometry(&app, 1), first_before.geometry);
    assert_eq!(stored_geometry(&app, 2), second_before.geometry);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 0);

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    let (center, radius, start, sweep) = arc_data(&app, 1);
    assert_point_near(center, point(0.0, 0.0));
    assert!((radius - 5.0).abs() < 1e-9);
    assert_point_near(point_at(center, radius, start), point(3.0, -4.0));
    assert_point_near(point_at(center, radius, start + sweep), point(3.0, 4.0));
    let (center, radius, start, sweep) = arc_data(&app, 2);
    assert_point_near(center, point(6.0, 0.0));
    assert!((radius - 5.0).abs() < 1e-9);
    assert_point_near(point_at(center, radius, start), point(3.0, 4.0));
    assert_point_near(point_at(center, radius, start + sweep), point(3.0, -4.0));
}

#[test]
fn fillet_zero_radius_refusals_are_named_and_write_nothing() {
    // Tangent circles: exactly one crossing, so one cut point leaves both closed
    // circles whole and there is no corner to write — a named refusal that says so.
    let (mut app, mut session) =
        curve_app(circle(point(0.0, 0.0), 5.0), circle(point(10.0, 0.0), 5.0));
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(3.5, 3.5), point(6.5, 3.5)),
        "FILLET.tangent_circles",
    );

    // Disjoint circles: no crossing at all.
    let (mut app, mut session) =
        curve_app(circle(point(0.0, 0.0), 5.0), circle(point(20.0, 0.0), 5.0));
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(3.5, 3.5), point(16.5, 3.5)),
        "FILLET.no_corner_intersection",
    );

    // No intersection at all: the line y = 10 misses the radius-5 circle.
    let (mut app, mut session) = curve_app(
        line(point(-10.0, 10.0), point(10.0, 10.0)),
        circle(point(0.0, 0.0), 5.0),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(5.0, 10.0), point(3.5, 3.5)),
        "FILLET.no_corner_intersection",
    );

    // The carriers cross at (0, ±√5), but outside the picked arc spans: the outer
    // halves (left half of the first arc, right half of the second) never meet, so
    // there is no corner on the picked objects.
    let (mut app, mut session) = curve_app(
        arc(
            point(-2.0, 0.0),
            3.0,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        ),
        arc(
            point(2.0, 0.0),
            3.0,
            -std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        ),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(
            1,
            2,
            point_at(point(-2.0, 0.0), 3.0, 1.2 * std::f64::consts::PI),
            point_at(point(2.0, 0.0), 3.0, 0.4 * std::f64::consts::PI),
        ),
        "FILLET.no_corner_intersection",
    );

    // A pick exactly on the corner selects no side of the line.
    let (mut app, mut session) = curve_app(
        line(point(-8.0, 0.0), point(8.0, 0.0)),
        circle(point(0.0, 0.0), 5.0),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(5.0, 0.0), point(3.5, 3.5)),
        "FILLET.pick_at_tangent",
    );

    // R = 0 with trim off would create nothing.
    let (mut app, mut session) = curve_app(
        line(point(-8.0, 0.0), point(8.0, 0.0)),
        circle(point(0.0, 0.0), 5.0),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    session.drafting_extras.corner_modes_mut().set_trim(false);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(8.0, 0.0), point(3.5, 3.5)),
        "FILLET.trim_off_no_arc",
    );
}
