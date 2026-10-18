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
use std::sync::Arc;

use cad_app::input::CanvasMetrics;
use cad_app::{CommandPayload, DrawToolKind, MeasurementToolKind, ToolSnapshot};
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
    admissible_names, canonical_command_name, normalize_command, unsupported_command_reason,
    COMMAND_ALIASES, COMMAND_TABLE,
};
pub use self::status::{
    unsupported_status, CadCamera, CadImportProgress, CadSelection, CadSelectionRow, CadStatus,
    CadTool,
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
    /// Verified successful-import transition counter.
    import_count: Cell<u64>,
    /// `(document generation, opened identity, incoming database pointer)` last
    /// counted, so a repeated snapshot never double-counts.
    last_import: RefCell<Option<(u64, DocumentIdentity, usize)>>,
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
    /// | `MEASURE` | `BeginMeasurement(Distance)` |
    /// | `CONFIRM` | `Confirm` |
    /// | `CANCEL` | `Cancel` |
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
        let Some(id) = normalize_command(text) else {
            return Err(unsupported_command_reason(text));
        };
        let interaction = match id {
            CommandId::CreateLine => Some(InteractionRequest::BeginDraw(DrawToolKind::Line)),
            CommandId::CreateCircle => Some(InteractionRequest::BeginDraw(DrawToolKind::Circle)),
            CommandId::CreateArc => Some(InteractionRequest::BeginDraw(DrawToolKind::Arc)),
            CommandId::MoveEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Move)),
            CommandId::CopyEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Copy)),
            CommandId::RotateEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Rotate)),
            CommandId::ScaleEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Scale)),
            CommandId::MirrorEntities => Some(InteractionRequest::BeginDraw(DrawToolKind::Mirror)),
            CommandId::OffsetEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Offset)),
            CommandId::TrimEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Trim)),
            CommandId::ExtendEntity => Some(InteractionRequest::BeginDraw(DrawToolKind::Extend)),
            CommandId::Measure => Some(InteractionRequest::BeginMeasurement(
                MeasurementToolKind::Distance,
            )),
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
            selection,
            self.import_count.get(),
            frame_count,
            cad_tool(&self.core.tool_snapshot()),
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

#[cfg(test)]
mod tests;
