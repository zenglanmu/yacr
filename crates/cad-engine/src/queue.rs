//! Pure owner-thread bounded request queue for the engine boundary.
//!
//! The queue owns admission, identity, ordering, quota and completion
//! bookkeeping only. It never executes CAD work and never interprets host
//! document/tool semantics: the host validates context and runs the request
//! through its own `dispatch_immediate`, driving this queue with
//! [`RequestQueue::begin`] and [`Executing::complete`].

use crate::protocol::{
    EngineRequest, PumpReport, RequestCompletion, RequestContext, RequestError, RequestId,
    RequestOrigin, RequestResult, SubmissionReceipt, SubmissionStatus,
};
use cad_app::interaction::{InteractionRequest, PointerPhase};
use cad_domain::CadError;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

/// Absolute outstanding quota: queued + executing + unclaimed completions.
pub const QUEUE_CAPACITY: usize = 256;
/// Normal (non-reserved) admission limit.
pub const NORMAL_LIMIT: usize = 240;
/// Reserved quota for interrupt/close-out interaction requests.
pub const RESERVE_LIMIT: usize = QUEUE_CAPACITY - NORMAL_LIMIT;
/// Default pump step budget for one owner-thread pass.
pub const PUMP_STEPS: usize = 64;
/// Default pump time budget (milliseconds) for one owner-thread pass.
pub const PUMP_MILLIS: f64 = 4.0;

/// Metadata of a request that a coalescing submission displaced.
///
/// Only identity/context metadata is carried: the displaced request's business
/// payload is not cloned (it is not `Clone` and must not be).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersededRequest {
    /// Engine id of the displaced request.
    pub id: RequestId,
    /// Engine id of the newer request that replaced it.
    pub by: RequestId,
    pub context: RequestContext,
    pub origin: RequestOrigin,
}

/// Result of an effects-aware submission.
///
/// `receipt` is the queued admission receipt. `superseded` is `Some` only when
/// the submission coalesced a previous request; the displaced request's
/// `Superseded` completion is still queued, unclaimed and counted against
/// quota (it is never stolen here). A rejected admission returns `Err`, so it
/// has no effects, allocates no id and mutates nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionEffects {
    pub receipt: SubmissionReceipt,
    pub superseded: Option<SupersededRequest>,
}

struct QueuedRequest {
    id: RequestId,
    origin: RequestOrigin,
    context: RequestContext,
    request: EngineRequest,
}

struct QueueState {
    next_id: u64,
    queued: VecDeque<QueuedRequest>,
    executing: Option<RequestId>,
    completions: VecDeque<RequestCompletion>,
}

impl QueueState {
    fn outstanding(&self) -> usize {
        self.queued.len() + usize::from(self.executing.is_some()) + self.completions.len()
    }
}

/// A request admitted for execution.
///
/// Owning it marks the queue as executing, so nested `submit`/`pump`/`dispatch`
/// are refused with `Busy`. Dropping it without a completion records an explicit
/// invariant failure, never a fake success.
pub struct Executing<'a> {
    queue: &'a RequestQueue,
    id: RequestId,
    origin: RequestOrigin,
    context: RequestContext,
    request: Option<EngineRequest>,
    finished: bool,
}

impl Executing<'_> {
    pub fn id(&self) -> RequestId {
        self.id
    }

    /// Trace-only request origin; not a permission.
    pub fn origin(&self) -> RequestOrigin {
        self.origin
    }

    pub fn context(&self) -> &RequestContext {
        &self.context
    }

    /// Take the owned request for execution. A second call returns `None` rather
    /// than fabricating a duplicate.
    pub fn take_request(&mut self) -> Option<EngineRequest> {
        self.request.take()
    }

    /// Publish the terminal completion and release the executing slot.
    pub fn complete(mut self, result: Result<RequestResult, RequestError>) {
        self.finished = true;
        let _ = self.queue.complete(self.id, result);
    }
}

impl Drop for Executing<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.queue.complete(
                self.id,
                Err(RequestError::Execution(CadError::Invariant(
                    "executing request dropped without a completion".into(),
                ))),
            );
        }
    }
}

/// Owner-thread bounded queue. Not `Send`: it lives entirely on the owner
/// thread and carries no cross-thread wakeup machinery.
pub struct RequestQueue {
    state: RefCell<QueueState>,
    capacity: usize,
    normal: usize,
    /// Empty-to-nonempty edge consumed by the host, which invokes its stored
    /// waker only after every queue borrow is released.
    wake_pending: Cell<bool>,
}

impl Default for RequestQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestQueue {
    /// A queue with the contract limits (capacity 256, normal 240, reserve 16).
    pub fn new() -> Self {
        Self::with_limits(QUEUE_CAPACITY, NORMAL_LIMIT)
    }

    /// A queue with explicit limits. `normal` may not exceed `capacity`.
    pub fn with_limits(capacity: usize, normal: usize) -> Self {
        assert!(normal <= capacity, "normal limit must not exceed capacity");
        Self {
            state: RefCell::new(QueueState {
                next_id: 0,
                queued: VecDeque::new(),
                executing: None,
                completions: VecDeque::new(),
            }),
            capacity,
            normal,
            wake_pending: Cell::new(false),
        }
    }

    /// Outstanding work: queued + executing + unclaimed completions.
    pub fn outstanding(&self) -> usize {
        self.state.borrow().outstanding()
    }

    /// Requests still waiting to begin execution.
    pub fn queued(&self) -> usize {
        self.state.borrow().queued.len()
    }

    /// The currently executing request, if any.
    pub fn executing_id(&self) -> Option<RequestId> {
        self.state.borrow().executing
    }

    /// Unclaimed terminal completions.
    pub fn completion_count(&self) -> usize {
        self.state.borrow().completions.len()
    }

    /// Alias for the executable-pending flag: whether requests are still queued
    /// to run. Unclaimed completions are not pending.
    pub fn pending(&self) -> bool {
        !self.state.borrow().queued.is_empty()
    }

    /// Build a `PumpReport` with the given processed count and the current
    /// executable-pending flag.
    pub fn report(&self, processed: usize) -> PumpReport {
        PumpReport {
            processed,
            pending: self.pending(),
        }
    }

    /// Admit a request with the default `Ui` origin.
    pub fn submit(
        &self,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        self.submit_with_origin(RequestOrigin::Ui, context, request)
    }

    /// Admit a request carrying a trace origin, returning just the receipt.
    ///
    /// API-compatible shorthand for [`RequestQueue::submit_with_origin_effects`].
    pub fn submit_with_origin(
        &self,
        origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        self.submit_with_origin_effects(origin, context, request)
            .map(|effects| effects.receipt)
    }

    /// Admit a request carrying a trace origin, returning the receipt plus the
    /// metadata of any request displaced by coalescing.
    ///
    /// Rejections (`QueueFull`, id exhaustion) happen before any mutation: they
    /// allocate no identity, change no state, produce no completion and return
    /// no effects. While an [`Executing`] guard is live the queue is busy and
    /// submission is refused with `Busy`.
    ///
    /// On a successful coalesce the displaced request's `Superseded` completion
    /// is still queued, unclaimed and counted against quota: this call never
    /// removes, polls or clones it. The business payload of the displaced
    /// request is not cloned; only its id/by/context/origin are reported.
    pub fn submit_with_origin_effects(
        &self,
        origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionEffects, RequestError> {
        let mut state = self.state.borrow_mut();
        if state.executing.is_some() {
            return Err(RequestError::Busy);
        }
        let outstanding = state.outstanding();
        // Coalesce only an adjacent tail MoveCursor with the identical full
        // context and the same origin (origin is trace metadata that must not be
        // overwritten). It replaces the tail and adds one Superseded completion.
        let coalesces = is_move_cursor(&request)
            && state.queued.back().is_some_and(|tail| {
                is_move_cursor(&tail.request) && tail.origin == origin && tail.context == context
            });
        if outstanding >= self.capacity {
            return Err(RequestError::QueueFull);
        }
        // The normal limit applies to every non-reserved acceptance, including a
        // coalescing MoveCursor: a coalesce still adds one Superseded completion
        // to the outstanding quota. Only the four reserved interrupt kinds may
        // use the extra 16 up to the absolute capacity.
        if !is_reserve_eligible(&request) && outstanding >= self.normal {
            return Err(RequestError::QueueFull);
        }
        // Identity is allocated only after every rejection check passed. The id
        // space is bounded: `u64::MAX` is the exhausted sentinel and is never
        // handed out, so exhaustion is an explicit error with no mutation (no
        // id, queue, completion, wake or counter change), never a wrap or panic.
        let Some(next_id) = state.next_id.checked_add(1) else {
            return Err(RequestError::Execution(CadError::Invariant(
                "request id space exhausted".into(),
            )));
        };
        let id = RequestId(state.next_id);
        state.next_id = next_id;
        let queued_was_empty = state.queued.is_empty();
        let superseded = if coalesces {
            let displaced = state.queued.pop_back().expect("coalesce tail present");
            state.queued.push_back(QueuedRequest {
                id,
                origin,
                context,
                request,
            });
            state.completions.push_back(RequestCompletion {
                id: displaced.id,
                result: Err(RequestError::Superseded { by: id }),
            });
            Some(SupersededRequest {
                id: displaced.id,
                by: id,
                context: displaced.context,
                origin: displaced.origin,
            })
        } else {
            state.queued.push_back(QueuedRequest {
                id,
                origin,
                context,
                request,
            });
            None
        };
        let queued_nonempty = !state.queued.is_empty();
        drop(state);
        if queued_was_empty && queued_nonempty {
            self.wake_pending.set(true);
        }
        Ok(SubmissionEffects {
            receipt: SubmissionReceipt {
                id,
                status: SubmissionStatus::Queued,
            },
            superseded,
        })
    }

    /// Begin the next queued request. Returns `None` when the queue is empty or
    /// already executing; the caller must finish the guard exactly once.
    pub fn begin(&self) -> Option<Executing<'_>> {
        let mut state = self.state.borrow_mut();
        if state.executing.is_some() {
            return None;
        }
        let next = state.queued.pop_front()?;
        state.executing = Some(next.id);
        drop(state);
        Some(Executing {
            queue: self,
            id: next.id,
            origin: next.origin,
            context: next.context,
            request: Some(next.request),
            finished: false,
        })
    }

    /// Allocate an engine id for a synchronous request that never enters the
    /// queue. It shares the checked allocator with [`RequestQueue::submit`] but
    /// consumes no quota, produces no completion and never wakes.
    ///
    /// It is `Busy` while executable work is queued or executing; unclaimed
    /// completions do not make it busy. Exhaustion is an explicit error and
    /// leaves the counter unchanged (rejected requests allocate no id).
    pub fn allocate_sync_id(&self) -> Result<RequestId, RequestError> {
        let mut state = self.state.borrow_mut();
        if !state.queued.is_empty() || state.executing.is_some() {
            return Err(RequestError::Busy);
        }
        let Some(next_id) = state.next_id.checked_add(1) else {
            return Err(RequestError::Execution(CadError::Invariant(
                "request id space exhausted".into(),
            )));
        };
        let id = RequestId(state.next_id);
        state.next_id = next_id;
        Ok(id)
    }

    /// Publish a completion for the currently executing request.
    ///
    /// An unknown or non-executing `id` is refused with an explicit invariant
    /// error and mutates nothing: a completion never fakes success for a
    /// request that was not running.
    pub fn complete(
        &self,
        id: RequestId,
        result: Result<RequestResult, RequestError>,
    ) -> Result<(), RequestError> {
        let mut state = self.state.borrow_mut();
        if state.executing != Some(id) {
            return Err(RequestError::Execution(CadError::Invariant(
                "completion for a request that is not executing".into(),
            )));
        }
        state.executing = None;
        state
            .completions
            .push_back(RequestCompletion { id, result });
        Ok(())
    }

    /// Take at most `limit` terminal completions, freeing their quota.
    pub fn poll_completions(&self, limit: usize) -> Vec<RequestCompletion> {
        let mut state = self.state.borrow_mut();
        let take = limit.min(state.completions.len());
        state.completions.drain(..take).collect()
    }

    /// Consume the empty-to-nonempty edge. The host invokes its stored waker
    /// only after this returns `true`, and only with all borrows released.
    pub fn take_wake(&self) -> bool {
        self.wake_pending.replace(false)
    }

    /// Re-arm the wake edge while executable work is still queued, so a busy
    /// host is notified again. Unclaimed completions do not re-arm. Returns
    /// whether the edge was re-armed.
    pub fn rearm_wake(&self) -> bool {
        let pending = self.pending();
        if pending {
            self.wake_pending.set(true);
        }
        pending
    }
}

/// The reserved quota is usable only by interrupt/close-out interaction
/// requests. `InteractionRequest::Cancel` and `PointerPhase::Cancel` are
/// distinct kinds; both are reserved, together with `Confirm` and a `Pointer`
/// `Up` phase.
fn is_reserve_eligible(request: &EngineRequest) -> bool {
    matches!(
        request,
        EngineRequest::Interaction(InteractionRequest::Confirm)
            | EngineRequest::Interaction(InteractionRequest::Cancel)
            | EngineRequest::Interaction(InteractionRequest::Pointer {
                phase: PointerPhase::Up,
                ..
            })
            | EngineRequest::Interaction(InteractionRequest::Pointer {
                phase: PointerPhase::Cancel,
                ..
            })
    )
}

fn is_move_cursor(request: &EngineRequest) -> bool {
    matches!(
        request,
        EngineRequest::Interaction(InteractionRequest::MoveCursor(_))
    )
}

/// Step-and-time pump budget. Time is supplied by the caller so this helper
/// stays usable on wasm32 (where `Instant` is unavailable) and deterministic in
/// tests (no sleeping).
#[derive(Debug, Clone, Copy)]
pub struct PumpBudget {
    max_steps: usize,
    max_millis: f64,
    steps: usize,
}

impl Default for PumpBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl PumpBudget {
    /// The contract default: 64 steps or 4 ms.
    pub fn new() -> Self {
        Self {
            max_steps: PUMP_STEPS,
            max_millis: PUMP_MILLIS,
            steps: 0,
        }
    }

    /// Explicit step/time budget.
    pub fn with_limits(max_steps: usize, max_millis: f64) -> Self {
        Self {
            max_steps,
            max_millis,
            steps: 0,
        }
    }

    /// Whether one more step may run, given `elapsed_millis` since pump start.
    pub fn allow(&self, elapsed_millis: f64) -> bool {
        self.steps < self.max_steps && elapsed_millis < self.max_millis
    }

    /// Record one executed step.
    pub fn record(&mut self) {
        self.steps = self.steps.saturating_add(1);
    }

    /// Steps recorded so far.
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// Whether the budget is exhausted for the given elapsed time.
    pub fn is_exhausted(&self, elapsed_millis: f64) -> bool {
        !self.allow(elapsed_millis)
    }
}

/// Monotonic pump clock for native hosts. Not compiled on wasm32, where
/// `Instant::now` is unavailable; wasm hosts supply elapsed from their own
/// platform clock.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct PumpClock {
    start: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl PumpClock {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }

    /// Milliseconds since this clock was created.
    pub fn elapsed_millis(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for PumpClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
}
