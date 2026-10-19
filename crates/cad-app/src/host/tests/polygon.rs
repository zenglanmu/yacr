//! POLYGON host tests: the session side-count lifecycle (ADR 0015 decision C)
//! and the capture → confirm mapping.
//!
//! Split out of `tools.rs` to keep that file under the 1000-line code limit.

use super::*;

/// The ADR-named lifecycle contract for the POLYGON side count: default 4, kept
/// across a tool switch and a cancel (sysvar semantics), untouched by a document
/// **edit**, refused out of range, reset to 4 by a session rebuild (new blank
/// document), and refused in Viewer mode.
#[test]
fn polygon_sides_defaults_to_four_survives_switch_and_cancel_and_refuses_out_of_range() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    // Default 4 (AutoCAD `POLYSIDES` initial value).
    assert_eq!(controller.session.polygon_sides(), 4);

    // Set a value: it survives a tool switch and a cancel.
    controller.session.set_polygon_sides(7).unwrap();
    assert_eq!(controller.session.polygon_sides(), 7);
    controller.begin_draw_tool(DrawToolKind::Polygon).unwrap();
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    controller.cancel_tool().unwrap();
    assert_eq!(
        controller.session.polygon_sides(),
        7,
        "the sysvar survives a tool switch and a cancel"
    );

    // A document **edit** (a real create) does not reset it.
    controller.create_line(p(0.0, 0.0), p(1.0, 0.0)).unwrap();
    assert_eq!(
        controller.session.polygon_sides(),
        7,
        "a document edit does not reset the sysvar"
    );

    // Out of range is refused explicitly, never clamped.
    for bad in [0u32, 1, 2, crate::draw_tool::MAX_POLYGON_SIDES + 1] {
        let error = controller.session.set_polygon_sides(bad).unwrap_err();
        assert!(
            matches!(error, CadError::InvalidInput(_)),
            "sides={bad} must be refused"
        );
        assert_eq!(
            controller.session.polygon_sides(),
            7,
            "a refused value must not mutate the sysvar"
        );
    }
    // The legal bounds themselves are accepted.
    controller
        .session
        .set_polygon_sides(crate::draw_tool::MIN_POLYGON_SIDES)
        .unwrap();
    assert_eq!(controller.session.polygon_sides(), 3);
    controller.session.set_polygon_sides(7).unwrap();

    // A session rebuild returns to 4.
    controller.reset_for_new_content();
    assert_eq!(
        controller.session.polygon_sides(),
        4,
        "a session rebuild resets the sysvar to the default"
    );
    controller.session.set_polygon_sides(9).unwrap();
    controller.new_blank_document().unwrap();
    assert_eq!(
        controller.session.polygon_sides(),
        4,
        "a new blank document resets the sysvar to the default"
    );

    // Viewer mode refuses the parameter (create commands are Work-only).
    controller.set_mode(AppMode::Viewer).unwrap();
    let error = controller.session.set_polygon_sides(5).unwrap_err();
    assert!(
        matches!(error, CadError::PermissionDenied),
        "a Viewer-mode parameter set must be refused: {error:?}"
    );
    assert_eq!(controller.session.polygon_sides(), 4);
}

#[test]
fn polygon_capture_confirms_one_entity_and_maps_the_exact_two_points() {
    let mut controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
    controller.set_snap_enabled(false);
    controller.session.set_polygon_sides(5).unwrap();
    let count = controller.drawing().unwrap().entity_count();

    controller.begin_draw_tool(DrawToolKind::Polygon).unwrap();
    let center = p(10.0, 10.0);
    let radius_point = p(20.0, 10.0);
    // One pick is not a polygon.
    controller.capture_tool_point(center).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(!preview.can_confirm());
    assert_eq!(preview.remaining_points(), Some(1));
    assert_eq!(
        preview.polygon_sides,
        Some(5),
        "the live preview carries the current side count"
    );
    assert!(controller.confirm_draw().is_err());

    controller.capture_tool_point(radius_point).unwrap();
    let preview = controller.session.draw_preview().unwrap();
    assert!(preview.can_confirm());
    assert_eq!(preview.points, vec![center, radius_point]);

    // The DrawIntent carries the two captured picks in click order and maps to
    // exactly those points on the command payload; the side count rides the
    // session, never the payload.
    let intent = controller.session.draw_intent().unwrap();
    assert_eq!(
        intent,
        crate::DrawIntent::Polygon {
            center,
            radius_point,
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
    assert_eq!(command.id, crate::CommandId::CreatePolygon);
    match command.payload {
        crate::CommandPayload::Points(points) => {
            assert_eq!(
                points,
                vec![center, radius_point],
                "payload matches capture exactly"
            )
        }
        _ => panic!("expected a Points payload"),
    }

    let outcome = controller.confirm_draw().unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let after = controller.drawing().unwrap();
    assert_eq!(after.entity_count(), count + 1);
    // The polygon is the newest model-space entity and is a **closed**
    // five-vertex straight polyline (the session side count).
    let model = after.model_space();
    let entity = model.last().unwrap();
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            assert_eq!(points.len(), 5);
            assert_eq!(bulges, &vec![0.0; 5]);
            assert!(*closed);
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert!(matches!(controller.session.tool, crate::ToolState::Idle));
}
