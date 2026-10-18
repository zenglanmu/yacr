use super::*;

// --- P3 gate remediation: public sync guard / config stale / epoch ordering ---

#[test]
fn public_command_and_execute_are_busy_while_queued_without_side_effects() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    let revision_before = core.controller.borrow().drawing().unwrap().revision();

    assert!(matches!(
        core.command(CommandId::FitDrawing, CommandPayload::None),
        Err(CadError::InvalidInput(reason)) if reason == "engine is busy"
    ));
    let cmd = command(&core, CommandId::FitDrawing, CommandPayload::None);
    assert!(matches!(core.execute(cmd), Err(CadError::InvalidInput(_))));
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision_before,
        "a refused synchronous write must have no side effects"
    );

    client.pump().unwrap();
    client.poll_completions(8);
    assert!(core
        .command(CommandId::FitDrawing, CommandPayload::None)
        .is_ok());
}

#[test]
fn sync_service_callback_reentry_is_busy_without_panicking() {
    let files = Rc::new(NestedProbeFiles::default());
    let platform = HostPlatform {
        files: files.clone(),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let core = HostCore::new(options(), platform, RenderSessionHandle::default()).unwrap();
    *files.core.borrow_mut() = Some(core.clone());

    // A public execute triggers the platform begin_open callback mid-write.
    let cmd = command(&core, CommandId::OpenDrawing, CommandPayload::None);
    core.execute(cmd).unwrap();
    assert_eq!(files.reentries.get(), 1, "the callback ran exactly once");

    // Break the Rc cycle.
    *files.core.borrow_mut() = None;
}

#[test]
fn coordinate_requests_with_an_old_config_revision_are_stale() {
    let core = core();
    let client: &dyn EngineClient = &core;
    // Capture a context with current metrics but the old config revision.
    let context = client.request_context();
    core.update_config_json(r#"{"ui":{"components":{"layerPanel":{"initiallyOpen":false}}}}"#)
        .unwrap();
    assert!(client.request_context().config_revision > context.config_revision);

    let revision_before = core.controller.borrow().drawing().unwrap().revision();
    for request in [
        move_cursor_world(1.0, 2.0),
        EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(p(
            1.0, 2.0,
        )))),
        EngineRequest::Interaction(InteractionRequest::SelectAt {
            point: PointInput::World(p(1.0, 2.0)),
            mode: cad_app::interaction::SelectionMode::Replace,
        }),
    ] {
        client.submit(context, request).unwrap();
    }
    client.pump().unwrap();
    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 3);
    assert!(
        completions
            .iter()
            .all(|completion| matches!(&completion.result, Err(RequestError::StaleContext))),
        "MoveCursor/CapturePoint/SelectAt must all be config-stale"
    );
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision_before,
        "stale coordinate requests must change nothing"
    );
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
}

#[test]
fn same_document_id_replacement_still_advances_the_epoch() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    let document = core.controller.borrow().document_id;
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();

    // New keeps the same DocumentId but replaces the drawing: the epoch must
    // still advance, and the queued request becomes stale.
    core.request_new().unwrap();
    assert_eq!(core.controller.borrow().document_id, document);
    assert!(client.request_context().document_generation > context.document_generation);
    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
}

#[test]
fn failed_open_does_not_advance_the_epoch() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    let client: &dyn EngineClient = &core;
    let epoch = client.request_context().document_generation;
    core.begin_open_attempt();
    core.open_pending.set(true);
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/not-a-drawing.dwg"),
        bytes: Arc::from(vec![0u8, 1, 2, 3]),
    });
    assert!(
        core.drain_files().is_err(),
        "the invalid bytes fail the open"
    );
    assert_eq!(
        client.request_context().document_generation,
        epoch,
        "a failed open must not advance the epoch (no fake publish)"
    );
}

#[test]
fn async_publish_advances_the_epoch_even_when_post_steps_fail() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.begin_open_attempt();
    core.open_pending.set(true);
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/dwg.dxf"),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    // Drive ticks until the async publish records the new epoch. `presenting_open`
    // may outlive the publish, so wait on the epoch — the property under test —
    // while polling render preparation like `tick` does.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while client.request_context().document_generation == context.document_generation {
        assert!(
            std::time::Instant::now() < deadline,
            "import did not publish"
        );
        core.poll_loading().unwrap();
        core.render.poll_preparation().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // Resources are Unsupported here (post-publish install fails non-fatally),
    // yet the publish already replaced the document, so the epoch advanced.
    assert!(client.request_context().document_generation > context.document_generation);
    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
}

// --- P3 platform gate: with_sync_request + deliver_external_open --------------

#[test]
fn with_sync_request_is_busy_while_queued_without_side_effects() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    let status_before = core.last_status();
    let input_before = core.render.input_revision();

    let calls = Rc::new(Cell::new(0usize));
    {
        let calls = calls.clone();
        let result = core.with_sync_request(move || -> Result<(), RequestError> {
            calls.set(calls.get() + 1);
            Ok(())
        });
        assert!(matches!(result, Err(RequestError::Busy)));
    }
    assert_eq!(calls.get(), 0, "the action must not run while busy");
    assert_eq!(core.last_status(), status_before);
    assert_eq!(core.render.input_revision(), input_before);
}

#[test]
fn with_sync_request_runs_the_action_under_the_guard_without_probe_side_effects() {
    let core = core();
    let status_before = core.last_status();
    let input_before = core.render.input_revision();
    let action_calls = Rc::new(Cell::new(0usize));
    let nested = Rc::new(Cell::new([false; 3]));
    {
        let core_for_action = core.clone();
        let action_calls = action_calls.clone();
        let nested = nested.clone();
        let result: Result<u32, RequestError> = core.with_sync_request(move || {
            action_calls.set(action_calls.get() + 1);
            let dispatch_busy = matches!(
                EngineClient::dispatch(
                    &core_for_action,
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            );
            let submit_busy = matches!(
                EngineClient::submit(
                    &core_for_action,
                    EngineClient::request_context(&core_for_action),
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            );
            let pump_busy = matches!(
                EngineClient::pump(&core_for_action),
                Err(RequestError::Busy)
            );
            nested.set([dispatch_busy, submit_busy, pump_busy]);
            Ok(7)
        });
        assert_eq!(result.unwrap(), 7);
    }
    assert_eq!(action_calls.get(), 1);
    assert_eq!(
        nested.get(),
        [true, true, true],
        "all three nested entries Busy"
    );
    assert_eq!(core.last_status(), status_before, "no status side effect");
    assert_eq!(
        core.render.input_revision(),
        input_before,
        "no execution probe / sync_render side effect"
    );
}

#[test]
fn deliver_external_open_replaces_and_stales_old_queued_requests() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    // Queued old input is allowed (the IO replace makes it stale).
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    let document = core.controller.borrow().document_id;
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();

    core.deliver_external_open(FileTarget::from_path("/tmp/external.dxf"), Arc::from(bytes))
        .unwrap();
    assert_eq!(
        core.controller.borrow().document_id,
        document,
        "external replace keeps the same DocumentId"
    );
    assert!(client.request_context().document_generation > context.document_generation);
    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
}

#[test]
fn deliver_external_open_failure_does_not_publish_or_advance_epoch() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let epoch = client.request_context().document_generation;
    let error = core.deliver_external_open(
        FileTarget::from_path("/tmp/not-a-drawing.dwg"),
        Arc::from(vec![0u8, 1, 2, 3]),
    );
    assert!(error.is_err(), "invalid bytes must not fake success");
    assert_eq!(
        client.request_context().document_generation,
        epoch,
        "a failed external open must not advance the epoch"
    );
}

#[test]
fn deliver_external_open_is_busy_during_active_work() {
    let core = core();
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    let inside_refused = Rc::new(Cell::new(false));
    {
        let core_for_action = core.clone();
        let inside_refused = inside_refused.clone();
        let result = core.with_sync_request(move || {
            let refused = core_for_action
                .deliver_external_open(FileTarget::from_path("/tmp/x.dxf"), Arc::from(bytes))
                .is_err();
            inside_refused.set(refused);
            Ok(())
        });
        result.unwrap();
    }
    assert!(
        inside_refused.get(),
        "an active sync window must refuse external delivery"
    );
}

#[test]
fn deliver_external_open_async_returns_unit_and_marks_loading() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();

    core.deliver_external_open(FileTarget::from_path("/tmp/external.dxf"), Arc::from(bytes))
        .unwrap();
    // Ok(_) means the delivery started, not that the document is opened; the
    // caller distinguishes completion via loading()/status.
    assert!(core.loading(), "an async start is observable via loading()");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while client.request_context().document_generation == context.document_generation {
        assert!(
            std::time::Instant::now() < deadline,
            "import did not publish"
        );
        core.poll_loading().unwrap();
        core.render.poll_preparation().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(client.request_context().document_generation > context.document_generation);
}

// --- P3 platform gate: synchronous open terminal snapshot --------------------

#[test]
fn external_open_ends_a_pending_picker_and_reports_opened_terminal() {
    let core = core();
    // A real OpenDrawing picker leaves a loading state pending.
    core.request_open().unwrap();
    assert!(core.loading(), "the picker read is pending");

    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.deliver_external_open(FileTarget::from_path("/tmp/ext.dxf"), Arc::from(bytes))
        .unwrap();

    assert!(
        !core.loading(),
        "external delivery must end the pending picker/read"
    );
    let snapshot = core
        .import_snapshot()
        .expect("a terminal snapshot is retained");
    assert!(!snapshot.running);
    assert!(
        matches!(&snapshot.terminal, Some(ImportTerminal::Opened { entities }) if *entities > 0),
        "the real opened entity count is retained: {snapshot:?}"
    );
}

#[test]
fn external_open_failure_reports_a_failed_terminal() {
    let core = core();
    core.request_open().unwrap();
    let error = core.deliver_external_open(
        FileTarget::from_path("/tmp/bad.dwg"),
        Arc::from(vec![0u8, 1, 2, 3]),
    );
    assert!(error.is_err(), "invalid bytes must not fake success");
    assert!(!core.loading());
    let snapshot = core
        .import_snapshot()
        .expect("a terminal snapshot is retained");
    assert!(!snapshot.running);
    assert!(
        matches!(&snapshot.terminal, Some(ImportTerminal::Failed { .. })),
        "the original failure is retained: {snapshot:?}"
    );
}

#[test]
fn sync_drain_open_and_external_open_share_the_terminal_snapshot() {
    // The external-delivery path.
    let core = core();
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.deliver_external_open(FileTarget::from_path("/tmp/a.dxf"), Arc::from(bytes))
        .unwrap();
    assert!(matches!(
        core.import_snapshot().unwrap().terminal,
        Some(ImportTerminal::Opened { .. })
    ));

    // The `drain_files` (FileEvent::Opened) path uses the same pipeline, so it
    // must produce the same terminal snapshot rather than a separate/omitted one.
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), false);
    core.begin_open_attempt();
    core.open_pending.set(true);
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/b.dxf"),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    assert!(matches!(
        core.import_snapshot().unwrap().terminal,
        Some(ImportTerminal::Opened { .. })
    ));
}

#[test]
fn worker_running_snapshot_wins_over_any_sync_terminal() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();
    core.deliver_external_open(FileTarget::from_path("/tmp/w.dxf"), Arc::from(bytes))
        .unwrap();

    let snapshot = core.import_snapshot().expect("async running snapshot");
    assert!(
        snapshot.running,
        "the real async snapshot wins and is not polluted by a sync terminal"
    );
}

// --- P3 platform latest: snapshot scoped to the current open attempt ---------

#[test]
fn old_async_terminal_does_not_mask_a_new_open_attempt() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();

    // Attempt 1: a worker publishes a terminal.
    core.begin_open_attempt();
    core.open_pending.set(true);
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/one.dxf"),
        bytes: Arc::from(bytes.clone()),
    });
    core.drain_files().unwrap();
    drive_until_idle(&core);
    assert!(
        matches!(
            &core.import_snapshot().unwrap().terminal,
            Some(ImportTerminal::Opened { .. })
        ),
        "attempt 1 shows its own terminal"
    );

    // Attempt 2: a new picker must show a real running snapshot, never attempt
    // 1's retained worker terminal.
    core.request_open().unwrap();
    let snapshot = core.import_snapshot().expect("new picker snapshot");
    assert!(
        snapshot.running,
        "the new picker hides the old worker terminal: {snapshot:?}"
    );

    // Attempt 2 continues into a worker: running, then its own terminal.
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/two.dxf"),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    assert!(
        core.import_snapshot().unwrap().running,
        "the new worker reports a real running snapshot"
    );
    drive_until_idle(&core);
    assert!(matches!(
        &core.import_snapshot().unwrap().terminal,
        Some(ImportTerminal::Opened { .. })
    ));
}

#[test]
fn new_sync_terminal_is_not_masked_by_an_ineligible_old_async_terminal() {
    let files = Rc::new(FakeFiles::default());
    let core = core_with(files.clone(), true);
    let (bytes, _report) = core.controller.borrow().export_dxf().unwrap();

    // Attempt 1: worker terminal retained in the controller.
    core.begin_open_attempt();
    core.open_pending.set(true);
    files.events.borrow_mut().push(FileEvent::Opened {
        target: FileTarget::from_path("/tmp/one.dxf"),
        bytes: Arc::from(bytes),
    });
    core.drain_files().unwrap();
    drive_until_idle(&core);
    assert!(matches!(
        &core.import_snapshot().unwrap().terminal,
        Some(ImportTerminal::Opened { .. })
    ));

    // A new attempt with its own real synchronous terminal must be shown even
    // though the controller still retains attempt 1's async terminal.
    core.begin_open_attempt();
    *core.sync_terminal.borrow_mut() =
        Some(ImportProgressSnapshot::terminal(ImportTerminal::Opened {
            entities: 3,
        }));
    let snapshot = core.import_snapshot().expect("new sync terminal");
    assert!(
        matches!(&snapshot.terminal, Some(ImportTerminal::Opened { entities }) if *entities == 3),
        "the new attempt's own terminal wins over the ineligible old async one: {snapshot:?}"
    );
}
