//! The process-global session state: the one live `Session`, its failure
//! vocabulary and the plumbing that installs/disposes it.
//!
//! `wasm` is single-threaded, so one `thread_local` `RefCell` is the whole
//! registry — no `Mutex`, no atomics, and `const` init keeps the first access
//! allocation-free. The invariants this module owns:
//!
//! * **one presenter**: `start` refuses to build a second session while one is
//!   live, and a replacement drops the previous presenter *before* the new one is
//!   created ([`begin_replace`] is the single disposal point);
//! * **one document**: `open_bytes` never mutates the live session until its
//!   geometry has loaded successfully, so a bad/unsupported/over-budget file
//!   leaves the open document (and its canvas pixels) intact.
//!
//! The registry is per **wasm instance**, and there is exactly one instance in
//! play: the page's when the main-thread carrier is used, the Worker's when the
//! isolated carrier is used (the page then instantiates no module at all, and
//! `start_offscreen` is refused outside a Worker — see [`super::surface`]). So
//! "one presenter per canvas" also means "one CAD document per canvas", which is
//! the invariant ADR 0024 D2 requires.

use std::cell::{Cell, RefCell};

use cad_domain::Point3;
use cad_render_wgpu::canvas::CanvasPresenter;
use cad_render_wgpu::{Camera2d, RenderTarget, Renderer};
use wasm_bindgen::prelude::*;

use crate::budget::BudgetRefusal;
use crate::view::{fit_camera, DrawingBounds};

/// Stable names of the steps a failure can be attributed to. They are the only
/// stable vocabulary in the status `step` field, so a host can key on them
/// (browser console messages, Linux exit codes) without parsing prose.
pub(super) const STEP_VIEWPORT: &str = "viewport";
/// A carrier requested in a context where it cannot be the single owner of the
/// session (an `OffscreenCanvas` outside a Worker). See [`super::surface`].
pub(super) const STEP_ISOLATION: &str = "isolation";
pub(super) const STEP_WEBGL2: &str = "webgl2";
pub(super) const STEP_IMPORT: &str = "import";
/// The file parsed but produced nothing drawable. Never a blank-canvas success.
pub(super) const STEP_EMPTY_SCENE: &str = "empty-scene";
pub(super) const STEP_SCENE: &str = "scene";
pub(super) const STEP_DEVICE: &str = "device";
pub(super) const STEP_UPLOAD: &str = "upload";
pub(super) const STEP_CAMERA: &str = "camera";
pub(super) const STEP_RENDER: &str = "render";
pub(super) const STEP_PRESENT: &str = "present";
pub(super) const STEP_NAVIGATION: &str = "navigation";
/// A rejected argument of `open_bytes` (an empty label, for example).
pub(super) const STEP_INPUT: &str = "input";
/// A named memory budget refused the file (see [`crate::budget`]). Carries a
/// structured `budget: {name, limit_bytes, measured_bytes}` object as well as the
/// message, so a refusal is always attributable and never a blank canvas.
pub(super) const STEP_BUDGET: &str = "budget";
pub(super) const STEP_STATE: &str = "state";

/// The stable refusal marker for a `dom-canvas` host without usable WebGL2
/// (ADR 0024 D5). Only the `webgl2` step carries it; the host emits this line
/// verbatim and exits with code 6 instead of showing a blank canvas.
pub(super) const WEBGL2_MARKER: &str = "yacr: unsupported-runtime: webgl2";

/// The document identity of the single session (one file, one database).
pub(super) const SESSION_DOCUMENT: u128 = 1;
pub(super) const SESSION_DATABASE: u128 = 1;

/// The value the `last_import_ms()` surface returns until one open has completed.
///
/// A negative number is not a measurement: it says "nothing has been measured
/// yet", so a host can never mistake it for a suspiciously fast import.
pub(super) const NO_MEASUREMENT: f64 = -1.0;

/// One named failure: the stable step, the human message, and — for a budget
/// refusal — the structured numbers behind it.
///
/// Keeping the structured facts next to the message is what lets `status()` report
/// `budget: {name, limit_bytes, measured_bytes}` without re-parsing prose, while
/// the message still names all three in words for a human reading the console.
#[derive(Debug, Clone)]
pub(super) struct Failure {
    pub(super) step: &'static str,
    pub(super) message: String,
    pub(super) budget: Option<BudgetRefusal>,
}

impl Failure {
    pub(super) fn new(step: &'static str, message: impl Into<String>) -> Self {
        Self {
            step,
            message: message.into(),
            budget: None,
        }
    }

    /// A [`STEP_BUDGET`] failure from a named budget refusal (see [`crate::budget`]).
    pub(super) fn from_refusal(refusal: BudgetRefusal) -> Self {
        Self {
            step: STEP_BUDGET,
            message: refusal.message.clone(),
            budget: Some(refusal),
        }
    }
}

/// The process-global session state.
pub(super) enum State {
    /// Nothing has started yet.
    Idle,
    /// `start`/`open_bytes` is in flight (it awaits the canvas presenter). A
    /// concurrent `start` or `open_bytes` is refused here, so one canvas never
    /// gets two presenters.
    Starting,
    /// A start failed before a session existed; the payload is the exact failure
    /// the attempt produced.
    Failed(Box<Failure>),
    /// A live session on one canvas.
    Live(Box<Session>),
}

thread_local! {
    // `wasm` is single-threaded, so one `RefCell` slot is the whole session
    // registry. `const` init keeps the first access allocation-free.
    pub(super) static STATE: RefCell<State> = const { RefCell::new(State::Idle) };
    /// The last completed import+build+upload duration (`web_time::Instant`), in
    /// milliseconds. Evidence surface only; see `last_import_ms_impl`.
    static LAST_IMPORT_MS: Cell<f64> = const { Cell::new(NO_MEASUREMENT) };
}

/// The one CAD session: presenter + renderer + camera.
pub(super) struct Session {
    pub(super) presenter: CanvasPresenter,
    pub(super) renderer: Renderer,
    /// The application camera. The render camera is derived from it through
    /// `camera2d_params()` on every frame, so the two cannot drift.
    pub(super) camera: cad_app::Camera,
    /// The drawing extents the current camera fit was computed from, retained
    /// through the open so `FIT` can refit without the (released) database.
    ///
    /// `None` when the imported database reported no bounds: the camera then
    /// keeps the default 2D top view and `FIT` has nothing to frame, which the
    /// shell status reports as `canFit: false` (never a claimed refit).
    pub(super) bounds: Option<DrawingBounds>,
    pub(super) metrics: Metrics,
    /// The source file name this session was opened from (`status`/`document`).
    pub(super) label: String,
    /// The source DWG length in bytes (evidence: what was actually opened).
    pub(super) bytes: usize,
    /// Resident draw batches after the initial upload (evidence for the status).
    pub(super) batches: usize,
    /// Draw submissions of the last presented frame.
    pub(super) draws: u64,
    /// `prepare_upload_batches` calls the scene upload used. Measured, not a
    /// performance claim: it proves the upload was streamed, not staged at once.
    pub(super) upload_chunks: usize,
    /// Largest number of batches one `prepare_upload_batches` call staged. With
    /// the whole-scene staging this used to equal `batches`; it is now bounded by
    /// `budget::UPLOAD_CHUNK_MAX_BATCHES`.
    pub(super) peak_batches_per_chunk: usize,
    /// Wasm linear-memory length measured after this open completed (scene
    /// uploaded, first frame presented). A high-water mark, not a live reading:
    /// wasm memory never shrinks, so this is the number an improvement must move.
    pub(super) heap_after_open: Option<f64>,
    /// Whether the imported database was kept for later capabilities. Always
    /// `crate::budget::DB_RETAINED`, reported so retention is never silent.
    pub(super) db_retained: bool,
    /// The last navigation/frame failure, kept so `status()` keeps reporting it
    /// while the session stays usable (a surface `Outdated`/`Lost` frame is
    /// retryable and must not destroy the session).
    pub(super) last_error: Option<Failure>,
    /// Import + display-representation build + GPU upload of this file, measured
    /// with `web_time::Instant`. A measurement of this open, not a performance
    /// claim.
    pub(super) import_ms: f64,
}

/// Canvas sizing in CSS pixels plus the device pixel ratio.
#[derive(Debug, Clone, Copy)]
pub(super) struct Metrics {
    pub(super) css: (f64, f64),
    pub(super) dpr: f64,
}

impl Metrics {
    pub(super) fn new(css_w: f64, css_h: f64, dpr: f64) -> Result<Self, String> {
        if !(css_w.is_finite() && css_h.is_finite() && dpr.is_finite()) {
            return Err(format!(
                "viewport must be finite: css={css_w}x{css_h}, dpr={dpr}"
            ));
        }
        if css_w <= 0.0 || css_h <= 0.0 {
            return Err(format!("canvas CSS size must be positive: {css_w}x{css_h}"));
        }
        if dpr <= 0.0 {
            return Err(format!("devicePixelRatio must be positive: {dpr}"));
        }
        Ok(Self {
            css: (css_w, css_h),
            dpr,
        })
    }

    /// The physical (device-pixel) size the presenter configures.
    pub(super) fn physical(self) -> Result<(u32, u32), String> {
        let width = (self.css.0 * self.dpr).round();
        let height = (self.css.1 * self.dpr).round();
        if width < 1.0 || height < 1.0 {
            return Err(format!(
                "physical size rounds to zero: {}x{} (css {}x{} at dpr {})",
                width, height, self.css.0, self.css.1, self.dpr
            ));
        }
        if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
            return Err(format!("physical size out of range: {width}x{height}"));
        }
        Ok((width as u32, height as u32))
    }
}

// ---------------------------------------------------------------------------
// Frame path and navigation
// ---------------------------------------------------------------------------

impl Session {
    /// The world-per-pixel scale the renderer will use this frame.
    pub(super) fn world_per_px(&self) -> Option<f64> {
        self.camera.camera2d_params().ok().map(|p| p.world_per_px)
    }

    /// Pan by a CSS-pixel screen delta.
    ///
    /// Mirrors the desktop path: the pointer delta becomes the world delta
    /// `[dx × wpp, -dy × wpp]` (screen y grows down, world y grows up) and the
    /// camera target+eye shift by its negation, so the model follows the pointer.
    pub(super) fn pan_css(&mut self, delta: [f64; 2]) -> Result<(), Failure> {
        let world_per_px = self
            .camera
            .camera2d_params()
            .map_err(|error| Failure::new(STEP_CAMERA, error.to_string()))?
            .world_per_px;
        let shift = Point3 {
            x: -delta[0] * world_per_px,
            y: delta[1] * world_per_px,
            z: 0.0,
        };
        if !(shift.x.is_finite() && shift.y.is_finite()) {
            return Err(Failure::new(
                STEP_NAVIGATION,
                "pan produced a non-finite world shift",
            ));
        }
        self.camera.target = Point3 {
            x: self.camera.target.x + shift.x,
            y: self.camera.target.y + shift.y,
            z: self.camera.target.z,
        };
        self.camera.eye = Point3 {
            x: self.camera.eye.x + shift.x,
            y: self.camera.eye.y + shift.y,
            z: self.camera.eye.z,
        };
        Ok(())
    }

    /// Refit the camera to the retained drawing extents at the session's
    /// **current** logical size, then present the frame.
    ///
    /// This is the `FIT` selector. It calls the very same [`fit_camera`] the open
    /// used — same centre, same axis-ratio/margin rule, same projection
    /// constructor — with the bounds retained by [`Self::bounds`], so a refit
    /// cannot disagree with the initial framing. No second fit rule, no synthetic
    /// camera, and camera-only: nothing is re-parsed or re-uploaded.
    ///
    /// With no retained bounds there is nothing to frame and the failure is
    /// explicit instead of silently pretending the view was reframed.
    pub(super) fn fit_view(&mut self) -> Result<(), Failure> {
        let bounds = self.bounds.ok_or_else(|| {
            Failure::new(
                STEP_NAVIGATION,
                "FIT needs the drawing extents, and this session retained none",
            )
        })?;
        self.camera = fit_camera(Some(bounds), [self.metrics.css.0, self.metrics.css.1]);
        self.render_and_present()
    }

    /// Render the resident scene with the current camera and present it.
    ///
    /// Camera-only: nothing is parsed, rebuilt or re-uploaded here.
    pub(super) fn render_and_present(&mut self) -> Result<(), Failure> {
        let params = self
            .camera
            .camera2d_params()
            .map_err(|error| Failure::new(STEP_CAMERA, error.to_string()))?;
        let camera = Camera2d {
            center: params.center,
            world_per_px: params.world_per_px,
            z_plane: 0.0,
        };
        let (width, height) = self.presenter.surface_size();
        let target = RenderTarget::new(width, height);
        let stats = self
            .renderer
            .render(camera, &target)
            .map_err(|error| Failure::new(STEP_RENDER, error.to_string()))?;
        self.draws = stats.draw_calls;
        let texture = self.renderer.frame_texture().ok_or_else(|| {
            Failure::new(
                STEP_RENDER,
                "the renderer reported a frame but holds no frame texture",
            )
        })?;
        self.presenter
            .present_texture(texture)
            .map_err(|error| Failure::new(STEP_PRESENT, error.to_string()))?;
        self.last_error = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The wasm heap, as this module can observe it
// ---------------------------------------------------------------------------

/// The size of the module's **wasm linear memory**, in bytes, or `None` when the
/// host does not expose it.
///
/// This is the module's own heap (`WebAssembly.Memory`), **not** process RSS:
/// there is no process to measure in a browser tab, and the page owns memory this
/// module cannot see. Record it as "wasm heap", never as resident memory.
///
/// It is a **high-water mark**: wasm memory grows and is never returned to the
/// host, so a freed buffer does not lower this number. What a release can do is
/// keep the peak — and therefore this reading — from growing.
pub(super) fn wasm_heap_bytes() -> Option<f64> {
    let memory: js_sys::WebAssembly::Memory = wasm_bindgen::memory().dyn_into().ok()?;
    let buffer: js_sys::ArrayBuffer = js_sys::WebAssembly::Memory::buffer(&memory)
        .dyn_into()
        .ok()?;
    Some(f64::from(js_sys::ArrayBuffer::byte_length(&buffer)))
}

// ---------------------------------------------------------------------------
// Plumbing: install, dispose, record
// ---------------------------------------------------------------------------

/// Record a fatal startup failure and turn it into the `Err` value the host sees.
pub(super) fn fail(failure: Failure) -> JsValue {
    let text = failure.message.clone();
    let step = failure.step;
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            *state = State::Failed(Box::new(failure));
        }
    });
    rejection(step, &text)
}

/// Report an `open_bytes` failure that must not destroy a live session.
///
/// With a live session the failure is recorded on it — `status()`/`document()`
/// then report the still-open file plus the failing step, and the canvas keeps
/// its pixels and stays navigable — and nothing is disposed. Without one, the
/// failure is an ordinary startup failure.
pub(super) fn document_failure(failure: Failure) -> JsValue {
    let step = failure.step;
    let text = failure.message.clone();
    let recorded = STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match &mut *state {
            State::Live(session) => {
                session.last_error = Some(failure);
                true
            }
            _ => false,
        },
        Err(_) => false,
    });
    if recorded {
        return rejection(step, &text);
    }
    fail(Failure::new(step, text))
}

/// The `Err` value for one failure: a message that names the step, echoed on the
/// browser console so a caller that swallows the rejection still sees why.
pub(super) fn rejection(step: &str, message: &str) -> JsValue {
    let text = format!("{step}: {message}");
    web_sys::console::error_1(&JsValue::from_str(&text));
    JsValue::from_str(&text)
}

/// Claim the single start slot for `start`.
///
/// `Idle` and `Failed` may start (a failed start is retried from scratch); an
/// in-flight or live session refuses the second call, so one canvas has exactly
/// one presenter even if the host calls `start` twice before the first await
/// resolves.
pub(super) fn begin_start() -> Result<(), JsValue> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => {
            if matches!(&*state, State::Idle | State::Failed(_)) {
                *state = State::Starting;
                return Ok(());
            }
            Err(rejection(
                STEP_STATE,
                "a canvas session is already starting or live; one canvas has exactly one presenter",
            ))
        }
        Err(_) => Err(rejection(STEP_STATE, "the session state is already borrowed")),
    })
}

/// Refuse `open_bytes` while another start/open is still in flight, without
/// changing any state. Checked before the (expensive) geometry load.
pub(super) fn ensure_not_starting() -> Result<(), JsValue> {
    STATE.with(|cell| match cell.try_borrow() {
        Ok(state) => match &*state {
            State::Starting => Err(rejection(
                STEP_STATE,
                "a canvas session is already starting or being replaced; await that call first",
            )),
            _ => Ok(()),
        },
        Err(_) => Err(rejection(
            STEP_STATE,
            "the session state is already borrowed",
        )),
    })
}

/// Move to `Starting` and dispose the previous session.
///
/// This is the one disposal point for a replacement: the old `Session` (renderer
/// *and* canvas presenter, i.e. its surface) is dropped here, before the new
/// presenter exists. So there is never a second presenter and never an
/// unreleased surface.
pub(super) fn begin_replace() -> Result<(), JsValue> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match std::mem::replace(&mut *state, State::Starting) {
            State::Live(previous) => {
                drop(previous);
                Ok(())
            }
            // `Idle`/`Failed` have nothing to dispose; the replacement is the
            // first session.
            State::Idle | State::Failed(_) => Ok(()),
            // Cannot happen (the caller checked `ensure_not_starting`), and it
            // must not silently drop an in-flight start.
            State::Starting => Err(rejection(
                STEP_STATE,
                "a canvas session is already starting or being replaced; await that call first",
            )),
        },
        Err(_) => Err(rejection(
            STEP_STATE,
            "the session state is already borrowed",
        )),
    })
}

/// Publish the new session as the one live session.
///
/// Called only after the geometry loaded, the upload committed and the first frame
/// was presented, so the registry can never hold a refused, failed or partially
/// uploaded open.
pub(super) fn install_live(session: Session) {
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            *state = State::Live(Box::new(session));
        }
    });
}

/// Publish one completed import measurement (see `last_import_ms_impl`).
pub(super) fn record_import(import_ms: f64) {
    LAST_IMPORT_MS.with(|cell| cell.set(import_ms));
}

/// The last recorded import duration, or [`NO_MEASUREMENT`].
pub(super) fn last_import_ms() -> f64 {
    LAST_IMPORT_MS.with(Cell::get)
}

/// Run `action` on the live session, translating its failure into the exported
/// error shape while keeping the session alive.
pub(super) fn with_live<T>(
    action: impl FnOnce(&mut Session) -> Result<T, Failure>,
) -> Result<T, JsValue> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match &mut *state {
            State::Live(session) => match action(session) {
                Ok(value) => Ok(value),
                Err(failure) => {
                    // A failed frame or navigation step is retryable: keep the
                    // session and let `status()` keep reporting the failure.
                    let step = failure.step;
                    let message = failure.message.clone();
                    session.last_error = Some(failure);
                    Err(rejection(step, &message))
                }
            },
            State::Idle => Err(rejection(
                STEP_STATE,
                "the canvas session has not started; call start() first",
            )),
            State::Starting => Err(rejection(
                STEP_STATE,
                "the canvas session is starting or being replaced; await that call first",
            )),
            State::Failed(_) => Err(rejection(
                STEP_STATE,
                "the canvas session failed to start; see status()",
            )),
        },
        Err(_) => Err(rejection(
            STEP_STATE,
            "the session state is already borrowed",
        )),
    })
}
