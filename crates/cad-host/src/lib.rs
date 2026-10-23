//! Platform-common host orchestration.
//!
//! [`HostCore`] owns the application session, renderer session, semantic updates,
//! and open/save/plot funnels. It is a cloneable owner-thread handle over `Rc` state.
//!
//! All platform I/O is expressed through the `cad-platform` contracts
//! ([`cad_platform::PlatformFiles`], [`cad_platform::ConfigStore`],
//! [`cad_platform::SurfaceSource`]); this crate never imports `wgpu`, a UI
//! framework, `web-sys` or any Android type, and carries no `cfg` for a host
//! platform.
//!
//! The database remains authoritative; application commands and host-owned file
//! operations share one synchronous dispatch boundary.
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use cad_app::host::HostController;
use cad_app::render_scene::{BoxSelectionPreview, OverlayInputs};
use cad_app::{
    input::CanvasMetrics,
    interaction::{
        InteractionRequest, Modifiers, PointInput, PointerButton, PointerId, SelectionMode,
    },
    viewer_config::ViewerConfigStore,
    Command, CommandId, CommandOutcome, CommandPayload, ImportProgressSnapshot, ImportTerminal,
    RawCommandTrace, ToolState,
};
use cad_db::DrawingDatabase;
use cad_domain::{CadError, CadResult, Point3, SelectionRef, SpaceId};
use cad_engine::{
    api::{ApiOutcome, ApiRequest, ApiTarget},
    events::EventRing,
    protocol::{
        ConfigSnapshot, EngineClient, EngineRequest, HostCapabilitiesState, HostState, HostUpdates,
        ImportState, InteractionOutcome, NavigationState, PumpReport, RawDiagnostic,
        RawDiagnosticParameter, RawEntityCapability, RenderBackendState, RequestCompletion,
        RequestContext, RequestError, RequestOrigin, RequestResult, ResourceState,
        SharedRequestResult, SnapHint, StatusArgument, StatusMessage, SubmissionReceipt,
    },
    queue::RequestQueue,
    render::{OverlayVisibility, RenderSessionHandle, ViewSnapshot},
};
use cad_platform::{
    ConfigLoad, ConfigStore, FileTarget, HostCapabilities, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};

mod command_log;
mod core;
mod fonts;
mod input;
mod open;
mod p4;
pub mod policy;
mod save;
mod settings;
mod state;

pub use command_log::{
    CommandLogEntry, CommandLogKind, CommandLogMeasurement, COMMAND_LOG_CAPACITY,
};
pub use settings::{
    normalize_wgpu_backend, CadSettingKey, CadSettingValue, HostSettings,
    SETTINGS_INVALID_PLOT_FORMAT, SETTINGS_INVALID_VALUE, SETTINGS_UNKNOWN_KEY,
    WGPU_BACKEND_PREFERENCE_KEY, WGPU_BACKEND_VALUES,
};
#[cfg(test)]
mod tests;
#[cfg(test)]
mod zoom_anchor_tests;

pub(crate) use core::{
    ActiveGuard, BoxGesture, FileAbort, HoverState, NavInner, PendingHover, PendingSave,
    RequestClock,
};
pub use core::{HostCore, HostOptions, HostPlatform};
#[cfg(not(target_arch = "wasm32"))]
pub use fonts::DesktopResources;
pub use fonts::{HostResources, UnsupportedResources};

/// Host policy mirroring the web host: users may persist the two dock panels'
/// initial-open flags and the Options-dialog settings that have real behavioural
/// backing (overlay visibility, the plot default, the two interaction switches,
/// the model/paper backgrounds, the crosshair size and the pickbox/aperture
/// sizes). A host `config.json` replaces this policy wholesale.
///
/// The settings leaves are listed individually (never a `view.overlays` prefix)
/// so no other configuration field becomes user-writable by inheritance.
pub const HOST_ALLOWED_PATHS_JSON: &str = r#"{"ui":{"userCustomization":{"allowedPaths":[
    "ui.components.layerPanel.initiallyOpen",
    "ui.components.propertiesPanel.initiallyOpen",
    "view.overlays.grid",
    "view.overlays.axes",
    "view.overlays.selectionHighlight",
    "view.overlays.snapHints",
    "view.background.model",
    "view.background.paper",
    "view.crosshair.sizePercent",
    "view.pickbox.sizePixels",
    "view.aperture.sizePixels",
    "ui.export.plotFormat",
    "interaction.snapEnabled",
    "interaction.hoverPreview"
]}}}"#;

/// A user-facing rendering of a command error.
///
/// `CadError::Display` delegates to `Debug`, so `to_string()` would print
/// `Unsupported("…")`/`NotImplemented("…")` with Debug quoting. This unwraps
/// those two variants to their inner (already-localized) string and falls back
/// to `Display` for the rest.
pub(crate) fn error_text(error: &CadError) -> String {
    match error {
        CadError::Unsupported(reason) => reason.clone(),
        CadError::NotImplemented(feature) => (*feature).to_string(),
        other => other.to_string(),
    }
}

/// Select valid platform surface metrics for the host canvas.
fn select_canvas_metrics(surface: SurfaceMetrics) -> CadResult<CanvasMetrics> {
    let metrics = cad_app::input::inset_canvas_metrics(
        [0.0; 2],
        surface.logical,
        surface.safe_insets,
        surface.scale,
    )
    .ok_or_else(|| CadError::InvalidInput("canvas metrics are degenerate".into()))?;
    Ok(CanvasMetrics::new(
        [0.0; 2],
        metrics.size_logical,
        metrics.dpi_scale,
    ))
}

impl HostCore {
    /// Build the core over a demo document and the platform contracts.
    pub fn new(
        options: HostOptions,
        platform: HostPlatform,
        render: RenderSessionHandle,
    ) -> CadResult<Self> {
        let controller = Rc::new(RefCell::new(HostController::with_demo_document(
            options.size,
        )?));
        let initial_metrics = CanvasMetrics::new([0.0; 2], options.size, 1.0);
        let mut viewer_config = ViewerConfigStore::default();
        viewer_config
            .update_config_json(HOST_ALLOWED_PATHS_JSON)
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        let initial_document = controller.borrow().drawing();
        if let Some(drawing) = initial_document.clone() {
            render.sync_drawing(Some(drawing));
        }
        // Native hosts get a real monotonic clock by default; wasm32 has none and
        // must inject one (an omitted clock is an explicit refusal, never a fake
        // elapsed of 0).
        #[cfg(not(target_arch = "wasm32"))]
        let default_clock: Option<Rc<dyn Fn() -> f64>> = {
            let clock = cad_engine::queue::PumpClock::new();
            Some(Rc::new(move || clock.elapsed_millis()))
        };
        #[cfg(target_arch = "wasm32")]
        let default_clock: Option<Rc<dyn Fn() -> f64>> = None;
        Ok(Self {
            controller,
            render,
            options,
            files: platform.files,
            config_store: platform.config_store,
            surface: platform.surface,
            resources: platform.resources,
            capabilities: platform.capabilities,
            viewer_config: Rc::new(RefCell::new(viewer_config)),
            canvas_metrics: Rc::new(Cell::new(initial_metrics)),
            metrics_revision: Rc::new(Cell::new(0)),
            full_dirty: Rc::new(Cell::new(true)),
            navigation_dirty: Rc::new(Cell::new(true)),
            config_dirty: Rc::new(Cell::new(true)),
            status_dirty: Rc::new(Cell::new(false)),
            last_backend: Rc::new(RefCell::new(None)),
            open_pending: Rc::new(Cell::new(false)),
            open_guard: Rc::new(RefCell::new(None)),
            open_abort: Rc::new(Cell::new(false)),
            open_target: Rc::new(RefCell::new(None)),
            open_failure: Rc::new(RefCell::new(None)),
            open_cancelled: Rc::new(Cell::new(false)),
            open_status_graded: Rc::new(Cell::new(false)),
            presenting_open: Rc::new(Cell::new(false)),
            save_pending: Rc::new(Cell::new(false)),
            pending_save: Rc::new(RefCell::new(None)),
            drawing_path: Rc::new(RefCell::new(None)),
            last_status: Rc::new(RefCell::new(None)),
            last_image_report: Rc::new(RefCell::new(None)),
            sync_terminal: Rc::new(RefCell::new(None)),
            open_attempt: Rc::new(Cell::new(0)),
            async_report_attempt: Rc::new(Cell::new(None)),
            navigation: Rc::new(NavInner::default()),
            hover: Rc::new(HoverState::default()),
            cursor_logical: Rc::new(Cell::new(None)),
            hover_now: Rc::new(std::time::Instant::now),
            startup_diagnostics: Rc::new(RefCell::new(Vec::new())),
            wgpu_backend: Rc::new(RefCell::new(None)),
            queue: Rc::new(RequestQueue::new()),
            request_waker: Rc::new(RefCell::new(None)),
            document_epoch: Rc::new(Cell::new(0)),
            last_document: Rc::new(RefCell::new(initial_document)),
            pump_active: Rc::new(Cell::new(false)),
            request_clock: Rc::new(RefCell::new(default_clock)),
            clock_sampling: Rc::new(Cell::new(false)),
            sync_active: Rc::new(Cell::new(false)),
            events: Rc::new(cad_engine::events::EventRing::new()),
            permits: Rc::new(RefCell::new(p4::PermitMap::new())),
            pending_trace: Rc::new(RefCell::new(Vec::new())),
            last_selection: Rc::new(RefCell::new(Vec::new())),
            selection_revision: Rc::new(Cell::new(0)),
            last_tool_generation: Rc::new(Cell::new(0)),
            command_log: Rc::new(RefCell::new(crate::command_log::CommandLog::default())),
        })
    }

    /// Whether the engine is busy for admission purposes: a pump is running, a
    /// clock is being sampled, or a public synchronous entry is executing.
    pub(crate) fn request_engine_busy(&self) -> bool {
        self.pump_active.get() || self.clock_sampling.get() || self.sync_active.get()
    }

    /// Run an external action under a single synchronous guard.
    ///
    /// Reuses [`Self::begin_sync_request`] (queued/executing/active check). The
    /// action runs with no internal borrow held (the queue snapshot is released
    /// before the call), and the RAII guard covers the whole action so a nested
    /// `dispatch`/`submit`/`pump` returns `Busy`. This performs no execution
    /// probe, no `sync_render` and no status side effect of its own: the caller's
    /// closure owns its own semantics (for example Web payload validation and
    /// store/reload, reporting its own catalog errors).
    pub fn with_sync_request<T>(
        &self,
        action: impl FnOnce() -> Result<T, RequestError>,
    ) -> Result<T, RequestError> {
        let _guard = self.begin_sync_request()?;
        action()
    }

    /// Record a post-commit host-state failure (render sync / status) as an
    /// explicit diagnostic, without turning the already-committed business
    /// completion into a failure.
    pub(crate) fn note_post_commit_error(&self, error: &CadError) {
        let _ = self.set_external_status(format!("post-commit state sync failed: {error:?}"));
    }

    /// Install the monotonic millisecond clock used for the pump time budget.
    ///
    /// The clock callback must return a finite, non-negative, non-decreasing
    /// timestamp in milliseconds. The first sample is validated (finite and
    /// non-negative) **before** it is installed, so a failing clock never
    /// replaces a working one; per-sample monotonicity is enforced across the
    /// pump pass. While queued/executing work or a pump/clock sample is active
    /// this returns the mapped Busy `CadError`.
    pub fn set_request_clock(&self, clock: RequestClock) -> CadResult<()> {
        if self.request_engine_busy() || self.queue.pending() || self.queue.executing_id().is_some()
        {
            return Err(Self::config_busy_error());
        }
        // Sampling a clock is itself a guarded window: a re-entrant
        // pump/dispatch/submit/set_request_clock must see Busy.
        let _guard = ActiveGuard::new(&self.clock_sampling);
        // The candidate is sampled (and its callback run) with no `request_clock`
        // borrow held, and only installed after the first sample validates.
        // Per-sample monotonicity is enforced by `pump` across the pass.
        let sample = clock();
        if !sample.is_finite() || sample < 0.0 {
            return Err(CadError::InvalidInput(
                "request clock must return a finite non-negative millisecond timestamp".into(),
            ));
        }
        *self.request_clock.borrow_mut() = Some(clock);
        Ok(())
    }

    /// Test-only: drop the installed clock to exercise the no-clock path.
    #[cfg(test)]
    pub(crate) fn clear_request_clock_for_test(&self) {
        *self.request_clock.borrow_mut() = None;
    }

    /// Test-only: install a deterministic instant source for the hover dwell.
    ///
    /// Production always uses `Instant::now`; this seam lets host tests advance
    /// the dwell without sleeping or spawning a timer. It is the smallest
    /// test-visible seam that keeps production on `std::time::Instant`.
    #[cfg(test)]
    pub(crate) fn set_hover_clock_for_test(&mut self, clock: Rc<dyn Fn() -> std::time::Instant>) {
        self.hover_now = clock;
    }

    /// Discard any in-progress pointer gesture.
    ///
    /// Called when the authoritative document, canvas metrics or config change:
    /// the pending gesture no longer refers to the current state. The hover
    /// resolved under the invalidated state is dropped with it, so a tint can
    /// never describe a document/metrics pair that is gone.
    pub(crate) fn clear_gestures(&self) {
        self.navigation.active.borrow_mut().clear();
        self.navigation.anchor.set(None);
        self.navigation.last.set(None);
        self.navigation.nav_last.set(None);
        self.navigation.dragged.set(false);
        self.navigation.navigating.set(false);
        self.navigation.button.set(None);
        self.navigation.press_modifiers.set(None);
        *self.navigation.box_gesture.borrow_mut() = None;
        self.clear_hover();
    }

    /// Record an admitted selector in the typed command echo.
    ///
    /// The UI layer owns the selector vocabulary (and `DIST`/`AREA`/zoom steps
    /// carry no `CommandId`), so it records the stable name at admission. This
    /// is observational only: it never admits, refuses or reorders anything.
    pub fn record_command(&self, name: &str) {
        self.command_log.borrow_mut().push_command(name);
    }

    /// The retained command echo, oldest first (at most
    /// [`COMMAND_LOG_CAPACITY`] entries).
    pub fn command_log_entries(&self) -> Vec<CommandLogEntry> {
        self.command_log.borrow().entries().cloned().collect()
    }

    pub(crate) fn record_command_result(&self, record: &cad_db::MeasurementRecord) {
        self.command_log.borrow_mut().push_result(record);
    }

    pub(crate) fn record_command_error(&self, error: &CadError) {
        self.command_log
            .borrow_mut()
            .push_error(command_log::error_code(error));
    }

    /// Record the authoritative document and bump the host document epoch iff it
    /// was actually replaced.
    ///
    /// Identity is `Arc::ptr_eq` **and** the database id, so a recycled pointer
    /// address is never mistaken for the same document. Called by every
    /// replacement entry (initial/new/open publish/external). A replacement also
    /// invalidates any in-progress gesture.
    pub(crate) fn note_document_replaced(&self) {
        let current = self.controller.borrow().drawing();
        let changed = {
            let last = self.last_document.borrow();
            match (last.as_ref(), current.as_ref()) {
                (Some(previous), Some(now)) => {
                    !Arc::ptr_eq(previous, now) || previous.id() != now.id()
                }
                (None, None) => false,
                _ => true,
            }
        };
        if changed {
            self.document_epoch
                .set(self.document_epoch.get().saturating_add(1));
            *self.last_document.borrow_mut() = current;
            self.clear_gestures();
            // The command echo belongs to one drawing: a verified replacement
            // (open/new) clears it so no entry from another drawing survives.
            // The sequence counter is kept monotonic across the clear.
            self.command_log.borrow_mut().clear();
            // A replacement rebuilds `SessionState`; re-apply the persisted
            // interaction defaults so an Options choice survives open/new.
            self.seed_session_interaction();
            // Same for the configured pickbox/aperture policy, which is derived
            // from the effective config and must survive a document replacement.
            self.seed_interaction_tolerance();
        }
    }

    /// The `CadError` a config write returns while queued work is pending.
    ///
    /// Mirrors the adapter's mapping of `RequestError::Busy`.
    pub(crate) fn config_busy_error() -> CadError {
        CadError::InvalidInput("engine is busy".into())
    }

    /// Refuse a config write while the queue has queued/executing work or a
    /// pump/clock/sync window is active.
    fn require_config_idle(&self) -> CadResult<()> {
        if self.request_engine_busy() || self.queue.pending() || self.queue.executing_id().is_some()
        {
            return Err(Self::config_busy_error());
        }
        Ok(())
    }

    pub fn set_status_message(&self, status: StatusMessage) -> CadResult<()> {
        *self.last_status.borrow_mut() = Some(status);
        self.status_dirty.set(true);
        Ok(())
    }

    pub fn set_catalog_status(&self, key: &str, values: &[(&str, &str)]) -> CadResult<()> {
        self.set_status_message(StatusMessage::Catalog {
            key: key.to_string(),
            args: values
                .iter()
                .map(|(key, value)| StatusArgument {
                    key: (*key).to_string(),
                    value: (*value).to_string(),
                })
                .collect(),
        })
    }

    pub fn set_external_status(&self, text: impl Into<String>) -> CadResult<()> {
        self.set_status_message(StatusMessage::ExternalText(text.into()))
    }

    /// Locale-neutral diagnostic text for errors that are returned to a caller.
    pub(crate) fn message(&self, key: &str, values: &[(&str, &str)]) -> String {
        let mut text = key.to_string();
        for (name, value) in values {
            text.push(' ');
            text.push_str(name);
            text.push('=');
            text.push_str(value);
        }
        text
    }

    pub fn last_status(&self) -> Option<StatusMessage> {
        self.last_status.borrow().clone()
    }

    /// Pure read of the session's current interactive tool state.
    ///
    /// Exposes the real `cad_app` snapshot (idle/selecting/drawing/measuring/
    /// panning) so callers can decide how to route pointer input without
    /// duplicating the tool state machine. Reading it has no side effects: it
    /// dispatches no command, captures no point and never mutates the capture.
    pub fn tool_snapshot(&self) -> cad_app::ToolSnapshot {
        self.controller.borrow().session.tool_snapshot()
    }

    /// Diagnostics from reading the host config / user preference at startup.
    pub fn startup_diagnostics(&self) -> Vec<String> {
        self.startup_diagnostics.borrow().clone()
    }

    /// Read the platform surface and apply validated canvas-local metrics.
    pub fn metrics(&self) -> CadResult<()> {
        let surface = self.surface.surface().ok_or(CadError::Cancelled)?;
        self.set_canvas_metrics(select_canvas_metrics(surface)?)
    }

    pub fn set_canvas_metrics(&self, canvas: CanvasMetrics) -> CadResult<()> {
        if !canvas.is_valid() || canvas.origin_logical != [0.0; 2] {
            return Err(CadError::InvalidInput(
                "canvas metrics must be valid and canvas-local".into(),
            ));
        }
        if self.canvas_metrics.get() == canvas {
            return Ok(());
        }
        self.canvas_metrics.set(canvas);
        self.metrics_revision
            .set(self.metrics_revision.get().saturating_add(1));
        // A metrics change invalidates world-space hovers and every *in-flight*
        // gesture whose positions were already recorded against the old canvas.
        // A stationary press (no movement, no box, not navigating) is preserved:
        // its canvas-local anchor is still a valid corner in the resized canvas,
        // and the first pointer event after a layout change reports the new DOM
        // rect **and** the press in the same batch. Clearing it here deleted that
        // press, so the following move had no anchor and collapsed to the
        // zero-area drag the move handler drops — the WINDOW/CROSSING box was
        // never dispatched (real GUI run: `wide_window_count=0`).
        if self.navigation.dragged.get()
            || self.navigation.navigating.get()
            || self.navigation.box_gesture.borrow().is_some()
        {
            self.clear_gestures();
        } else {
            self.clear_hover();
        }
        let mut c = self.controller.borrow_mut();
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .ok_or(CadError::Cancelled)?;
        cad_app::input::apply_canvas_metrics(viewport, &canvas)?;
        drop(c);
        self.navigation_dirty.set(true);
        self.sync_render()
    }

    /// Build and run one command through the single host funnel.
    ///
    /// Public synchronous entry: it is refused (mapped Busy `CadError`) while
    /// queued/executing work or a pump/clock/sync window is active. It routes
    /// through the unified P4 path (real `CommandStarted`/`CommandFinished`
    /// lifecycle + events), so host commands are traced exactly like engine
    /// requests.
    pub fn command(&self, id: CommandId, payload: CommandPayload) -> CadResult<()> {
        let c = self.controller.borrow();
        let command = Command {
            schema_version: 1,
            id,
            payload,
            document: c.document_id,
            viewport: c.viewport_id,
        };
        drop(c);
        self.execute(command)
    }

    /// Run one command through the unified P4 host/application boundary.
    ///
    /// A public synchronous write entry with the same Busy/RAII semantics as
    /// [`Self::command`], traced as a `Ui`-origin engine request.
    pub fn execute(&self, command: Command) -> CadResult<()> {
        match self.dispatch_with_origin_impl(RequestOrigin::Ui, EngineRequest::Command(command)) {
            Ok(completion) => match completion.result {
                Ok(_) => Ok(()),
                Err(error) => Err(p4::request_error_to_cad(error)),
            },
            Err(error) => Err(p4::request_error_to_cad(error)),
        }
    }

    /// Build and run a command with no admission/guard of its own.
    ///
    /// Used by `pump` and by the host's own navigation/interaction internals so
    /// they never self-Busy against the entry guard they are running under.
    pub(crate) fn immediate_command(
        &self,
        id: CommandId,
        payload: CommandPayload,
    ) -> CadResult<()> {
        let c = self.controller.borrow();
        let command = Command {
            schema_version: 1,
            id,
            payload,
            document: c.document_id,
            viewport: c.viewport_id,
        };
        drop(c);
        self.execute_with_outcome(command).map(|_| ())
    }

    /// Record one real command trace (host-owned operations record their own
    /// actual outcome; a `SwitchBackend` refusal records none, being no execution).
    fn record_raw_trace(&self, command: CommandId, result: Result<Arc<CommandOutcome>, CadError>) {
        self.pending_trace
            .borrow_mut()
            .push(RawCommandTrace { command, result });
    }

    /// Set a catalog status, recording a post-commit diagnostic if it fails.
    fn record_status(&self, key: &str, values: &[(&str, &str)]) {
        if let Err(error) = self.set_catalog_status(key, values) {
            self.note_post_commit_error(&error);
        }
    }

    fn execute_with_outcome(&self, command: Command) -> CadResult<Option<Arc<CommandOutcome>>> {
        let command_id = command.id;
        self.metrics()?;
        let hover_preview_before = self.controller.borrow().session.hover_preview();
        // The context a pending TRIM/EXTEND candidate was resolved in. A mode or
        // space switch invalidates it: the cursor did not move, but the entity
        // it resolved belongs to the previous context. The document's content
        // identity does too: the command line is not capture-modal, so an edit
        // that ran mid-capture (a typed `ERASE`/`UNDO`/`MOVE`) can remove the
        // entity the candidate/pick points at even though the mode and space are
        // unchanged. Keyed on the drawing's existing scene identity (id +
        // revision + structural counts + bounds), never a new counter.
        let capture_context_before = {
            let c = self.controller.borrow();
            (
                c.mode(),
                c.session.active_space.clone(),
                c.drawing().map(|drawing| drawing.scene_identity()),
            )
        };
        let owns_status = policy::owns_status(command_id);
        let none: Arc<CommandOutcome> = Arc::new(CommandOutcome::none());
        let result = match command.id {
            CommandId::CancelLoading => {
                self.cancel_loading();
                self.record_raw_trace(command_id, Ok(none));
                Ok(None)
            }
            // Host-owned funnels record their real host outcome (no App command
            // executed). An async Open success means the delivery STARTED, not
            // that the import finished.
            CommandId::OpenDrawing => match self.request_open() {
                Ok(()) => {
                    self.record_raw_trace(command_id, Ok(none));
                    Ok(None)
                }
                Err(error) => {
                    self.record_raw_trace(command_id, Err(error.clone()));
                    Err(error)
                }
            },
            CommandId::NewDrawing => match self.request_new() {
                Ok(()) => {
                    self.record_raw_trace(command_id, Ok(none));
                    Ok(None)
                }
                Err(error) => {
                    self.record_raw_trace(command_id, Err(error.clone()));
                    Err(error)
                }
            },
            CommandId::PlotDrawing => match self.request_plot() {
                Ok(()) => {
                    self.record_raw_trace(command_id, Ok(none));
                    Ok(None)
                }
                Err(error) => {
                    self.record_raw_trace(command_id, Err(error.clone()));
                    Err(error)
                }
            },
            CommandId::SaveDrawingAs => match self.request_save() {
                Ok(()) => {
                    self.record_raw_trace(command_id, Ok(none));
                    Ok(None)
                }
                Err(error) => {
                    self.record_raw_trace(command_id, Err(error.clone()));
                    Err(error)
                }
            },
            // A refusal that never executed records no command trace.
            CommandId::SwitchBackend => Err(CadError::Unsupported(
                "backend switching is not available through this host".into(),
            )),
            _ => {
                // Every other command runs through the traced app entry so its
                // single actual command trace is captured (shared `Arc` outcome).
                let traced = self.controller.borrow_mut().execute_traced(command)?;
                self.pending_trace.borrow_mut().extend(traced.commands);
                traced.result.map(Some)
            }
        };
        // Typed command echo (observational only): a real measurement result is
        // echoed as a `Result` entry; a refusal as an `Error` entry carrying only
        // the stable code. The `Diagnostic::message` strings are never copied.
        match &result {
            Ok(Some(outcome)) => {
                if let Some(record) = &outcome.measurement {
                    self.record_command_result(record);
                }
            }
            Ok(None) => {}
            Err(error) => self.record_command_error(error),
        }
        let open_graded = self.open_status_graded.replace(false);
        // A mode/space switch, or a document content change, invalidates the
        // pending capture candidate/picks even though no cursor move happened:
        // clear the capture's presentation candidate and the transient hover so
        // no stale `FLAG_HOVERED` tint survives (the candidate is host-visible
        // through the render overlay, and `sync_render` below republishes the
        // empty set and re-derives the pick tint against the new document).
        //
        // Clearing the candidate/hover is deliberate over cancelling the whole
        // capture: an edit that does not remove the picks (a different entity
        // erased, a selection MOVE) must leave the capture usable — the next
        // cursor move re-resolves the candidate against the new document — while
        // a capture whose picks became invalid is refused honestly at CONFIRM by
        // the command layer rather than silently committed against stale refs.
        let capture_context_changed = {
            let c = self.controller.borrow();
            c.mode() != capture_context_before.0
                || c.session.active_space != capture_context_before.1
                || c.drawing().map(|drawing| drawing.scene_identity()) != capture_context_before.2
        };
        if capture_context_changed {
            self.controller
                .borrow_mut()
                .session
                .set_draw_candidate(None);
            self.clear_hover();
        }
        // A toggle that disables the idle preview must drop the already painted
        // tint even when the pointer never moves again: the command reached the
        // host through this single funnel, so this is the one transition point.
        // The `sync_render` below republishes the empty overlay inputs.
        if command_id == CommandId::ToggleHoverPreview
            && hover_preview_before
            && !self.controller.borrow().session.hover_preview()
        {
            self.clear_hover();
        }
        // Status grading and the render sync are POST-business: a failure here is
        // an explicit diagnostic, never a reclassification of the committed
        // business completion.
        if let Err(error) = &result {
            if !owns_status {
                self.record_status("command.failed", &[("error", &error_text(error))]);
            }
        }
        if result.is_ok() && !self.loading() && !owns_status && !open_graded {
            let diagnostic = result
                .as_ref()
                .ok()
                .and_then(|outcome| outcome.as_ref())
                .and_then(|outcome| outcome.diagnostics.first());
            if diagnostic.is_some_and(|d| d.code == "fit.empty") {
                self.record_status("fit.empty", &[]);
            } else {
                self.record_status("command.completed", &[]);
            }
        }
        if matches!(command_id, CommandId::Pan | CommandId::Zoom) {
            self.navigation_dirty.set(true);
        } else {
            self.full_dirty.set(true);
            self.navigation_dirty.set(true);
        }
        if let Err(error) = self.sync_render() {
            self.note_post_commit_error(&error);
        }
        result
    }

    /// Whether a save-target chooser is in flight or a chosen target is awaiting
    /// its export.
    ///
    /// A platform with an asynchronous save picker (Android SAF) needs its poll
    /// driver to keep ticking until the chosen target has been drained and
    /// exported; a synchronous platform clears this within the gesture.
    pub fn save_pending(&self) -> bool {
        self.save_pending.get() || self.pending_save.borrow().is_some()
    }

    /// Whether a document open is in flight (picker, read, import or presentation).
    pub fn loading(&self) -> bool {
        self.open_pending.get()
            || self.presenting_open.get()
            || self
                .controller
                .borrow()
                .async_open_snapshot()
                .is_some_and(|s| s.running)
    }

    /// Cancel an in-flight open/read (`CancelLoading`).
    ///
    /// Sets the abort flag so a platform read that completes *after* the cancel
    /// is discarded rather than installed, and asks the async import manager to
    /// cancel a job that has already started.
    pub(crate) fn cancel_loading(&self) {
        self.open_abort.set(true);
        self.controller.borrow_mut().cancel_async_open();
    }

    /// The import-drawer snapshot for the current open state.
    ///
    /// Composition, in order:
    /// 1. the controller's real async snapshot (a worker job, running or its
    ///    retained terminal) always wins, so a synchronous terminal can never
    ///    pollute an active worker;
    /// 2. otherwise the Host's own synchronous terminal (real opened entity
    ///    count, or the original failure) so a completed sync deliver stays
    ///    observable after `loading()` goes false;
    /// 3. otherwise, while a platform read is in flight (picker/read, no job
    ///    yet) a synthesized running snapshot whose `cancellable` reflects the
    ///    abort flag; idle reports `None`.
    pub fn import_snapshot(&self) -> Option<ImportProgressSnapshot> {
        // The controller's retained async snapshot is scoped to the Host open
        // attempt that actually started the job: an old worker's terminal must
        // never mask a new picker/read or a new synchronous deliver. Within the
        // current attempt its running/terminal state still wins.
        let async_eligible = self.async_report_attempt.get() == Some(self.open_attempt.get());
        if async_eligible {
            if let Some(snapshot) = self.controller.borrow().async_open_snapshot() {
                return Some(snapshot);
            }
        }
        if let Some(terminal) = self.sync_terminal.borrow().as_ref() {
            return Some(terminal.clone());
        }
        if self.open_pending.get() {
            let mut running = ImportProgressSnapshot::running();
            running.cancellable = !self.open_abort.get();
            return Some(running);
        }
        None
    }

    /// Read and apply the durable config and user preference at startup.
    ///
    /// An absent file is normal; a malformed or unreadable one keeps the
    /// previous state and adds a human-readable diagnostic. Returns every
    /// diagnostic.
    pub fn load_startup(&self) -> Vec<String> {
        let mut diagnostics = Vec::new();
        if let Err(error) = self.update_config_json(HOST_ALLOWED_PATHS_JSON) {
            diagnostics.push(format!("host config defaults rejected: {error}"));
        }

        match self.config_store.load_config() {
            ConfigLoad::Absent => {}
            ConfigLoad::Text { path, text } => {
                if let Err(error) = self.set_config_json(&text) {
                    diagnostics.push(format!("config file {path} rejected: {error}"));
                }
            }
            ConfigLoad::Unreadable { path, reason } => {
                diagnostics.push(format!("config file {path} unreadable: {reason}"));
            }
        }

        match self.config_store.load_preference() {
            ConfigLoad::Absent => {}
            ConfigLoad::Text { path, text } => {
                // The renderer backend choice is a root preference leaf outside
                // the `ViewerConfig` schema, so the allowed-path projection above
                // cannot carry it: capture it here (unknown/absent -> `auto`) so
                // the settings snapshot reports it and every later write keeps it.
                *self.wgpu_backend.borrow_mut() = settings::wgpu_backend_from_preference(&text);
                match self.apply_user_preference_json(&settings::without_wgpu_backend(&text)) {
                    Ok(()) => {
                        if let Err(error) = self.persist_preference() {
                            diagnostics
                                .push(format!("preferences {path} not re-persisted: {error}"));
                        }
                    }
                    Err(error) => diagnostics.push(format!("preferences {path} rejected: {error}")),
                }
            }
            ConfigLoad::Unreadable { path, reason } => {
                diagnostics.push(format!("preferences {path} unreadable: {reason}"));
            }
        }

        // The loaded preference must actually seed the live session, not just
        // sit in the config store: `CadStatus.snap_enabled`/`hover_preview` read
        // `SessionState`, whose constructor defaults both switches on.
        self.seed_session_interaction();
        // The pickbox/aperture are config-derived rather than session switches,
        // so the same startup pass installs the persisted policy on the live
        // session (the next interaction would derive it anyway; doing it here
        // keeps the first frame and the first pick consistent).
        self.seed_interaction_tolerance();

        *self.startup_diagnostics.borrow_mut() = diagnostics.clone();
        diagnostics
    }

    /// Write the store's allowed projection of the user preference.
    ///
    /// The renderer backend choice sits outside the `ViewerConfig` schema, so it
    /// is merged back into the projected document here — every preference write
    /// (a setting, a startup re-persist, a clear) therefore keeps exactly one
    /// root `wgpu_backend` leaf and never drops it.
    fn persist_preference(&self) -> CadResult<()> {
        let projected = self.viewer_config.borrow().projected_user_preference_json();
        let merged = settings::merge_wgpu_backend_into_preference(
            &projected,
            self.wgpu_backend.borrow().as_deref(),
        )?;
        self.config_store.persist_preference(&merged)
    }

    pub fn config_snapshot(&self) -> ConfigSnapshot {
        let store = self.viewer_config.borrow();
        ConfigSnapshot {
            effective: store.effective().clone(),
            revision: store.revision,
            preset: store.effective().ui.preset,
            effective_json: store.effective_json(),
        }
    }

    pub fn set_config_json(&self, text: &str) -> CadResult<()> {
        self.require_config_idle()?;
        self.viewer_config
            .borrow_mut()
            .set_config_json(text)
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        self.clear_gestures();
        self.config_dirty.set(true);
        self.full_dirty.set(true);
        self.sync_render()
    }

    pub fn update_config_json(&self, text: &str) -> CadResult<()> {
        self.require_config_idle()?;
        let result = self
            .viewer_config
            .borrow_mut()
            .update_config_json(text)
            .map_err(|error| CadError::InvalidInput(error.to_string()));
        result?;
        self.clear_gestures();
        self.config_dirty.set(true);
        self.full_dirty.set(true);
        self.sync_render()
    }

    /// Apply a host-allowed user preference, then persist the allowed projection.
    ///
    /// The projection is written after the store accepts the patch (not from a
    /// `ConfigObserver`, which would re-borrow the store while it is mutably
    /// borrowed).
    pub fn apply_user_preference_json(&self, text: &str) -> CadResult<()> {
        self.require_config_idle()?;
        self.viewer_config
            .borrow_mut()
            .apply_user_preference_json(text)
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        self.clear_gestures();
        self.persist_preference()?;
        self.config_dirty.set(true);
        self.full_dirty.set(true);
        self.sync_render()
    }

    /// Drop the stored user preference so the host defaults apply again.
    ///
    /// This is a real clear of the single [`ViewerConfigStore`]: the accumulated
    /// preference is emptied, the allowed (now empty) projection is persisted
    /// through the same policy as [`Self::apply_user_preference_json`], and the
    /// config + render inputs are republished. It never fakes success: a failed
    /// persistence returns `Err` with the dirty markers retained for a retry,
    /// and there is no UI callback.
    pub fn clear_user_preference(&self) -> CadResult<()> {
        self.require_config_idle()?;
        self.viewer_config.borrow_mut().clear_user_preference();
        // The renderer backend choice is a user preference too, even though it
        // is not a `ViewerConfig` leaf: a real clear resets it to the `auto`
        // default and removes the leaf from the durable document.
        *self.wgpu_backend.borrow_mut() = None;
        self.clear_gestures();
        // Mark the publish before the fallible persistence, so a failure keeps
        // the change pending instead of silently dropping it.
        self.config_dirty.set(true);
        self.full_dirty.set(true);
        self.persist_preference()?;
        self.sync_render()
    }

    /// One timer tick: due hover dwell → file events → open/save funnels →
    /// metrics → render sync.
    ///
    /// The due-hover application is the only timing hook for the hover dwell; it
    /// is a no-op when nothing is pending or the deadline has not elapsed, so a
    /// stable tick schedules no render revision churn.
    pub fn tick(&self) -> CadResult<()> {
        self.apply_due_hover();
        self.poll_open()
            .and_then(|_| self.poll_pending_save())
            .and_then(|_| self.metrics())
            .and_then(|_| self.sync_view())
            .and_then(|_| self.render.poll_preparation().map(|_| ()))
    }

    /// Open the configured `--open` drawing at startup (or fit the demo).
    ///
    /// Uses the already-owned render session; no UI bridge is required.
    pub fn open_configured_drawing(&self) -> CadResult<()> {
        self.metrics()?;
        if let Some(path) = &self.options.drawing {
            let target = FileTarget::from_path(&path.to_string_lossy());
            self.begin_open_attempt();
            self.open_pending.set(true);
            if let Err(error) = self.files.begin_open_target(&target) {
                self.open_pending.set(false);
                return Err(error);
            }
            self.drain_files()?;
        } else {
            self.controller.borrow_mut().fit()?;
        }
        Ok(())
    }
}
