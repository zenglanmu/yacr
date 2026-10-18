use super::*;
use crate::protocol::InteractionOutcome;
use cad_app::interaction::{Modifiers, PointInput, PointerId, PointerKind};
use cad_domain::{DocumentId, Point3, ViewportId};

fn ctx(config_revision: u64) -> RequestContext {
    RequestContext {
        document: DocumentId(0),
        document_generation: 0,
        viewport: ViewportId(0),
        tool_generation: Some(1),
        metrics_revision: Some(1),
        config_revision,
    }
}

fn world() -> PointInput {
    PointInput::World(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    })
}

fn move_cursor() -> EngineRequest {
    EngineRequest::Interaction(InteractionRequest::MoveCursor(world()))
}

fn confirm() -> EngineRequest {
    EngineRequest::Interaction(InteractionRequest::Confirm)
}

fn pointer_up() -> EngineRequest {
    EngineRequest::Interaction(InteractionRequest::Pointer {
        id: PointerId(0),
        kind: PointerKind::Mouse,
        phase: PointerPhase::Up,
        button: None,
        position: world(),
        modifiers: Modifiers::default(),
    })
}

fn cancel() -> EngineRequest {
    EngineRequest::Interaction(InteractionRequest::Cancel)
}

fn pointer_cancel() -> EngineRequest {
    EngineRequest::Interaction(InteractionRequest::Pointer {
        id: PointerId(0),
        kind: PointerKind::Mouse,
        phase: PointerPhase::Cancel,
        button: None,
        position: world(),
        modifiers: Modifiers::default(),
    })
}

fn moved() -> Result<RequestResult, RequestError> {
    Ok(RequestResult::Interaction(InteractionOutcome::CursorMoved))
}

#[test]
fn outstanding_quota_counts_queued_executing_and_unclaimed_completions() {
    let queue = RequestQueue::new();
    assert_eq!(queue.outstanding(), 0);
    queue.submit(ctx(0), move_cursor()).unwrap();
    assert_eq!(queue.outstanding(), 1);

    let mut exec = queue.begin().unwrap();
    assert_eq!(queue.outstanding(), 1, "executing still occupies quota");
    assert!(exec.take_request().is_some());
    exec.complete(moved());
    assert_eq!(
        queue.outstanding(),
        1,
        "unclaimed completion occupies quota"
    );

    let done = queue.poll_completions(1);
    assert_eq!(done.len(), 1);
    assert_eq!(queue.outstanding(), 0, "polling frees quota");
}

#[test]
fn reserve_admits_only_interrupt_requests_and_rejects_at_capacity_atomically() {
    let queue = RequestQueue::with_limits(4, 3);
    assert_eq!(
        queue.submit(ctx(0), move_cursor()).unwrap().id,
        RequestId(0)
    );
    assert_eq!(
        queue.submit(ctx(1), move_cursor()).unwrap().id,
        RequestId(1)
    );
    assert_eq!(
        queue.submit(ctx(2), move_cursor()).unwrap().id,
        RequestId(2)
    );
    assert_eq!(queue.outstanding(), 3);

    // Normal quota exhausted: a normal request is rejected with no mutation.
    assert!(matches!(
        queue.submit(ctx(3), move_cursor()),
        Err(RequestError::QueueFull)
    ));
    assert_eq!(queue.queued(), 3, "rejection must not mutate state");

    // Reserve admits interrupt/close-out interactions.
    assert_eq!(queue.submit(ctx(3), confirm()).unwrap().id, RequestId(3));
    assert_eq!(queue.outstanding(), 4);

    // Absolute capacity: every reserve kind is refused too.
    for request in [confirm(), cancel(), pointer_up(), pointer_cancel()] {
        assert!(
            matches!(queue.submit(ctx(4), request), Err(RequestError::QueueFull)),
            "reserve must not exceed absolute capacity"
        );
    }
    assert_eq!(queue.outstanding(), 4);

    // Rejections never consumed an identity: free two slots, then the next
    // accepted normal request gets the id that the rejections did not take.
    let exec = queue.begin().unwrap();
    exec.complete(moved());
    queue.poll_completions(8);
    let exec = queue.begin().unwrap();
    exec.complete(moved());
    queue.poll_completions(8);
    assert_eq!(queue.outstanding(), 2);
    assert_eq!(
        queue.submit(ctx(9), move_cursor()).unwrap().id,
        RequestId(4),
        "rejected submissions must not advance the id counter"
    );
}

#[test]
fn reserve_admits_each_of_the_four_interrupt_kinds() {
    // Confirm, InteractionRequest::Cancel, Pointer Up and Pointer Cancel are
    // four distinct kinds; all four may use the reserved quota.
    for request in [confirm(), cancel(), pointer_up(), pointer_cancel()] {
        let queue = RequestQueue::with_limits(4, 3);
        queue.submit(ctx(0), move_cursor()).unwrap();
        queue.submit(ctx(1), move_cursor()).unwrap();
        queue.submit(ctx(2), move_cursor()).unwrap();
        assert_eq!(queue.outstanding(), 3);
        // A normal request is refused at the normal boundary...
        assert!(matches!(
            queue.submit(ctx(3), move_cursor()),
            Err(RequestError::QueueFull)
        ));
        // ...but the reserved interrupt kind is admitted into reserve.
        assert!(queue.submit(ctx(3), request).is_ok());
        assert_eq!(queue.outstanding(), 4);
    }
}

#[test]
fn id_exhaustion_is_an_explicit_error_without_mutation_or_panic() {
    let queue = RequestQueue::new();
    // Private test-only access: force the counter to the exhausted sentinel.
    queue.state.borrow_mut().next_id = u64::MAX;
    assert!(matches!(
        queue.submit(ctx(0), move_cursor()),
        Err(RequestError::Execution(CadError::Invariant(reason)))
            if reason.contains("id space exhausted")
    ));
    // Rejection is atomic: no id, queue, completion, wake or counter change.
    assert_eq!(queue.state.borrow().next_id, u64::MAX);
    assert_eq!(queue.outstanding(), 0);
    assert_eq!(queue.queued(), 0);
    assert_eq!(queue.completion_count(), 0);
    assert!(!queue.pending());
    assert!(!queue.take_wake());
}

#[test]
fn last_usable_id_is_max_minus_one_then_exhausts_atomically() {
    let queue = RequestQueue::new();
    queue.state.borrow_mut().next_id = u64::MAX - 1;
    assert_eq!(
        queue.submit(ctx(0), move_cursor()).unwrap().id,
        RequestId(u64::MAX - 1)
    );
    assert_eq!(queue.state.borrow().next_id, u64::MAX);

    // The next allocation is refused atomically rather than wrapping.
    assert!(matches!(
        queue.submit(ctx(1), move_cursor()),
        Err(RequestError::Execution(CadError::Invariant(_)))
    ));
    assert_eq!(queue.state.borrow().next_id, u64::MAX);
    assert_eq!(queue.queued(), 1, "the refused request was not admitted");
}

#[test]
fn coalesce_merges_only_adjacent_move_cursor_with_equal_context() {
    let queue = RequestQueue::new();
    assert_eq!(
        queue.submit(ctx(0), move_cursor()).unwrap().id,
        RequestId(0)
    );
    // Identical full context and adjacent tail: coalesce to the new id.
    assert_eq!(
        queue.submit(ctx(0), move_cursor()).unwrap().id,
        RequestId(1)
    );
    assert_eq!(queue.queued(), 1);
    assert_eq!(queue.outstanding(), 2, "superseded completion holds quota");

    let done = queue.poll_completions(8);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].id, RequestId(0));
    assert!(matches!(
        done[0].result,
        Err(RequestError::Superseded { by }) if by == RequestId(1)
    ));

    // A different context prevents coalescing.
    assert_eq!(
        queue.submit(ctx(1), move_cursor()).unwrap().id,
        RequestId(2)
    );
    assert_eq!(
        queue.submit(ctx(2), move_cursor()).unwrap().id,
        RequestId(3)
    );
    assert_eq!(queue.queued(), 3);

    // A non-MoveCursor tail prevents coalescing even with equal context.
    assert_eq!(queue.submit(ctx(3), confirm()).unwrap().id, RequestId(4));
    assert_eq!(
        queue.submit(ctx(2), move_cursor()).unwrap().id,
        RequestId(5)
    );
    assert_eq!(queue.queued(), 5);
    assert_eq!(queue.completion_count(), 0);
}

#[test]
fn allocate_sync_id_uses_the_checked_allocator_without_quota() {
    let queue = RequestQueue::new();
    assert_eq!(queue.allocate_sync_id().unwrap(), RequestId(0));
    assert_eq!(queue.allocate_sync_id().unwrap(), RequestId(1));
    // No quota, no completion, no pending work, no wake.
    assert_eq!(queue.outstanding(), 0);
    assert_eq!(queue.completion_count(), 0);
    assert!(!queue.pending());
    assert!(!queue.take_wake());
    // Sync ids share the allocator with queued submissions.
    assert_eq!(
        queue.submit(ctx(0), move_cursor()).unwrap().id,
        RequestId(2)
    );
}

#[test]
fn allocate_sync_id_is_busy_while_queued_or_executing() {
    let queue = RequestQueue::new();
    queue.submit(ctx(0), move_cursor()).unwrap();
    assert!(matches!(queue.allocate_sync_id(), Err(RequestError::Busy)));
    let exec = queue.begin().unwrap();
    assert!(matches!(queue.allocate_sync_id(), Err(RequestError::Busy)));
    exec.complete(moved());
    // An unclaimed completion alone does not block sync id allocation.
    assert_eq!(queue.allocate_sync_id().unwrap(), RequestId(1));
    assert_eq!(queue.outstanding(), 1, "the completion still holds quota");
}

#[test]
fn coalesce_requires_the_same_origin() {
    let queue = RequestQueue::new();
    assert_eq!(
        queue
            .submit_with_origin(RequestOrigin::Ui, ctx(0), move_cursor())
            .unwrap()
            .id,
        RequestId(0)
    );
    // Same context but a different origin must not merge.
    queue
        .submit_with_origin(RequestOrigin::Api, ctx(0), move_cursor())
        .unwrap();
    assert_eq!(queue.queued(), 2);
    assert_eq!(queue.completion_count(), 0);
}

#[test]
fn submit_with_origin_effects_reports_superseded_metadata_and_keeps_completion() {
    let queue = RequestQueue::new();
    let first = queue
        .submit_with_origin_effects(RequestOrigin::Ui, ctx(0), move_cursor())
        .unwrap();
    assert!(first.superseded.is_none());
    // A different origin must not coalesce.
    let second = queue
        .submit_with_origin_effects(RequestOrigin::Api, ctx(0), move_cursor())
        .unwrap();
    assert!(second.superseded.is_none());
    assert_eq!(queue.queued(), 2);

    // Same context and origin as the tail: coalesce reports the displaced
    // request's metadata with its original context and origin.
    let third = queue
        .submit_with_origin_effects(RequestOrigin::Api, ctx(0), move_cursor())
        .unwrap();
    let displaced = third
        .superseded
        .expect("coalesce reports the displaced request");
    assert_eq!(displaced.id, second.receipt.id);
    assert_eq!(displaced.by, third.receipt.id);
    assert_eq!(displaced.context, ctx(0));
    assert_eq!(displaced.origin, RequestOrigin::Api);

    // The displaced request's Superseded completion is still queued,
    // unclaimed, and counted against quota (never stolen by this call).
    assert_eq!(queue.queued(), 2, "the new request replaced the tail");
    assert_eq!(queue.completion_count(), 1);
    assert_eq!(queue.outstanding(), 3);
    let done = queue.poll_completions(8);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].id, displaced.id);
    assert!(matches!(
        done[0].result,
        Err(RequestError::Superseded { by }) if by == third.receipt.id
    ));
}

#[test]
fn submit_with_origin_effects_without_coalesce_has_no_superseded() {
    let queue = RequestQueue::new();
    let effects = queue
        .submit_with_origin_effects(RequestOrigin::Platform, ctx(0), move_cursor())
        .unwrap();
    assert!(effects.superseded.is_none());
    assert_eq!(effects.receipt.id, RequestId(0));
    assert_eq!(queue.completion_count(), 0);
    // The plain `submit_with_origin` shorthand returns the same receipt.
    let receipt = queue
        .submit_with_origin(RequestOrigin::Platform, ctx(1), move_cursor())
        .unwrap();
    assert_eq!(receipt.id, RequestId(1));
}

#[test]
fn submit_with_origin_effects_failed_admission_changes_nothing() {
    let queue = RequestQueue::with_limits(2, 1);
    queue.submit(ctx(0), move_cursor()).unwrap(); // id 0 fills the normal limit
    let next_before = queue.state.borrow().next_id;
    let tail_before = queue.state.borrow().queued.back().map(|entry| entry.id);
    let outstanding_before = queue.outstanding();

    // A coalesce-candidate submission at the normal boundary is rejected
    // atomically: no effects, no id, no tail mutation, no completion.
    assert!(matches!(
        queue.submit_with_origin_effects(RequestOrigin::Ui, ctx(0), move_cursor()),
        Err(RequestError::QueueFull)
    ));
    assert_eq!(queue.state.borrow().next_id, next_before);
    assert_eq!(
        queue.state.borrow().queued.back().map(|entry| entry.id),
        tail_before
    );
    assert_eq!(queue.outstanding(), outstanding_before);
    assert_eq!(queue.completion_count(), 0);
}

#[test]
fn coalesce_requires_every_context_field_to_match() {
    let base = ctx(0);
    // Each single-field change must break coalescing.
    let mut variants = Vec::new();
    let mut c = base;
    c.document = DocumentId(1);
    variants.push(c);
    let mut c = base;
    c.document_generation = 1;
    variants.push(c);
    let mut c = base;
    c.viewport = ViewportId(1);
    variants.push(c);
    let mut c = base;
    c.tool_generation = Some(2);
    variants.push(c);
    let mut c = base;
    c.metrics_revision = Some(2);
    variants.push(c);
    let mut c = base;
    c.config_revision = 1;
    variants.push(c);
    assert_eq!(variants.len(), 6, "every context field is covered");

    for changed in variants {
        let queue = RequestQueue::new();
        queue.submit(base, move_cursor()).unwrap();
        queue.submit(changed, move_cursor()).unwrap();
        assert_eq!(
            queue.queued(),
            2,
            "a changed context field must prevent coalescing"
        );
        assert_eq!(queue.completion_count(), 0);
    }

    // Identical full context does coalesce.
    let queue = RequestQueue::new();
    queue.submit(base, move_cursor()).unwrap();
    queue.submit(base, move_cursor()).unwrap();
    assert_eq!(queue.queued(), 1);
    assert_eq!(queue.completion_count(), 1);
}

#[test]
fn normal_limit_caps_continuous_coalescing_and_reserve_stays_sixteen() {
    let queue = RequestQueue::new();
    // Fill the normal quota with distinct contexts so nothing coalesces; the
    // last entry is a MoveCursor we then try to coalesce onto.
    for i in 0..240u64 {
        queue.submit(ctx(i), move_cursor()).unwrap();
    }
    assert_eq!(queue.outstanding(), 240);
    assert_eq!(queue.completion_count(), 0);
    let next_before = queue.state.borrow().next_id;
    let tail_before = queue.state.borrow().queued.back().map(|entry| entry.id);

    // A coalescing MoveCursor at the normal boundary is rejected atomically:
    // no id, no Superseded completion, unchanged tail.
    assert!(matches!(
        queue.submit(ctx(239), move_cursor()),
        Err(RequestError::QueueFull)
    ));
    assert_eq!(queue.outstanding(), 240);
    assert_eq!(queue.completion_count(), 0, "no superseded completion");
    assert_eq!(queue.state.borrow().next_id, next_before, "no id consumed");
    assert_eq!(
        queue.state.borrow().queued.back().map(|entry| entry.id),
        tail_before,
        "tail unchanged"
    );

    // The four reserved terminal kinds may still use the extra 16 up to 256.
    for _ in 0..16 {
        queue.submit(ctx(0), confirm()).unwrap();
    }
    assert_eq!(queue.outstanding(), 256);
    assert!(matches!(
        queue.submit(ctx(0), pointer_cancel()),
        Err(RequestError::QueueFull)
    ));
    assert_eq!(queue.outstanding(), 256);
}

#[test]
fn execution_guard_blocks_nested_work_and_dropping_records_failure() {
    let queue = RequestQueue::new();
    queue.submit(ctx(0), move_cursor()).unwrap();
    let mut exec = queue.begin().unwrap();
    assert_eq!(exec.id(), RequestId(0));

    // Nested work is refused while the guard is alive.
    assert!(matches!(
        queue.submit(ctx(1), move_cursor()),
        Err(RequestError::Busy)
    ));
    assert!(queue.begin().is_none());
    assert!(exec.take_request().is_some());

    // Dropping without a completion records an explicit invariant failure.
    drop(exec);
    assert_eq!(queue.outstanding(), 1);
    let done = queue.poll_completions(4);
    assert_eq!(done.len(), 1);
    assert!(matches!(
        done[0].result,
        Err(RequestError::Execution(CadError::Invariant(_)))
    ));
}

#[test]
fn unknown_or_non_running_completion_is_refused_without_mutation() {
    let queue = RequestQueue::new();
    queue.submit(ctx(0), move_cursor()).unwrap();
    // Not begun yet: completing the queued id must not fake success.
    assert!(matches!(
        queue.complete(RequestId(0), moved()),
        Err(RequestError::Execution(CadError::Invariant(_)))
    ));
    // An id that was never accepted is likewise refused.
    assert!(matches!(
        queue.complete(RequestId(99), moved()),
        Err(RequestError::Execution(CadError::Invariant(_)))
    ));
    assert_eq!(queue.outstanding(), 1);
    assert_eq!(queue.completion_count(), 0);
    assert_eq!(queue.executing_id(), None);
}

#[test]
fn pump_budget_uses_steps_and_injected_time_without_sleeping() {
    let mut budget = PumpBudget::new();
    for _ in 0..PUMP_STEPS {
        assert!(budget.allow(0.0));
        budget.record();
    }
    assert_eq!(budget.steps(), PUMP_STEPS);
    assert!(budget.is_exhausted(0.0), "step budget stops the pass");

    let mut timed = PumpBudget::with_limits(PUMP_STEPS, PUMP_MILLIS);
    assert!(timed.allow(PUMP_MILLIS - 0.1));
    assert!(!timed.allow(PUMP_MILLIS), "time budget stops the pass");
    timed.record();
    assert_eq!(timed.steps(), 1);
    assert!(timed.allow(PUMP_MILLIS - 0.1));
}

#[test]
fn wake_and_rearm_track_queued_work_only_never_completions() {
    let queue = RequestQueue::new();
    assert!(!queue.pending());
    assert!(!queue.take_wake());

    queue.submit(ctx(0), move_cursor()).unwrap();
    assert!(queue.pending());
    assert!(queue.take_wake(), "queued empty-to-nonempty wakes once");
    assert!(!queue.take_wake(), "the edge is consumed");

    // A submission into an already non-empty queue is not a new edge.
    queue.submit(ctx(1), move_cursor()).unwrap();
    assert!(!queue.take_wake());

    // Run both queued requests: now queued is empty and only completions
    // remain. Completion-only state is neither pending nor a wake source.
    let exec = queue.begin().unwrap();
    exec.complete(moved());
    let exec = queue.begin().unwrap();
    exec.complete(moved());
    assert_eq!(queue.queued(), 0);
    assert_eq!(queue.completion_count(), 2);
    assert!(!queue.pending(), "completions are not executable pending");
    assert!(!queue.take_wake(), "completion never wakes");
    assert!(
        !queue.rearm_wake(),
        "completion-only state must not re-arm the waker"
    );
    assert_eq!(
        queue.outstanding(),
        2,
        "unclaimed completions still hold quota"
    );

    // Queued work can re-arm even with completions present.
    queue.submit(ctx(2), move_cursor()).unwrap();
    assert!(queue.pending());
    assert!(queue.take_wake(), "queued empty-to-nonempty fires again");
    assert!(queue.rearm_wake());
}

#[test]
fn report_pending_is_queued_based_not_outstanding() {
    let queue = RequestQueue::new();
    queue.submit(ctx(0), move_cursor()).unwrap();
    let report = queue.report(0);
    assert_eq!(report.processed, 0);
    assert!(report.pending);

    let exec = queue.begin().unwrap();
    exec.complete(moved());
    assert_eq!(queue.queued(), 0);
    assert_eq!(queue.outstanding(), 1, "completion still holds quota");
    let report = queue.report(1);
    assert_eq!(report.processed, 1);
    assert!(
        !report.pending,
        "queued empty with completions outstanding reports pending=false"
    );
}
