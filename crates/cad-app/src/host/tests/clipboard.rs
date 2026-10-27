//! The clipboard cluster through the **host** path: capture → confirm → payload.
//!
//! These are the integration contracts behind `COPYBASE`/`CUTBASE`/`PASTECLIP`
//! that the pure command tests cannot cover: the session records the command that
//! started the capture, and `HostController::command_for_draw_intent` must map the
//! shared `CopyBase` intent to **that** command's payload (capture only vs capture
//! **and** delete) instead of guessing from the kind.

use super::*;
use crate::{CommandId, CommandPayload};

/// Draw one line and return its entity id, so the tests own their geometry instead
/// of depending on the demo fixture's first entity.
fn line_entity(controller: &mut HostController) -> EntityId {
    let outcome = controller.create_line(p(0.0, 0.0), p(10.0, 0.0)).unwrap();
    EntityId(outcome.objects[0].0)
}

/// A ref to a real entity of the open document.
fn entity_ref(controller: &HostController, entity: EntityId) -> SelectionRef {
    SelectionRef {
        document: controller.document_id,
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    }
}

/// The `(start, end)` of a stored LINE.
fn line_points(controller: &HostController, entity: EntityId) -> (Point3, Point3) {
    let geometry = controller
        .drawing()
        .unwrap()
        .entity(entity)
        .cloned()
        .expect("entity exists")
        .geometry;
    match geometry {
        SemanticGeometry::Line { start, end } => (start, end),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn copybase_capture_confirms_the_read_only_capture_with_its_base() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let entity = line_entity(&mut controller);
    controller.session.selection =
        crate::selection::SelectionSet::from_refs([entity_ref(&controller, entity)]);
    let before = controller.drawing().unwrap();
    let count = before.entity_count();
    let revision = before.revision();

    controller
        .begin_draw_tool(DrawToolKind::CopyBase)
        .expect("COPYBASE is Viewer-legal and this is a Work session");
    assert_eq!(
        controller.session.draw_command(),
        Some(CommandId::CopyToClipboardBase),
        "the session records the command that started the capture"
    );
    controller.capture_tool_point(p(2.0, 3.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();

    assert!(
        outcome.changes.is_none(),
        "COPYBASE is a read-only capture: no database write"
    );
    assert_eq!(controller.session.clipboard_len(), 1);
    assert_eq!(
        controller.session.clipboard_base,
        Some(p(2.0, 3.0)),
        "the captured point is the clipboard base"
    );
    let after = controller.drawing().unwrap();
    assert_eq!(after.entity_count(), count, "nothing is deleted");
    assert_eq!(after.revision(), revision, "no revision bump");
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
}

#[test]
fn cutbase_capture_confirms_the_cutting_payload_and_deletes_in_one_undo() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let entity = line_entity(&mut controller);
    controller.session.selection =
        crate::selection::SelectionSet::from_refs([entity_ref(&controller, entity)]);
    let count = controller.drawing().unwrap().entity_count();

    controller
        .begin_draw_tool(DrawToolKind::CutBase)
        .expect("CUTBASE is work-only and this is a Work session");
    assert_eq!(
        controller.session.draw_command(),
        Some(CommandId::CutToClipboardBase),
        "the same capture shape is tagged with the cutting command"
    );
    controller.capture_tool_point(p(1.0, 1.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();

    assert!(
        outcome.changes.is_some(),
        "CUTBASE writes: it deletes the captured refs"
    );
    assert_eq!(controller.session.clipboard_len(), 1);
    assert_eq!(controller.session.clipboard_base, Some(p(1.0, 1.0)));
    let drawing = controller.drawing().unwrap();
    assert!(
        drawing.entity(entity).is_none(),
        "the payload refs were deleted"
    );
    assert_eq!(drawing.entity_count(), count - 1);

    // One undo restores the cut entity.
    controller
        .application
        .execute(
            &mut controller.session,
            Command {
                schema_version: 1,
                id: CommandId::Undo,
                document: controller.document_id,
                viewport: controller.viewport_id,
                payload: CommandPayload::None,
            },
        )
        .unwrap();
    assert!(controller.drawing().unwrap().entity(entity).is_some());
}

#[test]
fn pasteclip_capture_translates_the_clipboard_by_insertion_minus_base() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let entity = line_entity(&mut controller);
    controller.session.selection =
        crate::selection::SelectionSet::from_refs([entity_ref(&controller, entity)]);

    // COPYBASE with the base (2, 3).
    controller.begin_draw_tool(DrawToolKind::CopyBase).unwrap();
    controller.capture_tool_point(p(2.0, 3.0)).unwrap();
    controller.confirm_draw().unwrap();

    // PASTECLIP at (7, 11) => translation (5, 8).
    controller.begin_draw_tool(DrawToolKind::PasteClip).unwrap();
    assert_eq!(
        controller.session.draw_command(),
        Some(CommandId::PasteFromClipboard)
    );
    controller.capture_tool_point(p(7.0, 11.0)).unwrap();
    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);

    let pasted_id = EntityId(outcome.objects[0].0);
    assert_ne!(pasted_id, entity, "the paste uses a fresh id");
    let (start, end) = line_points(&controller, pasted_id);
    assert!(
        (start.x - 5.0).abs() < 1e-9 && (start.y - 8.0).abs() < 1e-9,
        "{start:?}"
    );
    assert!(
        (end.x - 15.0).abs() < 1e-9 && (end.y - 8.0).abs() < 1e-9,
        "{end:?}"
    );
    assert_eq!(
        controller.session.clipboard_base,
        Some(p(2.0, 3.0)),
        "the paste leaves the clipboard (and its base) intact"
    );
}

#[test]
fn the_work_only_clipboard_tools_are_refused_in_a_viewer_session() {
    // The capture path shares the command authorization: a Viewer may start
    // `COPYBASE` (read-only) but not `CUTBASE` or `PASTECLIP`.
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    viewer.begin_draw(DrawToolKind::CopyBase).unwrap();
    assert_eq!(
        viewer.draw_command(),
        Some(CommandId::CopyToClipboardBase),
        "a Viewer capture is the read-only COPYBASE"
    );
    for kind in [DrawToolKind::CutBase, DrawToolKind::PasteClip] {
        assert_eq!(
            viewer.begin_draw(kind).unwrap_err(),
            CadError::PermissionDenied,
            "{kind:?}"
        );
    }
}
