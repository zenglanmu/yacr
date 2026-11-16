//! Edit policy around the CHAMFER/FILLET commands: the layer rule for the new
//! entity, the TRIM=OFF form, one undo step per invocation and the WORK-only
//! mode gate.
//!
//! Split out of `tests/drawing/chamfer_fillet.rs` to keep that file under the
//! code-size limit.

use super::*;

#[test]
fn fillet_and_chamfer_new_entity_follows_the_declared_layer_rule() {
    // The new entity goes on the **active layer** (the ADR records this as our
    // choice); the two updated lines keep their own layer.
    let (mut app, mut session) = l_corner();
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(5)),
    )
    .unwrap();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    let line_id = outcome.objects[2];
    let inserted = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(line_id.0))
        .unwrap();
    assert_eq!(
        inserted.layer,
        LayerId(5),
        "the new LINE follows the active layer"
    );
    assert_eq!(inserted.space, SpaceId::Model);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .unwrap()
            .layer,
        LayerId(0),
        "the trimmed source keeps its own layer"
    );

    let (mut app, mut session) = l_corner();
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(5)),
    )
    .unwrap();
    set_number(&mut session, ToolParameter::FilletRadius, 1.0);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    let arc_id = outcome.objects[2];
    let inserted = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(arc_id.0))
        .unwrap();
    assert_eq!(
        inserted.layer,
        LayerId(5),
        "the new ARC follows the active layer"
    );
    assert_eq!(inserted.space, SpaceId::Model);
}

#[test]
fn trim_off_leaves_both_objects_and_creates_only_the_join() {
    // CHAMFER: both lines byte-identical, one inserted LINE.
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 2.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 5.0);
    session.drafting_extras.corner_modes_mut().set_trim(false);
    let first_before = stored_geometry(&app, 1);
    let second_before = stored_geometry(&app, 2);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
    )
    .unwrap();
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 1);
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(stored_geometry(&app, 1), first_before);
    assert_eq!(stored_geometry(&app, 2), second_before);
    assert!(line_is(
        &app,
        outcome.objects[0].0,
        point(2.0, 0.0),
        point(0.0, 5.0)
    ));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::FilletRadius, 0.0);
    session.drafting_extras.corner_modes_mut().set_trim(false);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::FilletLines,
        fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );

    let (mut app, mut session) = l_corner();
    session.drafting_extras.corner_modes_mut().set_trim(false);
    assert_refused(
        &mut app,
        &mut session,
        CommandId::ChamferLines,
        chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        false,
    );
}

#[test]
fn repeated_invocations_are_one_undo_step_each() {
    let (mut app, mut session) = l_corner();
    set_number(&mut session, ToolParameter::ChamferDistanceA, 1.0);
    set_number(&mut session, ToolParameter::ChamferDistanceB, 1.0);
    let payload = || chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0));
    for _ in 0..2 {
        execute(&mut app, &mut session, CommandId::ChamferLines, payload()).unwrap();
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 2);
    for _ in 0..2 {
        execute(
            &mut app,
            &mut session,
            CommandId::Undo,
            CommandPayload::None,
        )
        .unwrap();
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 0);
    assert!(line_is(&app, 1, point(0.0, 0.0), point(10.0, 0.0)));
    assert!(line_is(&app, 2, point(0.0, 0.0), point(0.0, 10.0)));
}

#[test]
fn chamfer_and_fillet_are_work_only_and_refused_in_viewer() {
    let (mut app, mut session) = l_corner();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    let rev = revision(&app);
    for (id, payload) in [
        (
            CommandId::ChamferLines,
            chamfer(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        ),
        (
            CommandId::FilletLines,
            fillet(1, 2, point(8.0, 0.0), point(0.0, 8.0)),
        ),
    ] {
        let error = execute(&mut app, &mut viewer, id, payload).unwrap_err();
        assert_eq!(error, CadError::PermissionDenied, "{id:?}");
    }
    assert_eq!(revision(&app), rev);
    assert!(CommandId::ChamferLines.requires_work_mode());
    assert!(CommandId::FilletLines.requires_work_mode());
    let _ = &mut session;
}
