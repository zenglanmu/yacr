use super::*;

// --- P3 bounded request queue contracts -------------------------------------

#[test]
fn queue_completes_a_submitted_request_and_wakes_once() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let wakes = Rc::new(Cell::new(0usize));
    {
        let wakes = wakes.clone();
        let waker: Rc<dyn Fn()> = Rc::new(move || wakes.set(wakes.get() + 1));
        client.set_request_waker(Some(waker));
    }

    // MoveCursor needs an active cursor tool to execute cleanly.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    let context = client.request_context();
    let receipt = client.submit(context, move_cursor_world(1.0, 2.0)).unwrap();
    assert_eq!(wakes.get(), 1, "empty -> nonempty wakes exactly once");

    // A submission into a non-empty queue is not a new edge. Use a distinct
    // context so the two MoveCursors do not coalesce.
    let mut second_context = client.request_context();
    second_context.metrics_revision =
        Some(second_context.metrics_revision.unwrap().saturating_add(1));
    client
        .submit(second_context, move_cursor_world(3.0, 4.0))
        .unwrap();
    assert_eq!(wakes.get(), 1);

    let report = client.pump().unwrap();
    assert!(report.processed >= 1);
    assert!(!report.pending, "the pass drained the queued work");

    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 2);
    assert_eq!(completions[0].id, receipt.id);
    assert!(matches!(
        completions[0].result,
        Ok(RequestResult::Interaction(InteractionOutcome::CursorMoved))
    ));
    // Draining to completions-only must not wake again (completion != pending).
    assert_eq!(wakes.get(), 1);
    client.set_request_waker(None);
}

#[test]
fn dispatch_is_busy_while_queued_but_not_for_completions_only() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    // Queued work: the synchronous path must not jump the queue.
    assert!(matches!(
        client.dispatch(EngineRequest::Interaction(InteractionRequest::Cancel)),
        Err(RequestError::Busy)
    ));

    client.pump().unwrap();
    // Completions-only is not pending: the synchronous path is allowed again.
    assert!(matches!(
        client.dispatch(EngineRequest::Interaction(InteractionRequest::Cancel)),
        Ok(RequestResult::Interaction(InteractionOutcome::Cancelled))
    ));
    assert_eq!(client.poll_completions(8).len(), 1);
}

#[test]
fn submit_rejects_gesture_commands_without_allocating() {
    let core = core();
    let client: &dyn EngineClient = &core;
    for id in [
        CommandId::OpenDrawing,
        CommandId::PlotDrawing,
        CommandId::SaveDrawingAs,
        CommandId::SwitchBackend,
    ] {
        let request = EngineRequest::Command(command(&core, id, CommandPayload::None));
        assert!(
            matches!(
                client.submit(client.request_context(), request),
                Err(RequestError::Execution(CadError::InvalidInput(_)))
            ),
            "{id:?} must be refused by submit"
        );
    }
    assert_eq!(core.queue.outstanding(), 0, "a refusal allocates nothing");
}

#[test]
fn config_write_is_busy_while_queue_pending() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();

    assert!(matches!(
        core.update_config_json("{}"),
        Err(CadError::InvalidInput(reason)) if reason == "engine is busy"
    ));

    client.pump().unwrap();
    client.poll_completions(8);
    assert!(core.update_config_json("{}").is_ok());
}

#[test]
fn ordinary_edit_does_not_make_a_queued_command_stale() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    let epoch_before = context.document_generation;
    client
        .submit(
            context,
            EngineRequest::Command(command(&core, CommandId::FitDrawing, CommandPayload::None)),
        )
        .unwrap();

    // An ordinary edit bumps the session generation but must not bump the host
    // document epoch, so the queued command stays valid.
    core.controller
        .borrow_mut()
        .create_line(p(0.0, 0.0), p(10.0, 0.0))
        .unwrap();
    assert_eq!(
        client.request_context().document_generation,
        epoch_before,
        "an edit must not change the document epoch"
    );

    client.pump().unwrap();
    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 1);
    assert!(
        matches!(completions[0].result, Ok(RequestResult::Command(_))),
        "the ordinary edit must not make the queued command stale"
    );
}

#[test]
fn document_replacement_makes_a_queued_request_stale() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let context = client.request_context();
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();

    // Replace the authoritative document and record it (as the open/new funnel
    // does): the epoch must advance.
    core.controller.borrow_mut().new_blank_document().unwrap();
    core.note_document_replaced();
    assert!(client.request_context().document_generation > context.document_generation);

    client.pump().unwrap();
    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 1);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
}

#[test]
fn tool_metrics_and_config_staleness_are_checked() {
    // Tool generation: a queued tool request is stale after the tool restarts.
    {
        let core = core();
        let client: &dyn EngineClient = &core;
        core.controller
            .borrow_mut()
            .begin_draw_tool(DrawToolKind::Line)
            .unwrap();
        let context = client.request_context();
        client.submit(context, move_cursor_world(1.0, 2.0)).unwrap();
        core.controller
            .borrow_mut()
            .begin_draw_tool(DrawToolKind::Line)
            .unwrap();
        client.pump().unwrap();
        let completions = client.poll_completions(4);
        assert!(matches!(
            completions[0].result,
            Err(RequestError::StaleContext)
        ));
    }

    // Metrics: a canvas-local coordinate is stale after the metrics revision moves.
    {
        let core = core();
        let client: &dyn EngineClient = &core;
        core.controller
            .borrow_mut()
            .begin_draw_tool(DrawToolKind::Line)
            .unwrap();
        let revision = client.request_context().metrics_revision.unwrap();
        client
            .submit(
                client.request_context(),
                EngineRequest::Interaction(InteractionRequest::MoveCursor(
                    PointInput::CanvasLocal {
                        logical_position: [1.0, 2.0],
                        metrics_revision: revision,
                    },
                )),
            )
            .unwrap();
        core.set_canvas_metrics(CanvasMetrics::new([0.0; 2], [1000.0, 800.0], 1.0))
            .unwrap();
        client.pump().unwrap();
        let completions = client.poll_completions(4);
        assert!(matches!(
            completions[0].result,
            Err(RequestError::StaleContext)
        ));
    }

    // Config: a gesture captured before a config change is stale at execution.
    // (A config write is refused while work is queued, so the change happens
    // before the submit.)
    {
        let core = core();
        let client: &dyn EngineClient = &core;
        let context = client.request_context();
        core.update_config_json(r#"{"ui":{"components":{"layerPanel":{"initiallyOpen":false}}}}"#)
            .unwrap();
        assert!(client.request_context().config_revision > context.config_revision);
        client
            .submit(
                context,
                EngineRequest::Interaction(InteractionRequest::Pointer {
                    id: PointerId(0),
                    kind: cad_app::interaction::PointerKind::Mouse,
                    phase: cad_app::interaction::PointerPhase::Down,
                    button: Some(PointerButton::Primary),
                    position: PointInput::World(p(0.0, 0.0)),
                    modifiers: cad_app::interaction::Modifiers::default(),
                }),
            )
            .unwrap();
        client.pump().unwrap();
        let completions = client.poll_completions(4);
        assert!(matches!(
            completions[0].result,
            Err(RequestError::StaleContext)
        ));
    }
}

#[test]
fn stale_pointer_up_clears_pointer_life_without_navigating() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let revision = client.request_context().metrics_revision.unwrap();
    // Start a real gesture so there is pointer life to clear.
    client
        .dispatch(EngineRequest::Interaction(InteractionRequest::Pointer {
            id: PointerId(1),
            kind: cad_app::interaction::PointerKind::Mouse,
            phase: cad_app::interaction::PointerPhase::Down,
            button: Some(PointerButton::Primary),
            position: PointInput::CanvasLocal {
                logical_position: [10.0, 10.0],
                metrics_revision: revision,
            },
            modifiers: cad_app::interaction::Modifiers::default(),
        }))
        .unwrap();
    assert!(core.navigation.active.borrow().contains(&PointerId(1)));

    // Submit a stale Up (wrong epoch) while the pointer life is still open.
    let mut context = client.request_context();
    context.document_generation += 1;
    client
        .submit(
            context,
            EngineRequest::Interaction(InteractionRequest::Pointer {
                id: PointerId(1),
                kind: cad_app::interaction::PointerKind::Mouse,
                phase: cad_app::interaction::PointerPhase::Up,
                button: Some(PointerButton::Primary),
                position: PointInput::CanvasLocal {
                    logical_position: [10.0, 10.0],
                    metrics_revision: revision,
                },
                modifiers: cad_app::interaction::Modifiers::default(),
            }),
        )
        .unwrap();
    assert!(
        core.navigation.active.borrow().contains(&PointerId(1)),
        "submission alone must not clear the pointer life"
    );

    client.pump().unwrap();
    let completions = client.poll_completions(4);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::StaleContext)
    ));
    assert!(
        core.navigation.active.borrow().is_empty(),
        "a stale Up must clear the pointer life"
    );
    assert!(!core.navigation.navigating.get());
}

#[test]
fn a_direct_stale_pointer_up_clears_pointer_life() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let revision = client.request_context().metrics_revision.unwrap();
    // Start a real single-contact gesture that crosses the threshold into a box
    // gesture, so the pointer/box life is non-trivial.
    client
        .dispatch(EngineRequest::Interaction(mouse(
            1,
            cad_app::interaction::PointerPhase::Down,
            [100.0, 100.0],
            revision,
        )))
        .unwrap();
    client
        .dispatch(EngineRequest::Interaction(mouse(
            1,
            cad_app::interaction::PointerPhase::Move,
            [200.0, 180.0],
            revision,
        )))
        .unwrap();
    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "the 2D primary drag starts a box gesture"
    );

    // A *synchronous* Up tagged with a stale metrics revision errors in
    // `pointer_logical` before the release phase runs; it must still clear the
    // pointer life exactly like the queue path's stale handling.
    let stale = revision.wrapping_sub(1);
    let error = client
        .dispatch(EngineRequest::Interaction(InteractionRequest::Pointer {
            id: PointerId(1),
            kind: cad_app::interaction::PointerKind::Mouse,
            phase: cad_app::interaction::PointerPhase::Up,
            button: Some(PointerButton::Primary),
            position: PointInput::CanvasLocal {
                logical_position: [200.0, 180.0],
                metrics_revision: stale,
            },
            modifiers: cad_app::interaction::Modifiers::default(),
        }))
        .unwrap_err();
    assert!(
        matches!(
            error,
            RequestError::Execution(CadError::InvalidInput(ref reason))
                if reason == "stale canvas metrics revision"
        ),
        "a stale pointer Up must be an explicit refusal, got {error:?}"
    );
    assert!(
        core.navigation.active.borrow().is_empty(),
        "a refused direct Up must not leave the contact active"
    );
    assert!(core.navigation.button.get().is_none());
    assert!(core.navigation.press_modifiers.get().is_none());
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "no box state dangles after a refused direct Up"
    );
}

#[test]
fn tick_does_not_implicitly_pump_the_queue() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    let queued = core.queue.queued();
    assert_eq!(queued, 1);

    // A timer tick must not drain the queue: only an explicit pump may.
    core.tick().unwrap();
    assert_eq!(core.queue.queued(), queued, "tick must not pump");
    assert_eq!(core.queue.completion_count(), 0);
    assert!(core.queue.pending());
    assert!(matches!(
        client.dispatch(EngineRequest::Interaction(InteractionRequest::Cancel)),
        Err(RequestError::Busy)
    ));
}

// --- P3 clock contracts ------------------------------------------------------

#[test]
fn pump_step_budget_bounds_a_pass_with_a_controlled_clock() {
    let core = core();
    let client: &dyn EngineClient = &core;
    install_fake_clock(&core, Rc::new(Cell::new(0.0)));
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    for i in 0..70u64 {
        let mut context = client.request_context();
        context.metrics_revision = Some(1_000 + i);
        client
            .submit(context, move_cursor_world(i as f64, 0.0))
            .unwrap();
    }

    let report = client.pump().unwrap();
    assert_eq!(report.processed, 64, "one pass runs at most PUMP_STEPS");
    assert!(report.pending);
    assert_eq!(core.queue.queued(), 6);
    client.poll_completions(256);
}

#[test]
fn pump_time_budget_spans_requests_not_a_single_request() {
    let core = core();
    let client: &dyn EngineClient = &core;
    // Each sample advances 1 ms: starting after the setter's single sample, the
    // pass runs 3 requests before the 4 ms window closes.
    let value = Rc::new(Cell::new(0.0));
    {
        let value = value.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || {
            let next = value.get() + 1.0;
            value.set(next);
            next
        });
        core.set_request_clock(clock).unwrap();
    }
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    for i in 0..10u64 {
        let mut context = client.request_context();
        context.metrics_revision = Some(2_000 + i);
        client
            .submit(context, move_cursor_world(i as f64, 0.0))
            .unwrap();
    }

    let report = client.pump().unwrap();
    assert_eq!(report.processed, 3, "the 4 ms window spans the pass");
    assert!(report.pending);
    assert_eq!(core.queue.queued(), 7);
    client.poll_completions(256);
}

#[test]
fn pump_refuses_non_finite_or_negative_clock_samples() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        let core = core();
        let client: &dyn EngineClient = &core;
        let value = Rc::new(Cell::new(0.0));
        install_fake_clock(&core, value.clone());
        core.controller
            .borrow_mut()
            .begin_draw_tool(DrawToolKind::Line)
            .unwrap();
        client
            .submit(client.request_context(), move_cursor_world(1.0, 2.0))
            .unwrap();
        // A bad sample only appears once work is pending.
        value.set(bad);

        let error = client.pump().unwrap_err();
        assert!(
            matches!(error, RequestError::Execution(CadError::InvalidInput(_))),
            "{bad} must be refused, not faked"
        );
        assert_eq!(core.queue.queued(), 1, "no request is taken on a bad clock");
        assert_eq!(core.queue.completion_count(), 0);
        assert_eq!(core.queue.outstanding(), 1);
    }
}

#[test]
fn pump_rejects_a_backwards_clock_after_preserving_completed_work() {
    // A clock that turns invalid after one executed request.
    let host = core();
    let client: &dyn EngineClient = &host;
    let calls = Rc::new(Cell::new(0usize));
    {
        let calls = calls.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || {
            let n = calls.get() + 1;
            calls.set(n);
            if n <= 3 {
                0.0
            } else {
                f64::NAN
            }
        });
        host.set_request_clock(clock).unwrap();
    }
    host.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    for i in 0..3u64 {
        let mut context = client.request_context();
        context.metrics_revision = Some(3_000 + i);
        client
            .submit(context, move_cursor_world(i as f64, 0.0))
            .unwrap();
    }

    let error = client.pump().unwrap_err();
    assert!(matches!(
        error,
        RequestError::Execution(CadError::InvalidInput(_))
    ));
    assert_eq!(
        host.queue.completion_count(),
        1,
        "the already-executed completion must not be lost"
    );
    assert_eq!(host.queue.queued(), 2, "the rest stays queued");

    // A backwards sample is likewise an explicit failure.
    let backwards = core();
    let value = Rc::new(Cell::new(100.0));
    {
        let value = value.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || {
            let now = value.get();
            value.set(now - 10.0);
            now
        });
        backwards.set_request_clock(clock).unwrap();
    }
    backwards
        .controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    EngineClient::submit(
        &backwards,
        EngineClient::request_context(&backwards),
        move_cursor_world(1.0, 2.0),
    )
    .unwrap();
    assert!(matches!(
        EngineClient::pump(&backwards).unwrap_err(),
        RequestError::Execution(CadError::InvalidInput(_))
    ));
}

#[test]
fn set_request_clock_refuses_an_invalid_first_sample_without_installing() {
    let core = core();
    let client: &dyn EngineClient = &core;
    // NaN and negative first samples are refused; the default clock stays.
    let nan = Rc::new(Cell::new(f64::NAN));
    {
        let nan = nan.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || nan.get());
        assert!(core.set_request_clock(clock).is_err());
    }
    assert!(core.set_request_clock(Rc::new(|| -1.0)).is_err());

    // The default native clock is still in place: a normal pass works.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    assert_eq!(client.pump().unwrap().processed, 1);
}

#[test]
fn pump_without_a_clock_needs_no_clock_when_idle_and_refuses_pending() {
    let core = core();
    let client: &dyn EngineClient = &core;
    core.clear_request_clock_for_test();

    // Empty queue: 0 processed, not pending, no clock needed.
    assert_eq!(core.pump().unwrap(), PumpReport::default());

    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    let error = client.pump().unwrap_err();
    assert!(matches!(
        error,
        RequestError::Execution(CadError::Unsupported(_))
    ));
    assert_eq!(
        core.queue.queued(),
        1,
        "no request is taken without a clock"
    );
    assert_eq!(core.queue.completion_count(), 0);
}

#[test]
fn clock_and_business_callbacks_reenter_as_busy_without_panicking() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let nested_pump_busy = Rc::new(Cell::new(false));
    let nested_set_busy = Rc::new(Cell::new(false));
    let nested_dispatch_busy = Rc::new(Cell::new(false));
    {
        let core_for_clock = core.clone();
        let nested_pump_busy = nested_pump_busy.clone();
        let nested_set_busy = nested_set_busy.clone();
        let nested_dispatch_busy = nested_dispatch_busy.clone();
        let clock: Rc<dyn Fn() -> f64> = Rc::new(move || {
            nested_pump_busy.set(matches!(
                EngineClient::pump(&core_for_clock),
                Err(RequestError::Busy)
            ));
            nested_set_busy.set(core_for_clock.set_request_clock(Rc::new(|| 0.0)).is_err());
            nested_dispatch_busy.set(matches!(
                EngineClient::dispatch(
                    &core_for_clock,
                    EngineRequest::Interaction(InteractionRequest::Cancel)
                ),
                Err(RequestError::Busy)
            ));
            0.0
        });
        core.set_request_clock(clock).unwrap();
    }
    assert!(nested_pump_busy.get(), "re-entrant pump is Busy");
    assert!(
        nested_set_busy.get(),
        "re-entrant set_request_clock is Busy"
    );
    assert!(
        nested_dispatch_busy.get(),
        "re-entrant dispatch during a clock sample is Busy"
    );

    // The same guard holds while the clock runs inside a pump pass.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    client
        .submit(client.request_context(), move_cursor_world(1.0, 2.0))
        .unwrap();
    assert_eq!(client.pump().unwrap().processed, 1);
    assert!(nested_dispatch_busy.get());
    core.clear_request_clock_for_test();
}

#[test]
fn waker_runs_with_borrows_released_and_rearms_only_while_queued() {
    let core = core();
    let client: &dyn EngineClient = &core;
    let calls = Rc::new(Cell::new(0usize));
    let nested_busy = Rc::new(Cell::new(false));
    {
        let core_for_waker = core.clone();
        let calls = calls.clone();
        let nested_busy = nested_busy.clone();
        let waker: Rc<dyn Fn()> = Rc::new(move || {
            calls.set(calls.get() + 1);
            // A nested call from inside the waker must not panic: all queue
            // borrows are released, and the queue is pending so dispatch is Busy.
            let nested = EngineClient::dispatch(
                &core_for_waker,
                EngineRequest::Interaction(InteractionRequest::Cancel),
            );
            nested_busy.set(matches!(nested, Err(RequestError::Busy)));
        });
        client.set_request_waker(Some(waker));
    }

    client
        .submit(
            client.request_context(),
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert_eq!(calls.get(), 1, "empty -> nonempty wakes once");
    assert!(nested_busy.get(), "the waker runs with borrows released");

    // A second submit while the queue is non-empty is not a new edge.
    client
        .submit(
            client.request_context(),
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert_eq!(calls.get(), 1);

    // Draining everything leaves no executable pending: no further wake.
    client.pump().unwrap();
    assert_eq!(calls.get(), 1, "draining to completions must not re-arm");
    client.poll_completions(8);
    client.set_request_waker(None);
}
