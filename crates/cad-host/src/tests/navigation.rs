use super::*;

// --- navigation contracts ---------------------------------------------------

#[test]
fn navigation_changes_only_camera_and_invalid_pan_stays_an_error() {
    let core = core();
    let (entity, _) = add_two_lines(&core);
    select_entity(&core, entity);
    run_controller_command(
        &core,
        CommandId::Measure,
        CommandPayload::MeasureTool(cad_app::MeasurementToolKind::Distance),
    );
    core.controller
        .borrow_mut()
        .session
        .set_measurement_cursor(Some(p(0.0, 0.0)))
        .unwrap();
    let snap_hints_before = core.controller.borrow().snap_hints_near_cursor().unwrap();
    assert!(!snap_hints_before.is_empty());

    let before_drawing = core.controller.borrow().drawing().unwrap();
    let (
        viewport,
        generation,
        selection,
        history,
        measurement_preview,
        has_measurement,
        camera,
        saved_camera,
    ) = {
        let c = core.controller.borrow();
        let viewport = c
            .application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .unwrap();
        (
            c.viewport_id,
            c.session.generation,
            c.session.selection.refs().to_vec(),
            c.history_availability(),
            c.session.measurement_preview(),
            c.session.has_last_measurement(),
            viewport.camera,
            viewport.view_mode.saved_2d_camera(),
        )
    };
    run_controller_command(
        &core,
        CommandId::Pan,
        CommandPayload::Points(vec![p(15.0, -7.0)]),
    );
    run_controller_command(
        &core,
        CommandId::Zoom,
        CommandPayload::ZoomAt {
            factor: 1.5,
            cursor: [400.0, 300.0],
        },
    );

    let c = core.controller.borrow();
    let current = c.application.workspace.viewports.get(&viewport).unwrap();
    assert_ne!(current.camera, camera);
    assert_eq!(
        current.view_mode.saved_2d_camera(),
        current.camera,
        "2D saved camera tracks navigation"
    );
    assert_ne!(current.camera, saved_camera);
    assert_eq!(c.session.generation, generation);
    assert_eq!(c.session.selection.refs(), selection);
    assert_eq!(c.history_availability(), history);
    assert!(matches!(c.session.tool, cad_app::ToolState::Measuring(_)));
    assert_eq!(c.session.measurement_preview(), measurement_preview);
    assert_eq!(c.session.has_last_measurement(), has_measurement);
    assert!(!c.snap_hints_near_cursor().unwrap().is_empty());
    assert!(Arc::ptr_eq(&before_drawing, &c.drawing().unwrap()));
    drop(c);

    let invalid = core.command(CommandId::Pan, CommandPayload::None);
    assert!(matches!(invalid, Err(CadError::InvalidInput(_))));
    // An invalid navigation command must not silently no-op.
    assert!(matches!(
        EngineClient::dispatch(
            &core,
            EngineRequest::Interaction(InteractionRequest::Scroll {
                delta_logical: [0.0, f64::NAN],
                position: PointInput::CanvasLocal {
                    logical_position: [0.0, 0.0],
                    metrics_revision: 0,
                },
                modifiers: cad_app::interaction::Modifiers::default(),
            })
        ),
        Err(RequestError::Execution(CadError::InvalidInput(_)))
    ));
}

#[test]
fn off_center_scroll_zoom_keeps_the_anchor_and_uses_the_wheel_factor() {
    let core = core();
    let cursor = [123.0, 177.0];
    let (anchor_before, scale_before) = {
        let controller = core.controller.borrow();
        let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
        (
            viewport
                .screen_to_world(cursor, viewport.logical_size)
                .unwrap(),
            viewport.world_per_px(),
        )
    };
    let revision = core.request_context().metrics_revision.unwrap();
    let outcome = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::Scroll {
            delta_logical: [0.0, 200.0],
            position: PointInput::CanvasLocal {
                logical_position: cursor,
                metrics_revision: revision,
            },
            modifiers: cad_app::interaction::Modifiers::default(),
        }),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        RequestResult::Interaction(InteractionOutcome::ScrollObserved)
    ));
    let (anchor_after, scale_after) = {
        let controller = core.controller.borrow();
        let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
        (
            viewport
                .screen_to_world(cursor, viewport.logical_size)
                .unwrap(),
            viewport.world_per_px(),
        )
    };
    assert!((scale_after / scale_before - 1.0 / 1.3).abs() < 1e-9);
    assert!((anchor_after.x - anchor_before.x).abs() < 1e-9);
    assert!((anchor_after.y - anchor_before.y).abs() < 1e-9);
}

#[test]
fn pointer_navigation_pans_and_multi_touch_never_commits() {
    let core = core();
    let revision = core.request_context().metrics_revision.unwrap();
    let origin = {
        let controller = core.controller.borrow();
        let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
        viewport
            .screen_to_world([100.0, 100.0], viewport.logical_size)
            .unwrap()
    };
    let pointer = |id, phase, button, position| InteractionRequest::Pointer {
        id: PointerId(id),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button,
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: revision,
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    };
    // A middle drag pans the view; the world point under the drag start shifts.
    // (A *primary* 2D drag is a box selection now — ADR 0014 B.2 — and is covered
    // by `tests/box_select.rs`.)
    assert!(matches!(
        EngineClient::dispatch(
            &core,
            EngineRequest::Interaction(pointer(
                1,
                cad_app::interaction::PointerPhase::Down,
                Some(PointerButton::Middle),
                [100.0, 100.0],
            ))
        )
        .unwrap(),
        RequestResult::Interaction(InteractionOutcome::PointerObserved)
    ));
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(pointer(
            1,
            cad_app::interaction::PointerPhase::Move,
            Some(PointerButton::Middle),
            [160.0, 100.0],
        )),
    )
    .unwrap();
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(pointer(
            1,
            cad_app::interaction::PointerPhase::Up,
            Some(PointerButton::Middle),
            [160.0, 100.0],
        )),
    )
    .unwrap();
    let origin_after = {
        let controller = core.controller.borrow();
        let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
        viewport
            .screen_to_world([100.0, 100.0], viewport.logical_size)
            .unwrap()
    };
    assert_ne!(origin, origin_after, "a middle drag must pan the camera");
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));

    // A second contact replaces an active capture instead of committing it.
    let revision = core.request_context().metrics_revision.unwrap();
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(1.0, 1.0))
        .unwrap();
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Drawing(_)
    ));
    for (id, phase, position) in [
        (1u64, cad_app::interaction::PointerPhase::Down, [10.0, 10.0]),
        (2u64, cad_app::interaction::PointerPhase::Down, [40.0, 10.0]),
    ] {
        EngineClient::dispatch(
            &core,
            EngineRequest::Interaction(InteractionRequest::Pointer {
                id: PointerId(id),
                kind: cad_app::interaction::PointerKind::Touch,
                phase,
                button: None,
                position: PointInput::CanvasLocal {
                    logical_position: position,
                    metrics_revision: revision,
                },
                modifiers: cad_app::interaction::Modifiers::default(),
            }),
        )
        .unwrap();
    }
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "multi-touch cancels the active tool, never commits it"
    );
    assert!(core.controller.borrow().session.draw_preview().is_none());
}

// ---------------------------------------------------------------------------
// Camera-settle deferral through the REAL host sequence (Oracle Finding 2).
//
// The host tick applies the engine view snapshot and then the host-computed
// overlay `viewport_bounds` (`HostCore::canvas_bounds`, built from projected
// screen corners). If that host computation disagreed with the engine's
// `viewport_world_bounds` by even 1 ULP, `set_overlays` would see a bounds-only
// change on every tick, advance the content revision and clear the engine's
// deferred grid refresh — silently making the deferral a no-op in production
// while the pure engine contracts stayed green. These fences pin that the two
// bounds authorities agree bitwise and that the deferral survives a real pan.
// ---------------------------------------------------------------------------

#[test]
fn host_tick_preserves_the_deferred_camera_grid_refresh_across_a_pan_gesture() {
    let core = core();
    // Long enough that no assertion below can trip the settle.
    core.render
        .set_grid_settle_timeout(std::time::Duration::from_secs(3600));
    core.sync_render().unwrap();

    // Quiescent ticks must stay quiescent: no content churn, no armed refresh.
    let idle = core.render.diagnostics();
    core.sync_render().unwrap();
    assert!(
        !core.render.grid_refresh_pending(),
        "an idle host tick must not arm a deferred camera refresh"
    );
    assert_eq!(
        core.render.diagnostics().requested_token,
        idle.requested_token,
        "an idle host tick must not churn the content revision"
    );

    // A pan gesture driven through the real host sequence.
    let mut last_view = idle.view_revision;
    for step in 1..=3 {
        pan_active_camera(&core, 40.0 * step as f64);
        core.sync_render().unwrap();
        assert!(
            core.render.grid_refresh_pending(),
            "host tick {step} must keep the deferred grid refresh pending: the host's \
             `canvas_bounds` and the engine's `viewport_world_bounds` must agree bitwise"
        );
        let diagnostics = core.render.diagnostics();
        assert_eq!(
            diagnostics.view_revision,
            last_view + 1,
            "host tick {step} advances the view exactly once"
        );
        assert_eq!(
            diagnostics.input_revision, idle.input_revision,
            "a camera-only pan must not bump the scene identity"
        );
        last_view = diagnostics.view_revision;
    }

    // A zoom step is the same camera field family (`world_per_px == scale`), so it
    // must defer identically — and again through the host's own bounds.
    {
        let mut c = core.controller.borrow_mut();
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .expect("active viewport");
        let cad_app::Projection::Orthographic { scale } = viewport.camera.projection else {
            panic!("the 2D fixture must use an orthographic camera");
        };
        viewport.camera.projection = cad_app::Projection::Orthographic { scale: scale * 1.3 };
    }
    core.sync_render().unwrap();
    assert!(
        core.render.grid_refresh_pending(),
        "a zoom step must keep the deferred refresh pending through the host sequence"
    );

    // Fit-on-resize through the host sequence (Oracle Finding 1): the canvas halves
    // while the camera zooms in by 2x, so the world extent is identical and
    // `camera_only` is false. The pending refresh covers exactly those bounds and
    // must be turned into one content advance, never silently dropped.
    let armed = core.render.diagnostics();
    {
        let mut c = core.controller.borrow_mut();
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .expect("active viewport");
        let cad_app::Projection::Orthographic { scale } = viewport.camera.projection else {
            panic!("the 2D fixture must use an orthographic camera");
        };
        viewport.camera.projection = cad_app::Projection::Orthographic { scale: scale * 2.0 };
        viewport.logical_size = [
            viewport.logical_size[0] * 0.5,
            viewport.logical_size[1] * 0.5,
        ];
    }
    core.sync_render().unwrap();
    assert!(
        !core.render.grid_refresh_pending(),
        "a fit-on-resize must cover the deferred refresh instead of dropping it"
    );
    assert_eq!(
        core.render.diagnostics().requested_token,
        armed.requested_token + 1,
        "the covered bounds become exactly one content advance"
    );
    assert_eq!(
        core.render.diagnostics().input_revision,
        armed.input_revision,
        "a bounds-preserving fit-on-resize must not bump the scene identity"
    );
}

/// The real-app zoom shape: with a live cursor inside the viewport the host
/// includes the camera-derived `view_scale` in `set_overlays` alongside the new
/// bounds. That derived field must not cancel the settle the snapshot just
/// armed, or wheel zoom re-prepares on every notch while pan defers.
#[test]
fn host_wheel_zoom_with_a_live_cursor_keeps_the_deferred_camera_refresh_pending() {
    let core = core();
    core.render
        .set_grid_settle_timeout(std::time::Duration::from_secs(3600));
    // Put a live cursor inside the 2D viewport.
    EngineClient::dispatch(&core, move_cursor_world(0.0, 0.0)).unwrap();
    core.sync_render().unwrap();
    let idle = core.render.diagnostics();
    assert!(
        core.cursor_world().is_some(),
        "fixture: the live cursor must be inside the 2D viewport"
    );

    // One wheel-zoom step through the real host sequence (`sync_render` applies
    // the snapshot and then the host-built overlays, exactly like production).
    {
        let mut c = core.controller.borrow_mut();
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .expect("the host always has an active viewport");
        let cad_app::Projection::Orthographic { scale } = viewport.camera.projection else {
            panic!("the 2D fixture must use an orthographic camera");
        };
        viewport.camera.projection = cad_app::Projection::Orthographic { scale: scale * 1.3 };
    }
    core.sync_render().unwrap();

    assert!(
        core.render.grid_refresh_pending(),
        "a wheel zoom with a live cursor must keep the deferred refresh pending"
    );
    assert_eq!(
        core.render.diagnostics().requested_token,
        idle.requested_token,
        "the deferred zoom must not schedule a worker preparation"
    );
}

// --- device-aware gesture arbitration (input-modality contract) -------------

/// A single-contact touch phase with the session's primary-button report.
///
/// The session (`cad-ui-core`) always emits `Some(button)` on a pointer phase
/// (an omitted wire button is primary), so this is the shape a real one-finger
/// press/drag/release arrives in.
fn touch_primary(
    core: &HostCore,
    id: u64,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(id),
        kind: cad_app::interaction::PointerKind::Touch,
        phase,
        button: Some(PointerButton::Primary),
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: core.request_context().metrics_revision.unwrap(),
        },
        modifiers: cad_app::interaction::Modifiers::default(),
    }
}

fn pinch_request(
    core: &HostCore,
    position: [f64; 2],
    delta: [f64; 2],
    scale: f64,
) -> InteractionRequest {
    InteractionRequest::Pinch {
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: core.request_context().metrics_revision.unwrap(),
        },
        delta_logical: delta,
        scale,
    }
}

fn viewport_numbers(core: &HostCore, cursor: [f64; 2]) -> (Point3, f64, usize) {
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    (
        viewport
            .screen_to_world(cursor, viewport.logical_size)
            .unwrap(),
        viewport.world_per_px(),
        viewport.zoom_history_len(),
    )
}

#[test]
fn a_touch_primary_drag_pans_and_never_starts_a_box() {
    let core = core();
    let camera_before = active_camera(&core);
    let anchor = [100.0, 100.0];
    let current = [180.0, 140.0];

    dispatch_pointer(
        &core,
        touch_primary(&core, 1, cad_app::interaction::PointerPhase::Down, anchor),
    );
    dispatch_pointer(
        &core,
        touch_primary(&core, 1, cad_app::interaction::PointerPhase::Move, current),
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "one-finger touch drag is a pan: it must never start a box gesture"
    );
    assert!(
        core.render.overlays().selection_box.is_none(),
        "no rubber band is drawn for a touch drag"
    );

    dispatch_pointer(
        &core,
        touch_primary(&core, 1, cad_app::interaction::PointerPhase::Up, current),
    );
    assert_ne!(
        active_camera(&core),
        camera_before,
        "a one-finger drag pans the view"
    );
    assert!(
        core.controller.borrow().session.selection.is_empty(),
        "a touch drag never selects"
    );
}

#[test]
fn a_sub_threshold_touch_tap_does_not_navigate_and_still_allows_selection() {
    let core = core();
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let camera_before = active_camera(&core);

    dispatch_pointer(
        &core,
        touch_primary(&core, 1, cad_app::interaction::PointerPhase::Down, pixel),
    );
    dispatch_pointer(
        &core,
        touch_primary(&core, 1, cad_app::interaction::PointerPhase::Up, pixel),
    );
    assert_eq!(
        active_camera(&core),
        camera_before,
        "a sub-threshold touch tap must not pan"
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "a touch tap starts no box"
    );

    // The session emits this `SelectAt` for the tap (it is the tap classifier);
    // the host must accept it exactly like a mouse click's selection.
    let revision = core.request_context().metrics_revision.unwrap();
    dispatch_pointer(
        &core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: pixel,
                metrics_revision: revision,
            },
            mode: SelectionMode::Replace,
        },
    );
    assert_eq!(
        core.controller.borrow().session.selection.refs(),
        [picked_ref_at(&core, pixel)].as_slice(),
        "a touch tap selects the real pick"
    );
}

#[test]
fn a_pinch_pans_by_the_midpoint_delta_and_zooms_about_the_midpoint() {
    let core = core();
    let cursor = [200.0, 150.0];
    let (anchor_before, scale_before, history_before) = viewport_numbers(&core, cursor);
    let outcome = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(pinch_request(&core, cursor, [30.0, 0.0], 1.5)),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        RequestResult::Interaction(InteractionOutcome::PinchObserved)
    ));

    let (anchor_after, scale_after, history_after) = viewport_numbers(&core, cursor);
    assert!(
        (scale_after / scale_before - 1.0 / 1.5).abs() < 1e-9,
        "the incremental scale is the zoom factor (1.5 ⇒ world_per_px / 1.5), got {}",
        scale_after / scale_before
    );
    assert_eq!(
        history_after, history_before,
        "a pinch is continuous navigation: it records no `ZOOM PREVIOUS` step"
    );
    // The midpoint pan runs **first** (contract order: pan, then zoom), so the
    // world point under the midpoint shifts by the logical delta at the
    // pre-zoom scale; the zoom then anchors at the midpoint and leaves it fixed.
    assert!(
        (anchor_after.x - (anchor_before.x - 30.0 * scale_before)).abs() < 1e-9,
        "anchor_before={anchor_before:?} anchor_after={anchor_after:?} scale_before={scale_before}"
    );
    assert!((anchor_after.y - anchor_before.y).abs() < 1e-9);
}

#[test]
fn a_pinch_cancels_an_active_tool_and_a_bad_scale_refuses_before_cancelling() {
    let core = core();
    let cursor = [200.0, 150.0];
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(1.0, 1.0))
        .unwrap();
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Drawing(_)
    ));

    // A zero scale is refused explicitly *before* any state changes: the active
    // tool is not cancelled by a malformed event.
    let refused = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(pinch_request(&core, cursor, [5.0, 5.0], 0.0)),
    );
    assert!(matches!(
        refused,
        Err(RequestError::Execution(CadError::InvalidInput(_)))
    ));
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Drawing(_)),
        "a refused pinch leaves the tool untouched"
    );

    // A valid pinch is a navigation takeover: the tool is cancelled, never
    // committed, and no preview survives it.
    let outcome = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(pinch_request(&core, cursor, [5.0, 5.0], 1.1)),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        RequestResult::Interaction(InteractionOutcome::PinchObserved)
    ));
    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Idle),
        "a pinch cancels the active tool"
    );
    assert!(core.controller.borrow().session.draw_preview().is_none());
}
