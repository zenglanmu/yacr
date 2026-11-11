//! `POLYGON CIRCUMSCRIBED` create-command contract tests.
//!
//! The construction is AutoCAD's `POLYGON` → `Circumscribed about circle` (`C`):
//! the second pick is a point on the **inscribed (tangent) circle**, `r` is that
//! distance, the vertex radius is `R = r / cos(π/n)`, and the picked direction is
//! the **midpoint of the first edge**. The pinned properties are: `n` vertices all
//! on `R`, every edge midpoint exactly `r` from the centre, counter-clockwise
//! winding, one closed zero-bulge LWPOLYLINE, one transaction / one undo step, and
//! refusals that change nothing.

use super::*;

/// The stored ring of the newest entity, as `(points, bulges, closed)`.
fn stored_ring(app: &Application, object: ObjectId) -> (Vec<Point3>, Vec<f64>, bool) {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the created polygon entity");
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbPolyline");
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points.clone(), bulges.clone(), *closed),
        other => panic!("expected a polyline, got {other:?}"),
    }
}

/// Twice the signed area of a closed ring (positive = counter-clockwise).
fn twice_signed_area(points: &[Point3]) -> f64 {
    let mut sum = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        sum += a.x * b.y - b.x * a.y;
    }
    sum
}

fn assert_circumscribed(points: &[Point3], center: Point3, tangent_radius: f64, sides: usize) {
    assert_eq!(points.len(), sides, "one vertex per side");
    let half_step = std::f64::consts::PI / sides as f64;
    let vertex_radius = tangent_radius / half_step.cos();
    for point in points {
        let radius = (point.x - center.x).hypot(point.y - center.y);
        assert!(
            (radius - vertex_radius).abs() <= 1e-9,
            "vertex radius {radius} != {vertex_radius}"
        );
        assert_eq!(point.z, center.z, "the ring stays on the centre's plane");
    }
    // Every edge's midpoint is the point where that edge touches the tangent
    // circle, so its distance from the centre is exactly `r`.
    for index in 0..sides {
        let a = points[index];
        let b = points[(index + 1) % sides];
        let mid = Point3 {
            x: (a.x + b.x) * 0.5,
            y: (a.y + b.y) * 0.5,
            z: (a.z + b.z) * 0.5,
        };
        let distance = (mid.x - center.x).hypot(mid.y - center.y);
        assert!(
            (distance - tangent_radius).abs() <= 1e-9,
            "edge {index} midpoint distance {distance} != tangent radius {tangent_radius}"
        );
    }
    assert!(
        twice_signed_area(points) > 0.0,
        "the circumscribed ring winds counter-clockwise about +Z"
    );
}

#[test]
fn create_polygon_circumscribed_inserts_one_closed_tangent_ring() {
    let (mut app, mut session) = drawing_app();
    session.set_polygon_sides(4).unwrap();
    let center = point(0.0, 0.0);
    let tangent_point = point(2.0, 0.0);
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygonCircumscribed,
        CommandPayload::Points(vec![center, tangent_point]),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);

    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(cs.changes.len(), 1, "one transaction");

    let (points, bulges, closed) = stored_ring(&app, outcome.objects[0]);
    assert!(closed, "the circumscribed polygon is a closed polyline");
    assert_eq!(bulges, vec![0.0; 4], "straight edges only");
    assert_circumscribed(&points, center, 2.0, 4);
    // The picked direction is the midpoint of the first edge: for a square whose
    // tangent circle is radius 2 about the origin and whose pick is `(2, 0)`, the
    // first edge is the vertical `x = 2` segment from `(2, −2)` to `(2, 2)`.
    assert!((points[0].x - 2.0).abs() <= 1e-9, "{:?}", points[0]);
    assert!((points[0].y + 2.0).abs() <= 1e-9, "{:?}", points[0]);
    assert!((points[1].x - 2.0).abs() <= 1e-9, "{:?}", points[1]);
    assert!((points[1].y - 2.0).abs() <= 1e-9, "{:?}", points[1]);

    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");
}

#[test]
fn create_polygon_circumscribed_holds_for_every_legal_side_count() {
    for sides in [3u32, 5, 7, 12, crate::draw_tool::MAX_POLYGON_SIDES] {
        let (mut app, mut session) = drawing_app();
        session.set_polygon_sides(sides).unwrap();
        let center = point(3.0, 4.0);
        let tangent_point = point(3.0 + 1.5, 4.0);
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::CreatePolygonCircumscribed,
            CommandPayload::Points(vec![center, tangent_point]),
        )
        .unwrap();
        let (points, bulges, closed) = stored_ring(&app, outcome.objects[0]);
        assert!(closed);
        assert_eq!(bulges.len(), sides as usize);
        assert_circumscribed(&points, center, 1.5, sides as usize);
    }
}

#[test]
fn circumscribed_and_inscribed_polygons_differ_for_the_same_picks() {
    // The same centre + picked point produce different rings: the inscribed
    // variant puts a **vertex** on the pick, the circumscribed variant puts the
    // **midpoint of an edge** there. This is why the two are separate commands.
    let (mut app, mut session) = drawing_app();
    session.set_polygon_sides(4).unwrap();
    let picks = CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]);
    let inscribed = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygon,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap();
    let circumscribed = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygonCircumscribed,
        picks,
    )
    .unwrap();
    let (inscribed_points, _, _) = stored_ring(&app, inscribed.objects[0]);
    let (circumscribed_points, _, _) = stored_ring(&app, circumscribed.objects[0]);
    assert_ne!(inscribed_points, circumscribed_points);
    // The inscribed ring's first vertex is the picked point itself.
    assert!((inscribed_points[0].x - 2.0).abs() <= 1e-9);
    assert!(inscribed_points[0].y.abs() <= 1e-9);
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 2);
}

#[test]
fn create_polygon_circumscribed_refuses_degenerate_input_without_change() {
    let cases = [
        // Coincident picks: no tangent radius.
        (vec![point(3.0, 3.0), point(3.0, 3.0)], "zero_radius"),
        // Non-finite picks.
        (vec![point(f64::NAN, 0.0), point(1.0, 0.0)], "invalid_point"),
        (
            vec![point(0.0, 0.0), point(f64::INFINITY, 0.0)],
            "invalid_point",
        ),
        // Not coplanar on one +Z plane.
        (vec![point(0.0, 0.0), p3(2.0, 0.0, 5.0)], "not_planar"),
        // Missing picks.
        (vec![point(0.0, 0.0)], "tangent-circle point"),
        (vec![], "center point"),
    ];
    for (points, reason) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreatePolygonCircumscribed,
            CommandPayload::Points(points),
        )
        .unwrap_err();
        let CadError::InvalidInput(message) = err else {
            panic!("expected an InvalidInput refusal, got {err:?}");
        };
        assert!(message.contains(reason), "expected {reason}, got {message}");
        assert_eq!(revision(&app), rev, "a refused polygon must not write");
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn circumscribed_helper_refuses_an_out_of_range_side_count() {
    // The session setter already refuses an out-of-range count, so this defensive
    // guard is the only reachable path for a corrupted stored sysvar.
    for sides in [0u32, 1, 2, crate::draw_tool::MAX_POLYGON_SIDES + 1] {
        let err = crate::draw_tool::circumscribed_polygon_vertices(
            point(0.0, 0.0),
            point(1.0, 0.0),
            sides,
        )
        .unwrap_err();
        let CadError::InvalidInput(message) = err else {
            panic!("expected an InvalidInput refusal, got {err:?}");
        };
        assert!(message.contains("polygoncircumscribed.sides"), "{message}");
    }
}

#[test]
fn create_polygon_circumscribed_refuses_a_wrong_payload_and_missing_layer() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygonCircumscribed,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));

    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygonCircumscribed,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_polygon_circumscribed_undo_removes_the_ring() {
    let (mut app, mut session) = drawing_app();
    session.set_polygon_sides(6).unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygonCircumscribed,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap();
    let entity_id = EntityId(outcome.objects[0].0);
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
fn create_polygon_circumscribed_is_work_only() {
    assert!(CommandId::CreatePolygonCircumscribed.requires_work_mode());
}

#[test]
fn create_polygon_circumscribed_refuses_a_third_pick_instead_of_dropping_it() {
    // The circumscribed construction is exactly two picks (centre + tangent-circle
    // point). A longer list used to be silently truncated into a tangent ring that
    // ignored the extra input.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygonCircumscribed,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 2.0)]),
    )
    .unwrap_err();
    match &err {
        CadError::InvalidInput(message) => assert!(
            message.starts_with("polygoncircumscribed.payload_length"),
            "the extra pick must be refused by name: {message}"
        ),
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}
