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

use cad_app::input::{CanvasMetrics, DRAG_THRESHOLD_LOGICAL_PX};
use cad_app::interaction::{
    InteractionRequest, Modifiers, PointInput, PointerButton, PointerId, PointerKind, PointerPhase,
    SelectionMode,
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

use crate::protocol::{InputKind, ProbeInput, BUTTON_MIDDLE, BUTTON_PRIMARY, BUTTON_SECONDARY};

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

/// One row of the [`CadSelection`] block: a value plus its **stable machine
/// key** (`id`, `handle`, `type`, …).
///
/// The key is never translated by the host; the UI maps it to a catalog label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CadSelectionRow {
    pub key: String,
    pub value: String,
}

/// The `selection` block of [`CadStatus`] (frozen frontend shape).
///
/// The key is **always** present — never omitted and never `null` — and it
/// describes a real selection instead of a fabricated one:
///
/// * `count >= 0` and `empty == (count == 0)`;
/// * `rows` is non-empty **only** for a single selected object. A single ref
///   that no longer resolves still reports its honest `status: "unresolved"`
///   row rather than invented geometry;
/// * `mixedKeys` is non-empty **only** for a multi-object selection.
///
/// A session with no document (or no selection) carries the honest empty block
/// ([`CadSelection::empty`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadSelection {
    /// Number of selected objects.
    pub count: u64,
    /// Whether nothing is selected (`count == 0`).
    pub empty: bool,
    /// Property rows of a single selection; empty for any other count.
    pub rows: Vec<CadSelectionRow>,
    /// Machine keys whose values differ across a multi-selection; empty for any
    /// other count.
    pub mixed_keys: Vec<String>,
}

impl CadSelection {
    /// The honest empty selection: nothing is selected and nothing is invented.
    pub fn empty() -> Self {
        Self {
            count: 0,
            empty: true,
            rows: Vec::new(),
            mixed_keys: Vec::new(),
        }
    }

    /// A selection block with no property source: there is no authoritative
    /// document to read rows from, so only the **real** ref count is reported.
    ///
    /// No property row is fabricated, and a non-empty session selection is never
    /// hidden behind a `0` (a document replacement resets the session, so this
    /// normally *is* the empty block).
    fn refs_only(count: usize) -> Self {
        Self {
            count: count as u64,
            empty: count == 0,
            rows: Vec::new(),
            mixed_keys: Vec::new(),
        }
    }

    /// Project the application's read-only selection properties onto the frozen
    /// wire shape.
    ///
    /// `empty` is derived from `count` so the frozen invariant
    /// `empty == (count == 0)` holds by construction; the application derives
    /// both from the same selection set, so the two cannot disagree.
    fn from_properties(properties: &cad_app::SelectionProperties) -> Self {
        let count = properties.count as u64;
        let rows = if properties.count == 1 {
            properties
                .rows
                .iter()
                .map(|row| CadSelectionRow {
                    key: row.key.to_string(),
                    value: row.value.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let mixed_keys = if properties.count > 1 {
            properties
                .mixed_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect()
        } else {
            Vec::new()
        };
        Self {
            count,
            empty: count == 0,
            rows,
            mixed_keys,
        }
    }
}

/// The `cad_status` snapshot consumed by the frontend.
///
/// The field set is the frozen frontend contract: every key is always present,
/// an unknown value is `null`, and a count is never fabricated. `error` is a
/// technical, locale-neutral string from the real host state (a command/
/// completion failure, a resource diagnostic or a render diagnostic); it is
/// never a localized UI label. `import_progress` carries the real, live open
/// progress while one is running (see [`CadImportProgress`]) and `selection`
/// the real current selection (see [`CadSelection`]).
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
    /// The current selection; always present, never `null`.
    pub selection: CadSelection,
    /// Count of verified successful authoritative document transitions caused by
    /// an import. It is **not** a parsed-entity count and must not be read as
    /// proof that no reparse happened.
    pub import_count: u64,
    pub frame_count: u64,
}

/// A `cad_status` snapshot that advertises no CAD capability (probe mode or a
/// failed session): an explicit error, never a fabricated document/count.
///
/// The selection block is present and honest: no session means nothing is
/// selected (`count: 0`), never an omitted key.
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
        selection: CadSelection::empty(),
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
    /// Pointer phases map to a typed [`InteractionRequest::Pointer`] with the
    /// **pressed button carried through** (the DOM `MouseEvent.button` numbers,
    /// see [`crate::protocol::ProbeInput::button`]), and a wheel maps to
    /// [`InteractionRequest::Scroll`] with the DOM deltaY sign inverted to the
    /// host's wheel-up-zooms-in convention.
    ///
    /// A press/release without an intervening drag is a **click**: it emits the
    /// typed [`InteractionRequest::SelectAt`] (mode
    /// [`SelectionMode::Replace`]) at the release position, tagged with the
    /// **current** canvas metrics revision, plus the ordinary release
    /// [`InteractionRequest::Pointer`] so the host's gesture life is closed. Any
    /// drag beyond [`DRAG_THRESHOLD_LOGICAL_PX`] (or a release of a different
    /// button) keeps the plain pointer mapping, which the host resolves as
    /// navigation (a primary/middle drag pans in 2D). Box selection is **not**
    /// implemented: a primary drag still pans.
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
        let (requests, next_press) = plan_input(
            self.press.borrow().as_ref().copied(),
            input,
            self.metrics_revision(),
        )?;
        let context = self.core.request_context();
        for request in requests {
            match self
                .core
                .submit(context, EngineRequest::Interaction(request))
            {
                Ok(receipt) => self.track_receipt(receipt.id),
                Err(error) => {
                    // The gesture state is deliberately **not** advanced: an
                    // input the host never admitted must not leave a phantom
                    // press behind that a later release turns into a selection.
                    let text = request_error_text(&error);
                    *self.error.borrow_mut() = Some(text.clone());
                    return Err(text);
                }
            }
        }
        *self.press.borrow_mut() = next_press;
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
/// document there is no label, no entity count, no camera and no `FIT`. The
/// `selection` block is always carried, even when nothing is selected.
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
    selection: CadSelection,
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
        selection,
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

/// The retained press state of the single DOM pointer stream.
///
/// The wire protocol carries no pointer id, so the session keeps exactly one
/// press: the button it went down with and where, so a release can be
/// classified as a click (select) or as the end of a gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PointerPress {
    /// The button the press went down with.
    button: PointerButton,
    /// Canvas-local logical position of the press.
    down: [f64; 2],
    /// Whether an intervening move already crossed the drag threshold. A move
    /// that travels beyond the threshold and returns still counts as a drag, so
    /// a round trip never fabricates a click.
    dragged: bool,
}

impl PointerPress {
    /// Whether releasing `released` at `up` is a click (select) rather than the
    /// end of a navigation gesture.
    ///
    /// Click-select is the **primary** button only: the middle button is the pan
    /// gesture (and the secondary button is reserved for a future context menu),
    /// so releasing any other button takes the plain pointer path and leaves the
    /// selection alone. The release must name the same button the press went
    /// down with, and the total displacement must stay under the drag threshold.
    fn is_click(&self, released: PointerButton, up: [f64; 2]) -> bool {
        self.button == PointerButton::Primary
            && released == PointerButton::Primary
            && !self.dragged
            && logical_distance(self.down, up) < DRAG_THRESHOLD_LOGICAL_PX
    }
}

/// Map one wire button report to the typed pointer button.
///
/// An omitted report (`None`) means the primary button, so an existing frontend
/// that does not send `button` keeps working. Any other number — including the
/// DOM's `-1` "no button changed" — is refused with a stable reason; it is never
/// silently treated as primary.
fn pointer_button(report: Option<i32>) -> Result<PointerButton, String> {
    match report {
        None | Some(BUTTON_PRIMARY) => Ok(PointerButton::Primary),
        Some(BUTTON_MIDDLE) => Ok(PointerButton::Middle),
        Some(BUTTON_SECONDARY) => Ok(PointerButton::Secondary),
        Some(other) => Err(format!(
            "unsupported pointer button: {other} (expected 0, 1 or 2, or omit it for primary)"
        )),
    }
}

/// The finite canvas-local logical position of a pointer input.
fn pointer_logical(input: &ProbeInput) -> Result<[f64; 2], String> {
    match (input.x, input.y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok([f64::from(x), f64::from(y)]),
        _ => Err("pointer position requires finite x and y".to_string()),
    }
}

/// Euclidean distance between two canvas-local logical positions.
fn logical_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

/// The typed pointer request for one phase at the input's position.
fn pointer_request(
    input: &ProbeInput,
    revision: u64,
    phase: PointerPhase,
    button: Option<PointerButton>,
) -> Result<InteractionRequest, String> {
    Ok(InteractionRequest::Pointer {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        phase,
        button,
        position: canvas_point(input, revision, true)?,
        modifiers: Modifiers::default(),
    })
}

/// Classify one DOM input against the retained press state.
///
/// Pure: it returns the request(s) to admit, in order, plus the press state that
/// must be committed once every request was admitted. It owns the click/gesture
/// decision so the session only applies the outcome.
///
/// * `pointer_down` starts a press and reports it as an ordinary pointer phase;
/// * `pointer_move` prefers the button the press went down with (the DOM reports
///   no button change on a move), and marks a press as dragged once the pointer
///   leaves the drag-threshold disc around the press position;
/// * `pointer_up` of the same primary button without an intervening drag is a
///   click: it emits the ordinary release **and** a
///   [`InteractionRequest::SelectAt`] replace-selection at the release position;
/// * `pointer_cancel`/`blur` end any press without selecting;
/// * `pointer_leave` clears the cursor overlay (the same
///   [`InteractionRequest::ClearCursor`] the existing clear path uses) without
///   ending or disturbing any press — it is transient and non-terminal, so a
///   later release still resolves normally;
/// * a `resize` is refused (it is applied through
///   [`CadSession::set_canvas_metrics`]).
fn plan_input(
    press: Option<PointerPress>,
    input: &ProbeInput,
    revision: u64,
) -> Result<(Vec<InteractionRequest>, Option<PointerPress>), String> {
    match input.kind {
        InputKind::PointerDown => {
            let button = pointer_button(input.button)?;
            let down = pointer_logical(input)?;
            let request = pointer_request(input, revision, PointerPhase::Down, Some(button))?;
            Ok((
                vec![request],
                Some(PointerPress {
                    button,
                    down,
                    dragged: false,
                }),
            ))
        }
        InputKind::PointerMove => {
            // The reported value is validated even when the retained press owns
            // the button, so an undefined number is never silently ignored.
            let reported = pointer_button(input.button)?;
            let logical = pointer_logical(input)?;
            let hover = press.is_none();
            let (button, next) = match press {
                Some(press) => (
                    press.button,
                    Some(PointerPress {
                        dragged: press.dragged
                            || logical_distance(press.down, logical) >= DRAG_THRESHOLD_LOGICAL_PX,
                        ..press
                    }),
                ),
                // Without a press a move is a hover: not a gesture, and no press
                // state is invented.
                None => (reported, None),
            };
            let mut requests = vec![pointer_request(
                input,
                revision,
                PointerPhase::Move,
                Some(button),
            )?];
            if hover {
                // An idle move also asks the host what is under the cursor: the
                // typed cursor-position request tagged with the **current**
                // metrics revision, which the host refuses when stale and which
                // it resolves as the transient hover highlight. While a press is
                // active the gesture (pan/orbit/drag) owns the move, so no hover
                // is claimed (ADR-0012 §10 S2).
                requests.push(InteractionRequest::MoveCursor(canvas_point(
                    input, revision, true,
                )?));
            }
            Ok((requests, next))
        }
        InputKind::PointerUp => {
            let reported = pointer_button(input.button)?;
            let up = pointer_logical(input)?;
            let click = press.is_some_and(|press| press.is_click(reported, up));
            // The release always travels the ordinary pointer path, so the
            // host's gesture life is closed even if the selection is refused.
            let button = press.map_or(reported, |press| press.button);
            let mut requests = vec![pointer_request(
                input,
                revision,
                PointerPhase::Up,
                Some(button),
            )?];
            if click {
                // A click replaces the selection at the release position, tagged
                // with the **current** metrics revision: a revision the host no
                // longer has is its explicit refusal, never a fabricated pass.
                requests.push(InteractionRequest::SelectAt {
                    point: canvas_point(input, revision, true)?,
                    mode: SelectionMode::Replace,
                });
            }
            Ok((requests, None))
        }
        // A cancel/blur is terminal: it ends any press and never selects.
        InputKind::PointerCancel | InputKind::Blur => {
            Ok((vec![interaction_request(input, revision)?], None))
        }
        // A pointer leave is transient and non-terminal: it clears the cursor
        // overlay and **leaves the press untouched** (`press` is returned
        // unchanged), so it never ends or wedges an in-flight gesture. It
        // carries no coordinates, so it demands no metrics revision it cannot
        // satisfy. The host classifies the resulting `ClearCursor` as
        // overlay-only, so leaving the surface schedules no full-state rebuild.
        InputKind::PointerLeave => Ok((vec![InteractionRequest::ClearCursor], press)),
        InputKind::Wheel | InputKind::Resize => {
            Ok((vec![interaction_request(input, revision)?], press))
        }
    }
}

/// Map one DOM input to a typed interaction request (the stateless mapping).
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
            // The host owns the navigation decisions: a primary or middle drag
            // pans the 2D view (the primary button orbits a 3D one).
            pointer_request(input, revision, phase, Some(pointer_button(input.button)?))
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
        // A pointer leave is a non-terminal, coordinate-free event: it clears
        // the cursor overlay. Any coordinates it happens to carry are ignored
        // (exactly like cancel/blur), so it makes no metrics-revision demand.
        InputKind::PointerLeave => Ok(InteractionRequest::ClearCursor),
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
            button: None,
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
            CadSelection::empty(),
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
        // The selection block is always carried, never omitted.
        assert_eq!(status.selection, CadSelection::empty());
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
            CadSelection::empty(),
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

    // --- click-select (ADR-0012 §10 S1) --------------------------------------

    /// A pointer input of `kind` at a canvas-local logical position.
    fn pointer_at(
        kind: InputKind,
        sequence: u64,
        position: [f64; 2],
        button: Option<i32>,
    ) -> ProbeInput {
        let mut input = probe(kind);
        input.sequence = sequence;
        input.x = Some(position[0] as f32);
        input.y = Some(position[1] as f32);
        input.button = button;
        input
    }

    /// The press state of a fresh primary press at `down`.
    fn primary_press(down: [f64; 2]) -> PointerPress {
        PointerPress {
            button: PointerButton::Primary,
            down,
            dragged: false,
        }
    }

    /// The canvas-local logical pixel a world point projects to in the session's
    /// active view (the same projection the host picks with).
    fn screen_of(session: &CadSession, world: cad_domain::Point3) -> [f64; 2] {
        let c = session.core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        viewport
            .camera
            .world_to_screen(world, viewport.logical_size)
            .expect("the 2D plan view projects the world point")
    }

    /// The session's active viewport camera.
    fn viewport_camera(session: &CadSession) -> cad_app::Camera {
        let c = session.core.controller.borrow();
        c.application.workspace.viewports[&c.viewport_id].camera
    }

    /// A world point on the demo document's circle rim (a circle is pickable on
    /// its circumference only, per ADR-0012 §8).
    fn demo_circle_rim(session: &CadSession) -> cad_domain::Point3 {
        let c = session.core.controller.borrow();
        let drawing = c.drawing().expect("the demo document");
        let (center, radius) = drawing
            .model_space()
            .iter()
            .find_map(|entity| match &entity.geometry {
                cad_domain::SemanticGeometry::Circle { center, radius, .. } => {
                    Some((*center, *radius))
                }
                _ => None,
            })
            .expect("the demo document has a circle");
        cad_domain::Point3 {
            x: center.x + radius,
            y: center.y,
            z: center.z,
        }
    }

    #[test]
    fn plan_input_classifies_a_click_and_tags_it_with_the_current_revision() {
        let down = pointer_at(InputKind::PointerDown, 1, [10.0, 20.0], None);
        let (requests, press) = plan_input(None, &down, 7).unwrap();
        assert_eq!(requests.len(), 1);
        match &requests[0] {
            InteractionRequest::Pointer {
                phase: PointerPhase::Down,
                button: Some(PointerButton::Primary),
                position:
                    PointInput::CanvasLocal {
                        logical_position,
                        metrics_revision,
                    },
                ..
            } => {
                assert_eq!(*logical_position, [10.0, 20.0]);
                assert_eq!(*metrics_revision, 7);
            }
            other => panic!("unexpected request: {other:?}"),
        }
        let press = press.expect("a press is retained until its release");
        assert_eq!(press.button, PointerButton::Primary);
        assert!(!press.dragged);

        // A release with no movement is a click: the ordinary release (which
        // closes the host's gesture life) plus a replace-selection at the
        // release position tagged with the current metrics revision.
        let up = pointer_at(InputKind::PointerUp, 2, [10.75, 20.0], None);
        let (requests, next) = plan_input(Some(press), &up, 7).unwrap();
        assert!(next.is_none(), "a release ends the press");
        assert_eq!(requests.len(), 2, "release then select: {requests:?}");
        assert!(matches!(
            requests[0],
            InteractionRequest::Pointer {
                phase: PointerPhase::Up,
                ..
            }
        ));
        match &requests[1] {
            InteractionRequest::SelectAt {
                point:
                    PointInput::CanvasLocal {
                        logical_position,
                        metrics_revision,
                    },
                mode: SelectionMode::Replace,
            } => {
                assert_eq!(*logical_position, [10.75, 20.0]);
                assert_eq!(
                    *metrics_revision, 7,
                    "the click must carry the current revision, never a fabricated one"
                );
            }
            other => panic!("expected a SelectAt replace-selection: {other:?}"),
        }
    }

    #[test]
    fn plan_input_keeps_a_drag_on_the_navigation_path() {
        let press = primary_press([10.0, 20.0]);
        // A move past the drag threshold marks the gesture as a drag.
        let moved = pointer_at(
            InputKind::PointerMove,
            2,
            [10.0 + DRAG_THRESHOLD_LOGICAL_PX, 20.0],
            None,
        );
        let (requests, press) = plan_input(Some(press), &moved, 7).unwrap();
        assert!(matches!(
            requests.as_slice(),
            [InteractionRequest::Pointer {
                phase: PointerPhase::Move,
                ..
            }]
        ));
        let press = press.expect("the press continues");
        assert!(press.dragged);

        // Releasing back at the press position is still a drag, not a click: a
        // round trip never fabricates a selection.
        let back = pointer_at(InputKind::PointerUp, 3, [10.0, 20.0], None);
        let (requests, next) = plan_input(Some(press), &back, 7).unwrap();
        assert!(next.is_none());
        assert_eq!(
            requests.len(),
            1,
            "a dragged release must not select: {requests:?}"
        );
        assert!(matches!(
            requests[0],
            InteractionRequest::Pointer {
                phase: PointerPhase::Up,
                ..
            }
        ));

        // A release whose total displacement exceeds the threshold is not a
        // click either, even when no move event reported the drag.
        let far = pointer_at(
            InputKind::PointerUp,
            4,
            [10.0 + DRAG_THRESHOLD_LOGICAL_PX, 20.0],
            None,
        );
        let (requests, _) = plan_input(Some(primary_press([10.0, 20.0])), &far, 7).unwrap();
        assert_eq!(
            requests.len(),
            1,
            "an end-to-end jump is a drag: {requests:?}"
        );
    }

    #[test]
    fn plan_input_carries_the_pressed_button_through_a_gesture() {
        // The middle button is the pan gesture: the press button owns the move
        // even when the move reports no button change (the DOM's convention).
        let down = pointer_at(InputKind::PointerDown, 1, [0.0, 0.0], Some(BUTTON_MIDDLE));
        let (requests, press) = plan_input(None, &down, 3).unwrap();
        assert!(matches!(
            requests[0],
            InteractionRequest::Pointer {
                button: Some(PointerButton::Middle),
                ..
            }
        ));
        let press = press.expect("the middle press is retained");
        assert_eq!(press.button, PointerButton::Middle);

        let moved = pointer_at(InputKind::PointerMove, 2, [30.0, 10.0], None);
        let (requests, press) = plan_input(Some(press), &moved, 3).unwrap();
        assert!(
            matches!(
                requests[0],
                InteractionRequest::Pointer {
                    button: Some(PointerButton::Middle),
                    ..
                }
            ),
            "the press button owns the gesture: {requests:?}"
        );
        assert!(press.expect("still pressed").dragged);

        // A middle release is never a click: no selection is invented from a
        // pan gesture.
        let up = pointer_at(InputKind::PointerUp, 3, [0.0, 0.0], Some(BUTTON_MIDDLE));
        let (requests, next) = plan_input(press, &up, 3).unwrap();
        assert_eq!(requests.len(), 1, "a middle click must not select");
        assert!(next.is_none());

        // The secondary button likewise never selects.
        let down = pointer_at(
            InputKind::PointerDown,
            4,
            [0.0, 0.0],
            Some(BUTTON_SECONDARY),
        );
        let (requests, press) = plan_input(None, &down, 3).unwrap();
        assert!(matches!(
            requests[0],
            InteractionRequest::Pointer {
                button: Some(PointerButton::Secondary),
                ..
            }
        ));
        let up = pointer_at(InputKind::PointerUp, 5, [0.0, 0.0], Some(BUTTON_SECONDARY));
        let (requests, _) = plan_input(press, &up, 3).unwrap();
        assert_eq!(requests.len(), 1, "a secondary release must not select");

        // A release that names a different button than the press does not select.
        let up = pointer_at(InputKind::PointerUp, 6, [0.0, 0.0], Some(BUTTON_SECONDARY));
        let (requests, _) = plan_input(Some(primary_press([0.0, 0.0])), &up, 3).unwrap();
        assert_eq!(
            requests.len(),
            1,
            "a mismatched release button must not select: {requests:?}"
        );
    }

    #[test]
    fn plan_input_refuses_an_undefined_button_instead_of_treating_it_as_primary() {
        assert_eq!(pointer_button(None), Ok(PointerButton::Primary));
        assert_eq!(
            pointer_button(Some(BUTTON_PRIMARY)),
            Ok(PointerButton::Primary)
        );
        assert_eq!(
            pointer_button(Some(BUTTON_MIDDLE)),
            Ok(PointerButton::Middle)
        );
        assert_eq!(
            pointer_button(Some(BUTTON_SECONDARY)),
            Ok(PointerButton::Secondary)
        );
        for undefined in [-1, 3, 4, 99] {
            let error = pointer_button(Some(undefined)).unwrap_err();
            assert!(
                error.contains(&undefined.to_string()),
                "the refusal names the value: {error}"
            );
            assert!(
                error.contains("unsupported pointer button"),
                "the refusal is stable: {error}"
            );
        }

        // Every pointer phase refuses it, and a refused press leaves no state.
        for kind in [
            InputKind::PointerDown,
            InputKind::PointerMove,
            InputKind::PointerUp,
        ] {
            let input = pointer_at(kind, 1, [1.0, 2.0], Some(-1));
            let error = plan_input(None, &input, 0).unwrap_err();
            assert!(error.contains("unsupported pointer button"), "{error}");
        }
        // The stateless mapping refuses it too (no silent primary fallback).
        assert!(interaction_request(
            &pointer_at(InputKind::PointerDown, 1, [1.0, 2.0], Some(9)),
            0
        )
        .is_err());
    }

    #[test]
    fn plan_input_ends_the_press_on_cancel_and_keeps_it_across_a_wheel() {
        let cancel = pointer_at(InputKind::PointerCancel, 2, [1.0, 2.0], None);
        let (requests, next) = plan_input(Some(primary_press([0.0, 0.0])), &cancel, 0).unwrap();
        assert!(next.is_none(), "a cancel ends the press");
        assert!(matches!(
            requests[0],
            InteractionRequest::Pointer {
                phase: PointerPhase::Cancel,
                button: None,
                ..
            }
        ));

        let blur = probe(InputKind::Blur);
        let (_, next) = plan_input(Some(primary_press([0.0, 0.0])), &blur, 0).unwrap();
        assert!(next.is_none(), "a blur ends the press");

        let mut wheel = probe(InputKind::Wheel);
        wheel.x = Some(1.0);
        wheel.y = Some(2.0);
        wheel.dy = Some(-3.0);
        let press = primary_press([0.0, 0.0]);
        let (requests, next) = plan_input(Some(press), &wheel, 0).unwrap();
        assert!(matches!(
            requests[0],
            InteractionRequest::Scroll { delta_logical, .. } if delta_logical == [0.0, 3.0]
        ));
        assert_eq!(next, Some(press), "a wheel never disturbs the press");

        // Resize stays an explicit refusal (it goes through set_canvas_metrics).
        assert!(plan_input(None, &probe(InputKind::Resize), 0).is_err());
    }

    #[test]
    fn a_synthesized_click_selects_the_picked_entity_and_feeds_the_overlay_flags_path() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));
        // The pick the host will run at that pixel: the click must select exactly
        // this object (never an invented one).
        let expected = {
            let c = session.core.controller.borrow();
            let drawing = c.drawing().expect("the demo document");
            let viewport = &c.application.workspace.viewports[&c.viewport_id];
            cad_app::picking::pick_at_screen(
                &drawing,
                c.document_id,
                &viewport.camera,
                pixel,
                viewport.logical_size,
                &cad_domain::TolerancePolicy::default(),
                cad_app::BackFacePolicy::Cull,
            )
            .expect("the demo circle rim is pickable")
            .hit
            .expect("the demo circle rim is pickable")
            .source
        };
        let camera_before = viewport_camera(&session);

        session
            .handle_input(&pointer_at(InputKind::PointerDown, 1, pixel, None))
            .expect("the press is admitted");
        session
            .handle_input(&pointer_at(InputKind::PointerUp, 2, pixel, None))
            .expect("the release is admitted");
        assert_eq!(
            session.snapshot(None, None, 0).error,
            None,
            "a click must complete without a business error"
        );

        let selected = session
            .core
            .controller
            .borrow()
            .session
            .selection
            .refs()
            .to_vec();
        assert_eq!(
            selected,
            vec![expected.clone()],
            "a click replaces the selection with the real pick result"
        );
        assert_eq!(
            viewport_camera(&session),
            camera_before,
            "a click must not pan the view"
        );
        // The selection reaches the overlay inputs that produce FLAG_SELECTED.
        assert!(
            session.core.render.overlays().selection.contains(&expected),
            "the picked ref must reach the entity-flags input"
        );

        // And the frozen status block describes it honestly.
        let selection = session.snapshot(None, None, 0).selection;
        assert_eq!(selection.count, 1);
        assert!(!selection.empty);
        assert!(selection.mixed_keys.is_empty());
        let id_row = selection
            .rows
            .iter()
            .find(|row| row.key == "id")
            .expect("a single selection reports its identity rows");
        assert_eq!(id_row.value, expected.entity.0.to_string());
    }

    #[test]
    fn a_primary_drag_pans_without_selecting() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let camera_before = viewport_camera(&session);
        let start = screen_of(&session, demo_circle_rim(&session));
        let end = [
            start[0] + DRAG_THRESHOLD_LOGICAL_PX + 20.0,
            start[1] + DRAG_THRESHOLD_LOGICAL_PX + 20.0,
        ];

        session
            .handle_input(&pointer_at(InputKind::PointerDown, 1, start, None))
            .unwrap();
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 2, end, None))
            .unwrap();
        session
            .handle_input(&pointer_at(InputKind::PointerUp, 3, end, None))
            .unwrap();

        assert!(
            session
                .core
                .controller
                .borrow()
                .session
                .selection
                .is_empty(),
            "a drag beyond the threshold must not select"
        );
        assert_ne!(
            viewport_camera(&session),
            camera_before,
            "a primary drag still pans the view"
        );
        assert_eq!(
            session.snapshot(None, None, 0).selection,
            CadSelection::empty(),
            "the status block stays honestly empty"
        );
    }

    #[test]
    fn a_middle_drag_pans_without_selecting() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let camera_before = viewport_camera(&session);
        let start = screen_of(&session, demo_circle_rim(&session));
        let end = [start[0], start[1] + 40.0];

        session
            .handle_input(&pointer_at(
                InputKind::PointerDown,
                1,
                start,
                Some(BUTTON_MIDDLE),
            ))
            .unwrap();
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 2, end, None))
            .unwrap();
        session
            .handle_input(&pointer_at(
                InputKind::PointerUp,
                3,
                end,
                Some(BUTTON_MIDDLE),
            ))
            .unwrap();

        assert!(
            session
                .core
                .controller
                .borrow()
                .session
                .selection
                .is_empty(),
            "the middle button is a pan gesture, never a selection"
        );
        assert_ne!(
            viewport_camera(&session),
            camera_before,
            "a middle drag pans the view"
        );
    }

    #[test]
    fn cad_status_selection_block_is_always_present_and_honest() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        // Empty.
        let empty = session.snapshot(None, None, 0).selection;
        assert_eq!(empty, CadSelection::empty());
        assert_eq!(empty.count, 0);
        assert!(empty.empty);
        assert!(empty.rows.is_empty() && empty.mixed_keys.is_empty());

        let (entities, document) = {
            let c = session.core.controller.borrow();
            let drawing = c.drawing().expect("the demo document");
            let mut ids: Vec<_> = drawing.model_space().iter().map(|e| e.id).collect();
            ids.truncate(2);
            (ids, c.document_id)
        };
        assert_eq!(entities.len(), 2, "the demo document has ≥2 entities");
        let refs: Vec<_> = entities
            .iter()
            .map(|entity| cad_app::selection::model_ref(document, *entity))
            .collect();

        // Single: the rows mirror the application's own property extraction.
        session
            .core
            .controller
            .borrow_mut()
            .session
            .selection
            .replace([refs[0].clone()]);
        let single = session.snapshot(None, None, 0).selection;
        let expected = {
            let c = session.core.controller.borrow();
            let drawing = c.drawing().unwrap();
            cad_app::SelectionProperties::extract(&drawing, &c.session.selection)
        };
        assert_eq!(single.count, 1);
        assert!(!single.empty);
        assert_eq!(single.count as usize, expected.count);
        assert_eq!(single.rows.len(), expected.rows.len());
        for (wire, row) in single.rows.iter().zip(expected.rows.iter()) {
            assert_eq!(wire.key, row.key, "the machine key is carried verbatim");
            assert_eq!(wire.value, row.value);
        }
        assert!(
            single.rows.iter().any(|row| row.key == "id")
                && single.rows.iter().any(|row| row.key == "type"),
            "identity rows are the stable machine keys: {:?}",
            single.rows
        );
        assert!(
            single.mixed_keys.is_empty(),
            "a single selection has no mixed keys"
        );

        // Multi: a count plus the mixed machine keys, never rows.
        session
            .core
            .controller
            .borrow_mut()
            .session
            .selection
            .replace(refs.clone());
        let multi = session.snapshot(None, None, 0).selection;
        assert_eq!(multi.count, 2);
        assert!(!multi.empty);
        assert!(multi.rows.is_empty(), "a multi-selection carries no rows");
        assert!(
            !multi.mixed_keys.is_empty(),
            "two different entities must report mixed keys"
        );
        assert!(
            multi.mixed_keys.iter().any(|key| key == "id"),
            "the id machine key differs across distinct entities: {:?}",
            multi.mixed_keys
        );

        // Clearing it again is reflected by the next snapshot.
        session
            .core
            .controller
            .borrow_mut()
            .session
            .selection
            .clear();
        assert_eq!(session.snapshot(None, None, 0).selection.count, 0);
    }

    #[test]
    fn cad_status_selection_block_does_not_fabricate_rows_for_a_foreign_ref() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        // A ref that names an entity this document does not contain (a foreign or
        // deleted identity) must stay an honest `unresolved` report.
        let foreign = cad_domain::SelectionRef {
            document: cad_domain::DocumentId(4242),
            entity: cad_domain::EntityId(999_999),
            instance: cad_domain::InstancePath::default(),
            sub_element: None,
        };
        session
            .core
            .controller
            .borrow_mut()
            .session
            .selection
            .replace([foreign]);
        let selection = session.snapshot(None, None, 0).selection;
        assert_eq!(selection.count, 1);
        assert!(!selection.empty);
        let keys: Vec<&str> = selection.rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["id", "status"],
            "an unresolvable ref reports identity + status only"
        );
        assert_eq!(selection.rows[1].value, "unresolved");
        for fabricated in ["type", "layer", "length", "radius", "start"] {
            assert!(
                !keys.contains(&fabricated),
                "no geometry row may be invented for an unresolved ref"
            );
        }
        assert!(selection.mixed_keys.is_empty());
    }

    #[test]
    fn a_session_without_a_document_reports_an_honest_empty_selection_block() {
        let status = unsupported_status(Some("CAD is disabled".into()));
        assert_eq!(status.selection, CadSelection::empty());
        assert!(status.selection.empty);
        assert_eq!(status.selection.count, 0);
        assert!(status.selection.rows.is_empty());
        assert!(status.selection.mixed_keys.is_empty());
    }

    #[test]
    fn a_missing_property_source_never_hides_a_real_selection_behind_a_zero() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let (document, entity) = {
            let c = session.core.controller.borrow();
            let entity = c.drawing().expect("the demo document").model_space()[0].id;
            (c.document_id, entity)
        };
        session
            .core
            .controller
            .borrow_mut()
            .session
            .selection
            .replace([cad_app::selection::model_ref(document, entity)]);
        // Remove the document: the property extraction has no source any more.
        session
            .core
            .controller
            .borrow_mut()
            .application
            .workspace
            .documents
            .remove(&document);

        let status = session.snapshot(None, None, 0);
        assert_eq!(
            status.document_label, None,
            "the missing document is honest"
        );
        assert_eq!(
            status.selection.count, 1,
            "the real ref count must not be hidden behind a fabricated 0"
        );
        assert!(!status.selection.empty);
        assert!(
            status.selection.rows.is_empty(),
            "no property row may be invented without a property source"
        );
        assert!(status.selection.mixed_keys.is_empty());
    }

    // --- hover (ADR-0012 §10 S2) ---------------------------------------------

    /// The ref the click-selection pick path reports at a pixel in this session.
    fn picked_ref_at(session: &CadSession, pixel: [f64; 2]) -> cad_domain::SelectionRef {
        let c = session.core.controller.borrow();
        let drawing = c.drawing().expect("the demo document");
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        cad_app::picking::pick_at_screen(
            &drawing,
            c.document_id,
            &viewport.camera,
            pixel,
            viewport.logical_size,
            &cad_domain::TolerancePolicy::default(),
            cad_app::BackFacePolicy::Cull,
        )
        .expect("the pixel produces a pick")
        .hit
        .expect("the pixel is pickable")
        .source
    }

    /// Whether the pick path reports no hit at a pixel in this session.
    fn picks_nothing(session: &CadSession, pixel: [f64; 2]) -> bool {
        let c = session.core.controller.borrow();
        let drawing = c.drawing().expect("the demo document");
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        cad_app::picking::pick_at_screen(
            &drawing,
            c.document_id,
            &viewport.camera,
            pixel,
            viewport.logical_size,
            &cad_domain::TolerancePolicy::default(),
            cad_app::BackFacePolicy::Cull,
        )
        .expect("the pixel produces a pick")
        .hit
        .is_none()
    }

    /// The hovered refs that reached the producer contract (`OverlayInputs`).
    fn hovered_refs(session: &CadSession) -> Vec<cad_domain::SelectionRef> {
        session.core.render.overlays().hovered.refs().to_vec()
    }

    #[test]
    fn plan_input_emits_a_hover_cursor_request_tagged_with_the_current_revision() {
        let moved = pointer_at(InputKind::PointerMove, 1, [40.0, 50.0], None);
        let (requests, next) = plan_input(None, &moved, 9).unwrap();
        assert!(next.is_none(), "an idle move retains no press");
        assert_eq!(requests.len(), 2, "move then hover: {requests:?}");
        assert!(matches!(
            requests[0],
            InteractionRequest::Pointer {
                phase: PointerPhase::Move,
                ..
            }
        ));
        match &requests[1] {
            InteractionRequest::MoveCursor(PointInput::CanvasLocal {
                logical_position,
                metrics_revision,
            }) => {
                assert_eq!(*logical_position, [40.0, 50.0]);
                assert_eq!(
                    *metrics_revision, 9,
                    "the hover must carry the current revision, never a fabricated one"
                );
            }
            other => panic!("expected a hover cursor request: {other:?}"),
        }
    }

    #[test]
    fn plan_input_suppresses_hover_while_a_press_is_active() {
        // Within the drag threshold the press is a pending tap, not a hover.
        let moved = pointer_at(InputKind::PointerMove, 2, [10.0, 20.5], None);
        let (requests, press) = plan_input(Some(primary_press([10.0, 20.0])), &moved, 7).unwrap();
        assert_eq!(
            requests.len(),
            1,
            "a pressed move claims no hover: {requests:?}"
        );
        assert!(matches!(
            requests[0],
            InteractionRequest::Pointer {
                phase: PointerPhase::Move,
                ..
            }
        ));
        assert!(!press.expect("the press continues").dragged);

        // Beyond the threshold the gesture is a drag: still no hover.
        let dragged = pointer_at(
            InputKind::PointerMove,
            3,
            [10.0 + DRAG_THRESHOLD_LOGICAL_PX + 5.0, 20.0],
            None,
        );
        let (requests, press) = plan_input(Some(primary_press([10.0, 20.0])), &dragged, 7).unwrap();
        assert!(
            matches!(
                requests.as_slice(),
                [InteractionRequest::Pointer {
                    phase: PointerPhase::Move,
                    ..
                }]
            ),
            "a dragged move claims no hover: {requests:?}"
        );
        assert!(press.expect("the press continues").dragged);

        // The same holds for a middle-button press (the pan gesture).
        let down = pointer_at(InputKind::PointerDown, 4, [0.0, 0.0], Some(BUTTON_MIDDLE));
        let (_, middle_press) = plan_input(None, &down, 7).unwrap();
        let moved = pointer_at(InputKind::PointerMove, 5, [30.0, 10.0], None);
        let (requests, _) = plan_input(middle_press, &moved, 7).unwrap();
        assert_eq!(
            requests.len(),
            1,
            "a middle-button gesture owns its moves: {requests:?}"
        );
    }

    #[test]
    fn an_idle_move_over_an_entity_reaches_the_overlay_hovered_input() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));
        let expected = picked_ref_at(&session, pixel);

        session
            .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
            .expect("the hover move is admitted");

        assert_eq!(
            session.snapshot(None, None, 0).error,
            None,
            "a hover must complete without a business error"
        );
        assert_eq!(
            hovered_refs(&session),
            vec![expected],
            "the hover over the rim must reach the entity-flags input"
        );
        assert!(
            session
                .core
                .controller
                .borrow()
                .session
                .selection
                .is_empty(),
            "hover must never select"
        );
        assert_eq!(
            session.snapshot(None, None, 0).selection,
            CadSelection::empty(),
            "the frozen status block is untouched by a hover"
        );
    }

    #[test]
    fn an_idle_move_that_picks_nothing_clears_the_hovered_input() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
            .expect("the hover move is admitted");
        assert!(!hovered_refs(&session).is_empty(), "the rim is hovered");

        let empty = [1.0, 1.0];
        assert!(
            picks_nothing(&session, empty),
            "the chosen pixel must really be empty space"
        );
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 2, empty, None))
            .expect("the hover move is admitted");
        assert!(
            hovered_refs(&session).is_empty(),
            "a miss must clear the tint: no stale hover may survive"
        );
    }

    #[test]
    fn a_repeated_idle_move_at_the_same_position_is_skipped() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
            .expect("the hover move is admitted");
        let queries = session.core.hover_query_count();
        assert_eq!(queries, 1, "the first move ran one hover query");

        // Two more moves at the identical position: no new pick runs, and the
        // hover is unchanged. Observable behaviour (a counter), not a wall clock.
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 2, pixel, None))
            .expect("the repeated hover move is admitted");
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 3, pixel, None))
            .expect("the repeated hover move is admitted");
        assert_eq!(
            session.core.hover_query_count(),
            queries,
            "an unchanged position must not re-run the hover query"
        );
        assert!(!hovered_refs(&session).is_empty(), "the hover is unchanged");
    }

    #[test]
    fn a_press_clears_the_hover_and_the_release_claims_no_new_hover() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));
        let expected = picked_ref_at(&session, pixel);

        session
            .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
            .expect("the hover move is admitted");
        assert_eq!(hovered_refs(&session), vec![expected.clone()]);

        session
            .handle_input(&pointer_at(InputKind::PointerDown, 2, pixel, None))
            .expect("the press is admitted");
        assert!(
            hovered_refs(&session).is_empty(),
            "a press start clears the hover"
        );

        session
            .handle_input(&pointer_at(InputKind::PointerUp, 3, pixel, None))
            .expect("the release is admitted");
        assert!(
            hovered_refs(&session).is_empty(),
            "the released click claims no hover of its own"
        );

        // The next real move restores a hover for the (unchanged) view.
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 4, pixel, None))
            .expect("the hover move is admitted");
        assert_eq!(
            hovered_refs(&session),
            vec![expected],
            "a hover is restored only by the move after the press"
        );
    }

    #[test]
    fn a_primary_drag_leaves_the_hover_cleared_until_the_next_move() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let start = screen_of(&session, demo_circle_rim(&session));
        let end = [
            start[0] + DRAG_THRESHOLD_LOGICAL_PX + 20.0,
            start[1] + DRAG_THRESHOLD_LOGICAL_PX + 20.0,
        ];

        session
            .handle_input(&pointer_at(InputKind::PointerMove, 1, start, None))
            .expect("the hover move is admitted");
        assert!(!hovered_refs(&session).is_empty(), "the rim is hovered");

        session
            .handle_input(&pointer_at(InputKind::PointerDown, 2, start, None))
            .expect("the press is admitted");
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 3, end, None))
            .expect("the dragged move is admitted");
        assert!(
            hovered_refs(&session).is_empty(),
            "a drag must not leave a hover tint behind"
        );
        session
            .handle_input(&pointer_at(InputKind::PointerUp, 4, end, None))
            .expect("the release is admitted");
        assert!(hovered_refs(&session).is_empty());

        // The view panned, so the rim is at a new pixel: the next move resolves a
        // *new* hover there.
        let after_pan = screen_of(&session, demo_circle_rim(&session));
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 5, after_pan, None))
            .expect("the hover move is admitted");
        assert!(
            !hovered_refs(&session).is_empty(),
            "the first move after the press restores a hover"
        );
    }

    #[test]
    fn a_blur_clears_the_hovered_input() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));
        session
            .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
            .expect("the hover move is admitted");
        assert!(!hovered_refs(&session).is_empty(), "the rim is hovered");

        // The window lost focus: the hover it produced must not survive it.
        session
            .handle_input(&probe(InputKind::Blur))
            .expect("the blur is admitted");
        assert!(hovered_refs(&session).is_empty(), "a blur clears the hover");
    }

    #[test]
    fn plan_input_maps_pointer_leave_to_clear_cursor_without_touching_the_press() {
        // A bare leave yields exactly the overlay cursor clear: no pointer
        // request, no coordinates, and no revision demand it cannot satisfy.
        let leave = probe(InputKind::PointerLeave);
        let (requests, next) = plan_input(None, &leave, 0).unwrap();
        assert_eq!(requests, vec![InteractionRequest::ClearCursor]);
        assert!(next.is_none(), "a leave with no press retains none");

        // It also leaves an in-flight press untouched: the exact same press is
        // returned, so a later release still resolves against it.
        let press = primary_press([10.0, 20.0]);
        let (requests, next) = plan_input(Some(press), &leave, 0).unwrap();
        assert_eq!(requests, vec![InteractionRequest::ClearCursor]);
        assert_eq!(next, Some(press), "the leave must not disturb the press");
    }

    #[test]
    fn a_pointer_leave_clears_the_hover_overlay_only() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));

        session
            .handle_input(&pointer_at(InputKind::PointerMove, 1, pixel, None))
            .expect("the hover move is admitted");
        assert!(!hovered_refs(&session).is_empty(), "the rim is hovered");
        // Consume the hover's own dirty markers so the assertions below are
        // about the leave alone.
        let _ = EngineClient::take_updates(&session.core).unwrap();

        session
            .handle_input(&pointer_at(InputKind::PointerLeave, 2, pixel, None))
            .expect("the leave is admitted");
        assert!(
            hovered_refs(&session).is_empty(),
            "the leave clears the hover tint"
        );

        // Overlay-only: leaving the surface schedules neither the full host
        // state nor a navigation rebuild (the same overlay-only classification
        // the hover `MoveCursor` uses, ADR-0012 §10 S2).
        let updates = EngineClient::take_updates(&session.core).unwrap();
        assert!(
            updates.full.is_none(),
            "leaving the surface must not rebuild the full host state"
        );
        assert!(
            updates.navigation.is_none(),
            "leaving the surface is not a navigation change"
        );
    }

    #[test]
    fn a_pointer_leave_never_ends_or_wedges_an_in_flight_press() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));
        let expected = picked_ref_at(&session, pixel);

        session
            .handle_input(&pointer_at(InputKind::PointerDown, 1, pixel, None))
            .expect("the press is admitted");
        // The React lane only sends a leave with no button held, but the Rust
        // behaviour is defined regardless: it clears the cursor overlay and
        // leaves the press intact, so the release still resolves as a click.
        session
            .handle_input(&pointer_at(InputKind::PointerLeave, 2, pixel, None))
            .expect("the leave is admitted");
        session
            .handle_input(&pointer_at(InputKind::PointerUp, 3, pixel, None))
            .expect("the release is admitted");

        assert_eq!(
            session
                .core
                .controller
                .borrow()
                .session
                .selection
                .refs()
                .to_vec(),
            vec![expected],
            "the release after the leave still selects the picked entity"
        );
        assert_eq!(session.snapshot(None, None, 0).error, None);
    }

    #[test]
    fn a_stale_hover_cursor_is_refused_and_never_tints() {
        let session = session(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        }));
        let pixel = screen_of(&session, demo_circle_rim(&session));
        let context = session.core.request_context();
        let request =
            EngineRequest::Interaction(InteractionRequest::MoveCursor(PointInput::CanvasLocal {
                logical_position: pixel,
                metrics_revision: context.metrics_revision.unwrap_or(0),
            }));
        let receipt = session.core.submit(context, request).unwrap();
        session.track_receipt(receipt.id);
        // The canvas metrics advance before the queued hover runs, so the host
        // grades it stale.
        session.set_canvas_metrics([640.0, 480.0], 1.0).unwrap();
        session.pump().unwrap();
        assert!(
            session.drain_completions().is_err(),
            "a stale hover must not be an empty success"
        );
        assert!(
            hovered_refs(&session).is_empty(),
            "a refused hover must not tint anything"
        );
    }
}
