//! P4: origin-tagged requests, controlled API routing and bounded event tracing.
//!
//! The host drives the engine's bounded event ring: each accepted request owns a
//! `TracePermit` (reserved before any id/queue mutation), executes through the
//! same synchronous funnel as P3, and publishes an ordered, bounded trace. The
//! ring never deep-copies a `ChangeSet`: every payload stays behind its `Arc`.
use super::*;
use crate::input::admissible_for_submit;
use cad_db::ChangeSet;
use cad_domain::{Revision, TransactionId};
use cad_engine::events::{
    EngineEvent, EventEnvelope, EventPoll, EventWatermark, RequestKind, SubscriptionId, ToolKind,
    TracePermit, TRACE_ALLOWANCE,
};
use cad_engine::protocol::RequestId;
use std::collections::BTreeMap;

/// Maximum events one request may publish: RequestStarted + at most one
/// command pair + EntitiesChanged + SelectionChanged + ToolChanged + Finished.
const REQUEST_EVENT_BOUND: u64 = 7;

/// Identity of one traced request, grouped to keep signatures small.
struct TraceHeader {
    id: RequestId,
    origin: RequestOrigin,
    kind: RequestKind,
}

/// A pre- or post-execution snapshot: authoritative context plus the drawing
/// revision at that moment. `Started`/`CommandStarted` use the pre snapshot;
/// terminals and derived diffs use the post snapshot.
#[derive(Clone, Copy)]
pub(crate) struct TraceState {
    pub context: RequestContext,
    pub revision: Revision,
}

/// One executed request: its real result, its ordered events and the pre/post
/// snapshots used to place each event on the correct document state.
pub(crate) struct EngineExecution {
    pub result: Result<RequestResult, RequestError>,
    pub events: Vec<EngineEvent>,
    pub pre: TraceState,
    pub post: TraceState,
}

/// Reserved trace bookkeeping for one accepted (queued) request.
pub(crate) struct PermitEntry {
    pub permit: TracePermit,
    pub origin: RequestOrigin,
    /// The context captured at submit; a superseded request's terminal envelope
    /// must carry its own (old) context, not the current one.
    pub context: RequestContext,
}

/// Minimal tool classification consistent with the app tool state.
fn tool_kind(state: &ToolState) -> ToolKind {
    match state {
        ToolState::Idle => ToolKind::Idle,
        ToolState::Selecting => ToolKind::Selecting,
        ToolState::Drawing(_) => ToolKind::Drawing,
        ToolState::Measuring(_) => ToolKind::Measuring,
        ToolState::Panning => ToolKind::Panning,
    }
}

impl HostCore {
    fn current_revision(&self) -> Revision {
        self.controller
            .borrow()
            .drawing()
            .map(|drawing| drawing.revision())
            .unwrap_or(Revision(0))
    }

    /// Pre/post execution snapshot for one request.
    fn trace_state(&self) -> TraceState {
        TraceState {
            context: self.request_context(),
            revision: self.current_revision(),
        }
    }

    fn envelope(
        &self,
        context: &RequestContext,
        revision: Revision,
        request_id: Option<RequestId>,
        origin: RequestOrigin,
        transaction: Option<TransactionId>,
        event: EngineEvent,
    ) -> EventEnvelope {
        EventEnvelope {
            sequence: 0,
            request_id,
            origin,
            document: context.document,
            generation: context.document_generation,
            revision,
            transaction,
            event,
        }
    }

    /// Selection/tool diff events for the request that just ran, using the real
    /// session revisions (never fabricated).
    fn changed_events(&self) -> Vec<EngineEvent> {
        let mut events = Vec::new();
        let refs = self.controller.borrow().session.selection.refs().to_vec();
        {
            let mut last = self.last_selection.borrow_mut();
            if *last != refs {
                *last = refs.clone();
                let revision = self.selection_revision.get().saturating_add(1);
                self.selection_revision.set(revision);
                events.push(EngineEvent::SelectionChanged { revision, refs });
            }
        }
        let generation = self.controller.borrow().session.tool_generation;
        if generation != self.last_tool_generation.get() {
            self.last_tool_generation.set(generation);
            let kind = tool_kind(&self.controller.borrow().session.tool);
            events.push(EngineEvent::ToolChanged { generation, kind });
        }
        events
    }

    /// Route a controlled API request through the application's real transaction
    /// path (never a raw database write).
    fn run_api(&self, target: ApiTarget, request: ApiRequest) -> CadResult<ApiOutcome> {
        let mut guard = self.controller.borrow_mut();
        let HostController {
            application,
            session,
            ..
        } = &mut *guard;
        application.execute_api(session, target, request)
    }

    fn transaction_of(result: &Result<RequestResult, RequestError>) -> Option<TransactionId> {
        match result {
            Ok(RequestResult::Command(outcome)) => {
                outcome.changes.as_ref().map(|changes| changes.transaction)
            }
            Ok(RequestResult::Api(outcome)) => {
                outcome.changes.as_ref().map(|changes| changes.transaction)
            }
            _ => None,
        }
    }

    /// Execute one request through the host funnel and collect its trace.
    ///
    /// The returned [`EngineExecution`] carries the ordered events plus the
    /// pre/post snapshots. Entity changes are harvested from **every** successful
    /// raw command trace (so Confirm via the outer `Interaction` still surfaces
    /// its `ChangeSet`) as well as the outer command/API outcome, de-duplicated by
    /// `TransactionId` with the original `Arc` preserved (never re-created).
    pub(crate) fn execute_engine_request(&self, request: EngineRequest) -> EngineExecution {
        let pre = self.trace_state();
        self.pending_trace.borrow_mut().clear();
        let result = match request {
            EngineRequest::Command(command) => self
                .execute_with_outcome(command)
                .map(|outcome| {
                    RequestResult::Command(
                        outcome.unwrap_or_else(|| Arc::new(CommandOutcome::none())),
                    )
                })
                .map_err(RequestError::Execution),
            EngineRequest::Interaction(request) => self
                .dispatch_interaction(request)
                .map(RequestResult::Interaction)
                .map_err(RequestError::Execution),
            EngineRequest::Api { target, request } => self
                .run_api(target, request)
                .map(|outcome| RequestResult::Api(Arc::new(outcome)))
                .map_err(RequestError::Execution),
        };
        let traces = std::mem::take(&mut *self.pending_trace.borrow_mut());

        let mut events = Vec::new();
        let mut changes: Vec<Arc<ChangeSet>> = Vec::new();
        for trace in &traces {
            events.push(EngineEvent::CommandStarted {
                command: trace.command,
            });
            // The trace's own `Arc<CommandOutcome>` (same Arc the outer completion
            // shares when the command is the outer request).
            events.push(EngineEvent::CommandFinished {
                command: trace.command,
                result: Arc::new(trace.result.clone()),
            });
            if let Ok(outcome) = &trace.result {
                if let Some(change_set) = &outcome.changes {
                    changes.push(change_set.clone());
                }
            }
        }
        match &result {
            Ok(RequestResult::Command(outcome)) => {
                if let Some(change_set) = &outcome.changes {
                    changes.push(change_set.clone());
                }
            }
            Ok(RequestResult::Api(outcome)) => {
                if let Some(change_set) = &outcome.changes {
                    changes.push(change_set.clone());
                }
            }
            _ => {}
        }
        let mut seen: Vec<TransactionId> = Vec::new();
        for change_set in changes {
            if !seen.contains(&change_set.transaction) {
                seen.push(change_set.transaction);
                events.push(EngineEvent::EntitiesChanged {
                    changes: change_set,
                });
            }
        }
        events.extend(self.changed_events());
        let post = self.trace_state();
        EngineExecution {
            result,
            events,
            pre,
            post,
        }
    }

    fn report_event_failure(&self, error: &RequestError) {
        // Never silent: a trace publish anomaly is surfaced as an explicit
        // diagnostic while the already-executed completion is preserved.
        let _ = self.set_external_status(format!("event trace publish failed: {error:?}"));
    }

    /// Publish the ordered trace for one executed request through `permit`.
    ///
    /// `RequestStarted`/`CommandStarted` carry the pre-execution document state;
    /// `EntitiesChanged` its real `changes.after`/transaction; every other
    /// terminal event (and `RequestFinished`) the post-execution state.
    fn publish_request(
        &self,
        permit: &mut TracePermit,
        header: &TraceHeader,
        exec: EngineExecution,
    ) {
        let transaction = Self::transaction_of(&exec.result);
        let mut envelopes = Vec::with_capacity(exec.events.len() + 2);
        envelopes.push(self.envelope(
            &exec.pre.context,
            exec.pre.revision,
            Some(header.id),
            header.origin,
            transaction,
            EngineEvent::RequestStarted {
                classification: header.kind,
            },
        ));
        for event in exec.events {
            let (context, revision, event_transaction) = match &event {
                EngineEvent::CommandStarted { .. } => {
                    (&exec.pre.context, exec.pre.revision, transaction)
                }
                EngineEvent::EntitiesChanged { changes } => {
                    (&exec.post.context, changes.after, Some(changes.transaction))
                }
                _ => (&exec.post.context, exec.post.revision, transaction),
            };
            envelopes.push(self.envelope(
                context,
                revision,
                Some(header.id),
                header.origin,
                event_transaction,
                event,
            ));
        }
        let shared: SharedRequestResult = Arc::new(exec.result.clone());
        envelopes.push(self.envelope(
            &exec.post.context,
            exec.post.revision,
            Some(header.id),
            header.origin,
            transaction,
            EngineEvent::RequestFinished { result: shared },
        ));
        if let Err(error) = permit.publish_batch(envelopes) {
            self.report_event_failure(&error);
        }
    }

    /// Publish only the terminal `RequestFinished` (no `RequestStarted`).
    ///
    /// A stale queued request was never admitted for real execution and a
    /// superseded request never ran: both publish a lone `Finished` using their
    /// original permit and their captured (old) context.
    pub(crate) fn publish_finished_only(
        &self,
        permit: &mut TracePermit,
        origin: RequestOrigin,
        context: &RequestContext,
        id: RequestId,
        result: &Result<RequestResult, RequestError>,
    ) {
        let transaction = Self::transaction_of(result);
        let shared: SharedRequestResult = Arc::new(result.clone());
        let envelope = self.envelope(
            context,
            self.current_revision(),
            Some(id),
            origin,
            transaction,
            EngineEvent::RequestFinished { result: shared },
        );
        if let Err(error) = permit.publish(envelope) {
            self.report_event_failure(&error);
        }
    }

    // --- EngineClient P4 backends -------------------------------------------

    pub(crate) fn dispatch_with_origin_impl(
        &self,
        origin: RequestOrigin,
        request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError> {
        let _guard = self.begin_sync_request()?;
        // Reserve the trace before allocating identity: a rejection releases the
        // permit with no id and no side effect.
        let mut permit = self.events.reserve_trace(TRACE_ALLOWANCE)?;
        self.require_request_event_headroom(&permit)?;
        let id = self.queue.allocate_sync_id()?;
        let header = TraceHeader {
            id,
            origin,
            kind: request.kind(),
        };
        let exec = self.execute_engine_request(request);
        let result = exec.result.clone();
        self.publish_request(&mut permit, &header, exec);
        Ok(RequestCompletion { id, result })
    }

    /// Prove the fixed per-request event bound fits the reserved allowance before
    /// any business runs, so a committed batch is never truncated.
    fn require_request_event_headroom(&self, permit: &TracePermit) -> Result<(), RequestError> {
        if permit.remaining() < REQUEST_EVENT_BOUND {
            return Err(RequestError::Execution(CadError::Invariant(
                "trace allowance is below the request event bound".into(),
            )));
        }
        Ok(())
    }

    /// Execute one already-admitted queued request: confirm its permit **before**
    /// business, run the traced business, then publish and complete.
    ///
    /// A missing permit is an explicit invariant refusal that never runs the
    /// business and never re-reserves after the fact.
    pub(crate) fn run_queued(
        &self,
        executing: cad_engine::queue::Executing<'_>,
        origin: RequestOrigin,
        request: EngineRequest,
    ) {
        let id = executing.id();
        let kind = request.kind();
        let Some(entry) = self.permits.borrow_mut().remove(&id) else {
            executing.complete(Err(RequestError::Execution(CadError::Invariant(
                "queued request had no reserved trace permit".into(),
            ))));
            return;
        };
        let mut permit = entry.permit;
        if let Err(error) = self.require_request_event_headroom(&permit) {
            executing.complete(Err(error));
            return;
        }
        let header = TraceHeader { id, origin, kind };
        let exec = self.execute_engine_request(request);
        let result = exec.result.clone();
        self.publish_request(&mut permit, &header, exec);
        executing.complete(result);
    }

    /// Complete a stale queued request: publish only its terminal `Finished`
    /// (no `RequestStarted`) with its old context and permit.
    pub(crate) fn finish_queued_stale(
        &self,
        executing: cad_engine::queue::Executing<'_>,
        origin: RequestOrigin,
        context: RequestContext,
    ) {
        let id = executing.id();
        if let Some(entry) = self.permits.borrow_mut().remove(&id) {
            let mut permit = entry.permit;
            self.publish_finished_only(
                &mut permit,
                origin,
                &context,
                id,
                &Err(RequestError::StaleContext),
            );
        }
        executing.complete(Err(RequestError::StaleContext));
    }

    pub(crate) fn submit_with_origin_impl(
        &self,
        origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        if self.request_engine_busy() {
            return Err(RequestError::Busy);
        }
        if !admissible_for_submit(&request) {
            return Err(RequestError::Execution(CadError::InvalidInput(
                "request requires the synchronous gesture path".into(),
            )));
        }
        // Reserve before the queue mutation; a rejected submission releases it
        // with no id and no mutation.
        let permit = self.events.reserve_trace(TRACE_ALLOWANCE)?;
        match self
            .queue
            .submit_with_origin_effects(origin, context, request)
        {
            Ok(effects) => {
                self.permits.borrow_mut().insert(
                    effects.receipt.id,
                    PermitEntry {
                        permit,
                        origin,
                        context,
                    },
                );
                // A coalesced/displaced request never executed: publish its lone
                // `Finished` immediately with its OLD context, then drop the
                // permit (the real `Superseded` completion stays in the queue,
                // unclaimed, never stolen).
                if let Some(displaced) = effects.superseded {
                    if let Some(entry) = self.permits.borrow_mut().remove(&displaced.id) {
                        let PermitEntry {
                            mut permit,
                            origin,
                            context,
                        } = entry;
                        self.publish_finished_only(
                            &mut permit,
                            origin,
                            &context,
                            displaced.id,
                            &Err(RequestError::Superseded { by: displaced.by }),
                        );
                    }
                }
                if self.queue.take_wake() {
                    self.invoke_request_waker();
                }
                Ok(effects.receipt)
            }
            Err(error) => {
                drop(permit);
                Err(error)
            }
        }
    }

    /// Take terminal completions, freeing quota. Executed requests removed their
    /// permit at completion and displaced requests published theirs at submit, so
    /// no trace work remains here (this never steals a completion).
    pub(crate) fn poll_completions_impl(&self, limit: usize) -> Vec<RequestCompletion> {
        self.queue.poll_completions(limit)
    }

    pub(crate) fn subscribe_impl(&self) -> CadResult<SubscriptionId> {
        self.events.subscribe()
    }

    pub(crate) fn poll_events_impl(
        &self,
        id: SubscriptionId,
        limit: usize,
    ) -> CadResult<EventPoll> {
        self.events.poll_events(id, limit)
    }

    pub(crate) fn unsubscribe_impl(&self, id: SubscriptionId) -> CadResult<()> {
        self.events.unsubscribe(id)
    }

    pub(crate) fn event_watermark_impl(&self) -> EventWatermark {
        self.events.watermark()
    }

    /// Bridge for an external backend action (Web) that must run as a real,
    /// traced `SwitchBackend` command.
    ///
    /// Preflight validates the variant/payload/document/viewport/permission with
    /// no side effect; only then does it take the sync guard, reserve a trace and
    /// allocate a shared id, release every borrow, and invoke `action`. The
    /// action's result becomes the request's real completion (same `Arc` on
    /// success, a real-id failure completion otherwise) and drives a real
    /// `CommandStarted`/`CommandFinished` pair. This is only for `SwitchBackend`;
    /// it is not a general external-request classification and never nests
    /// `with_sync_request`.
    pub fn dispatch_external_with_origin(
        &self,
        origin: RequestOrigin,
        request: EngineRequest,
        action: impl FnOnce() -> Result<Arc<CommandOutcome>, RequestError>,
    ) -> Result<RequestCompletion, RequestError> {
        let EngineRequest::Command(command) = &request else {
            return Err(RequestError::Execution(CadError::InvalidInput(
                "dispatch_external_with_origin accepts a SwitchBackend command only".into(),
            )));
        };
        if command.id != CommandId::SwitchBackend {
            return Err(RequestError::Execution(CadError::InvalidInput(
                "dispatch_external_with_origin accepts a SwitchBackend command only".into(),
            )));
        }
        if command.schema_version != 1 {
            return Err(RequestError::Execution(CadError::InvalidInput(
                "unsupported command schema version".into(),
            )));
        }
        // The real cross-layer request carries the backend choice; a `None`
        // compatibility shim is refused (no fake bypass).
        if !matches!(command.payload, CommandPayload::Backend(_)) {
            return Err(RequestError::Execution(CadError::InvalidInput(
                "SwitchBackend requires a Backend(choice) payload".into(),
            )));
        }
        let context = self.request_context();
        if command.document != context.document || command.viewport != context.viewport {
            return Err(RequestError::StaleContext);
        }
        // Host-authoritative permission, with the borrow released before action.
        {
            let c = self.controller.borrow();
            c.session
                .authorize(CommandId::SwitchBackend)
                .map_err(RequestError::Execution)?;
        }

        let _guard = self.begin_sync_request()?;
        let mut permit = self.events.reserve_trace(TRACE_ALLOWANCE)?;
        self.require_request_event_headroom(&permit)?;
        let id = self.queue.allocate_sync_id()?;
        let pre = self.trace_state();

        // No internal borrow is held here: the external action runs freely.
        let action_result = action();
        let mut events = vec![EngineEvent::CommandStarted {
            command: CommandId::SwitchBackend,
        }];
        let finished: Arc<Result<Arc<CommandOutcome>, CadError>> = match &action_result {
            Ok(outcome) => Arc::new(Ok(outcome.clone())),
            Err(error) => Arc::new(Err(request_error_to_cad(error.clone()))),
        };
        events.push(EngineEvent::CommandFinished {
            command: CommandId::SwitchBackend,
            result: finished,
        });
        if let Ok(outcome) = &action_result {
            if let Some(changes) = &outcome.changes {
                events.push(EngineEvent::EntitiesChanged {
                    changes: changes.clone(),
                });
            }
        }
        events.extend(self.changed_events());
        let result: Result<RequestResult, RequestError> = action_result.map(RequestResult::Command);
        let post = self.trace_state();
        let exec = EngineExecution {
            result,
            events,
            pre,
            post,
        };
        let result = exec.result.clone();
        let header = TraceHeader {
            id,
            origin,
            kind: RequestKind::Command,
        };
        self.publish_request(&mut permit, &header, exec);
        Ok(RequestCompletion { id, result })
    }
}

/// Map a refusal/failure back to a `CadError` for a command-event payload.
pub(crate) fn request_error_to_cad(error: RequestError) -> CadError {
    match error {
        RequestError::Execution(error) => error,
        RequestError::Cancelled => CadError::Cancelled,
        RequestError::StaleContext => CadError::StaleResult,
        RequestError::Busy => CadError::InvalidInput("engine is busy".into()),
        RequestError::QueueFull => CadError::InvalidInput("request queue is full".into()),
        RequestError::Superseded { .. } => CadError::Cancelled,
    }
}

/// Default-constructible permit map alias for the host field.
pub(crate) type PermitMap = BTreeMap<RequestId, PermitEntry>;
