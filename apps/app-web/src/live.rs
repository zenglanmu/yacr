//! **The production `dom-canvas` session**: the real desktop `CadSession` behind
//! the canvas exports (ADR 0024 stage ⑧, the production flip).
//!
//! Until this module existed the `dom-canvas` carriers ran a *viewer* session
//! (`crate::session`): import → scene → its own `Renderer`, then camera-only
//! navigation, a released database and no command router. This module replaces
//! that on the **page** carrier with the session the native hosts drive —
//! `cad_ui_core::session::CadSession` over [`BrowserPlatform`] — so one document
//! authority (`cad-db`), one command router and one wire vocabulary serve every
//! carrier.
//!
//! Everything here is **target-independent on purpose** (no `web-sys`, no wgpu, no
//! `CanvasPresenter`): the platform contracts, the budgets, the document
//! replacement, the input synthesis, the popup gate and both status projections
//! are plain Rust over `cad-host`/`cad-ui-core`, so the acceptance for this flip is
//! asserted by `cargo test -p app-web` on the host. What a host test cannot reach is
//! the pixel half — the device, `render` and the canvas blit — which lives in
//! [`crate::page`] (wasm-only) and is evidenced by the browser run
//! (`docs/validation-dom-canvas-flip-2026-10-28.md`).
//!
//! # What changed for the caller (same exports, different session)
//!
//! | export | before (viewer) | now (real session) |
//! |---|---|---|
//! | `start` / `open_bytes` | import + build + upload + present in one call | build/replace the real session, apply the ADR 0025 budgets, one bounded first-frame attempt |
//! | `resize` | presenter + camera | presenter **and** `set_canvas_metrics` (the session's own metric revision) |
//! | `pan` / `zoom_at` | camera only | synthesized `ProbeInput`s through `CadSession::handle_input`, then one frame |
//! | `status` / `document` | viewer facts | `CadSession::snapshot` + the documented canvas facts |
//! | `shell_status`/`shell_commands`/`shell_aliases`/`shell_command` | three selectors | the full router vocabulary (`admissible_names()`), executed by the session |
//! | `invoke` | spike, feature-gated | production, all 20 wire commands |
//!
//! # The two things a real session changes about an open
//!
//! 1. **The database is retained.** The viewer released it after the display build
//!    ([`crate::budget::DB_RETAINED`] is `false`, and stays `false` for the worker
//!    carrier). The real session *is* the document authority, so
//!    [`DB_RETAINED`] is `true` here and the status says so. That is the whole
//!    point (commands, selection, layers, entity counts) and it is also the reason
//!    the wasm heap stays higher than the viewer's.
//! 2. **An open is a document replacement, not a rebuild**
//!    ([`CadSession::open_bytes`](cad_ui_core::session::CadSession::open_bytes) →
//!    `HostCore::deliver_external_open`). The device, the presenter, the platform
//!    adapters and the import counter survive, which is what the desktop hosts do
//!    and what makes `importCount` grow on a second open.
//!
//! # Where the budgets apply, and why the heap check moved
//!
//! ADR 0025's two budgets are applied **before** the bytes are parsed or
//! installed, with the viewer's own limits and refusal shape
//! ([`crate::budget`]):
//!
//! * [`budget::check_input_bytes`] before the copy/parse (64 MiB);
//! * [`budget::check_heap_after_build`] on the **heap this module already holds**
//!   before another document is installed (3 GiB), because the carrier cannot undo
//!   an install: `deliver_external_open` is the host's atomic document
//!   replacement, so a refusal *after* it would leave an over-budget document open
//!   while reporting a refusal. That is the honest difference from the viewer,
//!   which owned the build and could drop its freshly built scene inside the
//!   refusal.
//!
//! The heap is measured once more **after** the install and published
//! (`heap_after_open`) together with an explicit `over_budget` block when the
//! retained session ended up above the cap. Nothing is raised silently: the limit
//! and the measurement are both in the status.
//!
//! # Honest limits of this module
//!
//! * **One session, one canvas** is enforced by [`crate::page`]'s registry, not
//!   here; [`Live`] is just the owned state.
//! * **No pixels.** `render`/`present` are the caller's
//!   ([`crate::page::present_once`]).
//! * **No filesystem, no durable config, no font/image package**: the browser
//!   contracts, see [`crate::platform`].

use std::cell::RefCell;
use std::rc::Rc;

use cad_engine::render::RenderSessionHandle;
use cad_host::{HostOptions, RequestClock};
use cad_platform::HostCapabilities;
use cad_ui_core::popup::{self as popup_state, PopupAck, PopupGate, PopupOps, TextEditPayload};
use cad_ui_core::session::wire::PopupTarget;
use cad_ui_core::session::{BytesDocument, CadPopup, CadSession, CadStatus, CadTool};
use cad_ui_core::shell::projected_popup;
/// Input synthesis for `pan`/`zoom_at` (see the module docs).
pub mod input;

/// The shared refusal of the modal gate (ADR-0021 D2), re-exported so both
/// carriers use one word.
pub use cad_ui_core::shell::POPUP_MODAL_OPEN;
use serde_json::{json, Value};

use crate::budget::{self, BudgetRefusal};
use crate::platform::BrowserPlatform;

/// Whether the production session keeps the imported database after an open.
///
/// **`true`**, and it is not a flag this module may flip: the session *is* the
/// document authority (`cad-db`), which is what makes the command router,
/// selection, layers and `entityCount` real. The viewer's
/// [`crate::budget::DB_RETAINED`] describes the other carrier (the worker's
/// viewer session, which still releases the database) and stays `false`.
pub const DB_RETAINED: bool = true;

/// The compiled-in synthetic fixture (`fixtures/manifest`): four `LINE`s, no text
/// and no raster image, so the browser resource strategy is a real no-op.
pub const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../fixtures/dwg/synthetic-four-lines.dwg");
/// The label `start` reports as the open document.
pub const SYNTHETIC_LABEL: &str = "synthetic-four-lines.dwg";

// ---------------------------------------------------------------------------
// The stable failure vocabulary
// ---------------------------------------------------------------------------

/// Stable step names, the same vocabulary the viewer session publishes, so a host
/// that keys on `step` (evidence collection, the `webgl2` marker, exit codes) does
/// not have to learn a second set.
pub const STEP_VIEWPORT: &str = "viewport";
pub const STEP_CANVAS: &str = "canvas-element";
pub const STEP_WEBGL2: &str = "webgl2";
pub const STEP_IMPORT: &str = "import";
pub const STEP_BUDGET: &str = "budget";
pub const STEP_STATE: &str = "state";
pub const STEP_DEVICE: &str = "device";
pub const STEP_RENDER: &str = "render";
pub const STEP_PRESENT: &str = "present";
pub const STEP_UPLOAD: &str = "upload";
pub const STEP_INPUT: &str = "input";
pub const STEP_NAVIGATION: &str = "navigation";

/// The stable refusal marker of a `dom-canvas` host without usable WebGL2
/// (ADR 0024 D5). Only [`STEP_WEBGL2`] carries it.
pub const WEBGL2_MARKER: &str = "yacr: unsupported-runtime: webgl2";

/// One failed step: the stable step name, the technical message, and — for a
/// named memory budget — the structured numbers behind the refusal.
#[derive(Debug, Clone)]
pub struct Failure {
    pub step: &'static str,
    pub message: String,
    pub budget: Option<BudgetRefusal>,
}

impl Failure {
    pub fn new(step: &'static str, message: impl Into<String>) -> Self {
        Self {
            step,
            message: message.into(),
            budget: None,
        }
    }

    /// A [`STEP_BUDGET`] failure from a named budget refusal.
    pub fn from_refusal(refusal: BudgetRefusal) -> Self {
        Self {
            step: STEP_BUDGET,
            message: refusal.message.clone(),
            budget: Some(refusal),
        }
    }

    /// The stable marker this step carries, if any (only `webgl2` does).
    pub fn marker(&self) -> Option<&'static str> {
        (self.step == STEP_WEBGL2).then_some(WEBGL2_MARKER)
    }

    /// `step: message`, the shape every canvas export rejects with.
    pub fn describe(&self) -> String {
        format!("{}: {}", self.step, self.message)
    }

    /// Add this failure to a status object (`step`, `error`, `marker`, `budget`).
    pub fn apply(&self, object: &mut Value) {
        object["step"] = json!(self.step);
        object["error"] = json!(self.message);
        if let Some(marker) = self.marker() {
            object["marker"] = json!(marker);
        }
        if let Some(refusal) = &self.budget {
            object["budget"] = budget_json(refusal);
        }
    }
}

/// The structured `budget: {name, limit_bytes, measured_bytes}` block.
pub fn budget_json(refusal: &BudgetRefusal) -> Value {
    json!({
        "name": refusal.name,
        "limit_bytes": refusal.limit_bytes,
        "measured_bytes": refusal.measured_bytes,
    })
}

// ---------------------------------------------------------------------------
// Canvas metrics
// ---------------------------------------------------------------------------

/// Canvas sizing in **CSS** pixels plus the device pixel ratio.
///
/// The same validation and the same messages the viewer uses, so a bad viewport is
/// refused identically on both carriers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub css: [f64; 2],
    pub dpr: f64,
}

impl Metrics {
    pub fn new(css_w: f64, css_h: f64, dpr: f64) -> Result<Self, String> {
        if !(css_w.is_finite() && css_h.is_finite() && dpr.is_finite()) {
            return Err(format!(
                "viewport must be finite: css={css_w}x{css_h}, dpr={dpr}"
            ));
        }
        if css_w <= 0.0 || css_h <= 0.0 {
            return Err(format!("canvas CSS size must be positive: {css_w}x{css_h}"));
        }
        if dpr <= 0.0 {
            return Err("devicePixelRatio must be positive".to_string());
        }
        Ok(Self {
            css: [css_w, css_h],
            dpr,
        })
    }

    /// The physical (device-pixel) size the presenter configures.
    pub fn physical(self) -> Result<(u32, u32), String> {
        let width = (self.css[0] * self.dpr).round();
        let height = (self.css[1] * self.dpr).round();
        if width < 1.0 || height < 1.0 {
            return Err(format!(
                "physical size rounds to zero: {width}x{height} (css {}x{} at dpr {})",
                self.css[0], self.css[1], self.dpr
            ));
        }
        if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
            return Err(format!("physical size out of range: {width}x{height}"));
        }
        Ok((width as u32, height as u32))
    }

    /// The viewport centre in logical (CSS) pixels: the anchor a cursor-less
    /// selector (`ZOOMIN`/`ZOOMOUT`, a synthetic pan) uses.
    pub fn centre(self) -> [f64; 2] {
        [self.css[0] * 0.5, self.css[1] * 0.5]
    }
}

// ---------------------------------------------------------------------------
// The live session
// ---------------------------------------------------------------------------

/// The production session: the real `CadSession` plus the facts this carrier owns.
pub struct Live {
    /// The authoritative owner-thread CAD session (host core + `cad-db`).
    pub session: CadSession,
    /// The browser platform contracts it was built over (its surface cell is what
    /// `resize` reports into).
    pub platform: BrowserPlatform,
    /// The source file name of the open document.
    pub label: String,
    /// The source DWG length in bytes (evidence: what was actually opened).
    pub bytes: usize,
    /// The current canvas metrics.
    pub metrics: Metrics,
    /// Build + import + first preparation of the last install, in milliseconds,
    /// measured with the injected monotonic clock — never `std::time` on wasm.
    pub import_ms: f64,
    /// Wasm linear-memory length right after the last install (a high-water mark).
    pub heap_after_open: Option<f64>,
    /// The post-install measurement when it is over [`budget::MAX_WASM_HEAP_BYTES`].
    pub over_budget: Option<BudgetRefusal>,
    /// `tick` calls that returned `Ok`.
    pub ticks: u64,
    /// `render` calls that produced a frame.
    pub frames: u64,
    /// `render` calls that returned no frame (a real wait, never a present).
    pub waited: u64,
    /// The last failure, kept so `status()` keeps reporting it while the session
    /// stays usable.
    pub last_error: Option<Failure>,
    /// The monotonic millisecond clock the session was built with (the same one
    /// `HostCore` uses for its request pump), so a replacement can be timed without
    /// a second time source.
    clock: RequestClock,
    /// The single modal popup (ADR-0021 D1/D2), the same state machine the desktop
    /// hosts run.
    pub popups: PopupSlot,
    /// The synthetic pointer gesture `pan`/`zoom_at` maintain (see [`input`]).
    pub input: input::InputState,
}

impl Live {
    /// Build the session over `bytes` and open them.
    ///
    /// The platform surface is reported **before** the session exists: the host's
    /// open funnel validates the canvas metrics against it, and an unmapped surface
    /// is an explicit refusal downstream. Both memory budgets run before the bytes
    /// are handed to the importer, and a refusal leaves nothing installed (the
    /// caller's previous session, if any, is untouched — `start` builds a *new*
    /// one, and an open replaces in place through [`Live::replace_document`]).
    pub fn install(
        render: RenderSessionHandle,
        clock: RequestClock,
        label: &str,
        bytes: &[u8],
        metrics: Metrics,
        heap: &dyn Fn() -> Option<f64>,
    ) -> Result<Self, Failure> {
        let label = label.trim();
        if label.is_empty() {
            return Err(Failure::new(
                STEP_INPUT,
                "open_bytes needs a non-empty label (the source file name)",
            ));
        }
        // Both budgets, before anything is copied or parsed.
        budget::check_input_bytes(label, bytes.len()).map_err(Failure::from_refusal)?;
        budget::check_heap_after_build(label, heap()).map_err(Failure::from_refusal)?;

        let platform = BrowserPlatform::new();
        platform.surface().report(metrics.css, metrics.dpr);
        let started = clock();
        let session = CadSession::from_bytes(
            options(metrics),
            platform.host_platform(HostCapabilities::default()),
            render,
            Rc::clone(&clock),
            BytesDocument { label, bytes },
        )
        .map_err(|error| Failure::new(STEP_IMPORT, error.to_string()))?;
        let import_ms = (clock() - started).max(0.0);
        session
            .set_canvas_metrics(metrics.css, metrics.dpr)
            .map_err(|message| Failure::new(STEP_VIEWPORT, message))?;
        let measured = heap();
        Ok(Self {
            session,
            platform,
            label: label.to_string(),
            bytes: bytes.len(),
            metrics,
            import_ms,
            heap_after_open: measured,
            over_budget: over_budget(label, measured),
            ticks: 0,
            frames: 0,
            waited: 0,
            last_error: None,
            clock,
            popups: PopupSlot::default(),
            input: input::InputState::default(),
        })
    }

    /// Replace the open document with `bytes`, in place.
    ///
    /// This is the second open of the one session
    /// ([`CadSession::open_bytes`](cad_ui_core::session::CadSession::open_bytes) →
    /// the host's own `deliver_external_open`), so the device, the presenter, the
    /// platform adapters and the import counter survive. Both budgets run first;
    /// a refusal (or a failed parse) leaves the previous document, its pixels and
    /// its navigation exactly as they were.
    pub fn replace_document(
        &mut self,
        label: &str,
        bytes: &[u8],
        heap: &dyn Fn() -> Option<f64>,
    ) -> Result<(), Failure> {
        let label = label.trim();
        if label.is_empty() {
            return Err(Failure::new(
                STEP_INPUT,
                "open_bytes needs a non-empty label (the source file name)",
            ));
        }
        budget::check_input_bytes(label, bytes.len()).map_err(Failure::from_refusal)?;
        budget::check_heap_after_build(label, heap()).map_err(Failure::from_refusal)?;
        // A pan gesture belongs to the document that was open when it started.
        self.close_pan_inputs();
        let started = self.clock_now();
        self.session
            .open_bytes(label, bytes)
            .map_err(|error| Failure::new(STEP_IMPORT, error.to_string()))?;
        self.import_ms = (self.clock_now() - started).max(0.0);
        self.label = label.to_string();
        self.bytes = bytes.len();
        self.heap_after_open = heap();
        self.over_budget = over_budget(label, self.heap_after_open);
        Ok(())
    }

    /// The clock this module measures with (the session's own request clock).
    fn clock_now(&self) -> f64 {
        (self.clock)()
    }

    /// Refresh the canvas metrics: the session's own metric revision **and** the
    /// platform surface it validates against.
    pub fn set_metrics(&mut self, metrics: Metrics) -> Result<(), Failure> {
        self.platform.surface().report(metrics.css, metrics.dpr);
        self.session
            .set_canvas_metrics(metrics.css, metrics.dpr)
            .map_err(|message| Failure::new(STEP_VIEWPORT, message))?;
        self.metrics = metrics;
        Ok(())
    }

    /// One owner-thread tick, counted.
    pub fn tick(&mut self) -> Result<(), Failure> {
        self.session
            .tick()
            .map_err(|message| Failure::new(STEP_UPLOAD, message))?;
        self.ticks += 1;
        Ok(())
    }

    /// The physical size the presenter must be configured at.
    pub fn physical(&self) -> Result<(u32, u32), Failure> {
        self.metrics
            .physical()
            .map_err(|message| Failure::new(STEP_VIEWPORT, message))
    }

    /// The resident draw batches of the prepared scene, when one is published.
    ///
    /// A real count from the session's own prepared scene; `None` before the first
    /// preparation completes (never `0`, which would claim an empty drawing).
    pub fn batches(&self) -> Option<u64> {
        self.session
            .render_session()
            .current_prepared_scene()
            .map(|scene| scene.base.added.len() as u64)
    }

    /// The import progress the host reports right now (`prepared`/`total` pages).
    pub fn prepared(&self) -> Option<[u64; 2]> {
        self.session
            .render_session()
            .drawing_progress()
            .map(|(done, total)| [done as u64, total as u64])
    }

    /// Record a failure that must stay visible in the status (a retryable frame
    /// error), without destroying the session.
    pub fn note_failure(&mut self, failure: Failure) {
        self.last_error = Some(failure);
    }

    /// Clear the recorded failure after a step that succeeded.
    pub fn clear_failure(&mut self) {
        self.last_error = None;
    }

    // -- status projections -------------------------------------------------

    /// The frozen `cad_status` snapshot, projected from the authoritative session.
    ///
    /// `backend`/`adapter`/`frame_count` are the presenter's own values: without a
    /// live presenter nothing proves the backend came up. The popup block is the
    /// projected gate state (the session owns no window, so the host projects it).
    pub fn cad_status(&self, present: &PresenterFacts) -> CadStatus {
        let status = self.session.snapshot(
            Some(crate::shell::BACKEND_WEBGL2.to_string()),
            Some(present.adapter.clone()),
            present.presented,
        );
        projected_popup(status, self.popups.current().as_ref())
    }

    /// The whole `cad_status` wire object (`shell_status`).
    pub fn shell_status_json(&self, present: &PresenterFacts) -> String {
        let status = self.cad_status(present);
        let carrier = crate::shell::Carrier {
            canvas_session: Some(crate::shell::CanvasSession {
                label: self.label.clone(),
                batches: self.batches(),
                draws: None,
            }),
        };
        crate::shell::status_json(&status, &carrier)
    }

    /// The canvas status object the `start`/`open_bytes`/`resize`/`pan`/`zoom_at`
    /// exports answer with.
    ///
    /// The keys are the ones the viewer published, with two honest changes: `draws`
    /// is `null` (the session's engine owns the draw submission count and does not
    /// publish it to this layer), and `upload_chunks`/`peak_batches_per_chunk`/
    /// `bounds_retained` are gone (they described the viewer's chunked upload and
    /// its released database, neither of which exists here).
    ///
    /// `ok` means "the step completed on a live session": a **non-fatal** session
    /// diagnostic (an unresolved font/image package) is carried in `error` with
    /// `step: "session"` and does not clear it — the document really is open, and
    /// hiding that behind `ok: false` would make an opened drawing look like a
    /// failed open.
    pub fn canvas_status_json(&self, present: &PresenterFacts, heap: Option<f64>) -> String {
        let status = self.cad_status(present);
        let mut object = json!({
            // `ok` is "this call did what it was asked": a session that is live with
            // a document open is ok even when the session also carries a
            // **non-fatal** diagnostic (an unresolved font/image package, a host
            // catalog status). The diagnostic is still reported below — as `error`
            // with `step: "session"` — so nothing is hidden and nothing is
            // reclassified as a failed open.
            "ok": self.last_error.is_none(),
            "backend": present.backend,
            "adapter_name": present.adapter,
            "format": present.format,
            "target": present.target,
            "isolation": present.isolation,
            "physical": [present.physical.0, present.physical.1],
            "css": [self.metrics.css[0], self.metrics.css[1]],
            "dpr": self.metrics.dpr,
            "configure_count": present.configure_count,
            "label": self.label,
            "bytes": self.bytes,
            "entityCount": status.entity_count,
            "tool": status.tool.as_ref().map(tool_key),
            "selectionCount": status.selection.count,
            "presented": present.presented,
            "frames": self.frames,
            "ticks": self.ticks,
            "waited": self.waited,
            "batches": self.batches(),
            "draws": Value::Null,
            "prepared": self.prepared().map(|pages| json!(pages)),
            "world_per_px": status.camera.as_ref().map(|camera| camera.scale),
            "importCount": status.import_count,
            "import_ms": self.import_ms,
            "wasm_heap_bytes": heap,
            "heap_after_open": self.heap_after_open,
            "db_retained": DB_RETAINED,
        });
        if let Some(refusal) = &self.over_budget {
            object["over_budget"] = budget_json(refusal);
        }
        if let Some(failure) = &self.last_error {
            failure.apply(&mut object);
        } else if status.error.is_some() {
            // The session's own diagnostic (`fonts.failed`, a command/completion
            // failure, …) is the technical reason; it is reported, never swallowed.
            // It does not clear `ok`: the document is open and the session is live,
            // exactly like the desktop chrome, which shows `cad_status.error` while
            // the drawing stays interactive.
            object["step"] = json!("session");
            object["error"] = json!(status.error);
        }
        object.to_string()
    }

    /// The `document()` object: what this session holds, measured.
    pub fn document_json(&self, present: &PresenterFacts, heap: Option<f64>) -> String {
        let status = self.cad_status(present);
        let mut object = json!({
            // `ok` is "this call did what it was asked": a session that is live with
            // a document open is ok even when the session also carries a
            // **non-fatal** diagnostic (an unresolved font/image package, a host
            // catalog status). The diagnostic is still reported below — as `error`
            // with `step: "session"` — so nothing is hidden and nothing is
            // reclassified as a failed open.
            "ok": self.last_error.is_none(),
            "label": self.label,
            "bytes": self.bytes,
            "entityCount": status.entity_count,
            "batches": self.batches(),
            "draws": Value::Null,
            "presented": present.presented,
            "frames": self.frames,
            "importCount": status.import_count,
            "import_ms": self.import_ms,
            "wasm_heap_bytes": heap,
            "heap_after_open": self.heap_after_open,
            "db_retained": DB_RETAINED,
        });
        if let Some(refusal) = &self.over_budget {
            object["over_budget"] = budget_json(refusal);
        }
        if let Some(failure) = &self.last_error {
            failure.apply(&mut object);
        } else if let Some(error) = &status.error {
            // The session's own non-fatal diagnostic (`fonts.failed`, …) is reported
            // here too: `document()` never hides something `status()` reports.
            object["step"] = json!("session");
            object["error"] = json!(error);
        }
        object.to_string()
    }
}

/// The stable key of an active tool (`draw.line`, `measure.area`, …).
fn tool_key(tool: &CadTool) -> String {
    tool.key.clone()
}

/// Report the post-install measurement when it is over the named heap budget.
fn over_budget(label: &str, measured: Option<f64>) -> Option<BudgetRefusal> {
    budget::check_heap_after_build(label, measured).err()
}

/// The host options of the browser carrier.
///
/// `headless: false` is the interactive composition the Android/`web-adapter`
/// carrier already uses; there is no session-side file chooser (the page's picker
/// is a DOM capability), no import worker, and `drawing: None` because the
/// document *is* the bytes.
fn options(metrics: Metrics) -> HostOptions {
    HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: metrics.css,
    }
}

/// What the canvas presenter observed. The only GPU facts this layer may report.
#[derive(Debug, Clone)]
pub struct PresenterFacts {
    /// Stable backend token (`"gl"` for the WebGL2 backend).
    pub backend: String,
    /// The adapter name as the browser reports it (possibly masked: never a
    /// hardware claim).
    pub adapter: String,
    /// The configured surface format, `Debug`-formatted.
    pub format: String,
    /// The presenter's own carrier tag (`html-canvas` | `offscreen-canvas`).
    pub target: &'static str,
    /// The isolation derived from that tag (`main-thread` | `worker`).
    pub isolation: &'static str,
    /// The configured physical size.
    pub physical: (u32, u32),
    /// Frames that actually reached the canvas.
    pub presented: u64,
    /// `Surface::configure` calls since construction.
    pub configure_count: u64,
}

/// The status object of a start that never produced a session.
pub fn failure_status_json(failure: &Failure, heap: Option<f64>) -> String {
    let mut object = json!({
        "ok": false,
        "backend": Value::Null,
        "adapter_name": Value::Null,
        "format": Value::Null,
        "target": Value::Null,
        "isolation": Value::Null,
        "physical": Value::Null,
        "label": Value::Null,
        "bytes": 0,
        "entityCount": Value::Null,
        "tool": Value::Null,
        "presented": 0,
        "frames": 0,
        "ticks": 0,
        "waited": 0,
        "batches": Value::Null,
        "draws": Value::Null,
        "import_ms": -1.0,
        "wasm_heap_bytes": heap,
        "heap_after_open": Value::Null,
        "db_retained": DB_RETAINED,
    });
    failure.apply(&mut object);
    object.to_string()
}

// ---------------------------------------------------------------------------
// The popup gate (ADR-0021)
// ---------------------------------------------------------------------------

/// The host side of the single modal popup: one slot, cleared by every one of the
/// four lifecycle triggers.
///
/// The state machine itself is [`cad_ui_core::popup`] (the same one the desktop
/// hosts run); this type only supplies the slot, because a wasm module is
/// single-threaded and needs no `Mutex`.
#[derive(Debug, Default)]
pub struct PopupSlot {
    slot: RefCell<Option<CadPopup>>,
}

impl PopupGate for PopupSlot {
    fn current(&self) -> Option<CadPopup> {
        self.slot.borrow().clone()
    }

    fn replace(&self, next: Option<CadPopup>) {
        *self.slot.borrow_mut() = next;
    }

    fn take(&self) -> Option<CadPopup> {
        self.slot.borrow_mut().take()
    }
}

impl PopupSlot {
    /// Whether the modal input gate is set (ADR-0021 D2).
    pub fn modal_open(&self) -> bool {
        self.slot.borrow().is_some()
    }

    /// Clear the gate without an explicit close (reload / renderer crash / exit).
    pub fn reset(&self, reason: popup_state::PopupReset) {
        popup_state::on_popup_reset(self, reason);
    }
}

/// The [`PopupTarget`] of a carrier that owns its popup overlay in the same
/// document: the payloads are derived from the live CAD session, never supplied by
/// the caller.
pub struct LivePopups<'a> {
    pub live: &'a Live,
}

impl<'a> LivePopups<'a> {
    pub fn new(live: &'a Live) -> Self {
        Self { live }
    }
}

impl PopupTarget for LivePopups<'_> {
    fn open(&mut self, kind: &str) -> Result<PopupAck, String> {
        let ops = SessionPopupOps {
            session: &self.live.session,
        };
        popup_state::open_popup(&self.live.popups, kind, &ops)
    }

    fn close(&mut self) -> Result<(), String> {
        popup_state::close_popup(&self.live.popups)
    }
}

/// The platform half of the popup: the `textEdit` payload comes from the session's
/// own current selection (ADR-0021 D4). `about`/`options` need nothing.
struct SessionPopupOps<'a> {
    session: &'a CadSession,
}

impl PopupOps for SessionPopupOps<'_> {
    fn text_edit_payload(&self) -> Result<TextEditPayload, String> {
        let selected = self
            .session
            .selected_text_edit()
            .map_err(|reason| format!("popup.no_text_selection: {reason}"))?;
        TextEditPayload::from_selection(selected)
    }
}

// ---------------------------------------------------------------------------
// Input synthesis (the chrome's own events, until the front-end switch)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests;
