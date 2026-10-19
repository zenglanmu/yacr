use super::*;

// --- hover (ADR-0012 §10 S2) -------------------------------------------------

#[test]
fn an_idle_move_resolves_the_hover_through_the_pick_path_and_reaches_the_overlay_input() {
    let core = hover_core();
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
    let core = hover_core();
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
fn hover_keeps_its_highlight_until_the_cursor_leaves_the_pickbox_aperture() {
    let core = hover_core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let aperture = cad_domain::TolerancePolicy::default().pickbox_logical_pixels;

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(core.hover_query_count(), 1, "the first move queries once");

    // Several tiny moves that stay inside the pickbox re-run no pick: the
    // highlight is sticky while the cursor is within the shared aperture. The
    // offsets stay at or below the 3-logical-pixel pickbox.
    for step in 1..=3 {
        dispatch_pointer(
            &core,
            hover_cursor(&core, [pixel[0] + step as f64, pixel[1]]),
        );
    }
    assert_eq!(
        core.hover_query_count(),
        1,
        "moves inside the aperture must not re-run the hover query"
    );
    assert!(
        !hovered_refs(&core).is_empty(),
        "the highlight persists while the cursor stays in the pickbox"
    );

    // An exact repeat of the query position re-runs nothing either.
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(
        core.hover_query_count(),
        1,
        "an unchanged position must not re-run the hover query"
    );

    // The first move strictly beyond the aperture re-hits exactly once.
    let outside = [pixel[0] + aperture + 1.0, pixel[1]];
    dispatch_pointer(&core, hover_cursor(&core, outside));
    assert_eq!(
        core.hover_query_count(),
        2,
        "leaving the aperture re-runs exactly one hover query"
    );
}

#[test]
fn a_scene_identity_change_re_queries_an_unchanged_position() {
    let core = hover_core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(core.hover_query_count(), 1);
    assert!(!hovered_refs(&core).is_empty());

    // A real content edit advances the drawing's scene identity. The cached
    // query position then belongs to an older identity and must be re-resolved
    // rather than keeping a highlight that no longer matches the drawing.
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 0.0), p(1.0, 0.0))
        .expect("the edit succeeds");

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(
        core.hover_query_count(),
        2,
        "a scene-identity change re-runs the hover query at the same position"
    );
}

#[test]
fn an_entity_on_a_hidden_layer_is_neither_hovered_nor_click_selected() {
    let core = hover_core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let layer = demo_circle_layer(&core);
    assert!(
        !picks_nothing(&core, pixel),
        "the rim really is pickable while its layer is visible"
    );

    // Hide the circle's layer as a temporary session override — the same command
    // the layer panel issues. It changes no database revision, so the persisted
    // pick index is not rebuilt: the visibility rule must be applied at pick
    // time, not cached with the index.
    run_controller_command(
        &core,
        CommandId::SetLayerVisibilities,
        CommandPayload::LayerVisibilities(vec![(layer, false)]),
    );

    // Hover: the hidden entity is not a candidate, so nothing is tinted and no
    // non-resident ref can reach the flags producer (which is exactly what would
    // otherwise be reported as `flags.unresolved`).
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(
        hovered_refs(&core).is_empty(),
        "an entity on a hidden layer must not be hovered"
    );
    assert!(
        core.render.overlays().hovered.is_empty(),
        "no non-resident ref may reach the FLAG_HOVERED flags input"
    );

    // Click: the same pick path refuses it, so nothing is selected.
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
    assert!(
        core.controller.borrow().session.selection.is_empty(),
        "a click must not select an entity on a hidden layer"
    );
    assert!(
        core.render.overlays().selection.is_empty(),
        "no FLAG_SELECTED may land on a non-resident key"
    );

    // Restoring the layer makes the same entity pickable again.
    run_controller_command(&core, CommandId::RestoreLayers, CommandPayload::None);
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(
        !hovered_refs(&core).is_empty(),
        "the same entity is hovered once the layer is shown again"
    );
}

#[test]
fn a_hover_cursor_with_a_stale_metrics_revision_is_refused_explicitly() {
    let core = hover_core();
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
    let core = hover_core();
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
    let core = hover_core();
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
    let core = hover_core();
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
    let core = hover_core();
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
    let core = hover_core();
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
    let core = hover_core();
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

// --- idle hover preview toggle (default ON, AutoCAD-matching) ----------------

#[test]
fn idle_hover_preview_defaults_on_and_disabling_it_stops_picks_and_tint() {
    let core = core();
    // Immediate highlight, so the tint half is observable (the dwell has its
    // own tests).
    core.controller.borrow_mut().session.set_hover_dwell_ms(0);
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);
    let aperture = cad_domain::TolerancePolicy::default().pickbox_logical_pixels;
    assert!(
        core.controller.borrow().hover_preview_enabled(),
        "the idle hover preview defaults on (AutoCAD SELECTIONPREVIEW 3)"
    );

    // Default on: a move beyond the aperture runs exactly one pick and tints.
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(
        core.hover_query_count(),
        1,
        "the default-on preview runs one pick on an idle move"
    );
    assert_eq!(hovered_refs(&core), vec![expected]);

    // Turning it off clears the tint immediately and further moves (each beyond
    // the aperture) run no pick and leave no tint.
    core.command(CommandId::ToggleHoverPreview, CommandPayload::None)
        .expect("the toggle command is admitted");
    assert!(!core.controller.borrow().hover_preview_enabled());
    assert!(hovered_refs(&core).is_empty());
    let queries = core.hover_query_count();
    for step in 0..5 {
        let position = [pixel[0] + step as f64 * (aperture + 10.0), pixel[1]];
        dispatch_pointer(&core, hover_cursor(&core, position));
        assert_eq!(
            core.hover_query_count(),
            queries,
            "a disabled preview must run no pick on a plain cursor move"
        );
        assert!(hovered_refs(&core).is_empty(), "nothing may be tinted");
        assert!(
            core.render.overlays().hovered.is_empty(),
            "no FLAG_HOVERED may reach the overlay inputs"
        );
    }
}

#[test]
fn turning_hover_preview_on_restores_the_pick_and_the_tint() {
    let core = core();
    // Pin off explicitly: the default is on, and this contract is about the
    // off→on transition.
    core.controller
        .borrow_mut()
        .session
        .set_hover_preview(false);
    // Immediate highlight (the dwell has its own tests).
    core.controller.borrow_mut().session.set_hover_dwell_ms(0);
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);

    // While off, the same move is a no-op...
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(core.hover_query_count(), 0);
    assert!(hovered_refs(&core).is_empty());

    // ...and enabling it makes that exact move query once and tint the entity,
    // even though the position never changed (no stale cached query survives
    // the disabled window).
    core.command(CommandId::ToggleHoverPreview, CommandPayload::None)
        .expect("the toggle command is admitted");
    assert!(core.controller.borrow().hover_preview_enabled());
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(
        core.hover_query_count(),
        1,
        "the enabled preview queries once at the cursor"
    );
    assert_eq!(
        hovered_refs(&core),
        vec![expected.clone()],
        "the rim is hovered again once the preview is enabled"
    );

    // Moving strictly beyond the aperture re-hits exactly once more.
    let aperture = cad_domain::TolerancePolicy::default().pickbox_logical_pixels;
    let outside = [pixel[0] + aperture + 1.0, pixel[1]];
    dispatch_pointer(&core, hover_cursor(&core, outside));
    assert_eq!(
        core.hover_query_count(),
        2,
        "leaving the aperture beyond the preview's own query re-hits once"
    );
}

#[test]
fn the_hover_preview_command_flips_both_ways_and_clears_a_stationary_tint() {
    let core = hover_core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(hovered_refs(&core), vec![expected]);
    let queries = core.hover_query_count();

    // The cursor does not move again: turning the preview off must still clear
    // the already painted tint, through the single host command funnel.
    core.command(CommandId::ToggleHoverPreview, CommandPayload::None)
        .expect("the toggle command is admitted");
    assert!(!core.controller.borrow().hover_preview_enabled());
    assert!(
        hovered_refs(&core).is_empty(),
        "disabling the preview clears the stationary tint immediately"
    );
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(
        core.hover_query_count(),
        queries,
        "a disabled preview runs no pick on the next move"
    );
    assert!(hovered_refs(&core).is_empty());

    // The second toggle re-enables it and the next move hovers again.
    core.command(CommandId::ToggleHoverPreview, CommandPayload::None)
        .expect("the toggle command is admitted");
    assert!(core.controller.borrow().hover_preview_enabled());
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(
        core.hover_query_count(),
        queries + 1,
        "the re-enabled preview queries once at the cursor"
    );
    assert!(!hovered_refs(&core).is_empty());
}

#[test]
fn click_selection_is_unaffected_by_the_hover_preview_switch() {
    // Preview explicitly off (the session default is now on).
    let disabled = core();
    disabled
        .controller
        .borrow_mut()
        .session
        .set_hover_preview(false);
    disabled
        .controller
        .borrow_mut()
        .fit()
        .expect("the demo fits");
    let pixel = screen_of(&disabled, demo_circle_rim(&disabled));
    let expected = picked_ref_at(&disabled, pixel);
    let selected = click_select_at(&disabled, pixel);
    assert_eq!(selected, expected, "the click picks the same entity");
    assert!(
        disabled.render.overlays().selection.contains(&expected),
        "the selection still reaches the flags input with the preview off"
    );
    assert_eq!(
        disabled.hover_query_count(),
        0,
        "the click runs no hover query"
    );

    // Preview on: the same click selects identically and runs no hover query
    // either (the click path never consults the preview switch).
    let enabled = hover_core();
    enabled
        .controller
        .borrow_mut()
        .fit()
        .expect("the demo fits");
    let pixel = screen_of(&enabled, demo_circle_rim(&enabled));
    let expected = picked_ref_at(&enabled, pixel);
    let selected = click_select_at(&enabled, pixel);
    assert_eq!(selected, expected, "the click picks the same entity");
    assert!(
        enabled.render.overlays().selection.contains(&expected),
        "the selection is unaffected by the enabled preview"
    );
    assert_eq!(
        enabled.hover_query_count(),
        0,
        "the click runs no hover query even with the preview enabled"
    );
}

// --- pickbox (PICKBOX analogue) vs snap aperture (APERTURE analogue) --------

/// The pixel of the midpoint of the demo's first LINE, a different entity from
/// the circle rim used by the hover tests.
fn demo_line_midpoint_pixel(core: &HostCore) -> [f64; 2] {
    let mid = {
        let c = core.controller.borrow();
        let drawing = c.drawing().expect("the demo document");
        let (start, end) = drawing
            .model_space()
            .iter()
            .find_map(|entity| match &entity.geometry {
                cad_domain::SemanticGeometry::Line { start, end } => Some((*start, *end)),
                _ => None,
            })
            .expect("the demo document has a line");
        Point3 {
            x: (start.x + end.x) * 0.5,
            y: (start.y + end.y) * 0.5,
            z: 0.0,
        }
    };
    screen_of(core, mid)
}

#[test]
fn pick_and_hover_use_the_pickbox_value_and_snap_keeps_the_wider_aperture() {
    let policy = cad_domain::TolerancePolicy::default();
    assert!(
        (policy.pickbox_logical_pixels - 3.0).abs() < 1e-12,
        "the pickbox defaults to AutoCAD's PICKBOX initial value of 3"
    );
    assert!(
        (policy.interaction_logical_pixels - 12.0).abs() < 1e-12,
        "the snap/candidate aperture stays at the project's 12"
    );

    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let rim = demo_circle_rim(&core);
    let rim_pixel = screen_of(&core, rim);

    // 10 logical pixels outside the circle rim: beyond the 3 px pickbox but
    // still inside the 12 px snap aperture.
    let cursor_pixel = [rim_pixel[0] + 10.0, rim_pixel[1]];
    assert!(
        picks_nothing(&core, cursor_pixel),
        "a pick at 10 px must miss the 3 px pickbox"
    );

    let cursor_world = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        viewport
            .screen_to_world(cursor_pixel, viewport.logical_size)
            .expect("the point is on the canvas")
    };
    let snapped = core
        .controller
        .borrow()
        .resolve_snap(cursor_world)
        .expect("snap resolution succeeds");
    assert!(
        snapped.is_some(),
        "the same geometry must remain snappable through the 12 px snap aperture"
    );
}

// --- hover dwell (an explicit product extension, not AutoCAD) ----------------

#[test]
fn dwell_zero_applies_the_highlight_immediately() {
    let core = hover_core(); // dwell 0
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(
        hovered_refs(&core),
        vec![expected],
        "dwell 0 restores the immediate highlight"
    );
    assert!(core.hover.pending.borrow().is_none());
}

#[test]
fn dwell_defers_the_tint_until_the_deadline_then_tick_applies_it() {
    let mut core = hover_core();
    let clock = install_hover_clock(&mut core);
    core.controller.borrow_mut().session.set_hover_dwell_ms(300);
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let expected = picked_ref_at(&core, pixel);

    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert_eq!(core.hover_query_count(), 1, "the pick still runs");
    assert!(
        hovered_refs(&core).is_empty(),
        "the resolved hover is not tinted before the dwell"
    );
    assert!(
        core.hover.pending.borrow().is_some(),
        "the resolved ref is recorded as pending"
    );

    // Settle the render so the next revision change is attributable to the tint.
    core.sync_view().unwrap();
    let overlay_before = core.render.overlay_revision();

    clock.set(299);
    core.tick().unwrap();
    assert!(
        hovered_refs(&core).is_empty(),
        "299 ms is before the 300 ms deadline"
    );
    assert_eq!(
        core.render.overlay_revision(),
        overlay_before,
        "a tick with nothing due must not churn the overlay revision"
    );

    clock.set(300);
    core.tick().unwrap();
    assert_eq!(
        hovered_refs(&core),
        vec![expected],
        "the due pending hover is applied by the per-frame tick"
    );
    assert!(core.hover.pending.borrow().is_none());
    assert!(
        core.render.overlay_revision() > overlay_before,
        "applying the tint republishes the overlay inputs"
    );

    // An idle tick afterwards touches nothing.
    let overlay_after = core.render.overlay_revision();
    for now in [301, 500] {
        clock.set(now);
        core.tick().unwrap();
    }
    assert_eq!(
        core.render.overlay_revision(),
        overlay_after,
        "an idle tick with no pending hover is a no-op"
    );
}

#[test]
fn a_changed_pending_ref_restarts_the_dwell_timer() {
    let mut core = hover_core();
    let clock = install_hover_clock(&mut core);
    core.controller.borrow_mut().session.set_hover_dwell_ms(300);
    core.controller.borrow_mut().fit().expect("the demo fits");
    let rim_pixel = screen_of(&core, demo_circle_rim(&core));
    let line_pixel = demo_line_midpoint_pixel(&core);
    let rim_ref = picked_ref_at(&core, rim_pixel);
    let line_ref = picked_ref_at(&core, line_pixel);
    assert_ne!(rim_ref, line_ref, "the two pixels hit different entities");

    // Resolve the rim first: pending from t=0.
    dispatch_pointer(&core, hover_cursor(&core, rim_pixel));
    assert!(hovered_refs(&core).is_empty());
    assert!(core.hover.pending.borrow().is_some());

    // At t=200 a different ref is resolved: the timer restarts, nothing tints.
    clock.set(200);
    dispatch_pointer(&core, hover_cursor(&core, line_pixel));
    assert!(hovered_refs(&core).is_empty());
    assert_eq!(
        core.hover
            .pending
            .borrow()
            .as_ref()
            .map(|p| p.reference.clone()),
        Some(line_ref.clone()),
        "the pending ref is now the line"
    );

    // t=300 would have been the rim's deadline, but the line has only had 100 ms.
    clock.set(300);
    core.tick().unwrap();
    assert!(
        hovered_refs(&core).is_empty(),
        "the restarted dwell has not elapsed"
    );

    // The line becomes due 300 ms after it was resolved (t=500).
    clock.set(500);
    core.tick().unwrap();
    assert_eq!(
        hovered_refs(&core),
        vec![line_ref],
        "the last stable ref is the one applied"
    );
}

#[test]
fn leaving_geometry_clears_the_applied_tint_immediately() {
    let mut core = hover_core();
    let clock = install_hover_clock(&mut core);
    core.controller.borrow_mut().session.set_hover_dwell_ms(300);
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));

    // Establish an applied hover through the dwell.
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    clock.set(300);
    core.tick().unwrap();
    assert!(!hovered_refs(&core).is_empty(), "the rim is now tinted");

    // Moving onto empty space resolves `None`: the applied tint must drop at
    // once, not linger for another 300 ms.
    let empty = [1.0, 1.0];
    assert!(picks_nothing(&core, empty));
    dispatch_pointer(&core, hover_cursor(&core, empty));
    assert!(
        hovered_refs(&core).is_empty(),
        "leaving geometry clears the applied tint immediately"
    );
    assert!(core.hover.pending.borrow().is_none());

    // An idle tick cannot resurrect it.
    clock.set(900);
    core.tick().unwrap();
    assert!(hovered_refs(&core).is_empty());
}

#[test]
fn clear_hover_drops_the_pending_dwell_as_well_as_the_applied_tint() {
    let mut core = hover_core();
    let clock = install_hover_clock(&mut core);
    core.controller.borrow_mut().session.set_hover_dwell_ms(300);
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = screen_of(&core, demo_circle_rim(&core));

    // Pending only: a leave clears it.
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    assert!(core.hover.pending.borrow().is_some());
    assert!(hovered_refs(&core).is_empty());
    dispatch_pointer(&core, InteractionRequest::ClearCursor);
    assert!(
        core.hover.pending.borrow().is_none(),
        "clear_hover (cursor leave) drops the pending dwell"
    );

    // Applied: a leave clears that too.
    dispatch_pointer(&core, hover_cursor(&core, pixel));
    clock.set(300);
    core.tick().unwrap();
    assert!(!hovered_refs(&core).is_empty());
    dispatch_pointer(&core, InteractionRequest::ClearCursor);
    assert!(hovered_refs(&core).is_empty());
    assert!(core.hover.pending.borrow().is_none());
}

// --- pointer cursor overlay input (crosshair + pickbox) ----------------------

#[test]
fn an_idle_move_publishes_the_pointer_cursor_overlay_input() {
    let core = hover_core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let pixel = [200.0, 150.0];
    assert!(
        core.render.overlays().cursor.is_none(),
        "no cursor overlay before any move"
    );

    dispatch_pointer(&core, hover_cursor(&core, pixel));

    let overlays = core.render.overlays();
    let cursor = overlays
        .cursor
        .expect("a move publishes the cursor world point");
    let scale = overlays
        .view_scale
        .expect("a 2D view publishes the view scale");
    assert!(scale.is_finite() && scale > 0.0);
    // The world point is the viewport's own screen_to_world of the pixel.
    let expected = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        viewport
            .screen_to_world(pixel, viewport.logical_size)
            .expect("the 2D plan view projects the pixel")
    };
    assert!((cursor.x - expected.x).abs() < 1e-9);
    assert!((cursor.y - expected.y).abs() < 1e-9);

    // Leaving the surface clears the cursor overlay input (never a stale one).
    dispatch_pointer(&core, InteractionRequest::ClearCursor);
    assert!(core.render.overlays().cursor.is_none());
}
