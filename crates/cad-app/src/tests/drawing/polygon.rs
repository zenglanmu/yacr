//! POLYGON create-command contract tests (ADR 0015 decisions A/C, P3b).
//!
//! Split out of the RECTANG/ELLIPSE files to keep each under the 1000-line code
//! limit. The tests pin the shape that is not in dispute in the ADR: the two
//! picks (centre + radius point) and the **session** side count become **one
//! closed `n`-vertex LWPOLYLINE** (one zero bulge per vertex) in one transaction
//! and one undo step, with an explicit refusal (non-finite, zero radius,
//! non-coplanar, an out-of-range stored count, a mismatched payload, a missing
//! active layer) that changes nothing.

use super::*;

/// Assert the newest model-space entity is exactly a closed `sides`-vertex
/// straight polygon on layer 0 in model space.
fn assert_closed_polygon(app: &Application, object: ObjectId, sides: u32) -> EntityId {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the polygon entity");
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points.len(), sides as usize, "one vertex per side");
            assert_eq!(bulges.len(), points.len(), "one bulge per vertex");
            assert!(
                bulges.iter().all(|bulge| *bulge == 0.0),
                "POLYGON authors straight sides only: {bulges:?}"
            );
            assert!(*closed, "POLYGON authors a closed polyline");
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbPolyline");
    EntityId(object.0)
}

#[test]
fn create_polygon_inserts_one_closed_polyline_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    session.set_polygon_sides(6).unwrap();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygon,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one polygon");

    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(cs.changes.len(), 1, "one transaction");

    let entity_id = assert_closed_polygon(&app, outcome.objects[0], 6);
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // Undo removes the polygon exactly; redo restores it.
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
fn create_polygon_defaults_to_a_four_vertex_ring() {
    // The session sysvar default is 4 (AutoCAD `POLYSIDES` initial value).
    let (mut app, mut session) = drawing_app();
    assert_eq!(session.polygon_sides(), 4);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygon,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap();
    assert_closed_polygon(&app, outcome.objects[0], 4);
}

#[test]
fn create_polygon_refuses_a_degenerate_radius_without_change() {
    // The radius point coincides with the centre: no ring, nothing written.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygon,
        CommandPayload::Points(vec![point(3.0, 3.0), point(3.0, 3.0)]),
    )
    .unwrap_err();
    match err {
        CadError::InvalidInput(message) => {
            assert!(message.contains("radius"), "{message}")
        }
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev, "a degenerate polygon must not write");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_polygon_refuses_a_non_finite_pick_without_change() {
    let cases = [
        (point(f64::NAN, 0.0), point(1.0, 1.0)),
        (point(0.0, 0.0), point(f64::INFINITY, 1.0)),
    ];
    for (center, radius_point) in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreatePolygon,
            CommandPayload::Points(vec![center, radius_point]),
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)));
        assert_eq!(revision(&app), rev);
        assert!(!app.can_undo(&DocumentId(1)));
    }
}

#[test]
fn create_polygon_refuses_a_non_coplanar_radius_point_without_change() {
    // The flat `+Z` subset (ADR 0015 "NOT IMPLEMENTED: non-planar input") refuses
    // a radius point off the centre's plane instead of silently projecting it.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygon,
        CommandPayload::Points(vec![point(0.0, 0.0), p3(2.0, 0.0, 5.0)]),
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
fn create_polygon_refuses_a_missing_pick_and_a_wrong_payload() {
    // Fewer than two picks, and a non-`Points` payload, are explicit refusals
    // (POLYGON's payload is the construction input, like CIRCLE/RECTANG — never a
    // pre-built geometry).
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygon,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolygon,
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
fn create_polygon_on_a_missing_layer_is_refused_by_the_shared_funnel() {
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
        CommandId::CreatePolygon,
        CommandPayload::Points(vec![point(0.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}
