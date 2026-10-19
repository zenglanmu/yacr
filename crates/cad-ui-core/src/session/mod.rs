//! Owner-thread CAD session (`CadSession`) shared by the UI hosts.
//!
//! `CadSession` is a thin, deliberately `!Send` owner-thread wrapper over the
//! UI-free [`cad_host::HostCore`] and its [`RenderSessionHandle`]. It adds **no**
//! CAD authority and **no** parser of its own: the authoritative database stays
//! in `cad-db` behind the host controller, every mutation goes through the host
//! command/engine boundary, and the frontend only sees the derived [`CadStatus`]
//! snapshot and the command admission result.
//!
//! Threading: the session owns `Rc`/`RefCell` state and a `RenderSessionHandle`,
//! so it is created, driven (`tick`/`handle_input`/`command`) and dropped on the
//! host main thread only. It is not `Send`/`Sync`.
//!
//! Scope: this module implements the CAD-facing slice only — status, commands,
//! pointer/wheel input, canvas metrics, completion accounting and resource
//! diagnostics. The per-host IPC wiring and the GPU presenter live in each host
//! and call into this module.
//!
//! The composed [`HostPlatform`] must already report a surface
//! ([`cad_platform::SurfaceSource::surface`]) when the session is built: the
//! host's open funnel validates canvas metrics against it, and an unmapped
//! surface is an explicit `Cancelled`, never a fabricated canvas.

mod commands;
mod diagnostics;
mod input;
mod status;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use cad_app::input::CanvasMetrics;
use cad_app::{
    CommandPayload, DrawToolKind, MeasurementToolKind, ToolParameter, ToolSnapshot, ToolValue,
};
use cad_domain::{CadResult, DocumentIdentity};
use cad_engine::protocol::{EngineClient, EngineRequest, RequestError, RequestId, StatusMessage};
use cad_engine::render::RenderSessionHandle;
use cad_host::{HostCore, HostOptions, HostPlatform};

use crate::protocol::ProbeInput;

// Names referenced only by intra-doc links in this module.
#[allow(unused_imports)]
use cad_app::input::DRAG_THRESHOLD_LOGICAL_PX;
#[allow(unused_imports)]
use cad_app::interaction::{InteractionRequest, SelectionMode};
#[allow(unused_imports)]
use cad_app::CommandId;

use self::diagnostics::{
    cad_error_text, camera_status, request_error_text, resource_diagnostic_text,
    resource_keys_error, status_error,
};
use self::input::{cancel_target, plan_input, CancelTarget, PointerPress};
use self::status::{assemble_status, cad_tool};

pub use self::commands::{
    admissible_names, canonical_command_name, measurement_selector_kind, normalize_command,
    unsupported_command_reason, zoom_step_factor, COMMAND_ALIASES, COMMAND_TABLE,
    MEASUREMENT_SELECTORS, ZOOM_IN_FACTOR, ZOOM_OUT_FACTOR, ZOOM_STEP_SELECTORS,
};
pub use self::status::{
    unsupported_status, CadCamera, CadImportProgress, CadSelection, CadSelectionRow, CadStatus,
    CadTool, CadToolParams,
};

/// Completions drained per poll. The queue's absolute capacity is 256, so this
/// always frees every outstanding completion (no quota leak).
const COMPLETION_DRAIN: usize = 256;
/// Bound on tracked receipt ids; the queue itself never holds more than 256.
const PENDING_RECEIPT_LIMIT: usize = 256;
/// The host's stable diagnostic code for a missing/unresolved resource.
const RESOURCE_MISSING_CODE: &str = "resource.missing";
/// The host's resource-install failure status key.
const RESOURCE_FAILED_STATUS: &str = "fonts.failed";

/// Owner-thread CAD session: host core + derived status + accounting.
pub struct CadSession {
    core: HostCore,
    /// Last real operation error (submit refusal, completion failure, tick),
    /// a technical string.
    error: RefCell<Option<String>>,
    /// Resource error projected from the host's update state (partial/missing
    /// images or a `resource.missing` diagnostic), technical.
    resource_error: RefCell<Option<String>>,
    /// Document generation whose stale render resource caches were already
    /// cleared after a failed install.
    resource_guard: Cell<Option<u64>>,
    /// Successful-import transition counter: the number of times the host
    /// published a replacement authoritative document from a successful import.
    /// Its key is `(document generation, import report identity)` — host
    /// replacement + import report — never the render session observing the
    /// incoming drawing.
    import_count: Cell<u64>,
    /// `(document generation, opened identity)` last counted, so a repeated
    /// snapshot never double-counts. Both components are import-only: the
    /// generation advances only on an authoritative document replacement, and
    /// the identity is the successful import report's own identity.
    last_import: RefCell<Option<(u64, DocumentIdentity)>>,
    /// Receipt ids submitted through this session and not yet observed terminal.
    pending: RefCell<VecDeque<RequestId>>,
    /// Last actual present count recorded through [`CadSession::note_frame`].
    frames: Cell<u64>,
    /// Press state of the single DOM pointer stream (the wire protocol carries
    /// no pointer id), so a release can be classified as a click or as the end
    /// of a navigation gesture (see [`plan_input`]).
    press: RefCell<Option<PointerPress>>,
}

impl CadSession {
    /// Build the session: create the host core, read startup config, install
    /// fonts/images, then open the configured drawing (or fit the demo).
    ///
    /// Every fallible step is propagated: a real resource or open failure is
    /// never turned into an empty success. Startup config diagnostics are
    /// retained on the core ([`HostCore::startup_diagnostics`]) and are not
    /// fatal.
    pub fn new(
        options: HostOptions,
        platform: HostPlatform,
        render: RenderSessionHandle,
    ) -> CadResult<Self> {
        let core = HostCore::new(options, platform, render)?;
        let _startup_diagnostics = core.load_startup();
        // Real resource installs through the platform strategy. A strategy that
        // cannot resolve refuses explicitly, so a failure here is genuine and
        // propagated rather than ignored.
        core.install_fonts()?;
        core.install_images()?;
        core.open_configured_drawing()?;
        Ok(Self {
            core,
            error: RefCell::new(None),
            resource_error: RefCell::new(None),
            resource_guard: Cell::new(None),
            import_count: Cell::new(0),
            last_import: RefCell::new(None),
            pending: RefCell::new(VecDeque::new()),
            frames: Cell::new(0),
            press: RefCell::new(None),
        })
    }

    /// The host-owned render session the GPU lane attaches and draws through.
    pub fn render_session(&self) -> RenderSessionHandle {
        self.core.render.clone()
    }

    /// Diagnostics from reading the startup config/preference files.
    pub fn startup_diagnostics(&self) -> Vec<String> {
        self.core.startup_diagnostics()
    }

    /// Apply validated canvas-local logical metrics (origin `[0, 0]`).
    pub fn set_canvas_metrics(&self, logical: [f64; 2], scale: f64) -> Result<(), String> {
        let metrics = CanvasMetrics::new([0.0; 2], logical, scale);
        self.core
            .set_canvas_metrics(metrics)
            .map_err(|error| cad_error_text(&error))
    }

    /// One owner-thread tick: host file/open/save/metrics/preparation pass,
    /// advance the engine request queue, consume terminal completions, observe
    /// import/resource transitions.
    ///
    /// The host `tick` already drives scene preparation
    /// ([`RenderSessionHandle::poll_preparation`]), so this never polls it a
    /// second time; no drawing is reparsed.
    pub fn tick(&self) -> Result<(), String> {
        self.core.tick().map_err(|error| cad_error_text(&error))?;
        let pump = self.pump();
        let completed = self.drain_completions();
        self.observe_import();
        self.guard_stale_resources();
        pump?;
        completed
    }

    /// Run one named CAD command through the host funnel.
    ///
    /// The full [`CommandId`] vocabulary is mapped by [`normalize_command`].
    /// The interactive tool commands are routed through the typed
    /// [`InteractionRequest`] path so the real host capture starts:
    ///
    /// | selector | request |
    /// | --- | --- |
    /// | `LINE`..`EXTEND` | `BeginDraw(<kind>)` |
    /// | `RECTANG` | `BeginDraw(Rectang)` |
    /// | `ELLIPSE` | `BeginDraw(Ellipse)` |
    /// | `POLYGON` | `BeginDraw(Polygon)` |
    /// | `POINT` | `BeginDraw(Point)` |
    /// | `SPLINE` | `BeginDraw(Spline)` |
    /// | `DIMLINEAR` / `DIMALIGNED` | `BeginDraw(DimLinear)` / `BeginDraw(DimAligned)` |
    /// | `DIMRADIUS` / `DIMDIAMETER` / `DIMCENTER` | `BeginDraw(DimRadius)` / `BeginDraw(DimDiameter)` / `BeginDraw(DimCenter)` |
    /// | `ARRAYRECT` / `ARRAYPOLAR` | `BeginDraw(ArrayRect)` / `BeginDraw(ArrayPolar)` |
    /// | `ALIGN` | `BeginDraw(Align)` |
    /// | `STRETCH` | `BeginDraw(Stretch)` |
    /// | `MEASURE` / `DIST` | `BeginMeasurement(Distance)` |
    /// | `MEASUREPOLYLINE` | `BeginMeasurement(PolylineLength)` |
    /// | `MEASUREANGLE` | `BeginMeasurement(Angle)` |
    /// | `AREA` | `BeginMeasurement(Area)` |
    /// | `PAN` | `BeginPan` (modal pan; never a zero-delta pan) |
    /// | `CONFIRM` | `Confirm` |
    /// | `CANCEL` | `Cancel` |
    ///
    /// The interaction-only selectors ([`MEASUREMENT_SELECTORS`],
    /// [`ZOOM_STEP_SELECTORS`]) carry no [`CommandId`] and are dispatched
    /// directly: a measurement selector starts its own
    /// [`MeasurementToolKind`] (so the four measurement buttons never collapse
    /// onto Distance), and `ZOOMIN`/`ZOOMOUT` synthesize a real
    /// [`CommandPayload::ZoomAt`] about the viewport centre — the deliberate
    /// behaviour of a cursor-less zoom button, not an invented no-op.
    ///
    /// `CANCEL` keeps the host-owned open-cancel behaviour (the same selector is
    /// the import-cancel affordance): it always submits the typed cancel, and
    /// forwards the ordinary command **only when no draw/measurement capture was
    /// active when the cancel was submitted** (see [`cancel_target`]) — an `Esc`
    /// that ended a tool must not also abort an unrelated in-flight import.
    ///
    /// Every other mapped command is dispatched as `core.command(CommandId,
    /// CommandPayload::None)`, exactly as before. A name that maps to no command
    /// is refused **before** the host is touched, with a stable reason, so an
    /// unsupported command changes nothing and is never a silent no-op.
    ///
    /// A command whose variant requires a payload (for example `CreateLine` or
    /// `StandardView`) is still dispatched through the same funnel; the
    /// application/host layer then refuses it explicitly rather than fabricating
    /// a success. Selection-requiring tools are **not** pre-rejected here:
    /// `cad_app` refuses them at confirm (`DrawTool::is_ready`), the single
    /// authority for that decision.
    pub fn command(&self, text: &str) -> Result<(), String> {
        let Some(name) = canonical_command_name(text) else {
            return Err(unsupported_command_reason(text));
        };
        // Interaction-only selectors carry no `CommandId`: dispatch them through
        // the same typed funnel the other tools use, before the `CommandId` path.
        if let Some(kind) = measurement_selector_kind(name) {
            return self.submit_interactions(vec![InteractionRequest::BeginMeasurement(kind)]);
        }
        if let Some(factor) = zoom_step_factor(name) {
            return self.zoom_step(factor);
        }
        let Some(id) = normalize_command(name) else {
            // Unreachable for a name `canonical_command_name` admitted (an alias
            // always resolves to a `COMMAND_TABLE` selector); keep the explicit
            // refusal rather than fabricating a dispatch.
            return Err(unsupported_command_reason(text));
        };
        let interaction = match id {
            CommandId::CreateLine => Some(InteractionRequest::BeginDraw(DrawToolKind::Line)),
            CommandId::CreateCircle => Some(InteractionRequest::BeginDraw(DrawToolKind::Circle)),
            CommandId::CreateArc => Some(InteractionRequest::BeginDraw(DrawToolKind::Arc)),
            CommandId::CreatePolyline => {
                Some(InteractionRequest::BeginDraw(DrawToolKind::Polyline))
            }
            CommandId::CreateRectangle => {
                Some(InteractionRequest::BeginDraw(DrawToolKind::Rectang))
            }
            CommandId::CreateEllipse => Some(InteractionRequest::BeginDraw(DrawToolKind::Ellipse)),
            CommandId::CreatePolygon => Some(InteractionRequest::BeginDraw(DrawToolKind::Polygon)),
            CommandId::CreatePoint => Some(InteractionRequest::BeginDraw(DrawToolKind::Point)),
            CommandId::CreateSpline => Some(InteractionRequest::BeginDraw(DrawToolKind::Spline)),
            CommandId::DimLinear => Some(InteractionRequest::BeginDraw(DrawToolKind::DimLinear)),
            CommandId::DimAligned => Some(InteractionRequest::BeginDraw(DrawToolKind::DimAligned)),
            CommandId::DimRadius => Some(InteractionRequest::BeginDraw(DrawToolKind::DimRadius)),
            CommandId::DimDiameter => {
                Some(InteractionRequest::BeginDraw(DrawToolKind::DimDiameter))
            }
            CommandId::DimCenter => Some(InteractionRequest::BeginDraw(DrawToolKind::DimCenter)),
            CommandId::MoveEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Move)),
            CommandId::CopyEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Copy)),
            CommandId::RotateEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Rotate)),
            CommandId::ScaleEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Scale)),
            CommandId::MirrorEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Mirror)),
            CommandId::OffsetEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Offset)),
            CommandId::TrimEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Trim)),
            CommandId::ExtendEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Extend)),
            CommandId::ChamferLines => Some(InteractionRequest::BeginDraw(DrawToolKind::Chamfer)),
            CommandId::FilletLines => Some(InteractionRequest::BeginDraw(DrawToolKind::Fillet)),
            CommandId::ArrayRectangular => {
                Some(InteractionRequest::BeginDraw(DrawToolKind::ArrayRect))
            }
            CommandId::ArrayPolar => Some(InteractionRequest::BeginDraw(DrawToolKind::ArrayPolar)),
            CommandId::AlignEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Align)),
            CommandId::StretchEntities => {
                Some(InteractionRequest::BeginDraw(DrawToolKind::Stretch))
            }
            CommandId::Measure => Some(InteractionRequest::BeginMeasurement(
                MeasurementToolKind::Distance,
            )),
            // The bare `PAN` selector is the modal-pan affordance, exactly like
            // the other interactive tools: `Application::pan` needs a `Points`
            // world delta a selector cannot supply, so routing it through
            // `BeginPan` is what makes the button work instead of erroring.
            CommandId::Pan => Some(InteractionRequest::BeginPan),
            CommandId::ConfirmMeasurement => Some(InteractionRequest::Confirm),
            CommandId::CancelLoading => Some(InteractionRequest::Cancel),
            _ => None,
        };
        match interaction {
            Some(request) => {
                // Sample the capture state **before** the request is submitted:
                // the typed cancel resets the tool, so a post-submit sample
                // would always read "no capture" and forward the host cancel
                // unconditionally (including when an Esc only meant to end a
                // tool while an unrelated import was running).
                let was_capturing = self.capturing();
                self.submit_interactions(vec![request])?;
                match (id, cancel_target(was_capturing)) {
                    // `CANCEL` is also the host-owned open-cancel selector; only
                    // forward it when no draw/measurement capture consumed it.
                    (CommandId::CancelLoading, CancelTarget::ToolAndHost) => {
                        self.dispatch_command(id)
                    }
                    _ => Ok(()),
                }
            }
            None => self.dispatch_command(id),
        }
    }

    /// Set one session tool parameter (ADR 0016 decision A) through the single
    /// typed interaction path.
    ///
    /// The value travels as [`InteractionRequest::SetToolParameter`] — the same
    /// engine queue, admission and completion discipline every other interaction
    /// uses — so this session never writes the authoritative parameter itself: the
    /// host (`cad-app`) owns it, exactly one path in. The host validates the
    /// parameter's own kind and range (e.g. `PolygonSides`
    /// `cad_app::draw_tool::MIN_POLYGON_SIDES`..=`MAX_POLYGON_SIDES`, 3..=1024, an
    /// integer) and refuses a bad value **explicitly** (never a silent clamp), and
    /// it refuses in **Viewer** mode because the parameter channel authorizes
    /// Work-only create/edit commands. Either refusal is returned here as the real
    /// completion error, never a silent no-op; on success the value is visible
    /// through [`CadToolParams::sides`] while POLYGON is active.
    pub fn set_tool_parameter(
        &self,
        parameter: ToolParameter,
        value: ToolValue,
    ) -> Result<(), String> {
        self.submit_interactions(vec![InteractionRequest::SetToolParameter {
            parameter,
            value,
        }])
    }

    /// Apply a fixed zoom step about the **viewport centre**.
    ///
    /// A `ZOOMIN`/`ZOOMOUT` button supplies no cursor, so the step is anchored
    /// at the logical centre of the active viewport — the deliberate behaviour
    /// of a cursor-less zoom control (a real zoom about the viewport centre that
    /// keeps the centre fixed), never a fabricated no-op. The
    /// [`CommandPayload::ZoomAt`] payload is synthesized here because the
    /// router's payload-bearing `ZOOM` selector otherwise requires a
    /// caller-supplied factor. The application validates the factor and
    /// projection and refuses explicitly on failure; the outcome is never turned
    /// into an empty success.
    fn zoom_step(&self, factor: f64) -> Result<(), String> {
        let centre = {
            let c = self.core.controller.borrow();
            c.application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .map(|viewport| {
                    [
                        viewport.logical_size[0] * 0.5,
                        viewport.logical_size[1] * 0.5,
                    ]
                })
        };
        let Some(cursor) = centre else {
            return Err("zoom step needs an active viewport".into());
        };
        match self
            .core
            .command(CommandId::Zoom, CommandPayload::ZoomAt { factor, cursor })
        {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Whether a draw/measurement capture currently owns pointer input.
    fn capturing(&self) -> bool {
        matches!(
            self.core.tool_snapshot(),
            ToolSnapshot::Drawing(_) | ToolSnapshot::Measuring(_)
        )
    }

    /// Dispatch one non-interactive command through the ordinary host funnel.
    fn dispatch_command(&self, id: CommandId) -> Result<(), String> {
        match self.core.command(id, CommandPayload::None) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Apply one frontend input through the typed engine interaction path.
    ///
    /// Pointer phases map to a typed [`InteractionRequest::Pointer`] with the
    /// **pressed button carried through** (the DOM `MouseEvent.button` numbers,
    /// see [`crate::protocol::ProbeInput::button`]), and a wheel maps to
    /// [`InteractionRequest::Scroll`] with the DOM deltaY sign inverted to the
    /// host's wheel-up-zooms-in convention.
    ///
    /// A press/release without an intervening drag is a **click**: outside a
    /// capture it emits the typed [`InteractionRequest::SelectAt`] (mode
    /// [`SelectionMode::Replace`]) at the release position; inside a draw or
    /// measurement capture the same click is a typed
    /// [`InteractionRequest::CapturePoint`] instead. Both are tagged with the
    /// **current** canvas metrics revision and both follow the ordinary release
    /// [`InteractionRequest::Pointer`] so the host's gesture life is closed. Any
    /// drag beyond [`DRAG_THRESHOLD_LOGICAL_PX`] (or a release of a different
    /// button) keeps the plain pointer mapping, which the host resolves as
    /// navigation (a primary/middle drag pans in 2D); a drag never captures a
    /// point. Box selection is **not** implemented: a primary drag still pans.
    ///
    /// `Enter` (`CONFIRM`) confirms a capture and `Esc`/`CANCEL` cancels it
    /// through the command funnel; this method only routes the pointer stream.
    ///
    /// The request(s) are admitted through the engine queue (`submit`), the
    /// queue is advanced (`pump`) and its terminal completions are consumed
    /// (`poll_completions`), which frees queue quota. The database is never
    /// mutated here directly. Input sequence ordering is already enforced by the
    /// frontend gate, so it is not duplicated.
    ///
    /// **Return semantics:** `Ok` means the input was *admitted* (and possibly
    /// already completed); it is not a claim that CAD work executed. `Err` means
    /// the input was malformed/unsupported (an undefined button is refused with
    /// a stable reason), admission was refused, or a terminal completion failed.
    /// The actual completion outcome is reflected in [`CadStatus::error`] from
    /// the next [`CadSession::snapshot`].
    pub fn handle_input(&self, input: &ProbeInput) -> Result<(), String> {
        let capturing = self.capturing();
        let (requests, next_press) = plan_input(
            self.press.borrow().as_ref().copied(),
            input,
            self.metrics_revision(),
            capturing,
        )?;
        self.submit_interactions(requests)?;
        *self.press.borrow_mut() = next_press;
        Ok(())
    }

    /// Admit a batch of typed interactions through the single engine path.
    ///
    /// Shared by [`CadSession::command`] (tool start/confirm/cancel) and
    /// [`CadSession::handle_input`] (pointer capture), so queue admission, the
    /// error funnel, pumping and completion draining exist exactly once. A
    /// refusal records the technical reason in [`CadSession::error`] and is
    /// returned; nothing is turned into an empty success.
    fn submit_interactions(&self, requests: Vec<InteractionRequest>) -> Result<(), String> {
        let context = self.core.request_context();
        for request in requests {
            match self
                .core
                .submit(context, EngineRequest::Interaction(request))
            {
                Ok(receipt) => self.track_receipt(receipt.id),
                Err(error) => {
                    let text = request_error_text(&error);
                    *self.error.borrow_mut() = Some(text.clone());
                    return Err(text);
                }
            }
        }
        let pump = self.pump();
        let completed = self.drain_completions();
        pump?;
        completed
    }

    /// Record the actual number of CAD frames the presenter has presented.
    ///
    /// The GPU lane reports its real present count here; the status never
    /// invents a frame, so a session that has presented nothing reports `0`.
    pub fn note_frame(&self, frame_count: u64) {
        self.frames.set(frame_count);
    }

    /// The last actual present count recorded through [`Self::note_frame`].
    pub fn frames_presented(&self) -> u64 {
        self.frames.get()
    }

    /// Build the frontend status snapshot from the current authoritative state.
    ///
    /// `backend`/`adapter`/`frame_count` are the GPU lane's actual values. The
    /// drawing label, entity count and camera come from the authoritative
    /// controller document/viewport; an unknown value stays `null` (a 3D or
    /// non-orthographic view has no 2D camera block, and a missing document has
    /// no counts or camera).
    ///
    /// This is the sole host-update consumer in this host, so it also harvests
    /// the host's projected resource state ([`HostCore::take_updates`]) to
    /// report missing/partial images; a partial resource set is surfaced as a
    /// technical `error`, never as a silent "complete".
    pub fn snapshot(
        &self,
        backend: Option<String>,
        adapter: Option<String>,
        frame_count: u64,
    ) -> CadStatus {
        self.observe_import();
        self.observe_resources();
        let (drawing, label, camera, selection) = {
            let c = self.core.controller.borrow();
            let drawing = c.drawing();
            let label = c.document_name_hint.clone();
            let camera = c
                .application
                .workspace
                .viewports
                .get(&c.viewport_id)
                .and_then(|viewport| viewport.camera.camera2d_params().ok())
                .and_then(camera_status);
            // The real current selection, read from the authoritative
            // controller each time the snapshot is built. Without a document
            // there is no property source, so only the real ref count travels
            // (never an omitted block and never fabricated rows).
            let selection = c
                .selection_properties()
                .map(|properties| CadSelection::from_properties(&properties))
                .unwrap_or_else(|_| CadSelection::refs_only(c.selection().len()));
            (drawing, label, camera, selection)
        };
        let has_document = drawing.is_some();
        let entity_count = drawing
            .as_ref()
            .map(|database| database.entity_count() as u64);
        let loading = self.core.loading();
        let can_open = self.core.capabilities.open
            && (!self.core.options.headless || self.core.options.drawing.is_some())
            && !loading;
        // A frame the caller did not measure falls back to the last real present
        // count the GPU lane recorded; nothing is invented either way.
        let frame_count = if frame_count == 0 {
            self.frames.get()
        } else {
            frame_count
        };
        // The current best snap candidate kind (ADR 0014 A.7) reuses the cached
        // hint resolution the snap overlay consumes, so a per-frame status read
        // never walks the drawing a second time. A resolution failure is never a
        // silent "no candidate": the field stays `null` and the technical reason
        // is folded into `error`, exactly like the other host diagnostics.
        let (snap_kind, snap_error) = match self.core.controller.borrow().snap_hint_kind() {
            Ok(kind) => (kind.map(str::to_string), None),
            Err(error) => (None, Some(cad_error_text(&error))),
        };
        // Whether the host is drawing its own pointer crosshair right now: a 2D
        // view with a pointer position on the surface. The frontend hides the
        // system cursor exactly when this is true, so a 3D view (which draws no
        // crosshair) keeps the system arrow instead of being left pointerless.
        // Read from the same overlay input the renderer draws the crosshair from
        // (`HostCore::sync_render` only publishes it for a valid 2D view), never
        // from a guess about the view mode.
        let crosshair = self.core.render.overlays().cursor.is_some();
        // Technical error sources, in priority order: a host status failure
        // (command/open/resource-install), a projected partial-resource report,
        // the last recorded operation/completion failure, the render session's
        // own diagnostic, then a snap-resolution failure.
        let error = self
            .core
            .last_status()
            .as_ref()
            .and_then(status_error)
            .or_else(|| self.resource_error.borrow().clone())
            .or_else(|| self.error.borrow().clone())
            .or_else(|| self.core.render.view_diagnostic())
            .or(snap_error);
        // The projected tool carries the session's typed parameter values, so it is
        // built from the tool snapshot **and** the authoritative session store.
        let tool = {
            let snapshot = self.core.tool_snapshot();
            let controller = self.core.controller.borrow();
            cad_tool(&snapshot, &controller.session)
        };
        let mut status = assemble_status(
            has_document,
            label,
            entity_count,
            loading,
            can_open,
            error,
            camera,
            backend,
            adapter,
            selection,
            self.import_count.get(),
            frame_count,
            tool,
            self.core.controller.borrow().hover_preview_enabled(),
            self.core.controller.borrow().snap_enabled(),
            snap_kind,
            crosshair,
        );
        // Real live open progress (running only) from the host snapshot, so the
        // chrome can show a determinate/indeterminate bar without inventing data.
        status.import_progress = self
            .core
            .import_snapshot()
            .as_ref()
            .and_then(CadImportProgress::from_snapshot);
        status
    }

    /// Advance the engine queue by one bounded pass.
    fn pump(&self) -> Result<(), String> {
        match self.core.pump() {
            Ok(_) => Ok(()),
            // A re-entrant pump (an active pass) is a no-op, not a failure: the
            // active pass is already advancing the queue. It never claims any
            // specific request executed.
            Err(RequestError::Busy) => Ok(()),
            Err(error) => Err(request_error_text(&error)),
        }
    }

    /// Remember an admitted request's receipt id until its terminal completion
    /// is observed. Bounded by the queue's absolute capacity.
    fn track_receipt(&self, id: RequestId) {
        let mut pending = self.pending.borrow_mut();
        if pending.len() >= PENDING_RECEIPT_LIMIT {
            pending.pop_front();
        }
        pending.push_back(id);
    }

    /// Drain every terminal completion (freeing queue quota) and propagate the
    /// outcomes of the requests this session admitted.
    ///
    /// A failed completion sets the session error and is returned; a successful
    /// completion clears the session error; a `Superseded` completion is a
    /// terminal non-error. A completion for a request that was never tracked is
    /// still drained (quota freed) but carries no session outcome.
    fn drain_completions(&self) -> Result<(), String> {
        let completions = self.core.poll_completions(COMPLETION_DRAIN);
        let mut failure: Option<String> = None;
        let mut success = false;
        for completion in completions {
            let tracked = {
                let mut pending = self.pending.borrow_mut();
                match pending.iter().position(|id| *id == completion.id) {
                    Some(index) => {
                        pending.remove(index);
                        true
                    }
                    None => false,
                }
            };
            if !tracked {
                continue;
            }
            match &completion.result {
                Ok(_) => success = true,
                // Displaced by a newer coalesced request: terminal, not an error.
                Err(RequestError::Superseded { .. }) => {}
                Err(error) => {
                    let text = request_error_text(error);
                    if failure.is_none() {
                        failure = Some(text);
                    }
                }
            }
        }
        match failure {
            Some(text) => {
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
            None => {
                if success {
                    *self.error.borrow_mut() = None;
                }
                Ok(())
            }
        }
    }

    /// Count a host-published successful-import transition exactly once.
    ///
    /// The counter is not a parsed-entity count and proves nothing about
    /// reparsing; it records that the authoritative document was replaced by a
    /// **successful import**. The discriminator is the pair
    /// `(document generation, import report identity)`, both import-only:
    /// `document generation` advances only when the host publishes a replacement
    /// authoritative drawing (`note_document_replaced` runs solely from the
    /// open/import/`NewDrawing` pipelines, never from an edit commit), and the
    /// identity comes from the successful import report — it is **not** the render
    /// session observing the incoming drawing.
    ///
    /// An edit commit swaps the controller's drawing `Arc` and the host re-syncs
    /// the render session every tick, so the render session's `incoming`
    /// database pointer changes on every plain edit; it is deliberately **not**
    /// part of the key so an edit can never re-count. A failed/pending open keeps
    /// the old report and generation, `NewDrawing` clears the report (the
    /// early-reset path below), and navigation touches neither, so none of them
    /// increments the counter.
    fn observe_import(&self) {
        let generation = self.core.request_context().document_generation;
        let identity = self
            .core
            .controller
            .borrow()
            .last_import_report
            .as_ref()
            .map(|report| report.identity.clone());
        let Some(identity) = identity else {
            // No import report (startup demo, or `NewDrawing`): reset so a later
            // successful import counts again.
            *self.last_import.borrow_mut() = None;
            return;
        };
        let key = (generation, identity);
        if self.last_import.borrow().as_ref() != Some(&key) {
            *self.last_import.borrow_mut() = Some(key);
            self.import_count
                .set(self.import_count.get().saturating_add(1));
        }
    }

    /// Project the host's resource state into a technical error when images are
    /// missing/partial, so a partial resource set is never reported as a silent
    /// complete. Uses the existing host `resources.images`/`resource.missing`
    /// data and severity; no UI prose is hardcoded.
    fn observe_resources(&self) {
        let Ok(updates) = self.core.take_updates() else {
            return;
        };
        let Some(full) = updates.full else {
            return;
        };
        let mut error: Option<String> = None;
        if let Some(images) = full.resources.images.as_ref() {
            if !images.unresolved.is_empty() || !images.failed.is_empty() {
                error = Some(resource_keys_error(
                    RESOURCE_MISSING_CODE,
                    images.unresolved.iter().chain(images.failed.iter()),
                ));
            }
        }
        if error.is_none() {
            if let Some(diagnostic) = full
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.code == RESOURCE_MISSING_CODE)
            {
                error = Some(resource_diagnostic_text(diagnostic));
            }
        }
        *self.resource_error.borrow_mut() = error;
    }

    /// After a failed resource install on a new document, clear the previous
    /// document's font/image caches on the render session so stale resources are
    /// never presented as if the new document resolved them. The GPU device is
    /// untouched. Guarded once per document generation.
    fn guard_stale_resources(&self) {
        let failed = matches!(
            self.core.last_status(),
            Some(StatusMessage::Catalog { ref key, .. }) if key == RESOURCE_FAILED_STATUS
        );
        if !failed {
            return;
        }
        let generation = self.core.request_context().document_generation;
        if self.resource_guard.get() == Some(generation) {
            return;
        }
        self.core.render.set_fonts(None);
        self.core.render.set_images(None);
        self.resource_guard.set(Some(generation));
    }

    /// The host's current canvas metrics revision, for tagging canvas-local
    /// interaction coordinates.
    fn metrics_revision(&self) -> u64 {
        self.core.request_context().metrics_revision.unwrap_or(0)
    }
}

#[cfg(test)]
mod tests;
