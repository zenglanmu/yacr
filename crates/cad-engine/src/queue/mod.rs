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

mod pump;
pub use pump::PumpBudget;
#[cfg(not(target_arch = "wasm32"))]
pub use pump::PumpClock;

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

#[cfg(test)]
mod tests;
