use super::*;

// --- P4: origin dispatch, API routing and bounded event trace ----------------

#[test]
fn api_commit_is_one_transaction_and_the_entities_event_shares_the_change_arc() {
    use cad_domain::{EntityId, LayerId, Point3, Revision as Rev, SpaceId};
    use cad_engine::api::{ApiRequest, ApiTarget, EntityEdit, NewEntity};
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestOrigin, RequestResult};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let (document, database, viewport, revision) = {
        let c = core.controller.borrow();
        let drawing = c.drawing().unwrap();
        (
            c.document_id,
            drawing.id(),
            c.viewport_id,
            drawing.revision(),
        )
    };
    let target = ApiTarget {
        document,
        database,
        viewport,
    };
    let edit = EntityEdit::Insert {
        layer: LayerId(0),
        space: SpaceId::Model,
        entity: NewEntity::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
    };
    let completion = client
        .dispatch_with_origin(
            RequestOrigin::Api,
            EngineRequest::Api {
                target,
                request: ApiRequest::Commit {
                    expected_revision: Rev(revision.0),
                    edits: vec![edit],
                },
            },
        )
        .unwrap();
    let outcome = match &completion.result {
        Ok(RequestResult::Api(outcome)) => outcome.clone(),
        other => panic!("expected an api outcome, got {other:?}"),
    };
    let changes = outcome.changes.clone().expect("a real ChangeSet");
    assert_eq!(outcome.created.len(), 1);

    // One history step; the created entity exists; Undo removes it.
    assert_eq!(
        core.controller.borrow().application.history[&document].undo_depth(),
        1
    );
    assert!(core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .entity(EntityId(outcome.created[0].0))
        .is_some());
    core.command(CommandId::Undo, CommandPayload::None).unwrap();
    assert!(core
        .controller
        .borrow()
        .drawing()
        .unwrap()
        .entity(EntityId(outcome.created[0].0))
        .is_none());

    // The EntitiesChanged event shares the SAME ChangeSet Arc (no deep copy).
    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let shared = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::EntitiesChanged { changes } => Some(changes.clone()),
            _ => None,
        })
        .expect("an EntitiesChanged event");
    assert!(
        Arc::ptr_eq(&shared, &changes),
        "the event shares the command's ChangeSet Arc"
    );
}

#[test]
fn failed_api_write_emits_no_entity_event_and_keeps_the_revision() {
    use cad_domain::{LayerId, Point3, Revision as Rev, SpaceId};
    use cad_engine::api::{ApiRequest, ApiTarget, EntityEdit, NewEntity};
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let (document, database, viewport, revision) = {
        let c = core.controller.borrow();
        let drawing = c.drawing().unwrap();
        (
            c.document_id,
            drawing.id(),
            c.viewport_id,
            drawing.revision(),
        )
    };
    let target = ApiTarget {
        document,
        database,
        viewport,
    };
    let edit = EntityEdit::Insert {
        layer: LayerId(0),
        space: SpaceId::Model,
        entity: NewEntity::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
    };
    let completion = client
        .dispatch_with_origin(
            RequestOrigin::Api,
            EngineRequest::Api {
                target,
                request: ApiRequest::Commit {
                    // A stale expected revision must be refused.
                    expected_revision: Rev(revision.0 + 1),
                    edits: vec![edit],
                },
            },
        )
        .unwrap();
    assert!(completion.result.is_err(), "a stale commit is refused");

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    assert!(
        events
            .iter()
            .all(|envelope| !matches!(envelope.event, EngineEvent::EntitiesChanged { .. })),
        "a failed write must not emit an entity event"
    );
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        revision,
        "a failed write keeps the revision"
    );
}

#[test]
fn sync_and_queued_requests_allocate_distinct_ids_and_rejects_leak_no_permit() {
    use cad_engine::protocol::{EngineRequest, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;

    let sync = client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert_eq!(
        core.events.outstanding_reserved(),
        0,
        "sync permit released"
    );

    let receipt = client
        .submit_with_origin(
            RequestOrigin::Ui,
            client.request_context(),
            EngineRequest::Interaction(InteractionRequest::Cancel),
        )
        .unwrap();
    assert_ne!(sync.id, receipt.id, "ids are unique across both paths");
    assert_eq!(core.events.outstanding_reserved(), 16, "one live permit");

    client.pump().unwrap();
    assert_eq!(
        core.events.outstanding_reserved(),
        0,
        "an executed request releases its permit"
    );
    assert_eq!(client.poll_completions(8).len(), 1);

    // A refused submission consumes no id and leaks no permit.
    let before = core.events.outstanding_reserved();
    assert!(client
        .submit_with_origin(
            RequestOrigin::Ui,
            client.request_context(),
            EngineRequest::Command(command(&core, CommandId::OpenDrawing, CommandPayload::None)),
        )
        .is_err());
    assert_eq!(core.events.outstanding_reserved(), before);
}

#[test]
fn subscriptions_watermark_and_unknown_ids_are_real() {
    use cad_engine::events::EventPoll;
    use cad_engine::events::SubscriptionId;

    let core = core();
    let client: &dyn EngineClient = &core;
    let watermark = client.event_watermark();
    assert_eq!(watermark.capacity, 1024);
    assert_eq!(watermark.next_sequence, 0);

    let subscription = client.subscribe().unwrap();
    assert!(matches!(
        client.poll_events(subscription, 0).unwrap(),
        EventPoll::Events(ref events) if events.is_empty()
    ));
    client.unsubscribe(subscription).unwrap();
    assert!(client.unsubscribe(subscription).is_err(), "unknown id");
    assert!(client.poll_events(SubscriptionId(9999), 8).is_err());
}

#[test]
fn coalesced_superseded_finishes_at_submit_without_polling_completions() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{RequestError, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let context = client.request_context();
    let first = client
        .submit_with_origin(RequestOrigin::Ui, context, move_cursor_world(1.0, 2.0))
        .unwrap();
    let second = client
        .submit_with_origin(RequestOrigin::Ui, context, move_cursor_world(3.0, 4.0))
        .unwrap();
    assert_ne!(first.id, second.id);

    // The displaced request's Finished is published at submit time, before any
    // completion polling, and it never claimed a Started.
    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let displaced = events
        .iter()
        .find(|envelope| envelope.request_id == Some(first.id))
        .expect("the displaced request has a terminal event");
    assert!(matches!(
        &displaced.event,
        EngineEvent::RequestFinished { result }
            if matches!(**result, Err(RequestError::Superseded { .. }))
    ));
    assert!(
        events
            .iter()
            .all(|envelope| !(envelope.request_id == Some(first.id)
                && matches!(envelope.event, EngineEvent::RequestStarted { .. }))),
        "a superseded request never emits Started"
    );

    // Its real completion is still queued, untouched.
    let completions = client.poll_completions(8);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].id, first.id);
    assert!(matches!(
        completions[0].result,
        Err(RequestError::Superseded { .. })
    ));
}

#[test]
fn stale_queued_request_publishes_only_a_finished() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{RequestError, RequestOrigin};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let context = client.request_context();
    let receipt = client
        .submit_with_origin(RequestOrigin::Ui, context, move_cursor_world(1.0, 2.0))
        .unwrap();
    // Replace the document: the epoch advances and the queued request is stale.
    core.request_new().unwrap();
    client.pump().unwrap();

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let mine: Vec<_> = events
        .iter()
        .filter(|envelope| envelope.request_id == Some(receipt.id))
        .collect();
    assert_eq!(
        mine.len(),
        1,
        "a stale request has exactly one terminal event"
    );
    assert!(matches!(
        &mine[0].event,
        EngineEvent::RequestFinished { result } if matches!(**result, Err(RequestError::StaleContext))
    ));
    assert!(
        !mine
            .iter()
            .any(|envelope| matches!(envelope.event, EngineEvent::RequestStarted { .. })),
        "a stale request never emits Started"
    );
}

#[test]
fn public_command_emits_a_real_command_lifecycle() {
    use cad_engine::events::{EngineEvent, EventPoll};

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    core.command(CommandId::FitDrawing, CommandPayload::None)
        .unwrap();

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let has = |predicate: &dyn Fn(&EngineEvent) -> bool| events.iter().any(|e| predicate(&e.event));
    assert!(has(&|event| matches!(
        event,
        EngineEvent::RequestStarted { .. }
    )));
    assert!(has(&|event| matches!(
        event,
        EngineEvent::CommandStarted {
            command: CommandId::FitDrawing
        }
    )));
    assert!(has(&|event| matches!(
        event,
        EngineEvent::CommandFinished {
            command: CommandId::FitDrawing,
            ..
        }
    )));
    assert!(has(&|event| matches!(
        event,
        EngineEvent::RequestFinished { .. }
    )));
}

#[test]
fn dispatch_external_with_origin_preflight_rejects_without_action_or_id() {
    use cad_engine::protocol::{EngineRequest, RequestOrigin};

    let core = core();
    let calls = Rc::new(Cell::new(0usize));

    // Wrong command variant.
    {
        let calls = calls.clone();
        let result = core.dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(command(&core, CommandId::OpenDrawing, CommandPayload::None)),
            move || {
                calls.set(calls.get() + 1);
                Ok(Arc::new(CommandOutcome::none()))
            },
        );
        assert!(result.is_err());
    }
    // Stale document preflight.
    {
        let calls = calls.clone();
        let mut cmd = command(
            &core,
            CommandId::SwitchBackend,
            CommandPayload::Backend(cad_app::BackendChoice::WebGpu),
        );
        cmd.document = cad_domain::DocumentId(cmd.document.0 + 999);
        let result = core.dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(cmd),
            move || {
                calls.set(calls.get() + 1);
                Ok(Arc::new(CommandOutcome::none()))
            },
        );
        assert!(result.is_err());
    }
    // A `None` payload (compat shim) is refused: the real request carries a
    // Backend(choice) payload.
    {
        let calls = calls.clone();
        let result = core.dispatch_external_with_origin(
            RequestOrigin::Platform,
            EngineRequest::Command(command(
                &core,
                CommandId::SwitchBackend,
                CommandPayload::None,
            )),
            move || {
                calls.set(calls.get() + 1);
                Ok(Arc::new(CommandOutcome::none()))
            },
        );
        assert!(result.is_err());
    }

    assert_eq!(calls.get(), 0, "preflight must not run the action");
    assert_eq!(core.events.outstanding_reserved(), 0);
    assert_eq!(core.queue.outstanding(), 0);
}

#[test]
fn dispatch_external_with_origin_runs_a_real_traced_switch_backend() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::{EngineRequest, RequestOrigin, RequestResult};

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
            || Ok(Arc::new(CommandOutcome::none())),
        )
        .unwrap();
    assert!(matches!(&completion.result, Ok(RequestResult::Command(_))));

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandStarted {
            command: CommandId::SwitchBackend
        }
    )));
    assert!(events.iter().any(|envelope| matches!(
        envelope.event,
        EngineEvent::CommandFinished {
            command: CommandId::SwitchBackend,
            ..
        }
    )));
}

#[test]
fn confirm_surfaces_its_trace_changeset_as_an_entities_event_sharing_the_arc() {
    use cad_engine::events::{EngineEvent, EventPoll};
    use cad_engine::protocol::RequestOrigin;

    let core = core();
    let client: &dyn EngineClient = &core;
    let subscription = client.subscribe().unwrap();
    let world = |x: f64, y: f64| PointInput::World(p(x, y));
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::Line)),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::CapturePoint(world(0.0, 0.0))),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::CapturePoint(world(10.0, 0.0))),
        )
        .unwrap();
    client
        .dispatch_with_origin(
            RequestOrigin::Ui,
            EngineRequest::Interaction(InteractionRequest::Confirm),
        )
        .unwrap();

    let EventPoll::Events(events) = client.poll_events(subscription, 256).unwrap() else {
        panic!("expected events");
    };
    let trace_changes = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::CommandFinished { result, .. } => result
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|outcome| outcome.changes.clone()),
            _ => None,
        })
        .expect("the confirm command trace carries a ChangeSet");
    let entities_change = events
        .iter()
        .find_map(|envelope| match &envelope.event {
            EngineEvent::EntitiesChanged { changes } => Some(changes.clone()),
            _ => None,
        })
        .expect("the confirm trace surfaces an EntitiesChanged event");
    assert!(
        Arc::ptr_eq(&trace_changes, &entities_change),
        "the entity event shares the trace's ChangeSet Arc (no deep copy)"
    );
}
