//! POINT create-command contract tests (ADR 0015 decision A/E, P3c).
//!
//! The single click becomes **one `SemanticGeometry::Point`** in one transaction
//! and one undo step, with explicit refusals (a non-finite position, a mismatched
//! payload, a missing active layer) that change nothing. A single point has no
//! coplanarity constraint, so the ADR's "non-coplanar `+Z`" refusal is vacuous
//! here; the last test documents that rather than fabricating a constraint.

use super::*;

/// Assert the newest model-space entity is exactly a POINT at `position` on layer
/// 0 in model space, and return its id.
fn assert_point(app: &Application, object: ObjectId, position: Point3) -> EntityId {
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(object.0))
        .expect("the point entity");
    match &entity.geometry {
        SemanticGeometry::Point(stored) => assert_eq!(*stored, position),
        other => panic!("expected a point, got {other:?}"),
    }
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbPoint");
    EntityId(object.0)
}

#[test]
fn create_point_inserts_one_entity_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let position = point(3.0, 4.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePoint,
        CommandPayload::Points(vec![position]),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one point");
    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1, "exactly one revision bump");
    assert_eq!(cs.changes.len(), 1, "one transaction");

    let entity_id = assert_point(&app, outcome.objects[0], position);
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1, "one undo step");

    // Undo removes the point exactly; redo restores it.
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
fn create_point_refuses_non_finite_and_wrong_payload_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    // A non-finite position.
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePoint,
        CommandPayload::Points(vec![point(f64::NAN, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    // An empty point list.
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePoint,
        CommandPayload::Points(Vec::new()),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    // More than one point (POINT places exactly one).
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePoint,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    // A pre-built geometry payload is refused: POINT's payload is the raw click
    // (like CIRCLE/RECTANG/POLYGON), never a second source of truth.
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePoint,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Point(point(1.0, 2.0)))),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(revision(&app), rev, "a refused point must not write");
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_point_on_a_missing_layer_is_refused_by_the_shared_funnel() {
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
        CommandId::CreatePoint,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_point_has_no_coplanarity_constraint_for_a_single_point() {
    // The ADR lists "non-coplanar `+Z`" among the create-command refusals, but a
    // single point is trivially coplanar (there is nothing to be non-coplanar
    // with). The handler therefore accepts a point off `z = 0` rather than
    // fabricating a constraint: only finiteness, payload shape and the active
    // layer are checked.
    let (mut app, mut session) = drawing_app();
    let position = p3(2.0, -1.0, 5.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePoint,
        CommandPayload::Points(vec![position]),
    )
    .unwrap();
    assert_point(&app, outcome.objects[0], position);
}
