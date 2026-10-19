//! Hover, cursor-overlay and pointer-leave session tests.

use super::super::*;
use super::test_support::*;
#[test]
fn plan_input_emits_a_hover_cursor_request_tagged_with_the_current_revision() {
    let moved = pointer_at(InputKind::PointerMove, 1, [40.0, 50.0], None);
    let (requests, next) = plan_input(None, &moved, 9, false).unwrap();
    assert!(next.is_none(), "an idle move retains no press");
    assert_eq!(requests.len(), 2, "move then hover: {requests:?}");
    assert!(matches!(
        requests[0],
        InteractionRequest::Pointer {
            phase: PointerPhase::Move,
            ..
        }
    ));
    match &requests[1] {
        InteractionRequest::MoveCursor(PointInput::CanvasLocal {
            logical_position,
            metrics_revision,
        }) => {
            assert_eq!(*logical_position, [40.0, 50.0]);
            assert_eq!(
                *metrics_revision, 9,
                "the hover must carry the current revision, never a fabricated one"
            );
        }
        other => panic!("expected a hover cursor request: {other:?}"),
    }
}

#[test]
fn plan_input_suppresses_hover_while_a_press_is_active() {
    // Within the drag threshold the press is a pending tap, not a hover.
    let moved = pointer_at(InputKind::PointerMove, 2, [10.0, 20.5], None);
    let (requests, press) =
        plan_input(Some(primary_press([10.0, 20.0])), &moved, 7, false).unwrap();
    assert_eq!(
        requests.len(),
        1,
        "a pressed move claims no hover: {requests:?}"
    );
    assert!(matches!(
        requests[0],
        InteractionRequest::Pointer {
            phase: PointerPhase::Move,
            ..
        }
    ));
    assert!(!press.expect("the press continues").dragged);

    // Beyond the threshold the gesture is a drag: still no hover.
    let dragged = pointer_at(
        InputKind::PointerMove,
        3,
        [10.0 + DRAG_THRESHOLD_LOGICAL_PX + 5.0, 20.0],
        None,
    );
    let (requests, press) =
        plan_input(Some(primary_press([10.0, 20.0])), &dragged, 7, false).unwrap();
    assert!(
        matches!(
            requests.as_slice(),
            [InteractionRequest::Pointer {
                phase: PointerPhase::Move,
                ..
            }]
        ),
        "a dragged move claims no hover: {requests:?}"
    );
    assert!(press.expect("the press continues").dragged);

    // The same holds for a middle-button press (the pan gesture).
    let down = pointer_at(InputKind::PointerDown, 4, [0.0, 0.0], Some(BUTTON_MIDDLE));
    let (_, middle_press) = plan_input(None, &down, 7, false).unwrap();
    let moved = pointer_at(InputKind::PointerMove, 5, [30.0, 10.0], None);
    let (requests, _) = plan_input(middle_press, &moved, 7, false).unwrap();
    assert_eq!(
        requests.len(),
        1,
        "a middle-button gesture owns its moves: {requests:?}"
    );
}

#[test]
fn an_idle_move_over_an_entity_reaches_the_overlay_hovered_input() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    let expected = picked_ref_at(&session, pixel);

    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
        .expect("the hover move is admitted");

    assert_eq!(
        session.snapshot(None, None, 0).error,
        None,
        "a hover must complete without a business error"
    );
    assert_eq!(
        hovered_refs(&session),
        vec![expected],
        "the hover over the rim must reach the entity-flags input"
    );
    assert!(
        session
            .core
            .controller
            .borrow()
            .session
            .selection
            .is_empty(),
        "hover must never select"
    );
    assert_eq!(
        session.snapshot(None, None, 0).selection,
        CadSelection::empty(),
        "the frozen status block is untouched by a hover"
    );
}

#[test]
fn an_idle_move_that_picks_nothing_clears_the_hovered_input() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
        .expect("the hover move is admitted");
    assert!(!hovered_refs(&session).is_empty(), "the rim is hovered");

    let empty = [1.0, 1.0];
    assert!(
        picks_nothing(&session, empty),
        "the chosen pixel must really be empty space"
    );
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 2, empty, None))
        .expect("the hover move is admitted");
    assert!(
        hovered_refs(&session).is_empty(),
        "a miss must clear the tint: no stale hover may survive"
    );
}

#[test]
fn a_repeated_idle_move_at_the_same_position_is_skipped() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
        .expect("the hover move is admitted");
    let queries = session.core.hover_query_count();
    assert_eq!(queries, 1, "the first move ran one hover query");

    // Two more moves at the identical position: no new pick runs, and the
    // hover is unchanged. Observable behaviour (a counter), not a wall clock.
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 2, pixel, None))
        .expect("the repeated hover move is admitted");
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 3, pixel, None))
        .expect("the repeated hover move is admitted");
    assert_eq!(
        session.core.hover_query_count(),
        queries,
        "an unchanged position must not re-run the hover query"
    );
    assert!(!hovered_refs(&session).is_empty(), "the hover is unchanged");
}

#[test]
fn a_press_clears_the_hover_and_the_release_claims_no_new_hover() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    let expected = picked_ref_at(&session, pixel);

    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
        .expect("the hover move is admitted");
    assert_eq!(hovered_refs(&session), vec![expected.clone()]);

    session
        .handle_input(&pointer_at(InputKind::PointerDown, 2, pixel, None))
        .expect("the press is admitted");
    assert!(
        hovered_refs(&session).is_empty(),
        "a press start clears the hover"
    );

    session
        .handle_input(&pointer_at(InputKind::PointerUp, 3, pixel, None))
        .expect("the release is admitted");
    assert!(
        hovered_refs(&session).is_empty(),
        "the released click claims no hover of its own"
    );

    // The next real move restores a hover for the (unchanged) view.
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 4, pixel, None))
        .expect("the hover move is admitted");
    assert_eq!(
        hovered_refs(&session),
        vec![expected],
        "a hover is restored only by the move after the press"
    );
}

#[test]
fn a_primary_drag_leaves_the_hover_cleared_until_the_next_move() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let start = screen_of(&session, demo_circle_rim(&session));
    let end = [
        start[0] + DRAG_THRESHOLD_LOGICAL_PX + 20.0,
        start[1] + DRAG_THRESHOLD_LOGICAL_PX + 20.0,
    ];

    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, start, None))
        .expect("the hover move is admitted");
    assert!(!hovered_refs(&session).is_empty(), "the rim is hovered");

    session
        .handle_input(&pointer_at(InputKind::PointerDown, 2, start, None))
        .expect("the press is admitted");
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 3, end, None))
        .expect("the dragged move is admitted");
    assert!(
        hovered_refs(&session).is_empty(),
        "a drag must not leave a hover tint behind"
    );
    session
        .handle_input(&pointer_at(InputKind::PointerUp, 4, end, None))
        .expect("the release is admitted");
    assert!(hovered_refs(&session).is_empty());

    // The view panned, so the rim is at a new pixel: the next move resolves a
    // *new* hover there.
    let after_pan = screen_of(&session, demo_circle_rim(&session));
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 5, after_pan, None))
        .expect("the hover move is admitted");
    assert!(
        !hovered_refs(&session).is_empty(),
        "the first move after the press restores a hover"
    );
}

#[test]
fn a_blur_clears_the_hovered_input() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
        .expect("the hover move is admitted");
    assert!(!hovered_refs(&session).is_empty(), "the rim is hovered");

    // The window lost focus: the hover it produced must not survive it.
    session
        .handle_input(&probe(InputKind::Blur))
        .expect("the blur is admitted");
    assert!(hovered_refs(&session).is_empty(), "a blur clears the hover");
}

#[test]
fn plan_input_maps_pointer_leave_to_clear_cursor_without_touching_the_press() {
    // A bare leave yields exactly the overlay cursor clear: no pointer
    // request, no coordinates, and no revision demand it cannot satisfy.
    let leave = probe(InputKind::PointerLeave);
    let (requests, next) = plan_input(None, &leave, 0, false).unwrap();
    assert_eq!(requests, vec![InteractionRequest::ClearCursor]);
    assert!(next.is_none(), "a leave with no press retains none");

    // It also leaves an in-flight press untouched: the exact same press is
    // returned, so a later release still resolves against it.
    let press = primary_press([10.0, 20.0]);
    let (requests, next) = plan_input(Some(press), &leave, 0, false).unwrap();
    assert_eq!(requests, vec![InteractionRequest::ClearCursor]);
    assert_eq!(next, Some(press), "the leave must not disturb the press");
}

#[test]
fn a_pointer_leave_clears_the_hover_overlay_only() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));

    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
        .expect("the hover move is admitted");
    assert!(!hovered_refs(&session).is_empty(), "the rim is hovered");
    // Consume the hover's own dirty markers so the assertions below are
    // about the leave alone.
    let _ = EngineClient::take_updates(&session.core).unwrap();

    session
        .handle_input(&pointer_at(InputKind::PointerLeave, 2, pixel, None))
        .expect("the leave is admitted");
    assert!(
        hovered_refs(&session).is_empty(),
        "the leave clears the hover tint"
    );

    // Overlay-only: leaving the surface schedules neither the full host
    // state nor a navigation rebuild (the same overlay-only classification
    // the hover `MoveCursor` uses, ADR-0012 §10 S2).
    let updates = EngineClient::take_updates(&session.core).unwrap();
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
fn a_pointer_leave_never_ends_or_wedges_an_in_flight_press() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    let expected = picked_ref_at(&session, pixel);

    session
        .handle_input(&pointer_at(InputKind::PointerDown, 1, pixel, None))
        .expect("the press is admitted");
    // The React lane only sends a leave with no button held, but the Rust
    // behaviour is defined regardless: it clears the cursor overlay and
    // leaves the press intact, so the release still resolves as a click.
    session
        .handle_input(&pointer_at(InputKind::PointerLeave, 2, pixel, None))
        .expect("the leave is admitted");
    session
        .handle_input(&pointer_at(InputKind::PointerUp, 3, pixel, None))
        .expect("the release is admitted");

    assert_eq!(
        session
            .core
            .controller
            .borrow()
            .session
            .selection
            .refs()
            .to_vec(),
        vec![expected],
        "the release after the leave still selects the picked entity"
    );
    assert_eq!(session.snapshot(None, None, 0).error, None);
}

#[test]
fn hover_preview_defaults_on_and_disabling_it_stops_picks_and_tint() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    // Immediate highlight, so the tint half is observable (the dwell has its
    // own contracts elsewhere).
    session
        .core
        .controller
        .borrow_mut()
        .session
        .set_hover_dwell_ms(0);
    let pixel = screen_of(&session, demo_circle_rim(&session));
    let expected = picked_ref_at(&session, pixel);
    assert!(
        session.snapshot(None, None, 0).hover_preview,
        "the status reports the AutoCAD-matching default: on"
    );

    // Default on: a move beyond the aperture runs exactly one pick and tints.
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
        .expect("the idle move is admitted");
    assert_eq!(
        session.core.hover_query_count(),
        1,
        "the default-on preview runs one pick on an idle move"
    );
    assert_eq!(hovered_refs(&session), vec![expected]);

    // Turning it off through the public selector clears the tint, and further
    // moves run no pick and tint nothing.
    session
        .command("HOVERPREVIEW")
        .expect("the toggle is admitted");
    assert!(!session.snapshot(None, None, 0).hover_preview);
    assert!(hovered_refs(&session).is_empty());
    let queries = session.core.hover_query_count();
    for (sequence, step) in (0..4).enumerate() {
        let position = [pixel[0] + step as f64 * 60.0, pixel[1]];
        session
            .handle_input(&pointer_at(
                InputKind::PointerMove,
                sequence as u64 + 2,
                position,
                None,
            ))
            .expect("the idle move is admitted");
        assert_eq!(
            session.core.hover_query_count(),
            queries,
            "no hover pick may run while the preview is off"
        );
        assert!(
            hovered_refs(&session).is_empty(),
            "nothing may be tinted while the preview is off"
        );
    }
}

#[test]
fn the_hover_preview_command_toggles_both_ways_and_clears_a_stationary_tint() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    let expected = picked_ref_at(&session, pixel);
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
        .expect("the hover move is admitted");
    assert_eq!(hovered_refs(&session), vec![expected.clone()]);
    let queries = session.core.hover_query_count();

    // Turning it off through the public selector clears the already painted
    // tint even though the cursor never moved again.
    session
        .command("HOVERPREVIEW")
        .expect("the toggle is admitted");
    assert!(!session.snapshot(None, None, 0).hover_preview);
    assert!(
        hovered_refs(&session).is_empty(),
        "disabling the preview immediately clears the stationary tint"
    );
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 2, pixel, None))
        .expect("the idle move is admitted");
    assert_eq!(
        session.core.hover_query_count(),
        queries,
        "a disabled preview runs no pick on the next move"
    );
    assert!(hovered_refs(&session).is_empty());

    // Turning it back on restores the hover on the next real move.
    session
        .command("HOVERPREVIEW")
        .expect("the toggle is admitted");
    assert!(session.snapshot(None, None, 0).hover_preview);
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 3, pixel, None))
        .expect("the hover move is admitted");
    assert_eq!(hovered_refs(&session), vec![expected]);
    assert_eq!(
        session.core.hover_query_count(),
        queries + 1,
        "the enabled preview queries once at the new position"
    );
}

#[test]
fn a_stale_hover_cursor_is_refused_and_never_tints() {
    let session = hover_enabled_session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let pixel = screen_of(&session, demo_circle_rim(&session));
    let context = session.core.request_context();
    let request =
        EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::CanvasLocal {
            logical_position: pixel,
            metrics_revision: context.metrics_revision.unwrap_or(0),
        }));
    let receipt = session.core.submit(context, request).unwrap();
    session.track_receipt(receipt.id);
    // The canvas metrics advance before the queued hover runs, so the host
    // grades it stale.
    session.set_canvas_metrics([640.0, 480.0], 1.0).unwrap();
    session.pump().unwrap();
    assert!(
        session.drain_completions().is_err(),
        "a stale hover must not be an empty success"
    );
    assert!(
        hovered_refs(&session).is_empty(),
        "a refused hover must not tint anything"
    );
}

// --- pointer-crosshair status flag (the 3D pointer decision) ----------------

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// The frontend hides the system cursor over the CAD partition only while the
/// host is actually drawing its own crosshair: a 2D view with a pointer on the
/// surface. Without a pointer (and after leaving) the system cursor must stay.
#[test]
fn the_crosshair_flag_follows_the_drawn_crosshair() {
    let session = fresh_session();
    assert!(
        !session.snapshot(None, None, 0).crosshair,
        "no pointer on the surface: the host draws no crosshair, so the system cursor stays"
    );

    session
        .handle_input(&move_input(200.0, 150.0))
        .expect("the 2D move is admitted");
    assert!(
        session.snapshot(None, None, 0).crosshair,
        "a 2D pointer move publishes the cursor overlay input: the host draws the crosshair"
    );

    session
        .handle_input(&probe(InputKind::PointerLeave))
        .expect("the leave is admitted");
    assert!(
        !session.snapshot(None, None, 0).crosshair,
        "leaving the surface clears the crosshair, so the system cursor returns"
    );
}

/// A 3D view is **never left pointerless**: the crosshair is 2D-only, so the
/// flag must stay `false` there and the frontend keeps the system arrow.
#[test]
fn a_3d_view_keeps_the_system_cursor() {
    let session = fresh_session();
    {
        let mut c = session.core.controller.borrow_mut();
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .expect("an active viewport");
        let saved = viewport.view_mode.saved_2d_camera();
        viewport
            .set_view_mode(cad_app::ViewMode2d3d::ThreeD { saved_2d: saved })
            .expect("the 3D switch is admitted");
    }
    session
        .handle_input(&move_input(200.0, 150.0))
        .expect("the move is admitted");
    assert!(
        !session.snapshot(None, None, 0).crosshair,
        "a 3D view draws no crosshair (no planar view rectangle), so the system cursor must stay"
    );
}
