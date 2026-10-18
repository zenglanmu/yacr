//! RECTANG create-command contract tests (ADR 0015 decision A, P3a).
//!
//! Split out of the PLINE file to keep both under the 1000-line code limit. The
//! tests pin the shape that is not in dispute in the ADR: two opposite corners
//! become **one closed four-vertex LWPOLYLINE** (one zero bulge per vertex) in one
//! transaction and one undo step, with an explicit refusal (non-finite,
//! degenerate, non-coplanar corners, a mismatched payload, a missing active layer)
//! that changes nothing.

use super::*;

/// The canonical four-vertex outline a RECTANG commit stores, as a closed
/// straight-segment polyline (see `rectangle_geometry`).
fn expected_outline(corner1: Point3, corner2: Point3) -> Vec<Point3> {
    let min_x = corner1.x.min(corner2.x);
    let max_x = corner1.x.max(corner2.x);
    let min_y = corner1.y.min(corner2.y);
    let max_y = corner1.y.max(corner2.y);
    vec![
        point(min_x, min_y),
        point(max_x, min_y),
        point(max_x, max_y),
        point(min_x, max_y),
    ]
}

/// Assert the newest model-space entity is exactly the expected closed rectangle.
fn assert_closed_rectangle(
    app: &Application,
    object: ObjectId,
    corner1: Point3,
    corner2: Point3,
) -> EntityId {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the rectangle entity");
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points, &expected_outline(corner1, corner2));
            assert_eq!(bulges.len(), 4, "one bulge per vertex");
            assert!(
                bulges.iter().all(|bulge| *bulge == 0.0),
                "RECTANG authors straight segments only: {bulges:?}"
            );
            assert!(*closed, "RECTANG authors a closed polyline");
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbPolyline");
    EntityId(object.0)
}

#[test]
fn create_rectangle_inserts_one_closed_polyline_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 3.0)]),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one rectangle");

    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(cs.changes.len(), 1, "one transaction");

    let oid = outcome.objects[0];
    let entity_id = assert_closed_rectangle(&app, oid, point(0.0, 0.0), point(4.0, 3.0));
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // Undo removes the rectangle exactly; redo restores it.
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
fn create_rectangle_is_identical_for_both_click_diagonals() {
    // The two corners do not define a winding: whichever diagonal the user clicks,
    // the stored outline is the same canonical closed rectangle (lower-left first,
    // counter-clockwise). This is the corner-ordering rule under test.
    let outlines: Vec<Vec<Point3>> = [
        (point(-2.0, 1.0), point(5.0, 7.0)),
        (point(5.0, 7.0), point(-2.0, 1.0)),
        (point(5.0, 1.0), point(-2.0, 7.0)),
        (point(-2.0, 7.0), point(5.0, 1.0)),
    ]
    .into_iter()
    .map(|(first, second)| {
        let (mut app, mut session) = drawing_app();
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::CreateRectangle,
            CommandPayload::Points(vec![first, second]),
        )
        .unwrap();
        let entity = app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(outcome.objects[0].0))
            .unwrap();
        match &entity.geometry {
            SemanticGeometry::Polyline { points, closed, .. } => {
                assert!(*closed);
                points.clone()
            }
            other => panic!("expected a polyline, got {other:?}"),
        }
    })
    .collect();
    let expected = vec![
        point(-2.0, 1.0),
        point(5.0, 1.0),
        point(5.0, 7.0),
        point(-2.0, 7.0),
    ];
    for outline in &outlines {
        assert_eq!(outline, &expected, "both diagonals must store one outline");
    }
}

#[test]
fn create_rectangle_refuses_a_degenerate_span_without_change() {
    // A click-click on the same vertical or horizontal axis line has no area and
    // must create nothing (never a segment or a point).
    let cases = [
        (point(3.0, 0.0), point(3.0, 9.0)), // zero width
        (point(0.0, 3.0), point(9.0, 3.0)), // zero height
        (point(3.0, 3.0), point(3.0, 3.0)), // both
    ];
    for (first, second) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateRectangle,
            CommandPayload::Points(vec![first, second]),
        )
        .unwrap_err();
        match err {
            CadError::InvalidInput(message) => assert!(
                message.contains("distinct"),
                "{first:?}/{second:?}: refusals must name the degenerate axis: {message}"
            ),
            other => panic!("expected an InvalidInput refusal, got {other:?}"),
        }
        assert_eq!(revision(&app), rev, "degenerate rectangle must not write");
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn create_rectangle_refuses_a_non_finite_corner_without_change() {
    let cases = [
        (point(f64::NAN, 0.0), point(1.0, 1.0)),
        (point(0.0, 0.0), point(f64::INFINITY, 1.0)),
    ];
    for (first, second) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreateRectangle,
            CommandPayload::Points(vec![first, second]),
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)));
        assert_eq!(revision(&app), rev);
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn create_rectangle_refuses_non_coplanar_corners_without_change() {
    // The flat `+Z` subset (ADR 0015 "NOT IMPLEMENTED: non-planar input") refuses
    // a corner pair on two different planes instead of silently projecting one.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), p3(4.0, 3.0, 5.0)]),
    )
    .unwrap_err();
    match err {
        CadError::InvalidInput(message) => assert!(message.contains("+Z"), "{message}"),
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_rectangle_refuses_a_missing_corner_and_a_wrong_payload() {
    // Fewer than two corners, and a non-`Points` payload, are explicit refusals
    // (RECTANG's payload is the construction input, like CIRCLE — never a
    // pre-built geometry).
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateRectangle,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Polyline {
            points: vec![
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(1.0, 1.0),
                point(0.0, 1.0),
            ],
            bulges: vec![0.0; 4],
            closed: true,
        })),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_rectangle_on_a_missing_layer_is_refused_by_the_shared_funnel() {
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
        CommandId::CreateRectangle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(4.0, 3.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}
