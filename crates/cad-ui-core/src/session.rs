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

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;

use cad_app::input::CanvasMetrics;
use cad_app::interaction::{
    InteractionRequest, Modifiers, PointInput, PointerButton, PointerId, PointerKind, PointerPhase,
};
use cad_app::{CommandId, CommandPayload};
use cad_domain::{CadError, CadResult, DocumentIdentity};
use cad_engine::protocol::{
    EngineClient, EngineRequest, RawDiagnostic, RawDiagnosticParameter, RequestError, RequestId,
    StatusMessage,
};
use cad_engine::render::RenderSessionHandle;
use cad_host::{HostCore, HostOptions, HostPlatform};
use serde::Serialize;

use crate::protocol::{InputKind, ProbeInput};

/// The single pointer stream identity (the DOM protocol carries no id).
const POINTER_ID: PointerId = PointerId(1);
/// Completions drained per poll. The queue's absolute capacity is 256, so this
/// always frees every outstanding completion (no quota leak).
const COMPLETION_DRAIN: usize = 256;
/// Bound on tracked receipt ids; the queue itself never holds more than 256.
const PENDING_RECEIPT_LIMIT: usize = 256;
/// The host's stable diagnostic code for a missing/unresolved resource.
const RESOURCE_MISSING_CODE: &str = "resource.missing";
/// The host's resource-install failure status key.
const RESOURCE_FAILED_STATUS: &str = "fonts.failed";

/// Camera block of the frontend status (`{center, scale}`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CadCamera {
    /// World point at the centre of the viewport.
    pub center: [f64; 2],
    /// World units per logical pixel (strictly positive).
    pub scale: f64,
}

/// Live import progress for the frontend progress affordance.
///
/// Present **only while an import is genuinely running**: a terminal snapshot
/// (opened/cancelled/failed) is never projected here, so an idle or finished
/// document carries `importProgress: null` and the chrome shows no bar. Every
/// field is a real observed value — `entities_total`/`bytes` stay `null` when
/// the importer did not expose them, which the UI renders as an indeterminate
/// bar without a byte count; nothing is extrapolated to 100%.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadImportProgress {
    /// Stable machine key of the latest phase (e.g. `"entities"`); `null` before
    /// the first progress tick. The UI maps it to a catalog label.
    pub phase: Option<String>,
    /// Real entities normalised/inserted so far (`0` before the first tick).
    pub entities_done: u64,
    /// Total entities expected, or `null` when the format does not expose it.
    pub entities_total: Option<u64>,
    /// Source bytes measured so far, only when actually known.
    pub bytes: Option<u64>,
    /// Whether a cancel request can still be issued for this import.
    pub cancellable: bool,
}

impl CadImportProgress {
    /// Project a running [`cad_app::ImportProgressSnapshot`]; a terminal snapshot
    /// maps to `None`, so the affordance is only ever driven by a live import.
    fn from_snapshot(snapshot: &cad_app::ImportProgressSnapshot) -> Option<Self> {
        if !snapshot.running {
            return None;
        }
        Some(CadImportProgress {
            phase: snapshot.phase_key().map(str::to_string),
            entities_done: snapshot.entities_done as u64,
            entities_total: snapshot.entities_total.map(|total| total as u64),
            bytes: snapshot.bytes,
            cancellable: snapshot.cancellable,
        })
    }
}

/// The `cad_status` snapshot consumed by the frontend.
///
/// The field set is the frozen frontend contract: every key is always present,
/// an unknown value is `null`, and a count is never fabricated. `error` is a
/// technical, locale-neutral string from the real host state (a command/
/// completion failure, a resource diagnostic or a render diagnostic); it is
/// never a localized UI label. `import_progress` carries the real, live open
/// progress while one is running (see [`CadImportProgress`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadStatus {
    pub document_label: Option<String>,
    pub entity_count: Option<u64>,
    pub loading: bool,
    /// Live import progress, or `null` when no import is running.
    pub import_progress: Option<CadImportProgress>,
    pub error: Option<String>,
    pub can_open: bool,
    pub can_fit: bool,
    pub backend: Option<String>,
    pub adapter: Option<String>,
    pub camera: Option<CadCamera>,
    /// Count of verified successful authoritative document transitions caused by
    /// an import. It is **not** a parsed-entity count and must not be read as
    /// proof that no reparse happened.
    pub import_count: u64,
    pub frame_count: u64,
}

/// A `cad_status` snapshot that advertises no CAD capability (probe mode or a
/// failed session): an explicit error, never a fabricated document/count.
pub fn unsupported_status(error: Option<String>) -> CadStatus {
    CadStatus {
        document_label: None,
        entity_count: None,
        loading: false,
        import_progress: None,
        error,
        can_open: false,
        can_fit: false,
        backend: None,
        adapter: None,
        camera: None,
        import_count: 0,
        frame_count: 0,
    }
}

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
    /// Verified successful-import transition counter.
    import_count: Cell<u64>,
    /// `(document generation, opened identity, incoming database pointer)` last
    /// counted, so a repeated snapshot never double-counts.
    last_import: RefCell<Option<(u64, DocumentIdentity, usize)>>,
    /// Receipt ids submitted through this session and not yet observed terminal.
    pending: RefCell<VecDeque<RequestId>>,
    /// Last actual present count recorded through [`CadSession::note_frame`].
    frames: Cell<u64>,
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
    /// The full [`CommandId`] vocabulary is mapped by [`normalize_command`];
    /// every mapped command is dispatched as `core.command(CommandId,
    /// CommandPayload::None)`. A name that maps to no command is refused
    /// **before** the host is touched, with a stable reason, so an unsupported
    /// command changes nothing and is never a silent no-op.
    ///
    /// A command whose variant requires a payload (for example `CreateLine` or
    /// `StandardView`) is still dispatched through the same funnel; the
    /// application/host layer then refuses it explicitly rather than fabricating
    /// a success.
    pub fn command(&self, text: &str) -> Result<(), String> {
        let Some(id) = normalize_command(text) else {
            return Err(unsupported_command_reason(text));
        };
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
    /// Pointer phases map to a typed [`InteractionRequest::Pointer`] (the host
    /// owns the navigation decisions: a single left drag pans the view through
    /// the host's `BeginPan` navigation and is intentionally **not** selection
    /// or editing), and a wheel maps to [`InteractionRequest::Scroll`] with the
    /// DOM deltaY sign inverted to the host's wheel-up-zooms-in convention.
    ///
    /// The request is admitted through the engine queue (`submit`), the queue is
    /// advanced (`pump`) and its terminal completions are consumed
    /// (`poll_completions`), which frees queue quota. The database is never
    /// mutated here directly. Input sequence ordering is already enforced by the
    /// frontend gate, so it is not duplicated.
    ///
    /// **Return semantics:** `Ok` means the input was *admitted* (and possibly
    /// already completed); it is not a claim that CAD work executed. `Err` means
    /// the input was malformed/unsupported, admission was refused, or a
    /// terminal completion failed. The actual completion outcome is reflected in
    /// [`CadStatus::error`] from the next [`CadSession::snapshot`].
    pub fn handle_input(&self, input: &ProbeInput) -> Result<(), String> {
        let request = interaction_request(input, self.metrics_revision())?;
        let context = self.core.request_context();
        let receipt = match self
            .core
            .submit(context, EngineRequest::Interaction(request))
        {
            Ok(receipt) => receipt,
            Err(error) => {
                let text = request_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                return Err(text);
            }
        };
        self.track_receipt(receipt.id);
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
        let (drawing, label, camera) = {
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
            (drawing, label, camera)
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
        // Technical error sources, in priority order: a host status failure
        // (command/open/resource-install), a projected partial-resource report,
        // the last recorded operation/completion failure, then the render
        // session's own diagnostic.
        let error = self
            .core
            .last_status()
            .as_ref()
            .and_then(status_error)
            .or_else(|| self.resource_error.borrow().clone())
            .or_else(|| self.error.borrow().clone())
            .or_else(|| self.core.render.view_diagnostic());
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
            self.import_count.get(),
            frame_count,
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

    /// Count a verified successful import transition exactly once.
    ///
    /// The counter is not a parsed-entity count and proves nothing about
    /// reparsing; it records that the authoritative document was replaced by a
    /// **successful import**, using the host document generation, the opened
    /// document identity and the render session's observed incoming database.
    /// A failed/pending open keeps the old report and database, `NewDrawing`
    /// clears the report, and navigation never touches either, so none of them
    /// increments the counter. A missing incoming database is not a verified
    /// transition and is never counted with a `0` sentinel.
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
        let Some(incoming) = self
            .core
            .render
            .incoming_document()
            .borrow()
            .as_ref()
            .map(|database| Arc::as_ptr(database) as usize)
        else {
            // The render session has not observed the replacement yet: not a
            // verified transition, so do not count.
            return;
        };
        let key = (generation, identity, incoming);
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

/// Assemble the frozen frontend status from already-derived inputs.
///
/// Pure so the empty-document capability rules are directly testable: without a
/// document there is no label, no entity count, no camera and no `FIT`.
#[allow(clippy::too_many_arguments)]
fn assemble_status(
    has_document: bool,
    label: String,
    entity_count: Option<u64>,
    loading: bool,
    can_open: bool,
    error: Option<String>,
    camera: Option<CadCamera>,
    backend: Option<String>,
    adapter: Option<String>,
    import_count: u64,
    frame_count: u64,
) -> CadStatus {
    CadStatus {
        document_label: has_document.then_some(label),
        entity_count,
        loading,
        import_progress: None,
        error,
        can_open,
        can_fit: has_document && !loading,
        backend,
        adapter,
        camera: if has_document { camera } else { None },
        import_count,
        frame_count,
    }
}

/// The canonical command vocabulary: every [`CommandId`] variant with the stable
/// uppercase selector a frontend uses to name it.
///
/// This is the single mapping table; it is deliberately exhaustive over the
/// `cad-app` command set so a host never has to special-case a command. A
/// selector whose variant needs a payload still dispatches through the same
/// funnel and is refused explicitly by the application/host layer.
pub const COMMAND_TABLE: &[(&str, CommandId)] = &[
    ("OPEN", CommandId::OpenDrawing),
    ("NEW", CommandId::NewDrawing),
    ("PLOT", CommandId::PlotDrawing),
    ("SAVE", CommandId::SaveDrawingAs),
    ("CANCEL", CommandId::CancelLoading),
    ("PAN", CommandId::Pan),
    ("ZOOM", CommandId::Zoom),
    ("FIT", CommandId::FitDrawing),
    ("RESETVIEW", CommandId::ResetView),
    ("TOGGLELAYER", CommandId::ToggleLayer),
    ("SETLAYERS", CommandId::SetLayerVisibilities),
    ("RESTORELAYERS", CommandId::RestoreLayers),
    ("SPACE", CommandId::SwitchSpace),
    ("SELECT", CommandId::Select),
    ("SELECTALL", CommandId::SelectAll),
    ("MEASURE", CommandId::Measure),
    ("CONFIRM", CommandId::ConfirmMeasurement),
    ("CANCELMEASURE", CommandId::CancelMeasurement),
    ("UNDO", CommandId::Undo),
    ("REDO", CommandId::Redo),
    ("RESOURCES", CommandId::Resources),
    ("DIAGNOSTICS", CommandId::Diagnostics),
    ("SWITCHBACKEND", CommandId::SwitchBackend),
    ("SWITCH2D3D", CommandId::Switch2d3d),
    ("STANDARDVIEW", CommandId::StandardView),
    ("ORBIT", CommandId::Orbit),
    ("PROJECTION", CommandId::SwitchProjection),
    ("MODE", CommandId::SetMode),
    ("LINE", CommandId::CreateLine),
    ("CIRCLE", CommandId::CreateCircle),
    ("ARC", CommandId::CreateArc),
    ("MOVE", CommandId::MoveEntities),
    ("TRIM", CommandId::TrimEntity),
    ("EXTEND", CommandId::ExtendEntity),
    ("ERASE", CommandId::EraseEntities),
    ("COPY", CommandId::CopyEntities),
    ("ROTATE", CommandId::RotateEntities),
    ("SCALE", CommandId::ScaleEntities),
    ("MIRROR", CommandId::MirrorEntities),
    ("COPYCLIP", CommandId::CopyToClipboard),
    ("CUTCLIP", CommandId::CutToClipboard),
    ("PASTECLIP", CommandId::PasteFromClipboard),
    ("OFFSET", CommandId::OffsetEntity),
    ("SETACTIVELAYER", CommandId::SetActiveLayer),
];

/// Full-word command synonyms, always accepted, that map onto a canonical
/// [`COMMAND_TABLE`] selector.
pub const COMMAND_ALIASES: &[(&str, &str)] = &[
    ("EXPORT", "PLOT"),
    ("PRINT", "PLOT"),
    ("SAVEAS", "SAVE"),
    ("QSAVE", "SAVE"),
    ("ZOOM EXTENTS", "FIT"),
    ("ENTER", "CONFIRM"),
];

/// The canonical, stable uppercase selector a command name resolves to, or
/// `None` when the name maps to no [`CommandId`].
///
/// Names are matched case-insensitively after trimming surrounding whitespace;
/// internal whitespace is significant (multi-word selectors keep one space).
pub fn canonical_command_name(text: &str) -> Option<&'static str> {
    let key = text.trim().to_ascii_uppercase();
    if let Some((name, _)) = COMMAND_TABLE.iter().find(|(name, _)| *name == key) {
        return Some(name);
    }
    COMMAND_ALIASES
        .iter()
        .find(|(alias, _)| *alias == key)
        .and_then(|(_, canonical)| canonical_command_name(canonical))
}

/// The `CommandId` a command name names, if any.
///
/// Covers the full [`COMMAND_TABLE`] vocabulary (plus [`COMMAND_ALIASES`]); an
/// unmapped name returns `None` so the caller can refuse it explicitly.
pub fn normalize_command(text: &str) -> Option<CommandId> {
    let name = canonical_command_name(text)?;
    COMMAND_TABLE
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, id)| *id)
}

/// The stable rejection reason for a name that maps to no command.
pub fn unsupported_command_reason(text: &str) -> String {
    format!("unsupported CAD command: {}", text.trim())
}

/// Map one DOM input to a typed interaction request.
///
/// `resize` is deliberately refused: the caller applies it through
/// [`CadSession::set_canvas_metrics`], so an input that is not a CAD interaction
/// is never silently accepted.
fn interaction_request(input: &ProbeInput, revision: u64) -> Result<InteractionRequest, String> {
    match input.kind {
        InputKind::PointerDown | InputKind::PointerMove | InputKind::PointerUp => {
            let phase = match input.kind {
                InputKind::PointerDown => PointerPhase::Down,
                InputKind::PointerMove => PointerPhase::Move,
                _ => PointerPhase::Up,
            };
            Ok(InteractionRequest::Pointer {
                id: POINTER_ID,
                kind: PointerKind::Mouse,
                phase,
                // The DOM protocol only produces primary-button gestures; the
                // host uses the button for 3D orbit and ignores it for 2D pan.
                button: Some(PointerButton::Primary),
                position: canvas_point(input, revision, true)?,
                modifiers: Modifiers::default(),
            })
        }
        // A cancel/blur is a terminal event and carries no coordinates: a zero
        // position is supplied only to satisfy the typed request; the host's
        // cancel path ignores it.
        InputKind::PointerCancel | InputKind::Blur => Ok(InteractionRequest::Pointer {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            phase: PointerPhase::Cancel,
            button: None,
            position: canvas_point(input, revision, false)?,
            modifiers: Modifiers::default(),
        }),
        InputKind::Wheel => {
            let dx = input.dx.unwrap_or(0.0);
            let dy = input.dy.unwrap_or(0.0);
            if !dx.is_finite() || !dy.is_finite() {
                return Err("wheel delta must be finite".to_string());
            }
            Ok(InteractionRequest::Scroll {
                delta_logical: scroll_delta(f64::from(dx), f64::from(dy)),
                position: canvas_point(input, revision, true)?,
                modifiers: Modifiers::default(),
            })
        }
        InputKind::Resize => Err("resize is applied through set_canvas_metrics".to_string()),
    }
}

/// A canvas-local position tagged with the current metrics revision.
///
/// `required` selects the pointer/wheel path (finite `x` and `y` mandatory) from
/// the cancel/blur path (coordinates absent, so the origin is used).
fn canvas_point(input: &ProbeInput, revision: u64, required: bool) -> Result<PointInput, String> {
    match (input.x, input.y) {
        (Some(x), Some(y)) => {
            if !x.is_finite() || !y.is_finite() {
                return Err("pointer position must be finite".to_string());
            }
            Ok(PointInput::CanvasLocal {
                logical_position: [f64::from(x), f64::from(y)],
                metrics_revision: revision,
            })
        }
        _ if required => Err("pointer position requires finite x and y".to_string()),
        _ => Ok(PointInput::CanvasLocal {
            logical_position: [0.0, 0.0],
            metrics_revision: revision,
        }),
    }
}

/// Invert the DOM wheel deltaY sign to the host convention.
///
/// The DOM reports a wheel-up as a negative `deltaY`; the host's scroll handler
/// zooms in on a positive `delta_logical[1]`, so the sign is flipped exactly
/// once (matching the web host's `normalized_scroll_delta`).
fn scroll_delta(dx: f64, dy: f64) -> [f64; 2] {
    [dx, -dy]
}

/// A camera block from real 2D camera parameters, or `None` when unrenderable.
fn camera_status(params: cad_app::Camera2dParams) -> Option<CadCamera> {
    let [x, y] = [params.center.x, params.center.y];
    if !x.is_finite()
        || !y.is_finite()
        || !params.world_per_px.is_finite()
        || params.world_per_px <= 0.0
    {
        return None;
    }
    Some(CadCamera {
        center: [x, y],
        scale: params.world_per_px,
    })
}

/// A locale-neutral failure string from a host status, or `None` for a normal
/// status.
///
/// Failure catalog keys (`command.failed`, `linux.failed`, `fonts.failed`, …)
/// and explicit external text are surfaced as technical strings built from the
/// status data; no localized prose is hardcoded here.
fn status_error(status: &StatusMessage) -> Option<String> {
    match status {
        StatusMessage::ExternalText(text) => Some(text.clone()),
        StatusMessage::Catalog { key, args } => {
            if !(key == "command.failed" || key.ends_with(".failed")) {
                return None;
            }
            Some(
                args.iter()
                    .find(|argument| argument.key == "error")
                    .map(|argument| argument.value.clone())
                    .unwrap_or_else(|| key.clone()),
            )
        }
    }
}

/// A technical `resource.missing` report built from the raw resource keys.
fn resource_keys_error<'a>(code: &str, keys: impl Iterator<Item = &'a String>) -> String {
    let mut text = code.to_string();
    for key in keys {
        text.push(' ');
        text.push_str(key);
    }
    text
}

/// A technical report from a `resource.missing` diagnostic (code + raw keys).
fn resource_diagnostic_text(diagnostic: &RawDiagnostic) -> String {
    let mut text = diagnostic.code.clone();
    for parameter in &diagnostic.parameters {
        if let RawDiagnosticParameter::Key(key) = parameter {
            text.push(' ');
            text.push_str(key);
        }
    }
    text
}

/// A user-facing `CadError` string, unwrapping the explicit refusal variants.
fn cad_error_text(error: &CadError) -> String {
    match error {
        CadError::Unsupported(reason) => reason.clone(),
        CadError::NotImplemented(feature) => (*feature).to_string(),
        other => other.to_string(),
    }
}

/// A technical `RequestError` string (the `CadError` inside `Execution` is
/// unwrapped the same way the host does).
fn request_error_text(error: &RequestError) -> String {
    match error {
        RequestError::Execution(cad) => cad_error_text(cad),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_platform::{
        ConfigLoad, ConfigStore, FileEvent, FileTarget, HostCapabilities, PlatformFiles,
        SavePurpose, SurfaceMetrics, SurfaceSource,
    };
    use std::rc::Rc;

    /// The exhaustive `CommandId` → canonical-selector contract for
    /// [`COMMAND_TABLE`].
    ///
    /// The generated `canonical_selector` `match` has **no wildcard arm**, so a
    /// new `cad_app::CommandId` variant fails to compile until it is listed here.
    /// `ALL_COMMAND_IDS`/`EXPECTED_COMMAND_COUNT` come from the same list, so the
    /// test below can require every variant to appear in the table exactly once:
    /// "44/44" is enforced by the type checker and the test, not by prose.
    macro_rules! command_selector_contract {
        ($($variant:ident => $selector:literal),+ $(,)?) => {
            const EXPECTED_COMMAND_COUNT: usize = [$(stringify!($variant)),+].len();
            const ALL_COMMAND_IDS: &[CommandId] = &[$(CommandId::$variant),+];

            fn canonical_selector(id: CommandId) -> &'static str {
                match id {
                    $(CommandId::$variant => $selector,)+
                }
            }
        };
    }

    command_selector_contract! {
        OpenDrawing => "OPEN",
        NewDrawing => "NEW",
        PlotDrawing => "PLOT",
        SaveDrawingAs => "SAVE",
        CancelLoading => "CANCEL",
        Pan => "PAN",
        Zoom => "ZOOM",
        FitDrawing => "FIT",
        ResetView => "RESETVIEW",
        ToggleLayer => "TOGGLELAYER",
        SetLayerVisibilities => "SETLAYERS",
        RestoreLayers => "RESTORELAYERS",
        SwitchSpace => "SPACE",
        Select => "SELECT",
        SelectAll => "SELECTALL",
        Measure => "MEASURE",
        ConfirmMeasurement => "CONFIRM",
        CancelMeasurement => "CANCELMEASURE",
        Undo => "UNDO",
        Redo => "REDO",
        Resources => "RESOURCES",
        Diagnostics => "DIAGNOSTICS",
        SwitchBackend => "SWITCHBACKEND",
        Switch2d3d => "SWITCH2D3D",
        StandardView => "STANDARDVIEW",
        Orbit => "ORBIT",
        SwitchProjection => "PROJECTION",
        SetMode => "MODE",
        CreateLine => "LINE",
        CreateCircle => "CIRCLE",
        CreateArc => "ARC",
        MoveEntities => "MOVE",
        TrimEntity => "TRIM",
        ExtendEntity => "EXTEND",
        EraseEntities => "ERASE",
        CopyEntities => "COPY",
        RotateEntities => "ROTATE",
        ScaleEntities => "SCALE",
        MirrorEntities => "MIRROR",
        CopyToClipboard => "COPYCLIP",
        CutToClipboard => "CUTCLIP",
        PasteFromClipboard => "PASTECLIP",
        OffsetEntity => "OFFSET",
        SetActiveLayer => "SETACTIVELAYER",
    }

    /// A committed synthetic, writer-produced AC1032 DWG with four LINE
    /// entities (`fixtures/manifest`). It is not a vendor sample.
    const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../fixtures/dwg/synthetic-four-lines.dwg");

    struct FakeFiles {
        events: RefCell<Vec<FileEvent>>,
    }

    impl PlatformFiles for FakeFiles {
        fn begin_open(&self) -> CadResult<()> {
            Ok(())
        }
        fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
            Ok(())
        }
        fn begin_save(&self, _purpose: SavePurpose, _name: &str) -> CadResult<()> {
            Err(CadError::Unsupported("save unsupported".into()))
        }
        fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
            Err(CadError::Unsupported("write unsupported".into()))
        }
        fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
            false
        }
        fn set_source(&self, _target: Option<&FileTarget>) {}
        fn poll(&self) -> Vec<FileEvent> {
            std::mem::take(&mut self.events.borrow_mut())
        }
    }

    struct FakeConfig;
    impl ConfigStore for FakeConfig {
        fn load_config(&self) -> ConfigLoad {
            ConfigLoad::Absent
        }
        fn load_preference(&self) -> ConfigLoad {
            ConfigLoad::Absent
        }
        fn persist_preference(&self, _text: &str) -> CadResult<()> {
            Ok(())
        }
    }

    struct FakeSurface;
    impl SurfaceSource for FakeSurface {
        fn surface(&self) -> Option<SurfaceMetrics> {
            Some(SurfaceMetrics {
                logical: [800.0, 600.0],
                scale: 1.0,
                safe_insets: [0.0; 4],
            })
        }
    }

    /// A `SurfaceSource` whose metrics a test mutates, standing in for a live
    /// host surface (each app host owns its own platform surface).
    #[derive(Clone, Default)]
    struct MutableSurface {
        metrics: Rc<Cell<Option<SurfaceMetrics>>>,
    }

    impl MutableSurface {
        fn new() -> Self {
            Self::default()
        }

        fn set_metrics(&self, metrics: SurfaceMetrics) {
            self.metrics.set(Some(metrics));
        }
    }

    impl SurfaceSource for MutableSurface {
        fn surface(&self) -> Option<SurfaceMetrics> {
            self.metrics.get()
        }
    }

    struct FakeResources;
    impl cad_host::HostResources for FakeResources {
        fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
            Ok(())
        }
        fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
            Ok(())
        }
    }

    /// A strategy that always refuses, so `CadSession::new` must propagate.
    struct FailingResources;
    impl cad_host::HostResources for FailingResources {
        fn install_fonts(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
            Err(CadError::Unsupported("no font resolver".into()))
        }
        fn install_images(&self, _core: &HostCore, _render: &RenderSessionHandle) -> CadResult<()> {
            Err(CadError::Unsupported("no image resolver".into()))
        }
    }

    fn platform(files: Rc<FakeFiles>) -> HostPlatform {
        HostPlatform {
            files,
            config_store: Rc::new(FakeConfig),
            surface: Rc::new(FakeSurface),
            resources: Rc::new(FakeResources),
            capabilities: HostCapabilities {
                open: true,
                new: true,
                save: false,
                plot: false,
                import_worker: false,
            },
        }
    }

    fn options() -> HostOptions {
        HostOptions {
            fonts: Vec::new(),
            fonts_dir: None,
            images_dir: None,
            headless: true,
            drawing: None,
            size: [800.0, 600.0],
        }
    }

    fn session(files: Rc<FakeFiles>) -> CadSession {
        CadSession::new(options(), platform(files), RenderSessionHandle::default())
            .expect("session builds")
    }

    /// A session whose surface source is mutable, so a test can change the
    /// reported CAD rect exactly like a frontend resize handler.
    fn session_with_surface(files: Rc<FakeFiles>, surface: MutableSurface) -> CadSession {
        let mut host_platform = platform(files);
        host_platform.surface = Rc::new(surface);
        CadSession::new(options(), host_platform, RenderSessionHandle::default())
            .expect("session builds")
    }

    /// The active viewport's logical canvas size (what `Camera::zoom_at` anchors
    /// against).
    fn canvas_logical_size(session: &CadSession) -> [f64; 2] {
        let c = session.core.controller.borrow();
        c.application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .expect("an active viewport")
            .logical_size
    }

    fn probe(kind: InputKind) -> ProbeInput {
        ProbeInput {
            sequence: 1,
            kind,
            x: None,
            y: None,
            dx: None,
            dy: None,
            width: None,
            height: None,
            dpr: None,
        }
    }

    fn move_input(x: f32, y: f32) -> ProbeInput {
        let mut input = probe(InputKind::PointerMove);
        input.x = Some(x);
        input.y = Some(y);
        input
    }

    #[test]
    fn command_normalization_covers_every_variant() {
        assert_eq!(normalize_command(" open "), Some(CommandId::OpenDrawing));
        assert_eq!(normalize_command("FIT"), Some(CommandId::FitDrawing));
        assert_eq!(normalize_command("fit\n"), Some(CommandId::FitDrawing));
        assert_eq!(normalize_command("line"), Some(CommandId::CreateLine));
        assert_eq!(normalize_command("export"), Some(CommandId::PlotDrawing));
        // Contract: `canonical_selector` is an exhaustive `match` over
        // `CommandId` with no wildcard arm, so a new variant cannot be added
        // without a selector here. Each variant must then appear in
        // `COMMAND_TABLE` exactly once (no missing row, no duplicate mapping).
        assert_eq!(
            EXPECTED_COMMAND_COUNT, 44,
            "the cad_app CommandId set changed; update COMMAND_TABLE and this contract"
        );
        assert_eq!(ALL_COMMAND_IDS.len(), EXPECTED_COMMAND_COUNT);
        for id in ALL_COMMAND_IDS {
            let name = canonical_selector(*id);
            assert_eq!(normalize_command(name), Some(*id), "{name}");
            assert_eq!(canonical_command_name(name), Some(name), "{name}");
            assert_eq!(
                COMMAND_TABLE
                    .iter()
                    .filter(|(_, mapped)| *mapped == *id)
                    .count(),
                1,
                "{name} ({id:?}) must appear exactly once in COMMAND_TABLE"
            );
        }
        assert_eq!(COMMAND_TABLE.len(), EXPECTED_COMMAND_COUNT);
        // Every canonical selector maps to its command and is stable.
        for (name, id) in COMMAND_TABLE {
            assert_eq!(normalize_command(name), Some(*id), "{name}");
            assert_eq!(canonical_command_name(name), Some(*name), "{name}");
        }
        for rejected in ["", "BOGUS", "open fit", "OPENX"] {
            assert_eq!(normalize_command(rejected), None, "{rejected:?}");
            assert_eq!(canonical_command_name(rejected), None, "{rejected:?}");
        }
    }

    #[test]
    fn unknown_command_is_refused_before_any_host_change() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let before = session.snapshot(None, None, 0);
        let error = session.command("BOGUS").unwrap_err();
        assert!(error.contains("BOGUS"), "{error}");
        assert_eq!(session.snapshot(None, None, 0), before);
    }

    #[test]
    fn payload_requiring_command_is_refused_explicitly_not_silently() {
        // A mapped command whose variant needs a payload still goes through the
        // funnel; the application layer refuses it explicitly (never a silent
        // empty success).
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let error = session.command("LINE").unwrap_err();
        assert!(!error.is_empty(), "an explicit refusal, not empty success");
    }

    #[test]
    fn dom_wheel_sign_zooms_in_on_wheel_up() {
        // DOM wheel-up is a negative deltaY; the host zooms in on a positive dy.
        assert_eq!(scroll_delta(3.0, -100.0), [3.0, 100.0]);
        assert_eq!(scroll_delta(0.0, 100.0), [0.0, -100.0]);
        // A positive host delta yields a > 1 zoom factor (the host's formula).
        assert!(1.0 + scroll_delta(0.0, -100.0)[1] * 0.0015 > 1.0);
    }

    #[test]
    fn unsupported_and_malformed_inputs_are_explicit() {
        let resize = probe(InputKind::Resize);
        assert!(interaction_request(&resize, 0).is_err());

        let mut missing = probe(InputKind::PointerDown);
        missing.width = Some(100.0);
        assert!(interaction_request(&missing, 0).is_err());

        let mut non_finite = probe(InputKind::Wheel);
        non_finite.x = Some(1.0);
        non_finite.y = Some(1.0);
        non_finite.dy = Some(f32::NAN);
        assert!(interaction_request(&non_finite, 0).is_err());
    }

    #[test]
    fn pointer_and_wheel_requests_carry_canvas_local_revision() {
        let mut down = probe(InputKind::PointerDown);
        down.x = Some(20.0);
        down.y = Some(30.0);
        match interaction_request(&down, 7).unwrap() {
            InteractionRequest::Pointer {
                phase: PointerPhase::Down,
                position:
                    PointInput::CanvasLocal {
                        logical_position,
                        metrics_revision,
                    },
                button: Some(PointerButton::Primary),
                ..
            } => {
                assert_eq!(logical_position, [20.0, 30.0]);
                assert_eq!(metrics_revision, 7);
            }
            other => panic!("unexpected request: {other:?}"),
        }

        // A cancel without coordinates is accepted with the origin placeholder.
        assert!(interaction_request(&probe(InputKind::PointerCancel), 7).is_ok());
        assert!(interaction_request(&probe(InputKind::Blur), 7).is_ok());

        let mut wheel = probe(InputKind::Wheel);
        wheel.x = Some(5.0);
        wheel.y = Some(6.0);
        wheel.dx = Some(2.0);
        wheel.dy = Some(-4.0);
        match interaction_request(&wheel, 7).unwrap() {
            InteractionRequest::Scroll { delta_logical, .. } => {
                assert_eq!(delta_logical, [2.0, 4.0]);
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn status_error_is_locale_neutral() {
        let failed = StatusMessage::Catalog {
            key: "command.failed".into(),
            args: vec![cad_engine::protocol::StatusArgument {
                key: "error".into(),
                value: "parse failed".into(),
            }],
        };
        assert_eq!(status_error(&failed).as_deref(), Some("parse failed"));
        let opened = StatusMessage::Catalog {
            key: "linux.opened".into(),
            args: Vec::new(),
        };
        assert_eq!(status_error(&opened), None);
        assert_eq!(
            status_error(&StatusMessage::ExternalText("sync failed".into())).as_deref(),
            Some("sync failed")
        );
    }

    #[test]
    fn resource_diagnostics_are_technical_not_ui_text() {
        let diagnostic = RawDiagnostic {
            object: None,
            code: RESOURCE_MISSING_CODE.into(),
            message: String::new(),
            severity: None,
            completeness: None,
            parameters: vec![RawDiagnosticParameter::Key("logo.png".into())],
        };
        assert_eq!(
            resource_diagnostic_text(&diagnostic),
            "resource.missing logo.png"
        );
        let keys = ["a.png".to_string(), "b.png".to_string()];
        assert_eq!(
            resource_keys_error(RESOURCE_MISSING_CODE, keys.iter()),
            "resource.missing a.png b.png"
        );
    }

    #[test]
    fn blank_status_reports_no_document_capabilities() {
        let status = assemble_status(
            false,
            "ignored".into(),
            None,
            false,
            true,
            None,
            Some(CadCamera {
                center: [1.0, 2.0],
                scale: 1.0,
            }),
            None,
            None,
            0,
            0,
        );
        assert_eq!(status.document_label, None);
        assert_eq!(status.entity_count, None);
        assert_eq!(status.camera, None, "a missing document has no camera");
        assert!(!status.can_fit);
        assert!(status.can_open);
        assert_eq!(status.import_count, 0);
        assert_eq!(status.frame_count, 0);
    }

    #[test]
    fn document_status_reports_counts_and_camera() {
        let status = assemble_status(
            true,
            "fixture.dwg".into(),
            Some(3),
            false,
            true,
            None,
            Some(CadCamera {
                center: [10.0, 20.0],
                scale: 2.0,
            }),
            Some("Vulkan".into()),
            Some("hardware".into()),
            1,
            10,
        );
        assert_eq!(status.document_label.as_deref(), Some("fixture.dwg"));
        assert_eq!(status.entity_count, Some(3));
        assert!(status.can_fit);
        assert_eq!(status.frame_count, 10);
    }

    #[test]
    fn import_progress_projects_only_a_live_import_and_never_invents_counts() {
        // A pre-tick running snapshot: no phase, no totals, no bytes — the UI
        // must render indeterminate rather than a fabricated 0/0.
        let running = CadImportProgress::from_snapshot(&cad_app::ImportProgressSnapshot::running())
            .expect("a running import projects a progress block");
        assert_eq!(running.phase, None);
        assert_eq!(running.entities_done, 0);
        assert_eq!(running.entities_total, None);
        assert_eq!(running.bytes, None);
        assert!(running.cancellable);

        // A terminal (opened) snapshot is never exposed: the affordance must not
        // persist after the document is published.
        let opened = cad_app::ImportProgressSnapshot::terminal(cad_app::ImportTerminal::Opened {
            entities: 3,
        });
        assert_eq!(CadImportProgress::from_snapshot(&opened), None);
    }

    #[test]
    fn session_builds_and_reports_the_demo_document() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let status = session.snapshot(None, None, 0);
        assert!(status.document_label.is_some());
        assert!(status.entity_count.is_some());
        assert!(status.can_fit);
        assert!(!status.loading);
        assert_eq!(status.import_count, 0, "the demo document is not an import");
        assert!(status.camera.is_some(), "the demo view is a 2D plan view");
    }

    #[test]
    fn resource_install_failure_is_propagated_not_ignored() {
        let platform = HostPlatform {
            files: Rc::new(FakeFiles {
                events: RefCell::new(Vec::new()),
            }),
            config_store: Rc::new(FakeConfig),
            surface: Rc::new(FakeSurface),
            resources: Rc::new(FailingResources),
            capabilities: HostCapabilities {
                open: true,
                new: true,
                save: false,
                plot: false,
                import_worker: false,
            },
        };
        let result = CadSession::new(options(), platform, RenderSessionHandle::default());
        assert!(matches!(result, Err(CadError::Unsupported(_))));
    }

    #[test]
    fn open_admission_does_not_claim_a_completed_import() {
        let files = Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        });
        let session = session(files.clone());
        session.command("OPEN").expect("OPEN is admitted");
        // A pending picker keeps the old document and increments nothing.
        let status = session.snapshot(None, None, 0);
        assert_eq!(status.import_count, 0);
        assert!(status.document_label.is_some());
        assert!(status.entity_count.is_some());
    }

    #[test]
    fn failed_open_keeps_the_old_document_and_zero_imports() {
        let files = Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        });
        let session = session(files.clone());
        session.command("OPEN").expect("OPEN is admitted");
        files.events.borrow_mut().push(FileEvent::Failed {
            purpose: cad_platform::FilePurpose::Open,
            reason: "read failed".into(),
        });
        session.tick().expect("tick");
        let status = session.snapshot(None, None, 0);
        assert_eq!(status.import_count, 0, "a failed open is not an import");
        assert!(status.document_label.is_some(), "the old document remains");
        assert!(status.entity_count.is_some());
        assert_eq!(status.error.as_deref(), Some("read failed"));
    }

    #[test]
    fn successful_synthetic_import_counts_once_and_reports_real_entities() {
        let files = Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        });
        let session = session(files.clone());
        files.events.borrow_mut().push(FileEvent::Opened {
            target: FileTarget::from_path("synthetic-four-lines.dwg"),
            bytes: Arc::from(SYNTHETIC_DWG.to_vec().into_boxed_slice()),
        });
        session.command("OPEN").expect("OPEN is admitted");
        session.tick().expect("tick");
        let status = session.snapshot(None, None, 0);
        assert_eq!(status.import_count, 1, "one verified successful import");
        assert_eq!(
            status.entity_count,
            Some(4),
            "the synthetic DWG has 4 lines"
        );
        assert_eq!(
            status.document_label.as_deref(),
            Some("synthetic-four-lines.dwg")
        );
        assert!(status.camera.is_some());
        assert_eq!(status.error, None);
        // Navigation and repeated snapshots never re-count.
        let mut move_input = move_input(5.0, 6.0);
        move_input.sequence = 2;
        session.handle_input(&move_input).unwrap();
        assert_eq!(session.snapshot(None, None, 0).import_count, 1);
    }

    #[test]
    fn repeated_navigation_does_not_leak_queue_quota() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        // Well past the queue's normal (240) and absolute (256) capacity: if
        // completions were not consumed, admission would fail with QueueFull.
        for index in 0..600u64 {
            let mut input = move_input(index as f32, index as f32);
            input.sequence = index + 1;
            session
                .handle_input(&input)
                .unwrap_or_else(|error| panic!("navigation {index} refused: {error}"));
        }
        let status = session.snapshot(None, None, 0);
        assert_eq!(status.import_count, 0, "navigation is not an import");
        assert_eq!(status.error, None, "successful navigation leaves no error");
    }

    #[test]
    fn stale_context_completion_surfaces_as_an_error_not_empty_success() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let context = session.core.request_context();
        let request = EngineRequest::Interaction(InteractionRequest::Pointer {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            phase: PointerPhase::Move,
            button: Some(PointerButton::Primary),
            position: PointInput::CanvasLocal {
                logical_position: [1.0, 2.0],
                metrics_revision: context.metrics_revision.unwrap_or(0),
            },
            modifiers: Modifiers::default(),
        });
        let receipt = session.core.submit(context, request).unwrap();
        session.track_receipt(receipt.id);
        // The canvas metrics revision advances before the queued request runs,
        // so the host grades it stale.
        session.set_canvas_metrics([640.0, 480.0], 1.0).unwrap();
        session.pump().unwrap();
        let outcome = session.drain_completions();
        assert!(
            outcome.is_err(),
            "a stale completion must not be an empty success"
        );
        assert!(session.snapshot(None, None, 0).error.is_some());
    }

    #[test]
    fn canvas_metrics_accept_valid_and_refuse_degenerate_input() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        session.set_canvas_metrics([800.0, 600.0], 1.0).unwrap();
        assert!(session.set_canvas_metrics([0.0, 600.0], 1.0).is_err());
        assert!(session.set_canvas_metrics([800.0, 600.0], 0.0).is_err());
    }

    #[test]
    fn tick_tracks_the_live_surface_rect_so_later_inputs_anchor_correctly() {
        let surface = MutableSurface::new();
        surface.set_metrics(SurfaceMetrics {
            logical: [640.0, 480.0],
            scale: 1.0,
            safe_insets: [0.0; 4],
        });
        let session = session_with_surface(
            Rc::new(FakeFiles {
                events: RefCell::new(Vec::new()),
            }),
            surface.clone(),
        );
        assert_eq!(canvas_logical_size(&session), [640.0, 480.0]);
        // The frontend reports a new CAD rect. Until the owner tick re-reads the
        // surface the viewport still holds the creation-time size: exactly the
        // stale state that made an off-center wheel zoom anchor off the cursor.
        surface.set_metrics(SurfaceMetrics {
            logical: [1024.0, 768.0],
            scale: 1.0,
            safe_insets: [0.0; 4],
        });
        assert_eq!(
            canvas_logical_size(&session),
            [640.0, 480.0],
            "stale viewport until the owner tick re-reads the surface"
        );
        session.tick().expect("tick");
        assert_eq!(
            canvas_logical_size(&session),
            [1024.0, 768.0],
            "the tick must re-apply the accepted surface rect to the viewport"
        );
    }

    #[test]
    fn note_frame_is_the_actual_present_count() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        assert_eq!(session.frames_presented(), 0);
        session.note_frame(12);
        assert_eq!(session.frames_presented(), 12);
        assert_eq!(session.snapshot(None, None, 0).frame_count, 12);
        assert_eq!(session.snapshot(None, None, 99).frame_count, 99);
    }

    #[test]
    fn stale_resource_guard_clears_caches_once_per_generation() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        // No failure status: nothing to guard.
        session.guard_stale_resources();
        assert_eq!(session.resource_guard.get(), None);
        // A resource-install failure on the current generation is guarded once.
        session
            .core
            .set_catalog_status(RESOURCE_FAILED_STATUS, &[("error", "no resolver")])
            .unwrap();
        session.guard_stale_resources();
        let generation = session.core.request_context().document_generation;
        assert_eq!(session.resource_guard.get(), Some(generation));
        session.guard_stale_resources();
        assert_eq!(session.resource_guard.get(), Some(generation));
    }

    #[test]
    fn startup_diagnostics_accessor_is_exposed() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let _ = session.startup_diagnostics();
    }
}
