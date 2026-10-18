//! PLINE create-command contract tests (the phase-P3 open-ended draw template).
//!
//! Split out of `create_trim.rs` to keep both files under the 1000-line code
//! limit. The tests pin the honest subset: one open straight-segment polyline
//! per capture, one transaction and one undo step, and explicit refusals
//! (fewer than two points, all-identical points, non-finite points, a
//! mismatched programmatic payload, a missing active layer) that change nothing.

use super::*;

#[test]
fn create_polyline_inserts_one_open_polyline_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let captured = vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)];
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Points(captured.clone()),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "one entity for one capture");
    let oid = outcome.objects[0];
    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1);
    assert_eq!(cs.changes.len(), 1, "one transaction");
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
        .expect("new entity");
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points, &captured, "payload points inserted unchanged");
            assert_eq!(bulges.len(), captured.len(), "one bulge per captured point");
            assert!(bulges.iter().all(|bulge| *bulge == 0.0), "{bulges:?}");
            assert!(!*closed, "PLINE authors an open polyline");
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.object.type_key, "AcDbPolyline");
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // Undo removes the polyline exactly; redo restores it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
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
        .entity(EntityId(oid.0))
        .is_some());
}

#[test]
fn create_polyline_two_points_carry_one_bulge_each_and_stay_open() {
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 4.0)]),
    )
    .unwrap();
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .unwrap();
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points, &vec![point(0.0, 0.0), point(3.0, 4.0)]);
            assert_eq!(bulges, &vec![0.0, 0.0]);
            assert!(!*closed);
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
}

#[test]
fn create_polyline_refuses_degenerate_captures_without_change() {
    // Single point, all-identical points, and an empty capture are all explicit
    // refusals that leave the drawing, revision and history untouched.
    let cases = [
        vec![point(1.0, 1.0)],
        vec![point(2.0, 2.0), point(2.0, 2.0), point(2.0, 2.0)],
        Vec::new(),
    ];
    for points in cases {
        let (mut app, mut session) = drawing_app();
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::CreatePolyline,
            CommandPayload::Points(points.clone()),
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)), "{points:?}");
        assert_eq!(revision(&app), rev, "{points:?}");
        assert!(!app.can_undo(&DocumentId(1)), "{points:?}");
    }
}

#[test]
fn create_polyline_refuses_a_non_finite_point_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Points(vec![point(0.0, 0.0), point(f64::NAN, 1.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_polyline_on_a_missing_layer_is_refused_by_the_shared_funnel() {
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
        CommandId::CreatePolyline,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn create_polyline_rejects_a_wrong_geometry_payload() {
    // The programmatic `Geometry` arm accepts only a straight open Polyline; a
    // mismatched kind is refused instead of being coerced.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreatePolyline,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        })),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}
