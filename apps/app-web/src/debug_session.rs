//! **SPIKE (ADR 0024 stage ⑧ phase 2): the real desktop session inside wasm.**
//!
//! This module is additive, debug-only and clearly marked: it is **not** the
//! production viewer path (`crate::session`), it changes nothing about the
//! `dom-canvas` viewer exports, the chrome, the budgets or the Worker isolation,
//! and the production flip (replacing `session` behind the same exports) is
//! **not** made here.
//!
//! # What it proves (or refutes)
//!
//! Phase 1 proved that `cad_ui_core::session::CadSession` — the *desktop* session,
//! with the real command router, the real `cad-db` authority and the real
//! `RenderSessionHandle` — is constructible inside a browser module from raw
//! bytes (`CadSession::from_bytes`, [`crate::platform`]). What it could not prove
//! was the other half: that such a session can **own a page `<canvas>` and
//! actually draw**. Between the session and the pixels sit three seams that are
//! desktop seams, not wasm seams:
//!
//! 1. the device, which on the desktop comes from the native surface presenter
//!    and here comes from [`CanvasPresenter::device_queue`] — the session's
//!    handle is attached with [`RenderSessionHandle::attach_device`];
//! 2. the surface metrics, which on the desktop come from the GTK window and
//!    here are the canvas' CSS size / `devicePixelRatio`
//!    ([`ReportedSurface`]);
//! 3. the frame, which on the desktop is blitted by the native presenter and
//!    here is `RenderSessionHandle::render` → `PresentedFrame.texture` →
//!    [`CanvasPresenter::present_texture`] (the existing blit, not a second one).
//!
//! # The debug surface (stable names, spike-only)
//!
//! ```text
//! debug_session_start(canvas_id, css_w, css_h, dpr)  -> JSON status (async)
//! debug_session_tick()                               -> JSON status
//! debug_session_command(text)                        -> JSON status
//! debug_session_pointer(phase, x, y)                 -> JSON status
//! debug_session_status()                             -> JSON status
//! ```
//!
//! `phase` is `down` / `move` / `up`. Every call returns the same status object
//! (see [`DebugSession::status_json`]) and every failure is an `Err(JsValue)` that
//! names its step — never a fabricated success. Nothing here is exported on a
//! non-wasm target (see the `NOT_IMPLEMENTED` stubs in [`crate`]).
//!
//! # Compared with the viewer session, honestly
//!
//! The viewer (`crate::session`) owns its **own** `Renderer` and uploads the
//! whole scene at open, so a frame is one `Renderer::render` away. This spike owns
//! nothing: the session owns the renderer, and the scene arrives through the
//! engine's staged pipeline. A frame therefore needs
//! `CadSession::tick` (host pass: metrics, view sync, scene preparation) **and**
//! `RenderSessionHandle::apply_ready_updates` (the upload the desktop host drives
//! from its present bridge) before `render` has anything committed to draw. Until
//! all of that has happened `render` returns `Ok(None)` — which this module counts
//! as `waited`, never as a presented frame.
//!
//! The numbers it reports (`ticks`, `frames`, `presented`, `waited`) are counts of
//! what happened in this one session, not performance claims.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_host::{HostOptions, RequestClock};
use cad_platform::HostCapabilities;
use cad_render_wgpu::canvas::CanvasPresenter;
use cad_ui_core::protocol::{InputKind, ProbeInput, BUTTON_PRIMARY};
use cad_ui_core::session::{BytesDocument, CadSession};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::platform::BrowserPlatform;

/// The compiled-in synthetic fixture (`fixtures/manifest`): four `LINE`s, no text
/// and no raster image, so the browser resource strategy is a real no-op
/// ([`crate::platform::BrowserResources`]).
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../fixtures/dwg/synthetic-four-lines.dwg");
/// The label the session reports as the open document (`status().label`).
const SYNTHETIC_LABEL: &str = "synthetic-four-lines.dwg";

/// Stable step names of this spike's failures. They are the only stable vocabulary
/// in the status `step` field, exactly like the viewer session's.
const STEP_STATE: &str = "debug-state";
const STEP_VIEWPORT: &str = "debug-viewport";
const STEP_CANVAS: &str = "debug-canvas-element";
const STEP_PRESENTER: &str = "debug-presenter";
const STEP_DEVICE: &str = "debug-device";
const STEP_OPEN: &str = "debug-open";
const STEP_METRICS: &str = "debug-metrics";
const STEP_TICK: &str = "debug-tick";
const STEP_UPLOAD: &str = "debug-upload";
const STEP_RENDER: &str = "debug-render";
const STEP_PRESENT: &str = "debug-present";
const STEP_COMMAND: &str = "debug-command";
const STEP_INPUT: &str = "debug-input";

/// One step failure: the stable step plus the message (technical, ASCII).
type StepError = (&'static str, String);

fn step_error(step: &'static str, message: impl Into<String>) -> StepError {
    (step, message.into())
}

// ---------------------------------------------------------------------------
// The registry: one debug session per wasm instance
// ---------------------------------------------------------------------------

/// The one debug session of this module (wasm is single-threaded).
enum DebugState {
    /// Nothing started yet.
    Idle,
    /// `debug_session_start` is awaiting the presenter. A concurrent start is
    /// refused here so one canvas never gets two presenters.
    Starting,
    /// A live debug session.
    Live(Box<DebugSession>),
}

thread_local! {
    static DEBUG: RefCell<DebugState> = const { RefCell::new(DebugState::Idle) };
    /// The monotonic epoch of the request clock: `web_time::Instant` is
    /// `Performance.now()` in the browser (no `std::time` on this target).
    static CLOCK_EPOCH: web_time::Instant = web_time::Instant::now();
    /// Sequence number of the pointer stream (`ProbeInput::sequence` must strictly
    /// increase across one session's inputs).
    static INPUT_SEQUENCE: Cell<u64> = const { Cell::new(0) };
}

/// The session's request clock: monotonic milliseconds since this module's first
/// use, measured with `performance.now()` (via `web_time`), never `std::time`.
fn browser_clock() -> RequestClock {
    Rc::new(|| CLOCK_EPOCH.with(|epoch| epoch.elapsed().as_secs_f64() * 1000.0))
}

/// Everything one debug session owns.
struct DebugSession {
    /// The **real** desktop session (`cad-host` core, `cad-db` authority, the
    /// command router) — the same type the native host drives.
    session: CadSession,
    /// The page canvas presenter: the device source and the blit target.
    presenter: CanvasPresenter,
    /// `(css_w, css_h, dpr)` the session was sized from.
    metrics: (f64, f64, f64),
    /// `CadSession::tick` calls that returned `Ok`.
    ticks: u64,
    /// `render` calls that produced a frame texture.
    frames: u64,
    /// Frames that reached the canvas (`present_texture` returned `Ok`).
    presented: u64,
    /// `render` calls that returned `Ok(None)`: the scene was not committed yet.
    waited: u64,
    /// The last failure, kept so `debug_session_status` keeps reporting it.
    last_error: Option<(String, String)>,
}

impl DebugSession {
    /// One frame attempt: the host pass, the staged upload, the render, the blit.
    ///
    /// `Ok(())` means the call completed — **not** that a frame was presented: a
    /// `render` that returns `Ok(None)` is a legitimate wait (the scene is still
    /// being prepared/uploaded) and is counted as `waited`.
    fn advance(&mut self) -> Result<(), StepError> {
        self.session
            .tick()
            .map_err(|error| step_error(STEP_TICK, error))?;
        self.ticks += 1;
        let render = self.session.render_session();
        // The desktop host drives this from its present bridge
        // (`apps/app-linux/src/gtk_bridge/present.rs`); it is what publishes a
        // prepared scene revision to the GPU.
        render
            .apply_ready_updates()
            .map_err(|error| step_error(STEP_UPLOAD, error.to_string()))?;
        let logical = [self.metrics.0, self.metrics.1];
        let frame = render
            .render(logical, self.metrics.2)
            .map_err(|error| step_error(STEP_RENDER, error.to_string()))?;
        let Some(frame) = frame else {
            self.waited += 1;
            return Ok(());
        };
        self.frames += 1;
        self.presenter
            .present_texture(&frame.texture)
            .map_err(|error| step_error(STEP_PRESENT, error.to_string()))?;
        self.presented += 1;
        // The session's own frame accounting: the status then reports the frames
        // that actually reached the canvas, not a requested count.
        self.session.note_frame(self.presented);
        Ok(())
    }

    /// Run `action`, keeping a failure on the session so the next status reports it.
    fn record<T>(
        &mut self,
        action: impl FnOnce(&mut Self) -> Result<T, StepError>,
    ) -> Result<T, JsValue> {
        match action(self) {
            Ok(value) => {
                self.last_error = None;
                Ok(value)
            }
            Err((step, message)) => {
                self.last_error = Some((step.to_string(), message.clone()));
                Err(reject(step, &message))
            }
        }
    }

    /// The status object every debug export answers with.
    fn status_json(&self) -> String {
        // The session's own snapshot: the authoritative label, entity count, tool
        // and error, read from the live `cad-db` document (this session keeps it —
        // unlike the viewer, which releases the database after the upload).
        let status = self.session.snapshot(None, None, self.presented);
        let (width, height) = self.presenter.surface_size();
        let adapter = self.presenter.adapter_info();
        let mut object = serde_json::json!({
            "ok": self.last_error.is_none() && status.error.is_none(),
            "spike": true,
            "label": status.document_label,
            "entityCount": status.entity_count,
            "tool": status.tool.as_ref().map(|tool| tool.key.clone()),
            "canFit": status.can_fit,
            "sessionError": status.error,
            "css": [self.metrics.0, self.metrics.1],
            "dpr": self.metrics.2,
            "surface": [width, height],
            "adapter": adapter.name,
            "target": adapter.target.as_str(),
            "ticks": self.ticks,
            "frames": self.frames,
            "presented": self.presented,
            "waited": self.waited,
            // The presenter's own counter: it can only move when a blit ran, so it
            // cross-checks `presented` instead of trusting this module's logic.
            "presenterFrames": self.presenter.presented_frames(),
            // The module's own wasm linear memory (a high-water mark, never
            // process RSS): what the *real* session's retained database costs.
            "heapBytes": wasm_heap_bytes(),
        });
        if let Some((step, message)) = &self.last_error {
            object["step"] = serde_json::json!(step);
            object["error"] = serde_json::json!(message);
        }
        object.to_string()
    }
}

/// The `Err` value of one failure, also echoed on the console.
fn reject(step: &str, message: &str) -> JsValue {
    let text = format!("{step}: {message}");
    web_sys::console::error_1(&JsValue::from_str(&text));
    JsValue::from_str(&text)
}

/// The module's own **wasm linear memory** in bytes, or `None` if the host hides it.
///
/// Deliberately a private copy of the viewer session's reading rather than a shared
/// helper: the spike must not gain a reason to touch the production module. It is
/// the wasm heap (a high-water mark), **not** process RSS — a browser tab has no
/// process this module can see.
fn wasm_heap_bytes() -> Option<f64> {
    let memory: js_sys::WebAssembly::Memory = wasm_bindgen::memory().dyn_into().ok()?;
    let buffer: js_sys::ArrayBuffer = js_sys::WebAssembly::Memory::buffer(&memory)
        .dyn_into()
        .ok()?;
    Some(f64::from(js_sys::ArrayBuffer::byte_length(&buffer)))
}

/// Resolve `canvas_id` to the page's `HtmlCanvasElement`.
fn canvas_element(canvas_id: &str) -> Result<web_sys::HtmlCanvasElement, StepError> {
    let window = web_sys::window().ok_or_else(|| step_error(STEP_CANVAS, "no window object"))?;
    let document = window
        .document()
        .ok_or_else(|| step_error(STEP_CANVAS, "no document object"))?;
    let element = document
        .get_element_by_id(canvas_id)
        .ok_or_else(|| step_error(STEP_CANVAS, format!("no element with id {canvas_id:?}")))?;
    element
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| step_error(STEP_CANVAS, format!("element #{canvas_id} is not a canvas")))
}

/// Validate `(css_w, css_h, dpr)` and return the physical pixel size.
fn physical_size(css_w: f64, css_h: f64, dpr: f64) -> Result<(u32, u32), StepError> {
    if !(css_w.is_finite() && css_h.is_finite() && dpr.is_finite()) {
        return Err(step_error(
            STEP_VIEWPORT,
            format!("viewport must be finite: css={css_w}x{css_h}, dpr={dpr}"),
        ));
    }
    if css_w <= 0.0 || css_h <= 0.0 || dpr <= 0.0 {
        return Err(step_error(
            STEP_VIEWPORT,
            format!("css size and dpr must be positive: {css_w}x{css_h} at {dpr}"),
        ));
    }
    let width = (css_w * dpr).round();
    let height = (css_h * dpr).round();
    if width < 1.0 || height < 1.0 || width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return Err(step_error(
            STEP_VIEWPORT,
            format!("physical size out of range: {width}x{height}"),
        ));
    }
    Ok((width as u32, height as u32))
}

// ---------------------------------------------------------------------------
// Debug entry points (spike)
// ---------------------------------------------------------------------------

pub(crate) async fn start_impl(
    canvas_id: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
) -> Result<String, JsValue> {
    DEBUG.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match &*state {
            DebugState::Idle => {
                *state = DebugState::Starting;
                Ok(())
            }
            _ => Err(reject(
                STEP_STATE,
                "a debug session is already starting or live; one canvas has exactly one presenter",
            )),
        },
        Err(_) => Err(reject(STEP_STATE, "the debug session state is borrowed")),
    })?;

    let result = start_session(canvas_id, css_w, css_h, dpr).await;
    match result {
        Ok(session) => {
            let status = session.status_json();
            DEBUG.with(|cell| {
                if let Ok(mut state) = cell.try_borrow_mut() {
                    *state = DebugState::Live(Box::new(session));
                }
            });
            Ok(status)
        }
        Err(error) => {
            // The failed start leaves no session behind: `Idle` is the truthful
            // state, and the error carries the step (never a fake success).
            DEBUG.with(|cell| {
                if let Ok(mut state) = cell.try_borrow_mut() {
                    *state = DebugState::Idle;
                }
            });
            Err(error)
        }
    }
}

/// Everything a start does after the state slot is claimed (no state borrow is
/// held across the presenter await).
async fn start_session(
    canvas_id: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
) -> Result<DebugSession, JsValue> {
    let physical =
        physical_size(css_w, css_h, dpr).map_err(|(step, message)| reject(step, &message))?;
    let canvas = canvas_element(canvas_id).map_err(|(step, message)| reject(step, &message))?;
    // The real WebGL2 capability check (instance → surface → adapter → device →
    // configure), not a feature sniff.
    let presenter = CanvasPresenter::create(canvas, physical)
        .await
        .map_err(|error| reject(STEP_PRESENTER, &error.to_string()))?;

    // The browser's four platform contracts (phase 1). The surface is reported
    // **before** the session is built: the open funnel validates canvas metrics
    // against it, and an unmapped surface is an explicit refusal.
    let platform = BrowserPlatform::new();
    platform.surface().report([css_w, css_h], dpr);

    // The render session the desktop host would get from its surface bridge; here
    // the device comes from the page canvas presenter.
    let render = RenderSessionHandle::new(BackendPreference::WebGl2);
    let (device, queue) = presenter.device_queue();
    render
        .attach_device(device, queue)
        .map_err(|error| reject(STEP_DEVICE, &error))?;

    let options = HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        // The document is the bytes: a path here would be a second source.
        drawing: None,
        size: [css_w, css_h],
    };
    // A browser hosts no session-side file chooser (the page's picker is a DOM
    // capability) and no import worker: the same advertised capabilities the
    // Android/`web-adapter` carrier reports.
    let capabilities = HostCapabilities::default();
    let session = CadSession::from_bytes(
        options,
        platform.host_platform(capabilities),
        render,
        browser_clock(),
        BytesDocument {
            label: SYNTHETIC_LABEL,
            bytes: SYNTHETIC_DWG,
        },
    )
    .map_err(|error| reject(STEP_OPEN, &error.to_string()))?;
    session
        .set_canvas_metrics([css_w, css_h], dpr)
        .map_err(|error| reject(STEP_METRICS, &error))?;

    let mut debug = DebugSession {
        session,
        presenter,
        metrics: (css_w, css_h, dpr),
        ticks: 0,
        frames: 0,
        presented: 0,
        waited: 0,
        last_error: None,
    };
    // One frame attempt so a start reports something real: presenting may still
    // be waiting for the scene, which the status says with `waited`. A failed
    // attempt keeps the session live (the failure is reported, and the next
    // `debug_session_tick` may succeed), which is what the status shows.
    if let Err(error) = debug.record(DebugSession::advance) {
        DEBUG.with(|cell| {
            if let Ok(mut state) = cell.try_borrow_mut() {
                *state = DebugState::Live(Box::new(debug));
            }
        });
        return Err(error);
    }
    Ok(debug)
}

/// Advance the live session by one frame attempt and answer its status.
pub(crate) fn tick_impl() -> Result<String, JsValue> {
    with_live(|session| {
        session.record(DebugSession::advance)?;
        Ok(session.status_json())
    })
}

/// Run one CAD command through the **real** command router (`CadSession::command`).
///
/// The name is the same selector the desktop command line uses (`LINE`, `CONFIRM`,
/// `CANCEL`, `FIT`, …); an unsupported name is refused by the router by name.
pub(crate) fn command_impl(text: &str) -> Result<String, JsValue> {
    with_live(|session| {
        session.record(|inner| {
            inner
                .session
                .command(text)
                .map_err(|error| step_error(STEP_COMMAND, error))
        })?;
        Ok(session.status_json())
    })
}

/// Feed one pointer event through the **real** input seam (`CadSession::handle_input`).
///
/// `phase` is `down` / `move` / `up`; `x`/`y` are canvas-local **logical** (CSS)
/// pixels, exactly what the desktop `probe_input` IPC carries. A press/release in
/// the same place is a click, which inside a draw capture is a point capture — the
/// same classification the native hosts get.
pub(crate) fn pointer_impl(phase: &str, x: f64, y: f64) -> Result<String, JsValue> {
    let kind = match phase {
        "down" => InputKind::PointerDown,
        "move" => InputKind::PointerMove,
        "up" => InputKind::PointerUp,
        other => {
            return Err(reject(
                STEP_INPUT,
                &format!("unsupported phase {other:?} (expected down, move or up)"),
            ))
        }
    };
    if !(x.is_finite() && y.is_finite()) {
        return Err(reject(
            STEP_INPUT,
            &format!("pointer position must be finite: ({x}, {y})"),
        ));
    }
    let sequence = INPUT_SEQUENCE.with(|cell| {
        let next = cell.get().saturating_add(1);
        cell.set(next);
        next
    });
    let input = ProbeInput {
        sequence,
        kind,
        x: Some(x as f32),
        y: Some(y as f32),
        dx: None,
        dy: None,
        width: None,
        height: None,
        dpr: None,
        // The DOM `MouseEvent.button` of a primary press/release; a move reports
        // no button change, so it omits it (the wire allows that).
        button: match kind {
            InputKind::PointerDown | InputKind::PointerUp => Some(BUTTON_PRIMARY),
            _ => None,
        },
        shift: None,
    };
    with_live(|session| {
        session.record(|inner| {
            inner
                .session
                .handle_input(&input)
                .map_err(|error| step_error(STEP_INPUT, error))
        })?;
        Ok(session.status_json())
    })
}

/// The live debug session's status, without advancing it.
pub(crate) fn status_impl() -> String {
    DEBUG.with(|cell| match cell.try_borrow() {
        Ok(state) => match &*state {
            DebugState::Live(session) => session.status_json(),
            DebugState::Idle => no_session_status("no debug session; call debug_session_start"),
            DebugState::Starting => {
                no_session_status("the debug session is starting; await debug_session_start")
            }
        },
        Err(_) => no_session_status("the debug session state is borrowed (a frame is in flight)"),
    })
}

/// The status object of "there is no live debug session": explicit nulls.
fn no_session_status(message: &str) -> String {
    serde_json::json!({
        "ok": false,
        "spike": true,
        "step": STEP_STATE,
        "error": message,
        "label": null,
        "entityCount": null,
        "tool": null,
        "ticks": 0,
        "frames": 0,
        "presented": 0,
        "waited": 0,
    })
    .to_string()
}

/// Run `action` on the live debug session.
fn with_live(
    action: impl FnOnce(&mut DebugSession) -> Result<String, JsValue>,
) -> Result<String, JsValue> {
    DEBUG.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match &mut *state {
            DebugState::Live(session) => action(session),
            DebugState::Idle => Err(reject(
                STEP_STATE,
                "no debug session; call debug_session_start first",
            )),
            DebugState::Starting => Err(reject(
                STEP_STATE,
                "the debug session is starting; await debug_session_start first",
            )),
        },
        Err(_) => Err(reject(STEP_STATE, "the debug session state is borrowed")),
    })
}
