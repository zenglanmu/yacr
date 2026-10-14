#![cfg(target_os = "linux")]
//! P3 queue UI contracts against the real Slint component.
//!
//! One test owns the process-wide offscreen platform. The mock owns a real
//! `cad_engine::queue::RequestQueue` so the UI's `submit`/`pump`/`poll` ordering,
//! coalescing, `Busy` barrier, completion drain and waker rearm are exercised
//! against the frozen queue semantics rather than a fake no-op.
use cad_app::interaction::{InteractionRequest, PointInput};
use cad_app::{
    AppMode, CanvasMetrics, CommandOutcome, DrawPreview, DrawToolKind, HistoryAvailability,
    MeasurementToolKind, SelectionProperties,
};
use cad_domain::{CadError, CadResult, DocumentId, SpaceId, ViewportId};
use cad_engine::events::{EventPoll, EventWatermark, SubscriptionId};
use cad_engine::protocol::{
    ConfigSnapshot, EngineClient, EngineRequest, HostCapabilitiesState, HostState, HostUpdates,
    InteractionOutcome, PumpReport, RequestCompletion, RequestContext, RequestError, RequestOrigin,
    RequestResult, ResourceState, SubmissionReceipt,
};
use cad_engine::queue::{RequestQueue, PUMP_STEPS};
use cad_engine::render::RenderSessionHandle;
use cad_render_wgpu::BackendPreference;
use cad_ui_slint::{
    install_existing_cad_bridge, offscreen, EngineAdapter, UiAdapter, UiCommandSink,
    UiConfiguration,
};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExecMode {
    Ok,
    Stale,
    Invalid,
}

/// How one pump pass behaves, so a real clock/budget failure can be modelled
/// without faking completions for requests that never executed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PumpMode {
    /// Execute every queued request and report its real progress.
    Drain,
    /// Execute every queued request, then fail the pass (clock budget expired).
    ErrorAfterDrain,
    /// Execute exactly one queued request, then fail the pass; the remaining
    /// queued work is preserved (never turned into fake completions).
    ErrorAfterFirst,
}

/// Queue-backed owner-thread client. `dispatch` executes immediately (the sync
/// path); `submit`/`pump`/`poll_completions` use the real bounded queue.
struct QueueMockClient {
    session: RenderSessionHandle,
    queue: RequestQueue,
    context: Cell<RequestContext>,
    executed: RefCell<Vec<EngineRequest>>,
    updates: RefCell<HostUpdates>,
    config: RefCell<ConfigSnapshot>,
    exec_mode: Cell<ExecMode>,
    pump_mode: Cell<PumpMode>,
    pump_blocked: Cell<bool>,
    /// Real tool-capture state: `BeginDraw` starts it, `Cancel` clears it, so a
    /// "capture cleared" assertion checks genuine state, not a counter.
    capture_active: Cell<bool>,
    /// Count of calls through the explicit-origin variants. The UI must stay on
    /// the `Ui`-origin shorthands, so this stays zero.
    origin_variant_calls: Cell<usize>,
    waker: RefCell<Option<Rc<dyn Fn()>>>,
}

impl QueueMockClient {
    fn new(session: RenderSessionHandle, context: RequestContext) -> Self {
        Self {
            session,
            // Small limits so completion backpressure is reachable in a test.
            queue: RequestQueue::with_limits(2, 2),
            context: Cell::new(context),
            executed: RefCell::new(Vec::new()),
            updates: RefCell::new(HostUpdates::default()),
            config: RefCell::new(ConfigSnapshot {
                effective: cad_app::viewer_config::ViewerConfig::default(),
                revision: 0,
                preset: cad_app::viewer_config::Preset::Full,
                effective_json: "{}".into(),
            }),
            exec_mode: Cell::new(ExecMode::Ok),
            pump_mode: Cell::new(PumpMode::Drain),
            pump_blocked: Cell::new(false),
            capture_active: Cell::new(false),
            origin_variant_calls: Cell::new(0),
            waker: RefCell::new(None),
        }
    }

    /// Execute at most one queued request; returns whether one ran.
    fn execute_one(&self) -> bool {
        let Some(mut executing) = self.queue.begin() else {
            return false;
        };
        let request = executing.take_request();
        let result = match &request {
            Some(request) => self.result_for(request),
            None => Err(RequestError::Execution(CadError::Invariant(
                "executing slot without a request".into(),
            ))),
        };
        if let Some(request) = request {
            self.executed.borrow_mut().push(request);
        }
        executing.complete(result);
        true
    }

    fn drain_all(&self) -> usize {
        let mut processed = 0usize;
        while processed < PUMP_STEPS && self.execute_one() {
            processed += 1;
        }
        processed
    }

    fn result_for(&self, request: &EngineRequest) -> Result<RequestResult, RequestError> {
        // `BeginDraw`/`Cancel` always drive the real capture state, independent of
        // the injected failure mode, so a failed cursor can be followed by a
        // genuine Cancel that clears the capture.
        match request {
            EngineRequest::Interaction(InteractionRequest::BeginDraw(_)) => {
                self.capture_active.set(true);
                return Ok(RequestResult::Interaction(InteractionOutcome::ToolStarted));
            }
            EngineRequest::Interaction(InteractionRequest::Cancel) => {
                self.capture_active.set(false);
                return Ok(RequestResult::Interaction(InteractionOutcome::Cancelled));
            }
            _ => {}
        }
        match self.exec_mode.get() {
            ExecMode::Ok => match request {
                EngineRequest::Command(_) => {
                    Ok(RequestResult::Command(Arc::new(CommandOutcome::none())))
                }
                EngineRequest::Interaction(_) => {
                    Ok(RequestResult::Interaction(InteractionOutcome::CursorMoved))
                }
                // This mock owns no controlled-API surface: refuse honestly
                // instead of fabricating an API success.
                EngineRequest::Api { .. } => Err(RequestError::Execution(CadError::Unsupported(
                    "mock client does not implement the api request".into(),
                ))),
            },
            ExecMode::Stale => Err(RequestError::StaleContext),
            ExecMode::Invalid => Err(RequestError::Execution(CadError::InvalidInput(
                "mock execution failure".into(),
            ))),
        }
    }
}

impl EngineClient for QueueMockClient {
    fn dispatch(&self, request: EngineRequest) -> Result<RequestResult, RequestError> {
        let result = self.result_for(&request);
        self.executed.borrow_mut().push(request);
        result
    }

    fn request_context(&self) -> RequestContext {
        self.context.get()
    }

    fn take_updates(&self) -> CadResult<HostUpdates> {
        Ok(std::mem::take(&mut *self.updates.borrow_mut()))
    }

    fn set_canvas_metrics(&self, metrics: CanvasMetrics) -> CadResult<()> {
        if metrics.is_valid() {
            Ok(())
        } else {
            Err(CadError::InvalidInput("invalid canvas metrics".into()))
        }
    }

    fn render_session(&self) -> RenderSessionHandle {
        self.session.clone()
    }

    fn config_snapshot(&self) -> ConfigSnapshot {
        self.config.borrow().clone()
    }

    fn set_config_json(&self, _json: &str) -> CadResult<()> {
        self.config.borrow_mut().revision += 1;
        Ok(())
    }

    fn update_config_json(&self, _json: &str) -> CadResult<()> {
        self.config.borrow_mut().revision += 1;
        Ok(())
    }

    fn apply_user_preference_json(&self, _json: &str) -> CadResult<()> {
        self.config.borrow_mut().revision += 1;
        Ok(())
    }

    fn clear_user_preference(&self) -> CadResult<()> {
        Ok(())
    }

    fn submit(
        &self,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        let receipt = self.queue.submit(context, request)?;
        // Wake only on the queue's empty→nonempty edge, and only after the queue
        // borrow has been released (no synchronous pump, no reentrancy).
        if self.queue.take_wake() {
            let waker = self.waker.borrow().as_ref().cloned();
            if let Some(waker) = waker {
                waker();
            }
        }
        Ok(receipt)
    }

    fn pump(&self) -> Result<PumpReport, RequestError> {
        if self.pump_blocked.get() {
            // Simulate a budget-exhausted pass with executable work still queued.
            return Ok(PumpReport {
                processed: 0,
                pending: true,
            });
        }
        let clock_error =
            || RequestError::Execution(CadError::Invariant("mock pump clock expired".into()));
        match self.pump_mode.get() {
            PumpMode::Drain => {
                let processed = self.drain_all();
                Ok(self.queue.report(processed))
            }
            PumpMode::ErrorAfterDrain => {
                let _ = self.drain_all();
                Err(clock_error())
            }
            PumpMode::ErrorAfterFirst => {
                // Execute one, then fail; the remaining queued work stays queued
                // (no fabricated completions for requests that never ran).
                let _ = self.execute_one();
                Err(clock_error())
            }
        }
    }

    fn poll_completions(&self, limit: usize) -> Vec<RequestCompletion> {
        self.queue.poll_completions(limit)
    }

    fn set_request_waker(&self, waker: Option<Rc<dyn Fn()>>) {
        *self.waker.borrow_mut() = waker;
    }

    /// Origin-tagged sync dispatch: allocate a real sync id (no quota, no
    /// completion) and return the actual execution result under that id. The
    /// origin is trace-only metadata.
    fn dispatch_with_origin(
        &self,
        _origin: RequestOrigin,
        request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError> {
        self.origin_variant_calls
            .set(self.origin_variant_calls.get() + 1);
        let id = self.queue.allocate_sync_id()?;
        let result = self.result_for(&request);
        self.executed.borrow_mut().push(request);
        Ok(RequestCompletion { id, result })
    }

    fn submit_with_origin(
        &self,
        _origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        self.origin_variant_calls
            .set(self.origin_variant_calls.get() + 1);
        let receipt = self.queue.submit(context, request)?;
        if self.queue.take_wake() {
            let waker = self.waker.borrow().as_ref().cloned();
            if let Some(waker) = waker {
                waker();
            }
        }
        Ok(receipt)
    }

    // No event ring in this mock: an explicit refusal, never an empty success.
    fn subscribe(&self) -> CadResult<SubscriptionId> {
        Err(CadError::Unsupported(
            "mock engine client has no event ring".into(),
        ))
    }

    fn poll_events(&self, _id: SubscriptionId, _limit: usize) -> CadResult<EventPoll> {
        Err(CadError::Unsupported(
            "mock engine client has no event ring".into(),
        ))
    }

    fn unsubscribe(&self, _id: SubscriptionId) -> CadResult<()> {
        Err(CadError::Unsupported(
            "mock engine client has no event ring".into(),
        ))
    }

    fn event_watermark(&self) -> EventWatermark {
        EventWatermark::default()
    }
}

struct Sink;
impl UiCommandSink for Sink {
    fn send(&mut self, _command: cad_app::Command) -> CadResult<()> {
        Ok(())
    }
}

fn draw_host_state() -> HostState {
    HostState {
        capabilities: HostCapabilitiesState::default(),
        document_name: "untitled".into(),
        mode: AppMode::Viewer,
        history: HistoryAvailability::default(),
        layer_rows: Vec::new(),
        selection_properties: SelectionProperties::default(),
        layouts: Vec::new(),
        active_space: SpaceId::Model,
        draw_preview: Some(DrawPreview {
            kind: DrawToolKind::Line,
            points: Vec::new(),
            cursor: None,
            remaining: 2,
            selection_count: 0,
            ready: false,
        }),
        selected_draw_kind: Some(DrawToolKind::Line),
        measurement_preview: None,
        selected_measurement_kind: MeasurementToolKind::Distance,
        units: None,
        import_progress: None,
        diagnostics: Vec::new(),
        import: None,
        resources: ResourceState::default(),
    }
}

fn cursor_request(x: f64, y: f64) -> EngineRequest {
    EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::CanvasLocal {
        logical_position: [x, y],
        metrics_revision: 7,
    }))
}

#[test]
fn queue_ui_contracts() {
    offscreen::install().unwrap();
    let mut adapter = UiAdapter::new(UiConfiguration::default(), Sink, true).unwrap();
    adapter.component().show().unwrap();
    let _ = offscreen::snapshot(adapter.window());

    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let view =
        install_existing_cad_bridge(adapter.handle(), adapter.window(), session.clone()).unwrap();
    let context = RequestContext {
        document: DocumentId(1),
        document_generation: 1,
        viewport: ViewportId(1),
        tool_generation: Some(1),
        metrics_revision: Some(7),
        config_revision: 3,
    };
    let client = Rc::new(QueueMockClient::new(session.clone(), context));
    let attached = EngineAdapter::attach(&mut adapter, client.clone(), view.clone()).unwrap();

    // Enable the draw cursor path (only active captures submit MoveCursor).
    client.updates.borrow_mut().full = Some(draw_host_state());
    attached.flush_updates().unwrap();

    // Waker reads the queue (proving the borrow is released) and counts edges.
    let wakes = Rc::new(Cell::new(0));
    let observed = wakes.clone();
    let weak = Rc::downgrade(&client);
    attached
        .set_request_waker(Some(Rc::new(move || {
            if let Some(client) = weak.upgrade() {
                let _ = client.queue.queued();
            }
            observed.set(observed.get() + 1);
        })))
        .unwrap();

    // Empty queue: no processed work, not pending, no wake, no extra redraw.
    let redraw_before = session.redraw_requests();
    let report = attached.pump_requests().unwrap();
    assert_eq!((report.processed, report.pending), (0, false));
    assert_eq!(wakes.get(), 0, "an empty queue must not wake");
    assert_eq!(
        session.redraw_requests(),
        redraw_before,
        "completion/empty pump must not request a redraw"
    );

    // First cursor move: submit admits asynchronously and wakes once.
    adapter
        .component()
        .invoke_pointer_input(2, 0, 10.0, 20.0, 0);
    assert_eq!(wakes.get(), 1, "empty→nonempty wakes exactly once");
    assert_eq!(client.queue.outstanding(), 1, "one accepted cursor");

    // Adjacent equal-context cursor coalesces: the older id is superseded.
    adapter
        .component()
        .invoke_pointer_input(2, 0, 11.0, 21.0, 0);
    assert_eq!(
        wakes.get(),
        1,
        "coalescing into a non-empty queue must not wake again"
    );
    let status_before = adapter.component().get_status_label().to_string();
    let report = attached.pump_requests().unwrap();
    assert!(report.processed >= 1);
    assert_eq!(
        adapter.component().get_status_label().to_string(),
        status_before,
        "a superseded completion is quiet (no error, no success)"
    );
    assert_eq!(
        client.executed.borrow().len(),
        1,
        "coalesced cursors execute exactly once"
    );
    assert_eq!(
        client.queue.outstanding(),
        0,
        "completions drained, quota freed"
    );

    // A stale terminal error is surfaced through the origin catalog copy.
    adapter.handle().set_status("").unwrap();
    client.exec_mode.set(ExecMode::Stale);
    adapter
        .component()
        .invoke_pointer_input(2, 0, 12.0, 22.0, 0);
    let _ = attached.pump_requests().unwrap();
    assert!(
        !adapter
            .component()
            .get_status_label()
            .to_string()
            .is_empty(),
        "a stale completion must be reported, not swallowed"
    );
    client.exec_mode.set(ExecMode::Ok);

    // An execution failure: the receipt is accepted (queued) but is NOT a success;
    // the terminal `Execution(InvalidInput)` completion is shown as an error.
    adapter.handle().set_status("").unwrap();
    let executed_before_invalid = client.executed.borrow().len();
    client.exec_mode.set(ExecMode::Invalid);
    adapter
        .component()
        .invoke_pointer_input(2, 0, 13.0, 23.0, 0);
    assert_eq!(
        client.queue.outstanding(),
        1,
        "the submission is accepted even though it will fail to execute"
    );
    let _ = attached.pump_requests().unwrap();
    assert_eq!(
        client.executed.borrow().len(),
        executed_before_invalid + 1,
        "the request was actually executed and then failed"
    );
    let invalid_status = adapter.component().get_status_label().to_string();
    assert!(
        invalid_status.contains("mock execution failure"),
        "an execution failure completion must be shown as an error: {invalid_status}"
    );
    client.exec_mode.set(ExecMode::Ok);

    // Sync barrier: a budget-exhausted pump leaves queued work pending, so the
    // sync request reports Busy and is never auto-queued or dispatched.
    adapter.handle().set_status("").unwrap();
    let executed_before = client.executed.borrow().len();
    client.pump_blocked.set(true);
    adapter.component().invoke_confirm_measurement_requested();
    assert!(
        !adapter
            .component()
            .get_status_label()
            .to_string()
            .is_empty(),
        "remaining queued work must report Busy"
    );
    assert_eq!(
        client.executed.borrow().len(),
        executed_before,
        "a Busy barrier must not dispatch the sync request"
    );
    client.pump_blocked.set(false);

    // Completion backpressure: fill the tiny queue's completions, then prove the
    // pre-submit poll frees quota so the newest cursor is still admitted.
    let ctx = client.request_context();
    client.queue.submit(ctx, cursor_request(0.0, 0.0)).unwrap();
    let _ = client.pump().unwrap();
    client.queue.submit(ctx, cursor_request(1.0, 1.0)).unwrap();
    let _ = client.pump().unwrap();
    assert_eq!(client.queue.outstanding(), 2, "completions occupy quota");
    adapter.handle().set_status("").unwrap();
    adapter
        .component()
        .invoke_pointer_input(2, 0, 30.0, 40.0, 0);
    assert_eq!(
        client.queue.queued(),
        1,
        "the pre-submit poll must free quota before admitting the cursor"
    );
    assert_eq!(
        adapter.component().get_status_label().to_string(),
        "",
        "a successful admission must not report a queue-full error"
    );

    // Reset the queue (execute the cursor admitted by the backpressure test).
    client.pump_mode.set(PumpMode::Drain);
    client.exec_mode.set(ExecMode::Ok);
    let _ = attached.pump_requests().unwrap();
    assert_eq!(client.queue.outstanding(), 0);

    // Execution error wins over a later clock error: pump executes the first
    // queue entry (which fails), then the pass fails. The terminal execution
    // error must be drained and shown (its quota released), the clock error must
    // not overwrite it, the remaining queued work stays queued, and the sync
    // request is never dispatched.
    adapter.handle().set_status("").unwrap();
    let ctx0 = client.request_context();
    let mut ctx1 = ctx0;
    ctx1.config_revision += 1;
    client.exec_mode.set(ExecMode::Invalid);
    client.pump_mode.set(PumpMode::ErrorAfterFirst);
    client.context.set(ctx0);
    adapter
        .component()
        .invoke_pointer_input(2, 0, 50.0, 60.0, 0);
    client.context.set(ctx1);
    adapter
        .component()
        .invoke_pointer_input(2, 0, 51.0, 61.0, 0);
    assert_eq!(
        client.queue.queued(),
        2,
        "distinct-context cursors do not coalesce"
    );
    let executed_before_error = client.executed.borrow().len();
    adapter.component().invoke_confirm_measurement_requested();
    let error_status = adapter.component().get_status_label().to_string();
    assert!(
        error_status.contains("mock execution failure"),
        "the real execution error must win: {error_status}"
    );
    assert!(
        !error_status.contains("clock expired"),
        "the clock error must not overwrite the execution error: {error_status}"
    );
    assert_eq!(
        client.executed.borrow().len(),
        executed_before_error + 1,
        "only the scheduled cursor executed; the sync request was not dispatched"
    );
    assert_eq!(
        client.queue.queued(),
        1,
        "remaining queued work is preserved, not turned into fake completions"
    );
    assert_eq!(
        client.queue.outstanding(),
        1,
        "the execution completion was drained and released its quota"
    );

    // Executions succeed, then the pass fails: with no execution error to take
    // precedence, the clock error is genuinely returned.
    adapter.handle().set_status("").unwrap();
    let executed_before_clock = client.executed.borrow().len();
    client.exec_mode.set(ExecMode::Ok);
    client.pump_mode.set(PumpMode::ErrorAfterDrain);
    let clock = attached
        .pump_requests()
        .expect_err("a clock failure with no execution error must be returned");
    assert!(
        clock.to_string().contains("clock expired"),
        "the real clock error is returned: {clock}"
    );
    assert_eq!(
        client.executed.borrow().len(),
        executed_before_clock + 1,
        "the remaining queued request executed before the clock failure"
    );
    assert_eq!(
        client.queue.outstanding(),
        0,
        "its completion was still drained despite the failing pass"
    );
    assert_eq!(
        adapter.component().get_status_label().to_string(),
        "",
        "a successful execution must not be reported as a fake error"
    );
    client.pump_mode.set(PumpMode::Drain);

    // Regression guard: with the pump succeeding, an old async cursor error must
    // not block a new synchronous request. The barrier's pump executes the queued
    // cursor (a real completion error is shown) and the Cancel still dispatches,
    // clearing the real capture state (not a counter).
    client.exec_mode.set(ExecMode::Ok);
    assert_eq!(client.queue.outstanding(), 0);
    client.updates.borrow_mut().full = Some(draw_host_state());
    attached.flush_updates().unwrap();
    adapter.component().invoke_begin_draw_tool("line".into());
    assert!(client.capture_active.get(), "BeginDraw started a capture");
    client.exec_mode.set(ExecMode::Invalid);
    adapter
        .component()
        .invoke_pointer_input(2, 0, 60.0, 70.0, 0);
    assert_eq!(
        client.queue.queued(),
        1,
        "the cursor is queued for the pump"
    );
    adapter.handle().set_status("").unwrap();
    adapter.component().invoke_cancel_measurement_requested();
    assert!(
        !client.capture_active.get(),
        "an old cursor completion error must not block the current Cancel dispatch"
    );
    assert!(
        adapter
            .component()
            .get_status_label()
            .to_string()
            .contains("mock execution failure"),
        "the old async error is still shown"
    );
    client.exec_mode.set(ExecMode::Ok);

    // Pre-poll guard: an already-claimable old completion error is drained by the
    // barrier's first poll without blocking the new BeginDraw.
    client.exec_mode.set(ExecMode::Invalid);
    let ctx = client.request_context();
    client.queue.submit(ctx, cursor_request(2.0, 2.0)).unwrap();
    let _ = client.pump().unwrap();
    assert_eq!(
        client.queue.outstanding(),
        1,
        "one unclaimed error completion"
    );
    client.exec_mode.set(ExecMode::Ok);
    adapter.handle().set_status("").unwrap();
    adapter.component().invoke_begin_draw_tool("line".into());
    assert!(
        client.capture_active.get(),
        "a pre-poll error must not block the new BeginDraw dispatch"
    );
    assert!(
        adapter
            .component()
            .get_status_label()
            .to_string()
            .contains("mock execution failure"),
        "the pre-poll error was shown"
    );
    adapter.component().invoke_cancel_measurement_requested();
    assert!(
        !client.capture_active.get(),
        "Cancel clears the real capture state"
    );

    // P4: the UI stays on the `Ui`-origin `dispatch`/`submit` shorthands; it must
    // not switch to the explicit-origin variants.
    assert_eq!(
        client.origin_variant_calls.get(),
        0,
        "the UI must keep using the Ui-origin dispatch/submit shorthands"
    );
}
