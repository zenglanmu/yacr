use super::*;

#[test]
fn post_commit_render_failure_keeps_a_successful_interaction_and_diagnoses() {
    use cad_engine::protocol::{RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    // Force `sync_render` to fail by moving the session to a block space.
    core.controller.borrow_mut().session.active_space =
        cad_domain::SpaceId::Block(cad_domain::BlockId(1));

    let completion = client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert!(
        matches!(
            completion.result,
            Ok(RequestResult::Interaction(InteractionOutcome::Cancelled))
        ),
        "a post-commit render failure must not reclassify a successful interaction"
    );
    assert!(
        matches!(core.last_status(), Some(StatusMessage::ExternalText(_))),
        "the render failure is surfaced as an explicit diagnostic"
    );
}

#[test]
fn host_funnel_command_records_a_real_command_trace() {
    use cad_engine::events::{EngineEvent, EventPoll};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    core.command(CommandId::OpenDrawing, CommandPayload::None)
        .unwrap();

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandStarted {
            command: CommandId::OpenDrawing
        }
    )));
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandFinished {
            command: CommandId::OpenDrawing,
            ..
        }
    )));
    assert!(events
        .iter()
        .any(|envelope| matches!(envelope.event, EngineEvent::RequestFinished { .. })));
}

#[test]
fn document_replacement_trace_uses_pre_and_post_context() {
    use cad_engine::events::{EngineEvent, EventPoll};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let pre_epoch = client.request_context().document_generation;
    let pre_revision = core.controller.borrow().drawing().unwrap().revision();
    core.command(CommandId::NewDrawing, CommandPayload::None)
        .unwrap();
    let post_epoch = client.request_context().document_generation;
    let post_revision = core.controller.borrow().drawing().unwrap().revision();
    assert_eq!(post_epoch, pre_epoch + 1, "New advances the document epoch");

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let started = events
        .iter()
        .find(|envelope| matches!(envelope.event, EngineEvent::RequestStarted { .. }))
        .expect("a RequestStarted");
    let finished = events
        .iter()
        .find(|envelope| matches!(envelope.event, EngineEvent::RequestFinished { .. }))
        .expect("a RequestFinished");
    assert_eq!(started.generation, pre_epoch, "Started is pre-state");
    assert_eq!(
        started.revision, pre_revision,
        "Started revision is pre-state"
    );
    assert_eq!(
        finished.generation, post_epoch,
        "Finished is terminal-state"
    );
    assert_eq!(
        finished.revision, post_revision,
        "Finished revision is terminal-state"
    );
    assert_eq!(started.document, finished.document, "same DocumentId");
}

#[test]
fn a_real_tool_change_emits_a_tool_event() {
    use cad_engine::events::EngineEvent;

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    client
        .dispatch_with_origin(
            cad_engine::protocol::RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::Line)),
        )
        .unwrap();

    let cad_engine::events::EventPoll::Events(events) =
        client.poll_events(subscription, 256).unwrap()
    else {
        panic!("expected events");
    };
    assert!(
        events
            .iter()
            .any(|envelope| matches!(envelope.event, EngineEvent::ToolChanged { .. })),
        "a real tool generation change emits a ToolChanged event"
    );
}

#[test]
fn confirm_commits_then_a_render_failure_keeps_confirmation_and_undo_restores() {
    use cad_domain::EntityId;
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let (document, viewport_id) = {
        let c = core.controller.borrow();
        (c.document_id, c.viewport_id)
    };
    let original_camera = {
        let c = core.controller.borrow();
        c.application.workspace.viewports[&viewport_id].camera
    };
    let undo_depth = || {
        core.controller
            .borrow()
            .application
            .history
            .get(&document)
            .map_or(0, |record| record.undo_depth())
    };
    let pre_revision = core.controller.borrow().drawing().unwrap().revision();
    let pre_depth = undo_depth();

    // A real LINE draw: BeginDraw -> two captured points -> Confirm.
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::Line)),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(p(
                0.0, 0.0,
            )))),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(p(
                10.0, 0.0,
            )))),
        )
        .unwrap();

    // Inject a real `sync_render` failure (an invalid camera projection) that
    // must not reclassify the successful confirmation.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .unwrap()
        .camera = cad_app::Camera {
        projection: cad_app::Projection::Orthographic { scale: 0.0 },
        ..original_camera
    };

    let completion = client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::Confirm),
        )
        .unwrap();
    assert!(
        matches!(
            &completion.result,
            Ok(RequestResult::Interaction(InteractionOutcome::Confirmed))
        ),
        "the business confirmation stays successful through a render failure"
    );

    // Real database/history effects (not fabricated): revision + one history step.
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision().0,
        pre_revision.0 + 1
    );
    assert_eq!(undo_depth(), pre_depth + 1);
    // Capture is cleared after a successful confirmation.
    assert!(matches!(
        core.controller.borrow().session.tool,
        ToolState::Idle
    ));
    assert!(core.controller.borrow().session.draw_preview().is_none());

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    // The actually created entity - read from the event, then checked in the DB.
    let created = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::CommandFinished { result, .. } => result
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|outcome| outcome.objects.first().copied()),
            _ => None,
        })
        .expect("the confirm command created an entity");
    assert!(
        core.controller
            .borrow()
            .drawing()
            .unwrap()
            .entity(EntityId(created.0))
            .is_some(),
        "the created entity really exists in the database"
    );
    assert!(
        matches!(core.last_status(), Some(StatusMessage::ExternalText(_))),
        "the render failure is an explicit diagnostic"
    );

    // The trace shares one ChangeSet Arc between CommandFinished and EntitiesChanged.
    let trace_changes = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::CommandFinished { result, .. } => result
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|o| o.changes.clone()),
            _ => None,
        })
        .expect("ChangeSet");
    let entities_change = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::EntitiesChanged { changes } => Some(changes.clone()),
            _ => None,
        })
        .expect("EntitiesChanged");
    assert!(Arc::ptr_eq(&trace_changes, &entities_change));
    let finished = events.iter().find(|envelope| {
        envelope.request_id == Some(completion.id)
            && matches!(envelope.event, EngineEvent::RequestFinished { .. })
    });
    assert!(
        finished.is_some_and(|envelope| matches!(
            &envelope.event,
            EngineEvent::RequestFinished { result } if result.as_ref().is_ok()
        )),
        "the request terminal is a success"
    );

    // Restore the camera and Undo once: the real entity is restored.
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .unwrap()
        .camera = original_camera;
    core.command(CommandId::Undo, CommandPayload::None).unwrap();
    assert!(core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .entity(EntityId(created.0))
        .is_none());
    // Undo restores the history depth (a new monotonic revision is expected; the
    // revision number is not rewound).
    assert_eq!(undo_depth(), pre_depth);
    assert_eq!(
        core.controller
            .borrow()
            .application
            .history
            .get(&document)
            .map_or(0, |record| record.redo_depth()),
        1,
        "exactly one Redo remains available"
    );
}

#[test]
fn dispatch_external_with_origin_failure_returns_a_real_id_error_completion() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestError, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let completion = core
        .dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(command(
                &core,
                CommandId::SwitchBackend,
                CommandPayload::Backend(cad_app::BackendChoice::WebGpu),
            )),
            || {
                Err(RequestError::Execution(CadError::InvalidInput(
                    "backend failed".into(),
                )))
            },
        )
        .unwrap();

    assert!(
        matches!(
            &completion.result,
            Err(RequestError::Execution(CadError::InvalidInput(_)))
        ),
        "the failure is carried in the real-id completion"
    );

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let mine: Vec<_> = events
        .iter()
        .filter(|envelope| envelope.request_id == Some(completion.id))
        .collect();
    assert!(mine
        .iter()
        .any(|envelope| matches!(envelope.event, EngineEvent::RequestStarted { .. })));
    assert!(mine.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandStarted {
            command: CommandId::SwitchBackend
        }
    )));
    assert!(mine.iter().any(|envelope| matches!(
        &envelope.event,
        EngineEvent::CommandFinished { result, .. } if result.as_ref().is_err()
    )));
    assert!(mine.iter().any(|envelope| matches!(
        &envelope.event,
        EngineEvent::RequestFinished { result } if result.as_ref().is_err()
    )));
    assert!(
        mine.iter()
            .all(|envelope| envelope.origin == RequestOrigin::Platform),
        "the trace keeps the external origin"
    );
    assert!(
        mine.iter()
            .all(|envelope| !matches!(envelope.event, EngineEvent::EntitiesChanged { .. })),
        "a failed external command emits no entity event"
    );
}

#[test]
fn dispatch_external_with_origin_action_reentry_is_busy_without_panicking() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestError, RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let nested_busy = Rc::new(Cell::new(false));
    {
        let core_for_action = core.clone();
        let nested_busy = nested_busy.clone();
        let completion = core
            .dispatch_external_with_origin(
                RequestOrigin::Platform,
                EngineRequest::Command(command(
                    &core,
                    CommandId::SwitchBackend,
                    CommandPayload::Backend(cad_app::BackendChoice::WebGpu),
                )),
                move || {
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
                    nested_busy.set(dispatch_busy && submit_busy && pump_busy);
                    Ok(Arc::new(CommandOutcome::none()))
                },
            )
            .unwrap();
        assert!(matches!(&completion.result, Ok(RequestResult::Command(_))));
    }
    assert!(
        nested_busy.get(),
        "nested dispatch/submit/pump from the action are all Busy"
    );
    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandStarted {
            command: CommandId::SwitchBackend
        }
    )));
}
