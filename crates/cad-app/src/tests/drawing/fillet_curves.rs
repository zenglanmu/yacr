//! FILLET **curve↔curve** contracts (CIRCLE/ARC pairs, ADR 0016 FILLET).
//!
//! Circle↔circle, circle↔arc and arc↔arc share the offset-circle construction:
//! the fillet centre is an intersection of the two offset circles (external
//! `r + R` / internal `|r − R|`), the tangent point on each curve is the curve
//! point on the centre line, and one ARC of radius `R` joins them along the
//! **minor arc**: its signed sweep is the short way round (`|sweep| ≤ π`, with the
//! sign following the solution arrangement), not a forced counter-clockwise sweep.
//! Exact candidate-score ties keep the first candidate in loop order, and the
//! inserted arc's `start`/`start + sweep` are exactly the two tangent points. A
//! CIRCLE is never trimmed; ARC participants are trimmed at their tangent point on
//! the pick side. Every refused construction is named and changes nothing.

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

fn point_at(center: Point3, radius: f64, angle: f64) -> Point3 {
    point(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    )
}

fn on_circle(p: Point3, center: Point3, radius: f64) -> bool {
    let distance = ((p.x - center.x).powi(2) + (p.y - center.y).powi(2)).sqrt();
    (distance - radius).abs() < 1e-9
}

fn angle_of(center: Point3, p: Point3) -> f64 {
    (p.y - center.y).atan2(p.x - center.x)
}

fn assert_angle_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "angle {actual} != {expected}"
    );
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
fn fillet_circle_circle_inserts_one_tangent_arc_and_keeps_both_circles_whole() {
    let (mut app, mut session) =
        curve_app(circle(point(0.0, 0.0), 5.0), circle(point(10.0, 0.0), 5.0));
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    let first_before = db_entity(&app, 1);
    let second_before = db_entity(&app, 2);
    let rev = revision(&app);
    // The picks sit near the upper tangent points (45° on each circle).
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(
            1,
            2,
            point(
                5.0 / std::f64::consts::SQRT_2,
                5.0 / std::f64::consts::SQRT_2,
            ),
            point(
                10.0 - 5.0 / std::f64::consts::SQRT_2,
                5.0 / std::f64::consts::SQRT_2,
            ),
        ),
    )
    .expect("the circle↔circle fillet is supported");
    // Only the inserted ARC is written; one transaction and one undo step.
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 1);
    assert_eq!(outcome.objects.len(), 1, "only the inserted ARC");
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // Neither circle is trimmed: identical geometry and object revision.
    for (id, before) in [(1u128, &first_before), (2, &second_before)] {
        let after = db_entity(&app, id);
        assert_eq!(
            after.geometry, before.geometry,
            "circle {id} must stay whole"
        );
        assert_eq!(after.object.revision, before.object.revision);
    }

    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[0].0);
    assert!(
        (radius - 2.0).abs() < 1e-9,
        "the fillet radius is R: {radius}"
    );
    // External tangency: |centre − cᵢ| = rᵢ + R.
    assert!(on_circle(center, point(0.0, 0.0), 7.0), "{center:?}");
    assert!(on_circle(center, point(10.0, 0.0), 7.0), "{center:?}");
    // The pick side selects the upper solution.
    assert!(center.y > 0.0, "{center:?}");
    // Both tangent points lie on their circles and at distance R from the centre.
    let (p0, p1) = (
        point_at(center, radius, start),
        point_at(center, radius, start + sweep),
    );
    assert!(on_circle(p0, point(0.0, 0.0), 5.0), "{p0:?}");
    assert!(on_circle(p1, point(10.0, 0.0), 5.0), "{p1:?}");
    assert!(on_circle(p0, center, 2.0), "{p0:?}");
    assert!(on_circle(p1, center, 2.0), "{p1:?}");
    // The inserted arc is the **minor** arc between the tangent points: its span
    // is at most π and, for this arrangement, exactly the angle between the two
    // tangent radii: acos(-1/49) = 91.169° ≈ 1.59122 rad.
    let expected_span = std::f64::consts::FRAC_PI_2 + (1.0_f64 / 49.0).asin();
    assert!(
        sweep.abs() <= std::f64::consts::PI + 1e-12,
        "the inserted arc is the minor arc: {sweep}"
    );
    assert!(
        (sweep.abs() - expected_span).abs() < 1e-6,
        "span {} != {expected_span}",
        sweep.abs()
    );

    // UNDO removes the inserted ARC (one new revision); REDO re-adds it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .is_none());
    assert_eq!(revision(&app).0, rev.0 + 2, "UNDO is one further revision");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 0);

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    let redone = arc_data(&app, outcome.objects[0].0);
    assert_eq!(
        (redone.1, redone.2, redone.3),
        (radius, start, sweep),
        "REDO restores the same tangent ARC"
    );
    assert_eq!(revision(&app).0, rev.0 + 3);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn fillet_circle_arc_trims_the_arc_at_its_tangent_point_and_keeps_the_circle() {
    // The arc covers 90°..180° about (10,0); the fillet touches it at ~135.6°.
    let (mut app, mut session) = curve_app(
        circle(point(0.0, 0.0), 5.0),
        arc(
            point(10.0, 0.0),
            5.0,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::FRAC_PI_2,
        ),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    let circle_before = db_entity(&app, 1);
    let pick_first = point(
        5.0 / std::f64::consts::SQRT_2,
        5.0 / std::f64::consts::SQRT_2,
    );
    let pick_second = point_at(point(10.0, 0.0), 5.0, 140.0f64.to_radians());
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, pick_first, pick_second),
    )
    .expect("the circle↔arc fillet is supported");
    // One trimmed ARC + one inserted ARC; one transaction and one undo step.
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 2);
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // The CIRCLE is never trimmed.
    let circle_after = db_entity(&app, 1);
    assert_eq!(circle_after.geometry, circle_before.geometry);
    assert_eq!(circle_after.object.revision, circle_before.object.revision);

    // The inserted ARC: radius R, externally tangent to both curves.
    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[1].0);
    assert!((radius - 2.0).abs() < 1e-9, "{radius}");
    assert!(on_circle(center, point(0.0, 0.0), 7.0), "{center:?}");
    assert!(on_circle(center, point(10.0, 0.0), 7.0), "{center:?}");
    assert!(center.y > 0.0, "{center:?}");

    // The trimmed ARC keeps the portion away from the pick: its new start is the
    // tangent point at ~135.6° and its end stays the original 180° end.
    let expected_tangent = point(10.0 - 25.0 / 7.0, 5.0 * (24.0f64).sqrt() / 7.0);
    let (arc_center, arc_radius, arc_start, arc_sweep) = arc_data(&app, outcome.objects[0].0);
    assert!((arc_radius - 5.0).abs() < 1e-9);
    let tangent_angle = angle_of(arc_center, expected_tangent);
    assert_angle_near(arc_start, tangent_angle);
    assert_angle_near(arc_start + arc_sweep, std::f64::consts::PI);
    // The tangent point lies on both the source arc and the fillet arc.
    assert!(on_circle(expected_tangent, point(10.0, 0.0), 5.0));
    assert!(on_circle(expected_tangent, center, 2.0));

    // The inserted arc's own parameterisation reproduces both tangent points:
    // start is the circle tangent, start + sweep the arc tangent.
    assert_point_near(
        point_at(center, radius, start),
        point(25.0 / 7.0, 5.0 * (24.0f64).sqrt() / 7.0),
    );
    assert_point_near(point_at(center, radius, start + sweep), expected_tangent);
    assert!(
        sweep.abs() <= std::f64::consts::PI + 1e-12,
        "the inserted arc is the minor arc: {sweep}"
    );
}

#[test]
fn fillet_arc_arc_trims_both_arcs_and_inserts_one_arc() {
    let (mut app, mut session) = curve_app(
        arc(point(0.0, 0.0), 5.0, 0.0, std::f64::consts::FRAC_PI_2),
        arc(
            point(10.0, 0.0),
            5.0,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::FRAC_PI_2,
        ),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(
            1,
            2,
            point_at(point(0.0, 0.0), 5.0, 30.0f64.to_radians()),
            point_at(point(10.0, 0.0), 5.0, 150.0f64.to_radians()),
        ),
    )
    .expect("the arc↔arc fillet is supported");
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 3);
    assert_eq!(outcome.objects.len(), 3, "two trimmed arcs + one new arc");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[2].0);
    assert!((radius - 2.0).abs() < 1e-9, "{radius}");
    assert!(on_circle(center, point(0.0, 0.0), 7.0), "{center:?}");
    assert!(on_circle(center, point(10.0, 0.0), 7.0), "{center:?}");

    // Tangent points: ~44.4° on the first arc (0°..90°) and ~135.6° on the
    // second (90°..180°).
    let tangent_first = point(25.0 / 7.0, 5.0 * (24.0f64).sqrt() / 7.0);
    let tangent_second = point(10.0 - 25.0 / 7.0, 5.0 * (24.0f64).sqrt() / 7.0);

    // First arc: the pick (30°) is on the start side, so the tail is trimmed.
    let (c1, r1, start1, sweep1) = arc_data(&app, outcome.objects[0].0);
    assert!((r1 - 5.0).abs() < 1e-9);
    assert_angle_near(start1, 0.0);
    assert_angle_near(start1 + sweep1, angle_of(c1, tangent_first));
    // Second arc: the pick (150°) is beyond the tangent, so the head is trimmed.
    let (c2, r2, start2, sweep2) = arc_data(&app, outcome.objects[1].0);
    assert!((r2 - 5.0).abs() < 1e-9);
    assert_angle_near(start2, angle_of(c2, tangent_second));
    assert_angle_near(start2 + sweep2, std::f64::consts::PI);
    // Both tangent points sit on the inserted fillet arc.
    assert!(on_circle(tangent_first, center, 2.0));
    assert!(on_circle(tangent_second, center, 2.0));
    // ...and the inserted arc's start/start + sweep are exactly those points.
    assert_point_near(point_at(center, radius, start), tangent_first);
    assert_point_near(point_at(center, radius, start + sweep), tangent_second);
    assert!(
        sweep.abs() <= std::f64::consts::PI + 1e-12,
        "the inserted arc is the minor arc: {sweep}"
    );
}

#[test]
fn fillet_curve_refusals_are_named_and_write_nothing() {
    // Concentric circles: no tangent fillet circle exists.
    let (mut app, mut session) =
        curve_app(circle(point(0.0, 0.0), 5.0), circle(point(0.0, 0.0), 3.0));
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(3.5, 3.5), point(2.1, 2.1)),
        "concentric_curves",
    );

    // A radius with no real tangency: r = 1 circles 10 apart cannot both be
    // touched by a fillet of R = 3 (needs 4 + 4 or 2 + 2 ≤ 10).
    let (mut app, mut session) =
        curve_app(circle(point(0.0, 0.0), 1.0), circle(point(10.0, 0.0), 1.0));
    set_number(&mut session, ToolParameter::FilletRadius, 3.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(0.7, 0.7), point(9.3, 0.7)),
        "no_tangent_arc",
    );

    // Non-coplanar curves.
    let (mut app, mut session) = curve_app(
        circle(p3(0.0, 0.0, 0.0), 5.0),
        circle(p3(10.0, 0.0, 5.0), 5.0),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(3.5, 3.5), point(6.5, 3.5)),
        "not_coplanar",
    );

    // R = 0 has no sharp corner between two curves.
    let (mut app, mut session) =
        curve_app(circle(point(0.0, 0.0), 5.0), circle(point(10.0, 0.0), 5.0));
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    assert_refused(
        &mut app,
        &mut session,
        fillet(1, 2, point(3.5, 3.5), point(6.5, 3.5)),
        "radius_zero",
    );

    // A genuinely unsupported participant still names the pair.
    let (mut app, mut session) = curve_app(
        circle(point(0.0, 0.0), 5.0),
        SemanticGeometry::Ellipse {
            center: point(10.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    );
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    let rev = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(3.5, 3.5), point(6.5, 3.5)),
    )
    .unwrap_err();
    assert!(matches!(error, CadError::Unsupported(_)), "{error:?}");
    assert_eq!(revision(&app), rev);
    assert!(app
        .history
        .get(&DocumentId(1))
        .is_none_or(|h| h.undo_depth() == 0));
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
fn fillet_circle_circle_internal_offsets_insert_one_tangent_arc() {
    // r = 1 about (0,0) and r = 10 about (6,0) with R = 5: only the **internal**
    // offset branch (`d₁ = r₁ − R = −4`, `d₂ = r₂ − R = 5`) has intersections — the
    // external `r + R` offsets (6 and 15) are 6 apart, i.e. 9 > 6, so no external
    // circle exists. The two fillet centres are (9/4, ±5√7/4); the picks select
    // the upper one.
    let (mut app, mut session) =
        curve_app(circle(point(0.0, 0.0), 1.0), circle(point(6.0, 0.0), 10.0));
    set_number(&mut session, ToolParameter::FilletRadius, 5.0);
    let rev = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(-0.5625, -0.8268), point(-1.5, 6.6144)),
    )
    .expect("the internal-tangency curve↔curve fillet is supported");
    assert_eq!(outcome.objects.len(), 1, "only the inserted ARC");
    assert_eq!(revision(&app).0, rev.0 + 1);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (center, radius, start, sweep) = arc_data(&app, outcome.objects[0].0);
    assert!((radius - 5.0).abs() < 1e-9, "{radius}");
    // Exact declared tangency: the fillet encloses the small circle
    // (|centre − c₁| = R − r₁ = 4) and lies inside the large one
    // (|centre − c₂| = r₂ − R = 5).
    assert!(on_circle(center, point(0.0, 0.0), 4.0), "{center:?}");
    assert!(on_circle(center, point(6.0, 0.0), 5.0), "{center:?}");
    assert!(
        center.y > 0.0,
        "the picks select the upper solution: {center:?}"
    );

    // Both arc endpoints are the exact tangent points of the two circles.
    let tangent_first = point(-0.5625, -5.0 * 7.0f64.sqrt() / 16.0);
    let tangent_second = point(-1.5, 5.0 * 7.0f64.sqrt() / 2.0);
    let first = point_at(center, radius, start);
    let second = point_at(center, radius, start + sweep);
    assert_point_near(first, tangent_first);
    assert_point_near(second, tangent_second);
    assert!(on_circle(first, point(0.0, 0.0), 1.0), "{first:?}");
    assert!(on_circle(second, point(6.0, 0.0), 10.0), "{second:?}");
    assert!(
        sweep.abs() <= std::f64::consts::PI + 1e-12,
        "the inserted arc is the minor arc: {sweep}"
    );
}

#[test]
fn fillet_curve_refuses_a_non_positive_z_normal_by_name() {
    // A -Z ARC stores its angles in the -Z frame; reading them as +Z angles would
    // trim the wrong portion, so the pair is refused before any write.
    let mirrored_arc = SemanticGeometry::Arc {
        center: point(10.0, 0.0),
        normal: p3(0.0, 0.0, -1.0),
        radius: 5.0,
        start: std::f64::consts::FRAC_PI_2,
        sweep: std::f64::consts::FRAC_PI_2,
    };
    let (mut app, mut session) = curve_app(circle(point(0.0, 0.0), 5.0), mirrored_arc);
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    assert_unsupported(
        &mut app,
        &mut session,
        fillet(
            1,
            2,
            point(
                5.0 / std::f64::consts::SQRT_2,
                5.0 / std::f64::consts::SQRT_2,
            ),
            point_at(point(10.0, 0.0), 5.0, 140.0f64.to_radians()),
        ),
        "FILLET.unsupported_plane",
    );

    // The same check closes the pre-existing line↔circle hole for a tilted normal.
    let tilted_circle = SemanticGeometry::Circle {
        center: point(5.0, 5.0),
        normal: p3(0.0, 0.1, 0.994_987_437_106_62),
        radius: 2.0,
    };
    let (mut app, mut session) = curve_app(
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        tilted_circle,
    );
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    assert_unsupported(
        &mut app,
        &mut session,
        fillet(1, 2, point(5.0, 0.0), point(5.0, 3.0)),
        "FILLET.unsupported_plane",
    );
}

#[test]
fn fillet_curve_trim_off_still_refuses_a_tangent_outside_the_picked_arc() {
    // The second participant is an ARC covering only 0°..90°, while the two
    // external fillet tangents on its supporting circle are at ~135.6° and
    // ~224.4°: no candidate is feasible. With trim off nothing is trimmed, but
    // committing the tangent on the supporting circle outside the arc is still
    // the wrong fillet, so the named refusal must win.
    let (mut app, mut session) = curve_app(
        circle(point(0.0, 0.0), 5.0),
        arc(point(10.0, 0.0), 5.0, 0.0, std::f64::consts::FRAC_PI_2),
    );
    set_number(&mut session, ToolParameter::FilletRadius, 2.0);
    session.drafting_extras.corner_modes_mut().set_trim(false);
    assert_refused(
        &mut app,
        &mut session,
        fillet(
            1,
            2,
            point(25.0 / 7.0, -5.0 * (24.0f64).sqrt() / 7.0),
            point_at(point(10.0, 0.0), 5.0, 45.0f64.to_radians()),
        ),
        "FILLET.tangent_outside_arc",
    );
}
