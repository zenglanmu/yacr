use super::*;

// --- hover (ADR-0012 §10 S2) -------------------------------------------------

#[test]
fn an_idle_move_resolves_the_hover_through_the_pick_path_and_reaches_the_overlay_input() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);
    let database_before = document_identity(&core);
    let camera_before = active_camera(&core);
    assert!(
        hovered_refs(&core).is_empty(),
        "nothing is hovered before any move"
    );

    dispatch_pointer(&core, hover_cursor(&core, pixel));

    assert_eq!(
        hovered_refs(&core),
        vec![expected.clone()],
        "the hover must be exactly the pick_at_screen result"
    );
    assert_eq!(core.hover.queries.get(), 1, "exactly one hover pick ran");
    assert!(
        core.controller.borrow().session.selection.is_empty(),
        "hover never selects"
    );
    assert_eq!(active_camera(&core), camera_before, "hover never navigates");
    assert_eq!(
        document_identity(&core),
        database_before,
        "hover must not change the database"
    );
}

#[test]
fn a_hover_move_that_picks_nothing_clears_the_hovered_set() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    assert!(!picks_nothing(&core, pixel), "the rim really is pickable");
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(!hovered_refs(&core).is_empty(), "the rim is hovered");

    let empty = [1.0, 1.0];
    assert!(picks_nothing(&core, empty), "the corner really is empty");
    dispatch_pointer(&core, hover_cursor(&core, empty));

    assert!(
        hovered_refs(&core).is_empty(),
        "a miss clears the tint: no stale hover survives"
    );
    assert_eq!(core.hover.queries.get(), 2);
}

#[test]
fn an_unchanged_hover_position_runs_no_second_pick() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(core.hover.queries.get(), 1);
    // A repeated move at the same logical position is a cached no-op: no second
    // pick query, and the tint is unchanged. (Observable behaviour, not time.)
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(
        core.hover.queries.get(),
        1,
        "an unchanged position must not re-run the hover query"
    );
    assert!(!hovered_refs(&core).is_empty());

    // A different position queries again.
    let moved = [pixel[0] + 1.0, pixel[1] + 1.0];
    dispatch_pointer(&core, hover_cursor(&core, moved));
    assert_eq!(core.hover.queries.get(), 2);
}

#[test]
fn a_hover_cursor_with_a_stale_metrics_revision_is_refused_explicitly() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let client: &dyn EngineClient = &core;
    let revision = client.request_context().metrics_revision.unwrap();
    core.set_canvas_metrics(CanvasMetrics::new([0.0; 2], [1000.0, 800.0], 1.0))
        .unwrap();
    assert!(
        client.request_context().metrics_revision.unwrap() > revision,
        "the metrics change advances the revision"
    );

    let request = |metrics_revision: u64| {
        EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::CanvasLocal {
            logical_position: pixel,
            metrics_revision,
        }))
    };

    // The synchronous path refuses it with its stable, explicit reason.
    let error = client.dispatch(request(revision)).unwrap_err();
    assert!(
        matches!(
            error,
            RequestError::Execution(CadError::InvalidInput(ref reason))
                if reason == "stale canvas metrics revision"
        ),
        "a stale hover coordinate must be an explicit refusal, got {error:?}"
    );
    assert!(
        hovered_refs(&core).is_empty(),
        "a refused hover must not tint anything"
    );
    assert_eq!(
        core.hover.queries.get(),
        0,
        "a refused hover must not run a pick"
    );

    // The queued path grades it stale as well: never a silent success.
    client
        .submit(client.request_context(), request(revision))
        .unwrap();
    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert_eq!(completions.len(), 1);
    assert!(
        matches!(completions[0].result, Err(RequestError::StaleContext)),
        "a queued stale hover must be graded stale, got {:?}",
        completions[0].result
    );
    assert!(hovered_refs(&core).is_empty());
}

#[test]
fn a_press_clears_the_hover_and_suppresses_further_hover_queries() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let revision = core.request_context().metrics_revision.unwrap();

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(!hovered_refs(&core).is_empty(), "the rim is hovered");
    let queries = core.hover.queries.get();

    // A press starts a gesture (pan/orbit/drag or a tap): the tint is cleared.
    dispatch_pointer(
        &core,
        mouse(1, cad_app::interaction::PointerPhase::Down, pixel, revision),
    );
    assert!(
        hovered_refs(&core).is_empty(),
        "a press start clears the hover"
    );

    // Even a hover cursor delivered during the press is suppressed: no pick runs
    // and nothing is tinted.
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(hovered_refs(&core).is_empty());
    assert_eq!(
        core.hover.queries.get(),
        queries,
        "no hover pick runs while a press is active"
    );
}

#[test]
fn a_drag_leaves_the_hover_cleared_and_restores_a_new_hover_only_after_the_press_ends() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let start = screen_of(&core, demo_circle_rim(&core));
    let end = [
        start[0] + cad_app::DRAG_THRESHOLD_LOGICAL_PX + 30.0,
        start[1] + cad_app::DRAG_THRESHOLD_LOGICAL_PX + 30.0,
    ];
    let revision = core.request_context().metrics_revision.unwrap();

    dispatch_pointer(&core, hover_cursor(&core, start));
    assert!(!hovered_refs(&core).is_empty(), "the rim is hovered");

    dispatch_pointer(
        &core,
        mouse(1, cad_app::interaction::PointerPhase::Down, start, revision),
    );
    dispatch_pointer(
        &core,
        mouse(1, cad_app::interaction::PointerPhase::Move, end, revision),
    );
    assert!(
        hovered_refs(&core).is_empty(),
        "a drag leaves no hover tint behind"
    );

    // The release closes the gesture but claims no hover of its own.
    dispatch_pointer(
        &core,
        mouse(1, cad_app::interaction::PointerPhase::Up, end, revision),
    );
    assert!(
        hovered_refs(&core).is_empty(),
        "a release claims no new hover"
    );

    // The next real move restores a hover, resolved against the *panned* view.
    let after_pan = screen_of(&core, demo_circle_rim(&core));
    dispatch_pointer(&core, hover_cursor(&core, after_pan));
    assert!(
        !hovered_refs(&core).is_empty(),
        "the next move after the press restores a hover"
    );
}

#[test]
fn a_cancel_and_a_clear_cursor_clear_the_hover() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(!hovered_refs(&core).is_empty());
    dispatch_pointer(&core, InteractionRequest::ClearCursor);
    assert!(
        hovered_refs(&core).is_empty(),
        "the cursor left the surface: the hover must not survive"
    );

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(!hovered_refs(&core).is_empty());
    dispatch_pointer(&core, InteractionRequest::Cancel);
    assert!(hovered_refs(&core).is_empty(), "a cancel clears the hover");

    // A pointer cancel/blur path clears it too.
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(!hovered_refs(&core).is_empty());
    dispatch_pointer(
        &core,
        mouse(
            1,
            cad_app::interaction::PointerPhase::Cancel,
            pixel,
            core.request_context().metrics_revision.unwrap(),
        ),
    );
    assert!(
        hovered_refs(&core).is_empty(),
        "a pointer cancel clears the hover"
    );
}

#[test]
fn a_pointer_leave_clears_the_hover_and_does_not_rebuild_the_full_state() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(!hovered_refs(&core).is_empty(), "the rim is hovered");
    // Consume the hover's own dirty markers so the assertion below is about the
    // leave alone.
    EngineClient::take_updates(&core).unwrap();

    // The session maps `pointer_leave` to `ClearCursor`; the host must clear the
    // hover overlay without scheduling an O(layers+entities) full-state rebuild
    // (overlay-only, exactly like the hover `MoveCursor`).
    dispatch_pointer(&core, InteractionRequest::ClearCursor);
    assert!(
        hovered_refs(&core).is_empty(),
        "the cursor left the surface: the hover must not survive"
    );

    let updates = EngineClient::take_updates(&core).unwrap();
    assert!(
        updates.full.is_none(),
        "leaving the surface must not rebuild the full host state"
    );
    assert!(
        updates.navigation.is_none(),
        "leaving the surface is not a navigation change"
    );
}

#[test]
fn an_active_capture_keeps_the_tool_cursor_path_and_claims_no_hover() {
    let core = core();
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    let revision = core.request_context().metrics_revision.unwrap();

    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::CanvasLocal {
            logical_position: [100.0, 100.0],
            metrics_revision: revision,
        })),
    )
    .expect("a capture owns the cursor request");

    assert!(
        hovered_refs(&core).is_empty(),
        "a drawing capture must not produce a hover tint"
    );
    assert_eq!(
        core.hover.queries.get(),
        0,
        "no hover pick runs while a capture is active"
    );
    // The request really drove the tool preview cursor instead.
    let preview = core
        .controller
        .borrow()
        .session
        .draw_preview()
        .expect("the active Line capture has a preview");
    assert!(
        preview.cursor.is_some(),
        "the MoveCursor still moves the tool cursor"
    );
}

#[test]
fn a_pan_mode_session_suppresses_hover() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));

    dispatch_pointer(&core, InteractionRequest::BeginPan);
    dispatch_pointer(&core, hover_cursor(&core, pixel));

    assert!(
        hovered_refs(&core).is_empty(),
        "an active pan owns the pointer: no hover tint"
    );
    assert_eq!(
        core.hover.queries.get(),
        0,
        "no hover pick runs while the host is navigating"
    );
}
