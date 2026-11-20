//! FILLET **SPLINE** contracts.
//!
//! A SPLINE is a real FILLET participant: the fillet ARC is the numeric tangent
//! family of the spline (`corner_spline`, the same engine as the ellipse form),
//! and the trim is an **exact** B-spline sub-curve (`cad_geometry::sub_curve`), so
//! the kept piece ends exactly at the tangent point — no refit, no polyline.
//! `R = 0` is the sharp corner: trim both to their nearest intersection, insert
//! nothing. A pair that cannot be filleted (no tangency, no intersection, a
//! non-planar spline, a pick on the tangent point) is a **named** refusal that
//! writes nothing and records no undo step.
//!
//! The test splines are quadratic B-splines with the clamped-uniform shorthand
//! (`knots: []`), i.e. quadratic Béziers:
//!
//! * `arch()` — `P(t) = (2t, 4t − 4t²)`: a concave-down arch with its apex at
//!   `(1, 1)` and a horizontal tangent there (curvature radius 0.5);
//! * `bowl()` — `P(t) = (2t, 4 − 4t + 4t²)`: a concave-up bowl with its lowest
//!   point at `(1, 3)`, so `arch()` and `bowl()` face each other across a gap of 2.

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
            compat: Vec::new(),
            sources: Vec::new(),
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

/// A quadratic B-spline written with the clamped-uniform / unit-weight shorthand.
fn quadratic(control_points: [Point3; 3]) -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 2,
        knots: Vec::new(),
        control_points: control_points.to_vec(),
        weights: Vec::new(),
        closed: false,
    }
}

/// `P(t) = (2t, 4t − 4t²)`: apex `(1, 1)`, horizontal tangent, concave down.
fn arch() -> SemanticGeometry {
    quadratic([p3(0.0, 0.0, 0.0), p3(1.0, 2.0, 0.0), p3(2.0, 0.0, 0.0)])
}

/// `P(t) = (2t, 4 − 4t + 4t²)`: lowest point `(1, 3)`, horizontal tangent.
fn bowl() -> SemanticGeometry {
    quadratic([p3(0.0, 4.0, 0.0), p3(1.0, 2.0, 0.0), p3(2.0, 4.0, 0.0)])
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

/// `(centre, radius, start, sweep)` of a stored ARC.
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

/// The stored spline's degree and the two exact endpoints of its knot domain.
fn spline_data(app: &Application, entity: u128) -> (u32, Point3, Point3) {
    let geometry = stored_geometry(app, entity);
    let SemanticGeometry::Spline {
        degree,
        knots,
        control_points,
        weights,
        ..
    } = geometry
    else {
        panic!("expected a SPLINE, got {geometry:?}");
    };
    let curve = cad_geometry::expand_spline_source(degree, &knots, &control_points, &weights)
        .expect("the stored spline is a valid B-spline");
    let (t_min, t_max) = curve.domain();
    (degree, curve.evaluate(t_min), curve.evaluate(t_max))
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

/// The distance from `p` to the circle `center`/`radius` (2D).
fn on_circle(p: Point3, center: Point3, radius: f64) -> bool {
    let distance = ((p.x - center.x).powi(2) + (p.y - center.y).powi(2)).sqrt();
    (distance - radius).abs() < 1e-7
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

/// Run a FILLET payload that must be refused as an unsupported capability by name.
fn assert_unsupported(
    app: &mut Application,
    session: &mut SessionState,
    payload: CommandPayload,
    expected_code: &str,
) {
    let rev = revision(app);
    let error = execute(app, session, CommandId::FilletLines, payload).unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert!(
        error.to_string().contains(expected_code),
        "{error} must name {expected_code}"
    );
    assert_eq!(revision(app), rev, "a refused edit changes nothing");
    assert!(
        app.history
            .get(&DocumentId(1))
            .is_none_or(|history| history.undo_depth() == 0),
        "a refused edit records no undo step"
    );
}

#[test]
fn fillet_spline_line_trims_the_spline_exactly_and_inserts_one_tangent_arc() {
    // The arch's apex carries a +Y normal, so a radius-1 fillet centred at (1, 2)
    // is tangent at the apex (1, 1) and at (1, 3) on the line y = 3.
    let (mut app, mut session) = curve_app(arch(), line(point(-2.0, 3.0), point(4.0, 3.0)));
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    let rev = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        // The spline pick sits left of the apex; the line pick right of its tangent.
        fillet(1, 2, point(0.5, 0.75), point(3.0, 3.0)),
    )
    .expect("a spline↔line fillet is supported");
    assert_eq!(
        outcome.objects.len(),
        3,
        "trimmed spline + trimmed line + the inserted ARC"
    );
    assert_eq!(revision(&app).0, rev.0 + 1, "exactly one revision bump");
    assert_eq!(
        app.history[&DocumentId(1)].undo_depth(),
        1,
        "one fillet is one undo step"
    );

    // The inserted ARC: radius R, centred (1, 2), endpoints the two tangent points.
    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[2].0);
    assert_point_near(center, point(1.0, 2.0));
    assert!((radius - 1.0).abs() < 1e-9, "{radius}");
    assert_point_near(point_at(center, radius, start), point(1.0, 1.0));
    assert_point_near(point_at(center, radius, start + sweep), point(1.0, 3.0));

    // The spline is trimmed **exactly**: the kept piece ends at the tangent
    // parameter, so its domain end *is* (1, 1) — an exact sub-curve, same degree.
    let (degree, _, spline_end) = spline_data(&app, 1);
    assert_eq!(degree, 2, "the sub-curve keeps the source degree");
    assert_point_near(spline_end, point(1.0, 1.0));

    // The line keeps its pick side and starts at the tangent point.
    let SemanticGeometry::Line { start, end } = stored_geometry(&app, 2) else {
        panic!("expected the trimmed LINE");
    };
    assert_point_near(start, point(1.0, 3.0));
    assert_point_near(end, point(4.0, 3.0));
}

#[test]
fn fillet_spline_circle_keeps_the_circle_whole_and_trims_the_spline() {
    // The circle (1, 4) r = 1: the radius-1 fillet at (1, 2) is tangent to the
    // arch's apex (1, 1) and to the circle's bottom (1, 3).
    let (mut app, mut session) = curve_app(arch(), circle(point(1.0, 4.0), 1.0));
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    let circle_before = db_entity(&app, 2);
    let rev = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(0.5, 0.75), point(1.0, 3.0)),
    )
    .expect("a spline↔circle fillet is supported");
    assert_eq!(
        outcome.objects.len(),
        2,
        "trimmed spline + the inserted ARC"
    );
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let circle_after = db_entity(&app, 2);
    assert_eq!(
        circle_after.geometry, circle_before.geometry,
        "a CIRCLE participant is never trimmed"
    );
    assert_eq!(circle_after.object.revision, circle_before.object.revision);

    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[1].0);
    assert_point_near(center, point(1.0, 2.0));
    assert!((radius - 1.0).abs() < 1e-9, "{radius}");
    assert_point_near(point_at(center, radius, start), point(1.0, 1.0));
    assert_point_near(point_at(center, radius, start + sweep), point(1.0, 3.0));
    assert!(on_circle(point(1.0, 3.0), point(1.0, 4.0), 1.0));

    let (_, _, spline_end) = spline_data(&app, 1);
    assert_point_near(spline_end, point(1.0, 1.0));
}

#[test]
fn fillet_spline_arc_trims_the_arc_at_its_tangent_point() {
    // The half circle (1, 4) r = 1 from 180° to 360° covers the bottom point
    // (1, 3), where the radius-1 fillet touches it.
    let (mut app, mut session) = curve_app(
        arch(),
        arc(
            point(1.0, 4.0),
            1.0,
            std::f64::consts::PI,
            std::f64::consts::PI,
        ),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(
            1,
            2,
            point(0.5, 0.75),
            point_at(point(1.0, 4.0), 1.0, 1.25 * std::f64::consts::PI),
        ),
    )
    .expect("a spline↔arc fillet is supported");
    assert_eq!(outcome.objects.len(), 3, "both trims + the inserted ARC");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[2].0);
    assert_point_near(center, point(1.0, 2.0));
    assert!((radius - 1.0).abs() < 1e-9, "{radius}");
    assert_point_near(point_at(center, radius, start), point(1.0, 1.0));
    assert_point_near(point_at(center, radius, start + sweep), point(1.0, 3.0));

    // The pick (225°) is on the start side of the tangent (270°), so the tail of
    // the picked arc is trimmed: 180° → 270°.
    let (arc_center, arc_radius, arc_start, arc_sweep) = arc_data(&app, outcome.objects[1].0);
    assert_point_near(arc_center, point(1.0, 4.0));
    assert!((arc_radius - 1.0).abs() < 1e-9);
    assert_angle_near(arc_start, std::f64::consts::PI);
    assert_angle_near(arc_start + arc_sweep, 1.5 * std::f64::consts::PI);

    let (_, _, spline_end) = spline_data(&app, 1);
    assert_point_near(spline_end, point(1.0, 1.0));
}

#[test]
fn fillet_two_splines_trims_both_exactly_and_inserts_the_tangent_arc() {
    // `arch()` (apex (1,1), normal +Y) and `bowl()` (lowest point (1,3), normal
    // -Y): the radius-1 fillet at (1, 2) touches both.
    let (mut app, mut session) = curve_app(arch(), bowl());
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    let rev = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(0.5, 0.75), point(0.5, 3.25)),
    )
    .expect("a spline↔spline fillet is supported");
    assert_eq!(outcome.objects.len(), 3, "both trims + the inserted ARC");
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[2].0);
    assert_point_near(center, point(1.0, 2.0));
    assert!((radius - 1.0).abs() < 1e-9, "{radius}");
    assert_point_near(point_at(center, radius, start), point(1.0, 1.0));
    assert_point_near(point_at(center, radius, start + sweep), point(1.0, 3.0));

    let (_, arch_start, arch_end) = spline_data(&app, 1);
    assert_point_near(arch_end, point(1.0, 1.0));
    assert_point_near(arch_start, point(0.0, 0.0));
    let (_, bowl_start, bowl_end) = spline_data(&app, 2);
    assert_point_near(bowl_start, point(0.0, 4.0));
    assert_point_near(bowl_end, point(1.0, 3.0));
}

#[test]
fn fillet_spline_zero_radius_trims_to_the_sharp_corner() {
    // The arch crosses y = 0.5 at x = 1 ± √0.5. The picks select the left
    // intersection (0.29289…, 0.5); R = 0 trims both objects there and creates no
    // ARC — the sharp corner.
    let (mut app, mut session) = curve_app(arch(), line(point(0.0, 0.5), point(2.0, 0.5)));
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let rev = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(0.5, 0.75), point(0.5, 0.5)),
    )
    .expect("the R=0 sharp corner of a spline pair is supported");
    assert_eq!(
        outcome.objects.len(),
        2,
        "only the two trimmed objects, no joining ARC"
    );
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let corner = point(1.0 - 0.5_f64.sqrt(), 0.5);
    // The spline keeps its pick side: [t_corner, domain end] starts at the corner.
    let (_, spline_start, spline_end) = spline_data(&app, 1);
    assert_point_near(spline_start, corner);
    assert_point_near(spline_end, point(2.0, 0.0));
    let SemanticGeometry::Line { start, end } = stored_geometry(&app, 2) else {
        panic!("expected the trimmed LINE");
    };
    assert_point_near(start, corner);
    assert_point_near(end, point(2.0, 0.5));
}

#[test]
fn fillet_spline_trim_off_leaves_both_objects_untouched() {
    let (mut app, mut session) = curve_app(arch(), line(point(-2.0, 3.0), point(4.0, 3.0)));
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    session.drafting_extras.corner_modes_mut().set_trim(false);
    let spline_before = db_entity(&app, 1);
    let line_before = db_entity(&app, 2);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(0.5, 0.75), point(3.0, 3.0)),
    )
    .expect("trim-off spline fillet is supported");
    assert_eq!(outcome.objects.len(), 1, "only the joining ARC is created");
    assert_eq!(db_entity(&app, 1).geometry, spline_before.geometry);
    assert_eq!(db_entity(&app, 2).geometry, line_before.geometry);
    assert_eq!(
        db_entity(&app, 1).object.revision,
        spline_before.object.revision
    );
    assert_eq!(
        db_entity(&app, 2).object.revision,
        line_before.object.revision
    );
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn fillet_spline_ellipse_trims_both_profiled_curves() {
    // The ellipse a = 1, b = 0.5 about (1, 4) reaches (1, 3.5) at its lowest
    // point. The radius-1.25 fillet centred (1, 2.25) is tangent to the arch apex
    // (1, 1) and to that ellipse point.
    let (mut app, mut session) = curve_app(
        arch(),
        SemanticGeometry::Ellipse {
            center: point(1.0, 4.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(1.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.25);
    let rev = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        // The ellipse pick is just off its lowest point along the sweep.
        fillet(1, 2, point(0.5, 0.75), point(1.2, 3.455)),
    )
    .expect("a spline↔ellipse fillet is supported");
    assert_eq!(outcome.objects.len(), 3, "both trims + the inserted ARC");
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[2].0);
    assert_point_near(center, point(1.0, 2.25));
    assert!((radius - 1.25).abs() < 1e-9, "{radius}");
    assert_point_near(point_at(center, radius, start), point(1.0, 1.0));
    assert_point_near(point_at(center, radius, start + sweep), point(1.0, 3.5));

    let (_, _, spline_end) = spline_data(&app, 1);
    assert_point_near(spline_end, point(1.0, 1.0));
}

#[test]
fn fillet_two_splines_zero_radius_trims_both_to_their_intersection() {
    // A degree-1 spline is a straight segment, so this is the spline↔spline branch
    // of the sharp corner: the two splines cross at x = 1 ± √0.5 (numeric
    // intersection, accepted within the declared 1e-7 relative tolerance).
    let flat = SemanticGeometry::Spline {
        degree: 1,
        knots: Vec::new(),
        control_points: vec![p3(0.0, 0.5, 0.0), p3(2.0, 0.5, 0.0)],
        weights: Vec::new(),
        closed: false,
    };
    let (mut app, mut session) = curve_app(arch(), flat);
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let rev = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(0.5, 0.75), point(0.5, 0.5)),
    )
    .expect("the R=0 sharp corner of two splines is supported");
    assert_eq!(outcome.objects.len(), 2, "only the two trimmed splines");
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let corner = point(1.0 - 0.5_f64.sqrt(), 0.5);
    let (_, arch_start, arch_end) = spline_data(&app, 1);
    for (label, actual, expected) in [
        ("arch start", arch_start, corner),
        ("arch end", arch_end, point(2.0, 0.0)),
    ] {
        assert!(
            (actual.x - expected.x).abs() < 1e-6
                && (actual.y - expected.y).abs() < 1e-6
                && (actual.z - expected.z).abs() < 1e-6,
            "{label} ({}, {}) != ({}, {})",
            actual.x,
            actual.y,
            expected.x,
            expected.y
        );
    }
    let (degree, flat_start, flat_end) = spline_data(&app, 2);
    assert_eq!(degree, 1, "the sub-curve keeps the source degree");
    for (label, actual, expected) in [
        ("flat start", flat_start, corner),
        ("flat end", flat_end, point(2.0, 0.5)),
    ] {
        assert!(
            (actual.x - expected.x).abs() < 1e-6
                && (actual.y - expected.y).abs() < 1e-6
                && (actual.z - expected.z).abs() < 1e-6,
            "{label} ({}, {}) != ({}, {})",
            actual.x,
            actual.y,
            expected.x,
            expected.y
        );
    }
}

#[test]
fn fillet_spline_circle_zero_radius_trims_only_the_spline_to_the_intersection() {
    // Circle (1, 0.75) r = 1 crosses the arch at x = 1 ± √0.75 (y = 0.25). R = 0
    // trims to the left crossing and never trims the CIRCLE.
    let (mut app, mut session) = curve_app(arch(), circle(point(1.0, 0.75), 1.0));
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let circle_before = db_entity(&app, 2);
    let rev = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(0.5, 0.75), point(0.0, 0.75)),
    )
    .expect("the R=0 sharp corner of a spline↔circle pair is supported");
    assert_eq!(outcome.objects.len(), 1, "only the spline is trimmed");
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let circle_after = db_entity(&app, 2);
    assert_eq!(circle_after.geometry, circle_before.geometry);
    assert_eq!(circle_after.object.revision, circle_before.object.revision);

    let corner = point(1.0 - 0.75_f64.sqrt(), 0.25);
    let (_, spline_start, spline_end) = spline_data(&app, 1);
    for (label, actual, expected) in [
        ("spline start", spline_start, corner),
        ("spline end", spline_end, point(2.0, 0.0)),
    ] {
        assert!(
            (actual.x - expected.x).abs() < 1e-6
                && (actual.y - expected.y).abs() < 1e-6
                && (actual.z - expected.z).abs() < 1e-6,
            "{label} ({}, {}) != ({}, {})",
            actual.x,
            actual.y,
            expected.x,
            expected.y
        );
    }
}

#[test]
fn fillet_spline_ellipse_zero_radius_trims_both_to_the_intersection() {
    // The straight degree-1 spline y = 0.5 crosses the ellipse a = 1, b = 0.5 about
    // (1, 0.25) at x = 1 ± √0.75. R = 0 trims both (the ellipse keeps its pick side
    // [0, 150°]) and inserts nothing.
    let flat = SemanticGeometry::Spline {
        degree: 1,
        knots: Vec::new(),
        control_points: vec![p3(0.0, 0.5, 0.0), p3(2.0, 0.5, 0.0)],
        weights: Vec::new(),
        closed: false,
    };
    let (mut app, mut session) = curve_app(
        flat,
        SemanticGeometry::Ellipse {
            center: point(1.0, 0.25),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(1.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    );
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(0.5, 0.5), point(0.5, 0.683_012_7)),
    )
    .expect("the R=0 sharp corner of a spline↔ellipse pair is supported");
    assert_eq!(outcome.objects.len(), 2, "both trims, no joining ARC");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let corner = point(1.0 - 0.75_f64.sqrt(), 0.5);
    let (_, spline_start, spline_end) = spline_data(&app, 1);
    assert!((spline_start.x - corner.x).abs() < 1e-6);
    assert!((spline_start.y - corner.y).abs() < 1e-6);
    assert_point_near(spline_end, point(2.0, 0.5));

    let SemanticGeometry::Ellipse { start, sweep, .. } = stored_geometry(&app, 2) else {
        panic!("expected the trimmed ELLIPSE");
    };
    assert_angle_near(start, 0.0);
    assert_angle_near(sweep, 5.0 * std::f64::consts::PI / 6.0);
}

#[test]
fn fillet_spline_refusals_are_named_and_write_nothing() {
    // A spline whose control polygon is not planar has no +Z construction.
    let non_planar = quadratic([p3(0.0, 0.0, 0.0), p3(1.0, 2.0, 0.5), p3(2.0, 0.0, 0.0)]);
    let (mut app, mut session) = curve_app(non_planar, line(point(-2.0, 3.0), point(4.0, 3.0)));
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    assert_unsupported(
        &mut app,
        &mut session,
        fillet(1, 2, point(0.5, 0.75), point(3.0, 3.0)),
        "FILLET.unsupported_plane",
    );

    // No radius of 0.1 can touch both: the arch's apex is 2 away from the line.
    let (mut app, mut session) = curve_app(arch(), line(point(-2.0, 3.0), point(4.0, 3.0)));
    set_number(&mut session, ToolParameter::FilletRadius, 0.1);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(0.5, 0.75), point(3.0, 3.0)),
        "FILLET.no_tangent_arc",
    );

    // A pick on the tangent point selects neither side of the spline trim.
    let (mut app, mut session) = curve_app(arch(), line(point(-2.0, 3.0), point(4.0, 3.0)));
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(1.0, 1.0), point(3.0, 3.0)),
        "FILLET.pick_at_tangent",
    );

    // R = 0 needs a real intersection: the arch (max y = 1) never meets this circle.
    let (mut app, mut session) = curve_app(arch(), circle(point(1.0, 4.0), 1.0));
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(0.5, 0.75), point(1.0, 3.0)),
        "FILLET.no_corner_intersection",
    );

    // R = 0 with trim off would create nothing, so it is refused by name.
    let (mut app, mut session) = curve_app(arch(), line(point(0.0, 0.5), point(2.0, 0.5)));
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    session.drafting_extras.corner_modes_mut().set_trim(false);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(0.5, 0.75), point(0.5, 0.5)),
        "FILLET.trim_off_no_arc",
    );

    // A participant that is not a curve at all still names the pair: a 2D
    // polyline is the separate FILLET POLYLINE command.
    let (mut app, mut session) = curve_app(
        arch(),
        SemanticGeometry::Polyline {
            points: vec![
                point(0.0, 0.0),
                point(2.0, 0.0),
                point(2.0, 2.0),
                point(0.0, 2.0),
            ],
            bulges: vec![0.0; 4],
            closed: true,
            width: 0.0,
        },
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    assert_unsupported(
        &mut app,
        &mut session,
        fillet(1, 2, point(0.5, 0.75), point(2.0, 1.0)),
        "FILLET.unsupported_pair",
    );
}
