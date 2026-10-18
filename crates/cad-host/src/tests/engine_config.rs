use super::*;

// --- EngineClient surface: render identity, config authority, updates -------

#[test]
fn engine_client_uses_the_supplied_render_session_and_publishes_typed_state() {
    let core = core();
    let client: &dyn EngineClient = &core;
    assert!(client.render_session().same_session(&core.render));
    let updates = client.take_updates().unwrap();
    assert!(updates.full.is_some());
    assert!(updates.navigation.is_some());
    assert!(updates.config.is_some());
    assert_eq!(client.take_updates().unwrap(), HostUpdates::default());
}

#[test]
fn config_mutations_go_through_the_one_authority_store() {
    let core = core();
    core.update_config_json("{}").unwrap();
    let snapshot = core.config_snapshot();
    // The inherent accessor and the EngineClient surface expose one store.
    assert_eq!(EngineClient::config_snapshot(&core), snapshot);
    let updates = core.take_updates().unwrap();
    assert_eq!(updates.config.expect("config update is dirty"), snapshot);
    // A second drain reports no config change (unchanged is `None`, not empty).
    assert!(core.take_updates().unwrap().config.is_none());
}

#[test]
fn invalid_canvas_metrics_are_rejected_without_advancing_revision() {
    let core = core();
    let before = core.request_context().metrics_revision;
    assert!(core
        .set_canvas_metrics(CanvasMetrics::new([5.0, 0.0], [800.0, 600.0], 1.0))
        .is_err());
    assert_eq!(core.request_context().metrics_revision, before);
}

#[test]
fn a_failed_derivation_keeps_dirty_markers_for_retry() {
    let core = core();
    let (viewport_id, document_id) = {
        let c = core.controller.borrow();
        (c.viewport_id, c.document_id)
    };
    // Force the derived-state query to fail by removing the viewport.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .remove(&viewport_id);
    assert!(
        core.take_updates().is_err(),
        "a failed derivation must be Err, never an empty success"
    );
    // Restore and retry: the dirty markers were not consumed by the failure.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .insert(
            viewport_id,
            Viewport::new(viewport_id, document_id, [800.0, 600.0]),
        );
    let updates = core
        .take_updates()
        .expect("the retry succeeds once restored");
    assert!(
        updates.full.is_some(),
        "the full state is re-derived after a failed attempt"
    );
}

#[test]
fn interaction_changes_tool_generation_without_touching_the_document_or_session() {
    let core = core();
    let client: &dyn EngineClient = &core;
    assert!(client.render_session().same_session(&core.render));
    let before = client.request_context();
    let drawing_before = core.controller.borrow().drawing().unwrap();

    assert!(matches!(
        client
            .dispatch(EngineRequest::Interaction(InteractionRequest::BeginDraw(
                DrawToolKind::Line
            )))
            .unwrap(),
        RequestResult::Interaction(InteractionOutcome::ToolStarted)
    ));
    let after = client.request_context();
    assert!(
        after.tool_generation.unwrap() > before.tool_generation.unwrap(),
        "starting a tool advances the tool generation"
    );
    assert_eq!(after.document, before.document);
    assert_eq!(after.document_generation, before.document_generation);
    assert!(Arc::ptr_eq(
        &drawing_before,
        &core.controller.borrow().drawing().unwrap()
    ));
    assert!(client.render_session().same_session(&core.render));

    // Cancel returns the session to Idle without touching the document.
    assert!(matches!(
        client
            .dispatch(EngineRequest::Interaction(InteractionRequest::Cancel))
            .unwrap(),
        RequestResult::Interaction(InteractionOutcome::Cancelled)
    ));
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    assert!(Arc::ptr_eq(
        &drawing_before,
        &core.controller.borrow().drawing().unwrap()
    ));
}

// --- command contracts: MOVE snapshot + history -----------------------------

#[test]
fn move_uses_the_tool_start_selection_and_history_round_trips() {
    let core = core();
    let (a, b) = add_two_lines(&core);
    select_entity(&core, a);
    let original_a = line_endpoints(&core, a);
    let original_b = line_endpoints(&core, b);

    // The MOVE tool snapshots the selection at start; a later selection change
    // must not re-target the operation.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Move)
        .unwrap();
    select_entity(&core, b);
    core.controller
        .borrow_mut()
        .capture_tool_point(p(0.0, 0.0))
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(5.0, 2.0))
        .unwrap();
    core.controller.borrow_mut().confirm_draw().unwrap();

    assert_eq!(line_endpoints(&core, a), (p(5.0, 2.0), p(6.0, 2.0)));
    assert_eq!(line_endpoints(&core, b), original_b);

    run_controller_command(&core, CommandId::Undo, CommandPayload::None);
    assert_eq!(line_endpoints(&core, a), original_a);
    assert_eq!(line_endpoints(&core, b), original_b);
    run_controller_command(&core, CommandId::Redo, CommandPayload::None);
    assert_eq!(line_endpoints(&core, a), (p(5.0, 2.0), p(6.0, 2.0)));
    assert_eq!(line_endpoints(&core, b), original_b);
}

#[test]
fn a_failed_draw_confirmation_preserves_the_capture() {
    let core = core();
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    core.controller
        .borrow_mut()
        .capture_tool_point(p(1.0, 2.0))
        .unwrap();
    let before = core.controller.borrow().drawing().unwrap().revision();
    // One point is not enough to define a LINE: the confirmation is explicit.
    assert!(core.controller.borrow_mut().confirm_draw().is_err());
    assert!(core.controller.borrow().session.draw_preview().is_some());
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        before
    );
}
