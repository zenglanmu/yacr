//! Typed request and completion contracts for the engine boundary.
//!
//! This is an in-process semantic contract, not a stable serialized schema or
//! an implementation of request scheduling.

use crate::events::{EventPoll, EventWatermark, RequestKind, SubscriptionId};
use crate::render::RenderSessionHandle;
use cad_app::api::{ApiOutcome, ApiRequest, ApiTarget};
use cad_app::{
    interaction::InteractionRequest, recovery::BackendOutcome, viewer_config::ViewerConfig,
    AppMode, Camera, CanvasMetrics, Command, CommandOutcome, DrawPreview, DrawToolKind,
    HistoryAvailability, MeasurementPreview, MeasurementToolKind, ProjectionKind,
    SelectionProperties,
};
use cad_domain::{
    CadError, CadResult, Completeness, Diagnostic, DocumentId, DocumentIdentity, ObjectId, Point3,
    Precision, SpaceId, ViewportId,
};
use cad_representation::LayoutDescriptor;
use std::rc::Rc;
use std::sync::Arc;

/// Engine-scoped monotonic request identity, distinct from domain/platform IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u64);

/// Provenance of a request, recorded for tracing only. It is never a permission
/// or trust boundary: callers remain responsible for their own authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RequestOrigin {
    #[default]
    Ui,
    Api,
    Script,
    Platform,
}

pub enum EngineRequest {
    Command(Command),
    Interaction(InteractionRequest),
    /// A controlled API request targeting a host document/database/viewport.
    Api {
        target: ApiTarget,
        request: ApiRequest,
    },
}

impl EngineRequest {
    /// Minimal classification for trace events; it exposes no business state.
    pub fn kind(&self) -> RequestKind {
        match self {
            EngineRequest::Command(_) => RequestKind::Command,
            EngineRequest::Interaction(_) => RequestKind::Interaction,
            EngineRequest::Api { .. } => RequestKind::Api,
        }
    }
}

/// Context used to detect requests targeting replaced or stale state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestContext {
    pub document: DocumentId,
    /// Independent host document epoch: increments only when the authoritative
    /// document is replaced, never on edits, tool changes or camera moves.
    pub document_generation: u64,
    pub viewport: ViewportId,
    pub tool_generation: Option<u64>,
    pub metrics_revision: Option<u64>,
    /// Host config revision; coordinate/gesture requests are validated against it.
    pub config_revision: u64,
}

pub struct RequestEnvelope {
    pub id: RequestId,
    /// Trace-only provenance; not a permission.
    pub origin: RequestOrigin,
    pub context: RequestContext,
    pub request: EngineRequest,
}

/// Immediate result of submission; it does not report request execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmissionReceipt {
    pub id: RequestId,
    pub status: SubmissionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStatus {
    Queued,
}

#[derive(Debug, Clone)]
pub enum RequestResult {
    Command(Arc<CommandOutcome>),
    Interaction(InteractionOutcome),
    Api(Arc<ApiOutcome>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionOutcome {
    /// A tool capture started; this does not imply that any CAD command ran.
    ToolStarted,
    SelectionKindChanged,
    PanStarted,
    CursorCleared,
    PointCaptured,
    CursorMoved,
    Confirmed,
    Cancelled,
    SelectionUpdated,
    PointerObserved,
    ScrollObserved,
}

/// Semantic refusal/failure. Acceptance is not a claim that work executed.
#[derive(Debug, Clone)]
pub enum RequestError {
    Cancelled,
    QueueFull,
    Busy,
    StaleContext,
    Execution(CadError),
    /// The request was coalesced away by a newer adjacent request. This is a
    /// terminal state: it still consumes quota until polled, and is neither an
    /// error to display nor a success.
    Superseded {
        by: RequestId,
    },
}

#[derive(Debug, Clone)]
pub struct RequestCompletion {
    pub id: RequestId,
    pub result: Result<RequestResult, RequestError>,
}

/// Shallow-cloneable shared completion result. Cloning never deep-copies a
/// `ChangeSet`: every payload is already behind an `Arc`.
pub type SharedRequestResult = Arc<Result<RequestResult, RequestError>>;

/// Result of one owner-thread pump pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PumpReport {
    /// Requests advanced (executed or moved to a terminal state) in this pass.
    pub processed: usize,
    /// Whether executable work is still queued after the pass. Unclaimed
    /// completions are not executable pending and do not set this.
    pub pending: bool,
}

/// Host-owned effective configuration snapshot shared with attached adapters.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigSnapshot {
    pub effective: ViewerConfig,
    pub revision: u64,
    pub preset: cad_app::viewer_config::Preset,
    pub effective_json: String,
}

/// One raw, locale-independent snap candidate for adapter presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapHint {
    /// Stable machine key (for example `endpoint`), not a localized label.
    pub kind: String,
    pub point: Point3,
    pub space: SpaceId,
    pub source: cad_domain::SelectionRef,
    pub secondary: Option<cad_domain::SelectionRef>,
    pub precision: Precision,
    pub logical_pixel_distance: f64,
}

/// Navigation state needed to mirror the authoritative viewport and snap hints.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationState {
    pub viewport: ViewportId,
    pub camera: Camera,
    pub mode: ProjectionKind,
    pub space: SpaceId,
    pub snap_hints: Vec<SnapHint>,
}

/// Availability flags as currently determined by host capabilities and loading state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostCapabilitiesState {
    pub open: bool,
    pub new: bool,
    pub save: bool,
    pub plot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDiagnosticParameter {
    Object(ObjectId),
    Key(String),
    Identifier(String),
    Count(u64),
    Bytes(u64),
    Limit(u64),
    Depth(u64),
}

/// Diagnostic source data before any catalog lookup or UI formatting.
#[derive(Debug, Clone, PartialEq)]
pub struct RawDiagnostic {
    pub object: Option<ObjectId>,
    pub code: String,
    pub message: String,
    pub severity: Option<DiagnosticSeverity>,
    pub completeness: Option<Completeness>,
    pub parameters: Vec<RawDiagnosticParameter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// Raw import report summary; identifiers and diagnostics stay machine-readable.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportState {
    pub identity: DocumentIdentity,
    pub capabilities: Vec<RawEntityCapability>,
    pub completeness: Completeness,
    pub diagnostics: Vec<Diagnostic>,
    pub parse_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntityCapability {
    pub type_key: String,
    pub read: cad_domain::SupportStatus,
    pub semantic: cad_domain::SupportStatus,
    pub render: cad_domain::SupportStatus,
    pub pick: cad_domain::SupportStatus,
    pub measure: cad_domain::SupportStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontResourceState {
    pub catalog_entries: usize,
    pub requested: Vec<String>,
    pub planned: Vec<String>,
    pub registered: Vec<String>,
    pub failed: Vec<String>,
    pub unresolved: Vec<String>,
    pub default_face: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageResourceState {
    pub requested: Vec<String>,
    pub loaded: Vec<String>,
    pub unresolved: Vec<String>,
    pub failed: Vec<String>,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedProxyState {
    pub record_type: u32,
    pub reason: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyResourceState {
    pub decoded_records: usize,
    pub unsupported: Vec<UnsupportedProxyState>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceState {
    /// `None` means no report was available; an empty report remains `Some(empty)`.
    pub fonts: Option<FontResourceState>,
    pub images: Option<ImageResourceState>,
    pub proxy: Option<ProxyResourceState>,
    /// Resource keys from the active document, including a genuinely empty list.
    pub references: Option<Vec<String>>,
    pub images_modeled: bool,
}

/// Complete UI-independent state published by the host.
#[derive(Debug, Clone, PartialEq)]
pub struct HostState {
    pub capabilities: HostCapabilitiesState,
    pub document_name: String,
    pub mode: AppMode,
    pub history: HistoryAvailability,
    pub layer_rows: Vec<cad_app::layers::LayerRow>,
    pub selection_properties: SelectionProperties,
    pub layouts: Vec<LayoutDescriptor>,
    pub active_space: SpaceId,
    pub draw_preview: Option<DrawPreview>,
    /// `Some` only while a drawing tool is actively capturing; `None` when idle.
    /// Idle never falls back to a synthetic `Line`.
    pub selected_draw_kind: Option<DrawToolKind>,
    pub measurement_preview: Option<MeasurementPreview>,
    pub selected_measurement_kind: MeasurementToolKind,
    pub units: Option<cad_domain::UnitContext>,
    pub import_progress: Option<cad_app::ImportProgressSnapshot>,
    pub diagnostics: Vec<RawDiagnostic>,
    pub import: Option<ImportState>,
    pub resources: ResourceState,
}

/// Status payload remains raw until the UI resolves it through its active catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusMessage {
    Catalog {
        /// Existing catalog key, never localized by the engine.
        key: String,
        /// Preserve all current interpolation parameters in caller order.
        args: Vec<StatusArgument>,
    },
    /// Explicit text supplied by a platform or an already-rendered external source.
    ExternalText(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusArgument {
    pub key: String,
    pub value: String,
}

/// Raw backend lifecycle snapshot for incremental host-to-adapter updates.
///
/// `api` is the session's *actual* active backend label (or `None` before a
/// device exists); it is never derived from the requested preference. This
/// state is published on lifecycle transitions instead of forcing a full
/// `HostState` re-derivation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderBackendState {
    pub outcome: Option<BackendOutcome>,
    pub api: Option<String>,
}

/// Incremental typed update batch. `None` means unchanged/not published, not empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostUpdates {
    pub full: Option<HostState>,
    pub navigation: Option<NavigationState>,
    pub config: Option<ConfigSnapshot>,
    pub status: Option<StatusMessage>,
    /// Backend lifecycle delta; never implies a full `HostState` rebuild.
    pub backend: Option<RenderBackendState>,
}

/// Owner-thread engine surface consumed by the host/UI adapter.
///
/// `dispatch` stays the synchronous operation whose result describes work
/// performed/refused. P3 adds a bounded queue: `submit` admits work for later
/// execution, `pump` advances it, and `poll_completions` collects terminal
/// outcomes. The engine owns the queue and never invokes the waker itself.
pub trait EngineClient {
    /// Synchronous dispatch with the default `Ui` origin.
    fn dispatch(&self, request: EngineRequest) -> Result<RequestResult, RequestError>;
    fn request_context(&self) -> RequestContext;
    /// Derive and consume the pending dirty snapshot atomically; on `Err`, the
    /// producer must retain its dirty markers so callers can retry.
    fn take_updates(&self) -> CadResult<HostUpdates>;
    fn set_canvas_metrics(&self, metrics: CanvasMetrics) -> CadResult<()>;
    fn render_session(&self) -> RenderSessionHandle;
    fn config_snapshot(&self) -> ConfigSnapshot;
    fn set_config_json(&self, json: &str) -> CadResult<()>;
    fn update_config_json(&self, json: &str) -> CadResult<()>;
    fn apply_user_preference_json(&self, json: &str) -> CadResult<()>;
    /// Drop the stored user preference so the host defaults apply again.
    ///
    /// The default refuses explicitly: a client without a real preference store
    /// must never report a fake success. Implementers that own a store override
    /// this with a real clear.
    fn clear_user_preference(&self) -> CadResult<()> {
        Err(CadError::Unsupported(
            "clear_user_preference is not implemented by this engine client".into(),
        ))
    }
    /// Admit a request for later owner-thread execution (default `Ui` origin).
    /// Acceptance is not a claim that work executed: a rejected request
    /// allocates no identity and produces no completion.
    ///
    /// Required, not defaulted: an empty default would misrepresent queueing
    /// capability, so every implementer must be explicit.
    fn submit(
        &self,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError>;
    /// Advance owner-thread execution by one bounded pass.
    ///
    /// Required, not defaulted: an empty default would misrepresent capability.
    fn pump(&self) -> Result<PumpReport, RequestError>;
    /// Take at most `limit` terminal completions, freeing their quota.
    fn poll_completions(&self, limit: usize) -> Vec<RequestCompletion>;
    /// Register (or clear) the queue empty-to-nonempty wake callback. The engine
    /// must never invoke the waker while holding a queue borrow.
    fn set_request_waker(&self, waker: Option<Rc<dyn Fn()>>);

    // --- P4: origin-tagged requests and bounded event subscriptions ---
    /// Synchronous dispatch that records `origin` for tracing only. It is never
    /// a permission check.
    fn dispatch_with_origin(
        &self,
        origin: RequestOrigin,
        request: EngineRequest,
    ) -> Result<RequestCompletion, RequestError>;
    /// Queue a request carrying an explicit trace origin. `submit` is the `Ui`
    /// shorthand; neither is a permission check.
    fn submit_with_origin(
        &self,
        origin: RequestOrigin,
        context: RequestContext,
        request: EngineRequest,
    ) -> Result<SubmissionReceipt, RequestError>;
    /// Subscribe to the engine event ring at its current tail.
    fn subscribe(&self) -> CadResult<SubscriptionId>;
    /// Poll at most `limit` events for a subscription (`limit` is clamped to the
    /// ring maximum).
    fn poll_events(&self, id: SubscriptionId, limit: usize) -> CadResult<EventPoll>;
    /// Drop a subscription, freeing its slot.
    fn unsubscribe(&self, id: SubscriptionId) -> CadResult<()>;
    /// Current event-ring watermark, for host snapshots and resync.
    fn event_watermark(&self) -> EventWatermark;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{DocumentId, ViewportId};
    use std::cell::RefCell;

    struct MockEngineClient {
        updates: RefCell<HostUpdates>,
        session: RenderSessionHandle,
        /// Last user-preference JSON applied through this client; `None` is a
        /// genuinely empty store, exactly what a real clear leaves behind.
        user_preference: RefCell<Option<String>>,
    }

    impl EngineClient for MockEngineClient {
        fn dispatch(&self, _request: EngineRequest) -> Result<RequestResult, RequestError> {
            Err(RequestError::Busy)
        }

        fn request_context(&self) -> RequestContext {
            RequestContext {
                document: DocumentId(1),
                document_generation: 2,
                viewport: ViewportId(3),
                tool_generation: Some(4),
                metrics_revision: Some(5),
                config_revision: 6,
            }
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
            ConfigSnapshot {
                effective: ViewerConfig::default(),
                revision: 0,
                preset: cad_app::viewer_config::Preset::Full,
                effective_json: "{}".into(),
            }
        }

        fn set_config_json(&self, _json: &str) -> CadResult<()> {
            Ok(())
        }

        fn update_config_json(&self, _json: &str) -> CadResult<()> {
            Ok(())
        }

        fn apply_user_preference_json(&self, json: &str) -> CadResult<()> {
            *self.user_preference.borrow_mut() = Some(json.to_string());
            Ok(())
        }

        fn clear_user_preference(&self) -> CadResult<()> {
            *self.user_preference.borrow_mut() = None;
            Ok(())
        }

        // The mock owns no queue: it refuses admission rather than pretending
        // to accept work, reports an accurate empty pump, and holds no waker.
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

    #[test]
    fn synthetic_envelope_keeps_request_identity_and_context_typed() {
        let envelope = RequestEnvelope {
            id: RequestId(42),
            origin: RequestOrigin::Ui,
            context: RequestContext {
                document: DocumentId(1),
                document_generation: 4,
                viewport: ViewportId(2),
                tool_generation: Some(9),
                metrics_revision: Some(3),
                config_revision: 0,
            },
            request: EngineRequest::Interaction(InteractionRequest::BeginDraw(
                cad_app::DrawToolKind::Line,
            )),
        };
        assert_eq!(envelope.id, RequestId(42));
        assert_eq!(envelope.context.document_generation, 4);
        assert!(matches!(envelope.request, EngineRequest::Interaction(_)));
    }

    #[test]
    fn synthetic_submission_receipt_is_distinct_from_completion() {
        let receipt = SubmissionReceipt {
            id: RequestId(5),
            status: SubmissionStatus::Queued,
        };
        let completion = RequestCompletion {
            id: RequestId(5),
            result: Err(RequestError::Execution(cad_domain::CadError::InvalidInput(
                "synthetic cause".into(),
            ))),
        };
        assert_eq!(receipt.id, completion.id);
        assert_eq!(receipt.status, SubmissionStatus::Queued);
        assert!(matches!(
            completion.result,
            Err(RequestError::Execution(cad_domain::CadError::InvalidInput(cause)))
                if cause == "synthetic cause"
        ));
    }

    #[test]
    fn mock_engine_client_implements_the_owner_thread_surface() {
        let client: &dyn EngineClient = &MockEngineClient {
            updates: RefCell::new(HostUpdates::default()),
            session: RenderSessionHandle::default(),
            user_preference: RefCell::new(None),
        };
        assert_eq!(client.request_context().viewport, ViewportId(3));
        assert!(matches!(
            client.dispatch(EngineRequest::Interaction(InteractionRequest::Cancel)),
            Err(RequestError::Busy)
        ));
        assert!(client.take_updates().unwrap().full.is_none());
        assert_eq!(client.config_snapshot().revision, 0);
        assert!(client.set_config_json("{}").is_ok());
        assert!(client.update_config_json("{}").is_ok());
        assert!(client.apply_user_preference_json("{}").is_ok());
        assert!(client.clear_user_preference().is_ok());
        assert!(client
            .set_canvas_metrics(CanvasMetrics::new([0.0; 2], [640.0, 480.0], 2.0))
            .is_ok());
    }

    #[test]
    fn mock_queue_surface_is_honestly_empty() {
        let client = MockEngineClient {
            updates: RefCell::new(HostUpdates::default()),
            session: RenderSessionHandle::default(),
            user_preference: RefCell::new(None),
        };
        let context = client.request_context();
        assert!(matches!(
            client.submit(
                context,
                EngineRequest::Interaction(InteractionRequest::Cancel)
            ),
            Err(RequestError::Busy)
        ));
        let report = client.pump().unwrap();
        assert_eq!((report.processed, report.pending), (0, false));
        assert!(client.poll_completions(8).is_empty());
        client.set_request_waker(Some(Rc::new(|| {})));
        client.set_request_waker(None);
    }

    #[test]
    fn mock_clear_user_preference_really_empties_the_store() {
        let client = MockEngineClient {
            updates: RefCell::new(HostUpdates::default()),
            session: RenderSessionHandle::default(),
            user_preference: RefCell::new(None),
        };
        assert!(client.user_preference.borrow().is_none());
        client.apply_user_preference_json(r#"{"ui":{}}"#).unwrap();
        assert_eq!(
            client.user_preference.borrow().as_deref(),
            Some(r#"{"ui":{}}"#)
        );
        client.clear_user_preference().unwrap();
        assert!(client.user_preference.borrow().is_none());
        // Clearing an already-empty store is a real no-op, never a fake success
        // that leaves stale state behind.
        client.clear_user_preference().unwrap();
        assert!(client.user_preference.borrow().is_none());
    }

    /// Minimal implementer that deliberately omits `clear_user_preference`, so
    /// the trait default is exercised rather than the mock override.
    struct NoStoreClient;

    impl EngineClient for NoStoreClient {
        fn dispatch(&self, _request: EngineRequest) -> Result<RequestResult, RequestError> {
            Err(RequestError::Busy)
        }
        fn request_context(&self) -> RequestContext {
            RequestContext {
                document: DocumentId(0),
                document_generation: 0,
                viewport: ViewportId(0),
                tool_generation: None,
                metrics_revision: None,
                config_revision: 0,
            }
        }
        fn take_updates(&self) -> CadResult<HostUpdates> {
            Ok(HostUpdates::default())
        }
        fn set_canvas_metrics(&self, _metrics: CanvasMetrics) -> CadResult<()> {
            Ok(())
        }
        fn render_session(&self) -> RenderSessionHandle {
            RenderSessionHandle::default()
        }
        fn config_snapshot(&self) -> ConfigSnapshot {
            ConfigSnapshot {
                effective: ViewerConfig::default(),
                revision: 0,
                preset: cad_app::viewer_config::Preset::Full,
                effective_json: "{}".into(),
            }
        }
        fn set_config_json(&self, _json: &str) -> CadResult<()> {
            Ok(())
        }
        fn update_config_json(&self, _json: &str) -> CadResult<()> {
            Ok(())
        }
        fn apply_user_preference_json(&self, _json: &str) -> CadResult<()> {
            Ok(())
        }
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

    #[test]
    fn default_clear_user_preference_refuses_instead_of_faking_success() {
        let client = NoStoreClient;
        assert!(matches!(
            client.clear_user_preference(),
            Err(CadError::Unsupported(reason))
                if reason.contains("clear_user_preference")
        ));
    }

    #[test]
    fn backend_updates_carry_raw_state_without_a_full_derivation() {
        use cad_app::recovery::ActiveBackendKind;
        use cad_app::BackendChoice;

        // A forced WebGL2 preference that actually ran on WebGPU: the raw `api`
        // label must follow the device, not the preference.
        let updates = HostUpdates {
            backend: Some(RenderBackendState {
                outcome: Some(BackendOutcome::Initialized {
                    preference: BackendChoice::WebGl2,
                    actual: ActiveBackendKind::WebGpu,
                }),
                api: Some("webgpu".into()),
            }),
            ..HostUpdates::default()
        };
        assert!(
            updates.full.is_none(),
            "backend delta must not force full state"
        );
        let backend = updates.backend.as_ref().unwrap();
        assert_eq!(backend.api.as_deref(), Some("webgpu"));
        assert!(backend.outcome.as_ref().unwrap().is_live());
        assert_eq!(
            backend.outcome.as_ref().unwrap().actual(),
            Some(ActiveBackendKind::WebGpu)
        );

        // Default (no device yet): both fields absent, and equality is structural.
        assert_eq!(
            RenderBackendState::default(),
            RenderBackendState {
                outcome: None,
                api: None,
            }
        );
    }

    fn idle_host_state() -> HostState {
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
            selected_draw_kind: None,
            measurement_preview: None,
            selected_measurement_kind: MeasurementToolKind::Distance,
            units: None,
            import_progress: None,
            diagnostics: Vec::new(),
            import: None,
            resources: ResourceState::default(),
        }
    }

    #[test]
    fn idle_draw_kind_is_none_and_only_a_capture_produces_some() {
        let idle = idle_host_state();
        assert_eq!(idle.selected_draw_kind, None);
        assert!(idle.units.is_none());
        let capturing = HostState {
            selected_draw_kind: Some(DrawToolKind::Line),
            ..idle.clone()
        };
        assert_ne!(
            idle.selected_draw_kind, capturing.selected_draw_kind,
            "idle must stay distinct from an active Line capture"
        );
        assert_eq!(capturing.selected_draw_kind, Some(DrawToolKind::Line));
    }

    #[test]
    fn catalog_status_preserves_key_and_every_raw_argument() {
        let status = StatusMessage::Catalog {
            key: "command.failed".into(),
            args: vec![
                StatusArgument {
                    key: "error".into(),
                    value: "unsupported source operation".into(),
                },
                StatusArgument {
                    key: "source".into(),
                    value: "drawing.dwg".into(),
                },
            ],
        };
        let StatusMessage::Catalog { key, args } = status else {
            panic!("catalog message expected");
        };
        assert_eq!(key, "command.failed");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0].key, "error");
        assert_eq!(args[0].value, "unsupported source operation");
        assert_eq!(args[1].key, "source");
        assert_eq!(args[1].value, "drawing.dwg");
    }
}
