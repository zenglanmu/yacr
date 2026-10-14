#![cfg(target_os = "linux")]
//! P2-B mock contracts for the attached engine path.
//!
//! One test owns the process-wide Slint offscreen platform (the platform is
//! thread-local and installs at most once), so every contract runs on that
//! single thread: unique dispatch, session mismatch rejection, config authority,
//! clear-preference delegation, backend-only deltas, status localization, error
//! precedence, idle draw-kind retention, pinch single-dispatch and canvas-local
//! (never world) input.
use cad_app::api::ApiOutcome;
use cad_app::interaction::{InteractionRequest, PointInput};
use cad_app::{
    AppMode, Camera, CanvasMetrics, CommandId, CommandPayload, DrawToolKind, HistoryAvailability,
    MeasurementToolKind, ProjectionKind, SelectionProperties,
};
use cad_domain::{CadError, CadResult, DocumentId, SpaceId, ViewportId};
use cad_engine::events::{EventPoll, EventWatermark, SubscriptionId};
use cad_engine::protocol::{
    ConfigSnapshot, EngineClient, EngineRequest, HostCapabilitiesState, HostState, HostUpdates,
    InteractionOutcome, NavigationState, PumpReport, RenderBackendState, RequestCompletion,
    RequestContext, RequestError, RequestOrigin, RequestResult, ResourceState, StatusArgument,
    StatusMessage, SubmissionReceipt,
};
use cad_engine::render::{RenderSessionHandle, ViewSnapshot};
use cad_render_wgpu::BackendPreference;
use cad_representation::SpaceSelection;
use cad_ui_slint::{
    install_existing_cad_bridge, offscreen, EngineAdapter, UiAdapter, UiCommandSink,
    UiConfiguration,
};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// Legacy sink the attached adapter must never reach.
struct LegacySink(Rc<RefCell<Vec<CommandId>>>);

impl UiCommandSink for LegacySink {
    fn send(&mut self, command: cad_app::Command) -> CadResult<()> {
        self.0.borrow_mut().push(command.id);
        Ok(())
    }
}

/// Owner-thread client that records requests/config mutations and can be told to
/// refuse execution and/or fail the update publish, so precedence is observable.
struct MockClient {
    session: RenderSessionHandle,
    requests: RefCell<Vec<EngineRequest>>,
    updates: RefCell<HostUpdates>,
    config: RefCell<ConfigSnapshot>,
    config_ops: RefCell<Vec<&'static str>>,
    accept_dispatch: Cell<bool>,
    /// When set, `dispatch` returns a controlled-API result even for a command,
    /// so the adapter's type-mismatch handling is observable.
    accept_api: Cell<bool>,
    fail_updates: Cell<bool>,
}

impl MockClient {
    fn new(session: RenderSessionHandle) -> Self {
        Self {
            session,
            requests: RefCell::new(Vec::new()),
            updates: RefCell::new(HostUpdates::default()),
            config: RefCell::new(ConfigSnapshot {
                effective: cad_app::viewer_config::ViewerConfig::default(),
                revision: 0,
                preset: cad_app::viewer_config::Preset::Full,
                effective_json: "{}".into(),
            }),
            config_ops: RefCell::new(Vec::new()),
            accept_dispatch: Cell::new(false),
            accept_api: Cell::new(false),
            fail_updates: Cell::new(false),
        }
    }

    fn record_config(&self, operation: &'static str) -> CadResult<()> {
        self.config_ops.borrow_mut().push(operation);
        let mut config = self.config.borrow_mut();
        config.revision += 1;
        // Flip a real field so a UI that only mirrors the snapshot is detectable.
        config.effective.view.overlays.grid = !config.effective.view.overlays.grid;
        config.effective_json = format!("{{\"op\":\"{operation}\"}}");
        Ok(())
    }

    fn set_pointer(&self, enabled: bool) {
        self.config.borrow_mut().effective.interaction.pointer = enabled;
    }
}

impl EngineClient for MockClient {
    fn dispatch(&self, request: EngineRequest) -> Result<RequestResult, RequestError> {
        self.requests.borrow_mut().push(request);
        if self.accept_api.get() {
            return Ok(RequestResult::Api(Arc::new(ApiOutcome {
                created: Vec::new(),
                changes: None,
            })));
        }
        if self.accept_dispatch.get() {
            Ok(RequestResult::Interaction(InteractionOutcome::Confirmed))
        } else {
            Err(RequestError::Execution(CadError::InvalidInput(
                "mock refusal".into(),
            )))
        }
    }

    fn request_context(&self) -> RequestContext {
        RequestContext {
            document: DocumentId(1),
            document_generation: 1,
            viewport: ViewportId(1),
            tool_generation: Some(1),
            metrics_revision: Some(7),
            config_revision: 0,
        }
    }

    fn take_updates(&self) -> CadResult<HostUpdates> {
        if self.fail_updates.get() {
            return Err(CadError::InvalidInput("update boom".into()));
        }
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
        self.record_config("set")
    }

    fn update_config_json(&self, _json: &str) -> CadResult<()> {
        self.record_config("update")
    }

    fn apply_user_preference_json(&self, _json: &str) -> CadResult<()> {
        self.record_config("preference")
    }

    fn clear_user_preference(&self) -> CadResult<()> {
        self.config_ops.borrow_mut().push("clear");
        let mut config = self.config.borrow_mut();
        config.revision += 1;
        config.effective = cad_app::viewer_config::ViewerConfig::default();
        config.effective_json = "{}".into();
        Ok(())
    }

    // This mock owns no queue: it refuses admission and reports an accurate empty
    // pump rather than pretending to accept or execute queued work.
    fn submit(
        &self,
        _context: RequestContext,
        _request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        Err(RequestError::Busy)
    }

    fn pump(&self) -> Result<PumpReport, RequestError> {
        Ok(PumpReport::default())
    }

    fn poll_completions(&self, _limit: usize) -> Vec<RequestCompletion> {
        Vec::new()
    }

    fn set_request_waker(&self, _waker: Option<Rc<dyn Fn()>>) {}

    // No sync-id allocator and no event ring here: refuse explicitly rather than
    // returning a fabricated id (for example a hard-coded `RequestId(0)`) or an
    // empty success.
    fn dispatch_with_origin(
        &self,
        _origin: RequestOrigin,
        _request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError> {
        Err(RequestError::Busy)
    }

    fn submit_with_origin(
        &self,
        _origin: RequestOrigin,
        _context: RequestContext,
        _request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError> {
        Err(RequestError::Busy)
    }

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

/// Minimal idle full-state report; the draw kind / units stay explicit.
fn host_state(
    selected_draw_kind: Option<DrawToolKind>,
    units: Option<cad_domain::UnitContext>,
) -> HostState {
    HostState {
        capabilities: HostCapabilitiesState::default(),
        document_name: "untitled".into(),
        mode: AppMode::Viewer,
        history: HistoryAvailability::default(),
        layer_rows: Vec::new(),
        selection_properties: SelectionProperties::default(),
        layouts: Vec::new(),
        active_space: SpaceId::Model,
        draw_preview: None,
        selected_draw_kind,
        measurement_preview: None,
        selected_measurement_kind: MeasurementToolKind::Distance,
        units,
        import_progress: None,
        diagnostics: Vec::new(),
        import: None,
        resources: ResourceState::default(),
    }
}

#[test]
fn attached_engine_owns_dispatch_config_backend_status_and_canvas_local_input() {
    offscreen::install().unwrap();
    let legacy = Rc::new(RefCell::new(Vec::new()));
    let mut adapter =
        UiAdapter::new(UiConfiguration::default(), LegacySink(legacy.clone()), true).unwrap();
    adapter.component().show().unwrap();
    let _ = offscreen::snapshot(adapter.window());

    // The engine client must share the facade's render session.
    let session = RenderSessionHandle::new(BackendPreference::Auto);
    let view =
        install_existing_cad_bridge(adapter.handle(), adapter.window(), session.clone()).unwrap();

    // A foreign session is rejected before any callback is rewired.
    let foreign = Rc::new(MockClient::new(RenderSessionHandle::new(
        BackendPreference::Auto,
    )));
    let rejected = EngineAdapter::attach(&mut adapter, foreign, view.clone());
    assert!(
        matches!(rejected, Err(CadError::InvalidInput(_))),
        "a mismatched render session must be rejected"
    );

    let client = Rc::new(MockClient::new(session));
    client.updates.borrow_mut().status = Some(StatusMessage::Catalog {
        key: "command.failed".into(),
        args: vec![StatusArgument {
            key: "error".into(),
            value: "boom".into(),
        }],
    });
    let attached = EngineAdapter::attach(&mut adapter, client.clone(), view.clone()).unwrap();

    // Config authority: every entry delegates to the single client store and the
    // UI only mirrors the returned snapshot (no second writable store).
    adapter.handle().set_config_json("{}").unwrap();
    adapter.handle().update_config_json("{}").unwrap();
    adapter.handle().apply_user_preference_json("{}").unwrap();
    adapter.handle().clear_user_preference().unwrap();
    assert_eq!(
        *client.config_ops.borrow(),
        vec!["set", "update", "preference", "clear"],
        "clear must delegate to the engine client, not refuse"
    );
    assert_eq!(
        adapter.handle().effective_config(),
        client.config.borrow().effective,
        "the UI must mirror the client snapshot rather than compute its own"
    );

    // The frozen metrics API is wired to the shared session.
    attached.refresh_metrics().unwrap();

    // A catalog status re-translates from the cached key and keeps its argument.
    attached.flush_updates().unwrap();
    let zh = adapter.component().get_status_label().to_string();
    adapter.handle().set_locale("en").unwrap();
    let en = adapter.component().get_status_label().to_string();
    assert_ne!(
        zh, en,
        "a catalog status must re-translate on a locale switch"
    );
    assert!(
        zh.contains("boom") && en.contains("boom"),
        "status arguments must survive localization"
    );

    // ExternalText is presented verbatim and is untouched by a locale switch.
    client.updates.borrow_mut().status = Some(StatusMessage::ExternalText("verbatim".into()));
    attached.flush_updates().unwrap();
    assert_eq!(
        adapter.component().get_status_label().to_string(),
        "verbatim"
    );
    adapter.handle().set_locale("zh-CN").unwrap();
    assert_eq!(
        adapter.component().get_status_label().to_string(),
        "verbatim",
        "ExternalText must never be re-localized"
    );

    // A refused interaction is surfaced, never a silent success.
    let before = adapter.component().get_status_label().to_string();
    adapter.component().invoke_confirm_measurement_requested();
    let after = adapter.component().get_status_label().to_string();
    assert_ne!(before, after, "a refused interaction must be reported");
    assert!(
        matches!(
            client.requests.borrow().last(),
            Some(EngineRequest::Interaction(InteractionRequest::Confirm))
        ),
        "confirm must be the single dispatched interaction"
    );

    // Unique dispatch: attached commands go to the engine, never the legacy sink.
    let legacy_before = legacy.borrow().len();
    adapter.component().invoke_fit_requested();
    assert!(
        matches!(
            client.requests.borrow().last(),
            Some(EngineRequest::Command(command)) if command.id == CommandId::FitDrawing
        ),
        "fit must dispatch exactly one engine command"
    );
    assert_eq!(
        legacy.borrow().len(),
        legacy_before,
        "an attached command must not reach the legacy sink"
    );

    // Canvas input stays canvas-local; the UI never solves a world point.
    let before = client.requests.borrow().len();
    adapter.component().invoke_canvas_pick(12.0, 34.0);
    let requests = client.requests.borrow();
    let picked: Vec<_> = requests[before..].iter().collect();
    assert!(!picked.is_empty(), "an enabled canvas pick must dispatch");
    for request in picked {
        assert!(
            matches!(
                request,
                EngineRequest::Interaction(InteractionRequest::SelectAt {
                    point: PointInput::CanvasLocal { .. },
                    ..
                })
            ),
            "canvas input must be canvas-local, never world-solved"
        );
    }
    drop(requests);

    // A controlled-API result routed to a command verb is a type mismatch: the
    // adapter reports an explicit failure, never a fake command success.
    adapter.handle().set_status("").unwrap();
    client.accept_api.set(true);
    adapter.component().invoke_erase_requested();
    assert!(
        !adapter
            .component()
            .get_status_label()
            .to_string()
            .is_empty(),
        "an api result for a command must be an explicit failure, not a fake success"
    );
    client.accept_api.set(false);

    // Error precedence: a failed publish must not mask the execution error, and
    // an already-committed request is never retried.
    client.fail_updates.set(true);
    adapter.component().invoke_confirm_measurement_requested();
    let precedence = adapter.component().get_status_label().to_string();
    assert!(
        precedence.contains("mock refusal"),
        "the execution error must win: {precedence}"
    );
    assert!(
        !precedence.contains("update boom"),
        "a publish failure must not replace the execution error"
    );
    // With execution accepted, the publish failure is the reported UI failure.
    client.accept_dispatch.set(true);
    adapter.component().invoke_confirm_measurement_requested();
    let publish = adapter.component().get_status_label().to_string();
    assert!(
        publish.contains("update boom"),
        "an accepted request with a failed publish must report the publish error"
    );
    client.accept_dispatch.set(false);
    client.fail_updates.set(false);

    // Backend is an independent delta: applied even without a full state, and a
    // later full update must not clear the real renderer label.
    client.updates.borrow_mut().full = Some(host_state(None, None));
    attached.flush_updates().unwrap();
    assert_eq!(
        adapter.component().get_unit_label().to_string(),
        "drawing units",
        "an unavailable unit context must not render an empty label"
    );
    client.updates.borrow_mut().backend = Some(RenderBackendState {
        outcome: None,
        api: Some("vulkan".into()),
    });
    attached.flush_updates().unwrap();
    assert!(
        adapter
            .component()
            .get_diagnostics_backend_label()
            .to_string()
            .contains("vulkan"),
        "the backend delta must update the real renderer label"
    );
    client.updates.borrow_mut().full = Some(host_state(None, None));
    attached.flush_updates().unwrap();
    assert!(
        adapter
            .component()
            .get_diagnostics_backend_label()
            .to_string()
            .contains("vulkan"),
        "a full update must not clear the cached backend label"
    );

    // Idle draw kind: `Some` updates the retained selection, `None` preserves it
    // instead of resetting to a synthetic Line.
    assert_eq!(adapter.selected_draw_kind(), None);
    client.updates.borrow_mut().full = Some(host_state(Some(DrawToolKind::Circle), None));
    attached.flush_updates().unwrap();
    assert_eq!(adapter.selected_draw_kind(), Some(DrawToolKind::Circle));
    client.updates.borrow_mut().full = Some(host_state(None, None));
    attached.flush_updates().unwrap();
    assert_eq!(
        adapter.selected_draw_kind(),
        Some(DrawToolKind::Circle),
        "an idle (None) full update must not reset the draw kind to Line"
    );

    // Pinch: attached, the engine owns zoom. A start/end never fabricates a zoom;
    // one adjacent update dispatches exactly one canvas-local-anchored ZoomAt.
    client.requests.borrow_mut().clear();
    adapter.component().invoke_pinch_input(0, 1.0, 10.0, 20.0);
    assert!(
        client.requests.borrow().is_empty(),
        "a pinch start must not dispatch a zoom"
    );
    adapter.component().invoke_pinch_input(1, 1.25, 11.0, 22.0);
    {
        let requests = client.requests.borrow();
        assert_eq!(requests.len(), 1, "one adjacent update is one dispatch");
        match &requests[0] {
            EngineRequest::Command(command) => {
                assert_eq!(command.id, CommandId::Zoom);
                match &command.payload {
                    CommandPayload::ZoomAt { factor, cursor } => {
                        assert!((*factor - 1.25).abs() < 1e-9, "adjacent ratio");
                        assert_eq!(*cursor, [11.0, 22.0], "canvas-local anchor");
                    }
                    _ => panic!("expected a ZoomAt payload"),
                }
            }
            other => panic!("expected a command dispatch, got {}", request_kind(other)),
        }
    }
    adapter.component().invoke_pinch_input(2, 1.25, 11.0, 22.0);
    assert_eq!(
        client.requests.borrow().len(),
        1,
        "a pinch end must not dispatch a zoom"
    );

    // A config change mid-gesture cancels the zoom without dispatching.
    client.requests.borrow_mut().clear();
    adapter.component().invoke_pinch_input(0, 1.0, 10.0, 20.0);
    client.set_pointer(false);
    adapter.handle().set_config_json("{}").unwrap();
    adapter.component().invoke_pinch_input(1, 1.5, 11.0, 22.0);
    assert!(
        client.requests.borrow().is_empty(),
        "a config change mid-gesture must cancel zoom"
    );

    // A disabled pointer gate suppresses pinch zoom entirely.
    client.requests.borrow_mut().clear();
    adapter.component().invoke_pinch_input(0, 1.0, 10.0, 20.0);
    adapter.component().invoke_pinch_input(1, 2.0, 11.0, 22.0);
    assert!(
        client.requests.borrow().is_empty(),
        "a disabled pointer must not dispatch zoom"
    );
    client.set_pointer(true);
    adapter.handle().set_config_json("{}").unwrap();

    // Navigation must not rebuild the CAD snapshot: the host already synced the
    // shared session's authoritative camera/metrics (safe insets / DPR). The UI
    // only mirrors the raw navigation state, so the session size, camera, mode
    // and input token are untouched, and an orthographic 3D mode stays 3D.
    let session = client.render_session();
    let host_camera = Camera::top_view_2d();
    session
        .apply_view_snapshot(ViewSnapshot {
            document: DocumentId(1),
            space: SpaceSelection::Model,
            camera: host_camera,
            mode: ProjectionKind::TwoD,
            // Non-zero-inset authoritative size, different from any uncropped UI
            // surface size; the UI must never overwrite it.
            canvas_size: [1000.0, 800.0],
        })
        .unwrap();
    let token_before = session.input_revision();
    let mut nav_camera = Camera::top_view_2d();
    nav_camera.target.x += 5.0;
    client.updates.borrow_mut().navigation = Some(NavigationState {
        viewport: ViewportId(1),
        camera: nav_camera,
        mode: ProjectionKind::ThreeD,
        space: SpaceId::Model,
        snap_hints: Vec::new(),
    });
    attached.flush_updates().unwrap();
    assert_eq!(
        session.view_snapshot().canvas_size,
        [1000.0, 800.0],
        "the UI must not overwrite the host's authoritative canvas metrics"
    );
    assert_eq!(
        session.camera(),
        host_camera,
        "the UI must not rewrite the host-synced camera"
    );
    assert_eq!(
        session.view_mode(),
        ProjectionKind::TwoD,
        "the UI must not rewrite the host-synced session mode"
    );
    assert_eq!(
        session.input_revision(),
        token_before,
        "applying navigation must not consume the session input token"
    );
    assert!(
        adapter.component().get_view_3d(),
        "an orthographic 3D mode must stay 3D, never guessed from perspective"
    );
    // Repeated flush with nothing pending is stable (no token churn).
    attached.flush_updates().unwrap();
    assert_eq!(session.input_revision(), token_before);
    assert_eq!(session.view_snapshot().canvas_size, [1000.0, 800.0]);
}

/// Small label for a request kind, without requiring `Debug` on the protocol.
fn request_kind(request: &EngineRequest) -> &'static str {
    match request {
        EngineRequest::Command(_) => "command",
        EngineRequest::Interaction(_) => "interaction",
        EngineRequest::Api { .. } => "api",
    }
}
