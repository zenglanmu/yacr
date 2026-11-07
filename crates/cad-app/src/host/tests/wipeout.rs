//! WIPEOUT host tests: the open-ended polygon capture → confirm mapping.
//!
//! The capture is AutoCAD's default WIPEOUT flow (first point, then every further
//! boundary point, Enter finishes) expressed by the existing open-ended machine.
//! The pinned contract: fewer than three points is never confirmable, the intent
//! carries every captured vertex in click order, the payload is exactly those
//! points, and the commit stores one `AcDbWipeout` mask.

use super::*;

#[test]
fn wipeout_capture_needs_three_points_and_maps_the_exact_ring() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    let before = controller.drawing().unwrap();
    let count = before.entity_count();

    controller.begin_draw_tool(DrawToolKind::Wipeout).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.is_open_ended(), "WIPEOUT is an open-ended capture");
    assert_eq!(preview.remaining_points(), Some(3));

    let ring = [p(10.0, 10.0), p(40.0, 10.0), p(40.0, 30.0), p(10.0, 30.0)];
    for (index, point) in ring.iter().take(2).enumerate() {
        controller.capture_tool_point(*point).unwrap();
        let preview = controller.session.draw_preview().unwrap();
        assert!(
            !preview.can_confirm(),
            "{} point(s) is not a ring",
            index + 1
        );
        assert_eq!(preview.remaining_points(), Some(3 - (index + 1)));
    }
    assert!(
        controller.confirm_draw().is_err(),
        "two points can never be confirmed as a wipeout"
    );

    controller.capture_tool_point(ring[2]).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm(), "three points define a real ring");
    assert_eq!(
        preview.remaining_points(),
        None,
        "more points stay accepted"
    );

    // A fourth point is accepted (the capture is a polygon, not a fixed arity).
    controller.capture_tool_point(ring[3]).unwrap();
    assert_eq!(
        controller.session.draw_preview().unwrap().points,
        ring.to_vec()
    );

    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        crate::DrawIntent::Wipeout {
            points: ring.to_vec(),
        }
    );
    let selection = controller
        .session
        .draw_selection_snapshot()
        .unwrap()
        .to_vec();
    let command = controller
        .command_for_draw_intent(intent, &selection)
        .unwrap();
    assert_eq!(command.id, crate::CommandId::Wipeout);
    match command.payload {
        crate::CommandPayload::Points(points) => {
            assert_eq!(points, ring.to_vec(), "payload matches the capture exactly")
        }
        _ => panic!("expected a Points payload"),
    }

    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let after = controller.drawing().unwrap();
    assert_eq!(after.entity_count(), count + 1);
    let model = after.model_space();
    let entity = model.last().unwrap();
    assert_eq!(entity.object.type_key, "AcDbWipeout");
    match &entity.geometry {
        SemanticGeometry::Mask { boundary, inverted } => {
            assert_eq!(boundary, &ring.to_vec());
            assert!(!*inverted);
        }
        other => panic!("expected a Mask, got {other:?}"),
    }
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
}

#[test]
fn wipeout_cancel_writes_nothing() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    let revision = controller.drawing().unwrap().revision();
    controller.begin_draw_tool(DrawToolKind::Wipeout).unwrap();
    controller.capture_tool_point(p(0.0, 0.0)).unwrap();
    controller.capture_tool_point(p(10.0, 0.0)).unwrap();
    controller.cancel_tool().unwrap();
    assert_eq!(controller.drawing().unwrap().revision(), revision);
    assert!(!controller.application.can_undo(&controller.document_id));
}
