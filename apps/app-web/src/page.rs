//! The **page carrier** of the production `dom-canvas` session (wasm only).
//!
//! One process-global session slot (wasm is single-threaded, so a `thread_local`
//! `RefCell` is the registry), one `CanvasPresenter`, and the exports that drive
//! them:
//!
//! ```text
//! start(canvas_id, css_w, css_h, dpr)                    -> JSON status (async)
//! open_bytes(canvas_id, label, css_w, css_h, dpr, bytes) -> JSON status (async)
//! frame()                                                -> JSON frame line
//! resize(css_w, css_h, dpr)                              -> JSON status
//! pan(dx, dy) / zoom_at(factor, x, y)                    -> JSON status
//! fit_view()                                             -> JSON status
//! status() / document()                                  -> JSON objects
//! probe_input(args_json)                                 -> JSON pointer ack
//! invoke(name, args_json)                                -> {"ok":...} envelope
//! shell_status() / shell_commands() / shell_aliases() / shell_command(name)
//! ```
//!
//! # What this module adds to [`crate::live`]
//!
//! [`crate::live`] owns the session state and every rule; this module owns the two
//! things that need a browser or a GPU:
//!
//! 1. **The device and the surface.** `start`/`open_bytes` create the WebGL2
//!    presenter from the page's `<canvas>` (the real capability check: instance →
//!    surface → adapter → device → configure, never a feature sniff) and attach its
//!    device/queue to the session's `RenderSessionHandle`.
//! 2. **The frame.** [`present_once`] is the desktop host's present bridge in three
//!    calls: `CadSession::tick` (host pass, metrics, view sync, scene preparation)
//!    → `RenderSessionHandle::apply_ready_updates` (publish a prepared scene
//!    revision to the GPU) → `render` → `CanvasPresenter::present_texture` (the
//!    existing blit, not a second one). A `render` that reports no frame is a real
//!    wait and is counted as one; it is never a presented frame.
//!
//! # One session, one canvas — enforced here
//!
//! [`State`] has the same shape the viewer session uses: `Idle` → `Starting` →
//! `Live`, with `Failed` for a start that never produced a session. [`begin_start`]
//! claims the slot before the (async) presenter exists, so two concurrent `start`
//! calls cannot build two presenters for one canvas, and `open_bytes` **replaces the
//! document in the live session** instead of building a second one.
//!
//! # The frame loop is the host's
//!
//! Nothing here starts a timer: `frame()` is one explicit step, meant to be called
//! from the page's `requestAnimationFrame` loop (that is the whole loop). The
//! navigation exports (`resize`/`pan`/`zoom_at`/`fit_view`) each present one frame
//! after their step, so the current chrome — which predates `frame()` — keeps
//! repainting until the front-end switch lands.
//!
//! # This module is not the worker carrier
//!
//! The isolated carrier (`start_offscreen`/`open_bytes_offscreen`) still runs the
//! **viewer** session ([`crate::session`]) in the Worker, untouched by this flip:
//! it keeps its own status shape, its own `open` and its own camera-only
//! navigation, and it has **no** `invoke`/`probe_input`/`shell_*` surface (those
//! refuse with the explicit "no live CAD session" message there). Moving the worker
//! onto this session is the remaining half of ADR 0024 stage ⑧ and is recorded as
//! not done in `docs/handoff.md`.

use std::cell::RefCell;

use cad_engine::render::{BackendPreference, RenderSessionHandle};
use cad_render_wgpu::canvas::CanvasPresenter;
use cad_ui_core::protocol::ProbeInput;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::direct::{self, DirectReads, DirectSink};
use crate::live::{
    self, Failure, Live, LivePopups, Metrics, PresenterFacts, STEP_CANVAS, STEP_DEVICE,
    STEP_RENDER, STEP_STATE, STEP_UPLOAD, STEP_VIEWPORT, STEP_WEBGL2, SYNTHETIC_DWG,
    SYNTHETIC_LABEL,
};
use crate::shell;

/// First-frame attempts a `start`/`open_bytes` makes before it reports `waited`.
///
/// The scene reaches the GPU through the engine's staged pipeline, so the first
/// `render` after an install can legitimately report no frame. A *bounded* number
/// of attempts inside the open call reproduces what every native host gets for free
/// (its frame loop is already running) without starting a timer inside wasm; if the
/// scene still is not ready, the export returns with `waited` counted and the
/// caller's `frame()` loop finishes the job.
const FIRST_FRAME_ATTEMPTS: u32 = 6;

/// The one page session of this wasm instance.
enum State {
    /// Nothing started yet.
    Idle,
    /// `start`/`open_bytes` is awaiting the presenter (the slot is claimed, so a
    /// concurrent start cannot create a second presenter).
    Starting,
    /// A start failed before a session existed: the exact failure, so `status()`
    /// reports the real reason instead of a healthy empty carrier.
    Failed(Box<Failure>),
    /// The live session and its canvas presenter.
    Live(Box<PageSession>),
}

/// The live session plus the canvas presenter that owns its device and its pixels.
struct PageSession {
    live: Live,
    presenter: CanvasPresenter,
}

/// What one [`present_once`] call did.
enum FrameStep {
    /// A frame was rendered and blitted onto the canvas.
    Presented,
    /// `render` reported no frame: the scene is not committed yet (a real wait).
    Waited,
}

thread_local! {
    /// The one page session (wasm is single-threaded).
    static STATE: RefCell<State> = const { RefCell::new(State::Idle) };
    /// The monotonic epoch of the request clock: `web_time::Instant` is
    /// `performance.now()` in the browser (`std::time` panics on this target).
    static CLOCK_EPOCH: web_time::Instant = web_time::Instant::now();
}

/// The session's request clock, in milliseconds since this module's first use.
fn browser_clock() -> cad_host::RequestClock {
    std::rc::Rc::new(|| CLOCK_EPOCH.with(|epoch| epoch.elapsed().as_secs_f64() * 1000.0))
}

/// The module's own **wasm linear memory** in bytes, or `None` if the host hides it.
///
/// The wasm heap (a high-water mark), **not** process RSS: a browser tab has no
/// process this module can see. Deliberately a small private reader rather than a
/// shared one: the viewer session owns the identical function for its own carrier,
/// and the two carriers must not grow a dependency on each other.
fn heap_gauge() -> Option<f64> {
    let memory: js_sys::WebAssembly::Memory = wasm_bindgen::memory().dyn_into().ok()?;
    let buffer: js_sys::ArrayBuffer = js_sys::WebAssembly::Memory::buffer(&memory)
        .dyn_into()
        .ok()?;
    Some(f64::from(js_sys::ArrayBuffer::byte_length(&buffer)))
}

/// The `Err` value of one failure: `step: message`, also echoed on the console so a
/// caller that swallows the rejection still sees why.
fn reject(failure: &Failure) -> JsValue {
    let text = failure.describe();
    web_sys::console::error_1(&JsValue::from_str(&text));
    JsValue::from_str(&text)
}

/// Resolve `canvas_id` to the page's `HtmlCanvasElement`.
fn canvas_element(canvas_id: &str) -> Result<web_sys::HtmlCanvasElement, Failure> {
    let window = web_sys::window()
        .ok_or_else(|| Failure::new(STEP_CANVAS, "the host page has no window object"))?;
    let document = window
        .document()
        .ok_or_else(|| Failure::new(STEP_CANVAS, "the host page has no document object"))?;
    let element = document
        .get_element_by_id(canvas_id)
        .ok_or_else(|| Failure::new(STEP_CANVAS, format!("no element with id {canvas_id:?}")))?;
    element
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| Failure::new(STEP_CANVAS, format!("element #{canvas_id} is not a canvas")))
}

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

/// Whether this carrier owns the session slot (`Starting` or `Live`).
///
/// The shared exports (`resize`/`pan`/`zoom_at`/`status`/…) ask this to decide
/// between the page session and the Worker's viewer session; the two can never both
/// be live (`Surface::resolve` refuses the worker carrier outside a Worker, and a
/// page canvas cannot be resolved inside one).
pub(crate) fn is_engaged() -> bool {
    STATE.with(|cell| {
        matches!(
            &*cell.borrow(),
            State::Starting | State::Live(_) | State::Failed(_)
        )
    })
}

/// Claim the single start slot; refuse a second start while one is engaged.
fn begin_start() -> Result<(), JsValue> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match &*state {
            State::Idle => {
                *state = State::Starting;
                Ok(())
            }
            State::Failed(failure) => {
                // A failed start is retried from scratch, not inherited.
                let _ = failure;
                *state = State::Starting;
                Ok(())
            }
            State::Starting | State::Live(_) => Err(reject(&Failure::new(
                STEP_STATE,
                "a canvas session is already starting or live; one canvas has exactly one presenter",
            ))),
        },
        Err(_) => Err(reject(&Failure::new(
            STEP_STATE,
            "the session state is already borrowed",
        ))),
    })
}

/// Publish the new session as the one live session.
fn install_live(page: PageSession) {
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            *state = State::Live(Box::new(page));
        }
    });
}

/// Record a failure that ended a start attempt, and turn it into the `Err` value.
fn fail_start(failure: Failure) -> JsValue {
    let error = reject(&failure);
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            *state = State::Failed(Box::new(failure));
        }
    });
    error
}

/// Run `action` on the live page session, or answer `None` when there is none.
fn with_live<T>(action: impl FnOnce(&mut PageSession) -> T) -> Option<T> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match &mut *state {
            State::Live(page) => Some(action(page)),
            _ => None,
        },
        Err(_) => None,
    })
}

/// The refusal of an export that needs a live session.
fn no_session(step: &'static str) -> Failure {
    STATE.with(|cell| match &*cell.borrow() {
        State::Starting => Failure::new(
            step,
            "the canvas session is starting or being replaced; await that call first",
        ),
        State::Failed(failure) => Failure::new(
            step,
            format!(
                "the canvas session failed to start ({}: {})",
                failure.step, failure.message
            ),
        ),
        _ => Failure::new(
            step,
            "the canvas session has not started; call start() first",
        ),
    })
}

/// The presenter facts of one page session.
fn facts(page: &PageSession) -> PresenterFacts {
    let info = page.presenter.adapter_info();
    PresenterFacts {
        backend: info.backend.to_string(),
        adapter: info.name,
        format: format!("{:?}", info.format),
        target: info.target.as_str(),
        isolation: if info.target.is_worker() {
            crate::capability::ISOLATION_WORKER
        } else {
            crate::capability::ISOLATION_MAIN_THREAD
        },
        physical: page.presenter.surface_size(),
        presented: page.presenter.presented_frames(),
        configure_count: page.presenter.configure_count(),
    }
}

// ---------------------------------------------------------------------------
// The frame step
// ---------------------------------------------------------------------------

/// One frame step: tick, publish the prepared scene, render, blit.
///
/// `Ok(FrameStep::Waited)` means the call completed and **nothing was presented**
/// (the scene is not committed yet). A failed step is recorded on the session so
/// `status()` keeps reporting it, and the session stays usable: a surface
/// `Outdated`/`Lost` frame is retryable and must not destroy the open document.
fn present_once(page: &mut PageSession) -> Result<FrameStep, Failure> {
    page.live.tick()?;
    let render = page.live.session.render_session();
    // The desktop host drives this from its present bridge: it is what publishes a
    // prepared scene revision to the GPU.
    render
        .apply_ready_updates()
        .map_err(|error| Failure::new(STEP_UPLOAD, error.to_string()))?;
    let metrics = page.live.metrics;
    let frame = render
        .render(metrics.css, metrics.dpr)
        .map_err(|error| Failure::new(STEP_RENDER, error.to_string()))?;
    let Some(frame) = frame else {
        page.live.waited += 1;
        return Ok(FrameStep::Waited);
    };
    page.presenter
        .present_texture(&frame.texture)
        .map_err(|error| Failure::new(live::STEP_PRESENT, error.to_string()))?;
    page.live.frames += 1;
    // The session's own frame accounting: the status reports the frames that
    // actually reached the canvas, never a requested count.
    page.live
        .session
        .note_frame(page.presenter.presented_frames());
    Ok(FrameStep::Presented)
}

/// Present one frame, keeping a retryable failure on the session.
///
/// Returns whether this call actually put a frame on the canvas.
fn frame_step(page: &mut PageSession) -> bool {
    match present_once(page) {
        Ok(FrameStep::Presented) => {
            page.live.clear_failure();
            true
        }
        Ok(FrameStep::Waited) => false,
        Err(failure) => {
            page.live.note_failure(failure);
            false
        }
    }
}

/// The first-frame attempts an open makes (see [`FIRST_FRAME_ATTEMPTS`]).
fn settle_first_frame(page: &mut PageSession) {
    for _ in 0..FIRST_FRAME_ATTEMPTS {
        match present_once(page) {
            Ok(FrameStep::Presented) => {
                page.live.clear_failure();
                return;
            }
            Ok(FrameStep::Waited) => continue,
            Err(failure) => {
                // Keep the session: the caller's frame loop may succeed on the next
                // attempt, and the status carries the reason either way.
                page.live.note_failure(failure);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

pub(crate) async fn start_impl(
    canvas_id: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
) -> Result<String, JsValue> {
    begin_start()?;
    match start_session(canvas_id, css_w, css_h, dpr, SYNTHETIC_LABEL, SYNTHETIC_DWG).await {
        Ok(page) => {
            let status = page_with_status(&page);
            install_live(page);
            Ok(status)
        }
        Err(failure) => Err(fail_start(failure)),
    }
}

pub(crate) async fn open_bytes_impl(
    canvas_id: &str,
    label: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
    bytes: &[u8],
) -> Result<String, JsValue> {
    // A live session replaces its document in place (the desktop open); there is no
    // second session and no second presenter.
    let live_exists = STATE.with(|cell| matches!(&*cell.borrow(), State::Live(_)));
    if live_exists {
        let metrics = Metrics::new(css_w, css_h, dpr)
            .map_err(|message| document_failure(Failure::new(STEP_VIEWPORT, message)))?;
        return with_live(|page| {
            let outcome = replace_document(page, label, bytes, metrics);
            match outcome {
                Ok(()) => Ok(page_with_status(page)),
                Err(failure) => {
                    page.live.note_failure(failure.clone());
                    Err(reject(&failure))
                }
            }
        })
        .unwrap_or_else(|| Err(reject(&no_session(live::STEP_IMPORT))));
    }
    // No session yet: the first open builds one (the page may open a file before it
    // starts the fixture).
    begin_start()?;
    match start_session(canvas_id, css_w, css_h, dpr, label, bytes).await {
        Ok(page) => {
            let status = page_with_status(&page);
            install_live(page);
            Ok(status)
        }
        Err(failure) => Err(fail_start(failure)),
    }
}

/// Replace the document of a live session, re-sizing the presenter if asked.
fn replace_document(
    page: &mut PageSession,
    label: &str,
    bytes: &[u8],
    metrics: Metrics,
) -> Result<(), Failure> {
    let physical = metrics
        .physical()
        .map_err(|message| Failure::new(STEP_VIEWPORT, message))?;
    page.presenter
        .resize(physical)
        .map_err(|error| Failure::new(live::STEP_PRESENT, error.to_string()))?;
    page.live.set_metrics(metrics)?;
    page.live.replace_document(label, bytes, &heap_gauge)?;
    settle_first_frame(page);
    Ok(())
}

/// Everything an open does after the slot is claimed (no state borrow across the
/// presenter await).
async fn start_session(
    canvas_id: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
    label: &str,
    bytes: &[u8],
) -> Result<PageSession, Failure> {
    let metrics =
        Metrics::new(css_w, css_h, dpr).map_err(|message| Failure::new(STEP_VIEWPORT, message))?;
    let physical = metrics
        .physical()
        .map_err(|message| Failure::new(STEP_VIEWPORT, message))?;
    // The real WebGL2 capability check (instance → surface → adapter → device →
    // configure), not a feature sniff: a host that fails here must emit the `webgl2`
    // marker instead of showing a blank canvas (ADR 0024 D5).
    let canvas = canvas_element(canvas_id)?;
    let presenter = CanvasPresenter::create(canvas, physical)
        .await
        .map_err(|error| Failure::new(STEP_WEBGL2, error.to_string()))?;

    // The render session the desktop host would get from its surface bridge; here
    // the device comes from the page canvas presenter.
    let render = RenderSessionHandle::new(BackendPreference::WebGl2);
    let (device, queue) = presenter.device_queue();
    render
        .attach_device(device, queue)
        .map_err(|error| Failure::new(STEP_DEVICE, error))?;

    let live = Live::install(render, browser_clock(), label, bytes, metrics, &heap_gauge)?;
    let mut page = PageSession { live, presenter };
    settle_first_frame(&mut page);
    Ok(page)
}

/// The status object of a page session (canvas facts + the session's own).
fn page_with_status(page: &PageSession) -> String {
    page.live.canvas_status_json(&facts(page), heap_gauge())
}

/// An `open_bytes` failure that must not destroy a live session.
fn document_failure(failure: Failure) -> JsValue {
    reject(&failure)
}

// ---------------------------------------------------------------------------
// Navigation and frames
// ---------------------------------------------------------------------------

pub(crate) fn frame_impl() -> Result<String, JsValue> {
    with_live(|page| {
        let presented = frame_step(page);
        let present = facts(page);
        Ok(serde_json::json!({
            "ok": page.live.last_error.is_none(),
            "presented": presented,
            "frames": page.live.frames,
            "waited": page.live.waited,
            "ticks": page.live.ticks,
            "canvasFrames": present.presented,
        })
        .to_string())
    })
    .unwrap_or_else(|| Err(reject(&no_session(STEP_STATE))))
}

pub(crate) fn resize_impl(css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    with_live(|page| {
        let metrics = Metrics::new(css_w, css_h, dpr)
            .map_err(|message| reject(&Failure::new(STEP_VIEWPORT, message)))?;
        let physical = metrics
            .physical()
            .map_err(|message| reject(&Failure::new(STEP_VIEWPORT, message)))?;
        page.presenter
            .resize(physical)
            .map_err(|error| reject(&Failure::new(live::STEP_PRESENT, error.to_string())))?;
        page.live
            .set_metrics(metrics)
            .map_err(|failure| reject(&failure))?;
        frame_step(page);
        Ok(page_with_status(page))
    })
    .unwrap_or_else(|| Err(reject(&no_session(STEP_VIEWPORT))))
}

pub(crate) fn pan_impl(dx: f64, dy: f64) -> Result<String, JsValue> {
    with_live(|page| {
        page.live
            .pan_by([dx, dy])
            .map_err(|failure| reject(&failure))?;
        frame_step(page);
        Ok(page_with_status(page))
    })
    .unwrap_or_else(|| Err(reject(&no_session(live::STEP_NAVIGATION))))
}

pub(crate) fn zoom_at_impl(factor: f64, x: f64, y: f64) -> Result<String, JsValue> {
    with_live(|page| {
        page.live
            .zoom_about(factor, [x, y])
            .map_err(|failure| reject(&failure))?;
        frame_step(page);
        Ok(page_with_status(page))
    })
    .unwrap_or_else(|| Err(reject(&no_session(live::STEP_NAVIGATION))))
}

pub(crate) fn fit_view_impl() -> Result<String, JsValue> {
    with_live(|page| {
        page.live
            .command("FIT")
            .map_err(|failure| reject(&failure))?;
        frame_step(page);
        Ok(page_with_status(page))
    })
    .unwrap_or_else(|| Err(reject(&no_session(live::STEP_NAVIGATION))))
}

// ---------------------------------------------------------------------------
// Status, document, input, wire
// ---------------------------------------------------------------------------

pub(crate) fn status_impl() -> String {
    with_live(|page| page_with_status(page)).unwrap_or_else(|| {
        STATE.with(|cell| match &*cell.borrow() {
            State::Starting => live::failure_status_json(
                &Failure::new(
                    STEP_STATE,
                    "the canvas session is starting or being replaced; await that call first",
                ),
                heap_gauge(),
            ),
            State::Failed(failure) => live::failure_status_json(failure, heap_gauge()),
            _ => live::failure_status_json(&no_session(STEP_STATE), heap_gauge()),
        })
    })
}

pub(crate) fn document_impl() -> String {
    with_live(|page| page.live.document_json(&facts(page), heap_gauge()))
        .unwrap_or_else(|| empty_document())
}

/// The `document()` object with no live session: explicit nulls, no invented file.
fn empty_document() -> String {
    serde_json::json!({
        "ok": false,
        "label": null,
        "bytes": 0,
        "entityCount": null,
        "batches": null,
        "draws": null,
        "presented": 0,
        "frames": 0,
        "importCount": 0,
        "import_ms": -1.0,
        "wasm_heap_bytes": heap_gauge(),
        "heap_after_open": null,
        "db_retained": live::DB_RETAINED,
    })
    .to_string()
}

pub(crate) fn last_import_ms_impl() -> f64 {
    with_live(|page| page.live.import_ms).unwrap_or(-1.0)
}

/// Feed one wire pointer input, exactly as the desktop `probe_input` IPC does.
///
/// The argument object is the **same** `ProbeInput` the desktop command
/// deserializes (kind/sequence/x/y/button/shift), and the effect is the session's
/// own `handle_input`. A refusal is an answer (`accepted: false` plus a stable
/// reason), never a silent drop; an unparseable argument object is a transport
/// error and rejects.
pub(crate) fn probe_input_impl(args_json: &str) -> Result<String, JsValue> {
    let text = if args_json.trim().is_empty() {
        "{}"
    } else {
        args_json
    };
    let input: ProbeInput = serde_json::from_str(text)
        .map_err(|error| JsValue::from_str(&format!("invalid-input: {error}")))?;
    let sequence = input.sequence;
    let kind = input.kind.as_str();
    with_live(|page| match page.live.probe(&input) {
        Ok(()) => Ok(serde_json::json!({
            "ok": true,
            "accepted": true,
            "sequence": sequence,
            "kind": kind,
        })
        .to_string()),
        Err(failure) => Ok(serde_json::json!({
            "ok": false,
            "accepted": false,
            "sequence": sequence,
            "kind": kind,
            "step": failure.step,
            "reason": failure.message,
        })
        .to_string()),
    })
    .unwrap_or_else(|| Err(reject(&no_session(live::STEP_INPUT))))
}

/// The one `dom-canvas` wire entry: serve a Tauri command name in-process.
pub(crate) fn invoke_impl(name: &str, args_json: &str) -> String {
    let served = with_live(|page| {
        // A command ends any synthetic pan gesture: the two must never own the
        // single pointer press at the same time.
        page.live.close_pan_inputs();
        let modal = page.live.popups.modal_open();
        let mut sink = DirectSink::with_gates(&page.live.session, true, modal);
        let mut popups = LivePopups::new(&page.live);
        direct::dispatch(
            name,
            args_json,
            &mut sink,
            &DirectReads::new(&page.live.session),
            &mut popups,
        )
    });
    match served {
        Some(result) => direct::envelope(result),
        None => direct::envelope(direct::without_session(name)),
    }
}

// ---------------------------------------------------------------------------
// The shell surface
// ---------------------------------------------------------------------------

pub(crate) fn shell_status_impl() -> String {
    with_live(|page| page.live.shell_status_json(&facts(page))).unwrap_or_else(|| {
        // A failed start is reported instead of an empty status that would look
        // healthy; with no session at all the honest empty status is the truth.
        let failure = STATE.with(|cell| match &*cell.borrow() {
            State::Failed(failure) => Some(failure.describe()),
            _ => None,
        });
        let status = failure.map_or_else(shell::empty_status, |error| {
            cad_ui_core::session::unsupported_status(Some(error))
        });
        shell::status_json(&status, &shell::Carrier::default())
    })
}

pub(crate) fn shell_commands_impl() -> String {
    shell::commands_json()
}

pub(crate) fn shell_aliases_impl() -> String {
    shell::aliases_json()
}

/// Execute one line through the **shared** wire layer, or refuse it.
///
/// A gate refusal keeps the stable [`shell::SHELL_COMMAND_REFUSAL`] prefix; a
/// refusal the session itself made is returned verbatim (it is the session's own
/// diagnostic, not a transport problem).
pub(crate) fn shell_command_impl(command: &str) -> Result<String, String> {
    let served = with_live(|page| {
        page.live.close_pan_inputs();
        let modal = page.live.popups.modal_open();
        let mut sink = DirectSink::with_gates(&page.live.session, true, modal);
        let outcome = cad_ui_core::session::wire::command(&mut sink, command);
        match outcome {
            Ok(_) => Ok(shell::accepted_json()),
            Err(error) => match sink.take_refusal() {
                // The session refused the applied request: its words, unchanged.
                Some(refusal) => Err(refusal),
                // The shared gate refused it before the sink: keep the machine
                // prefix a caller keys on, then the gate's own sentence.
                None => Err(shell::refusal(&error)),
            },
        }
    });
    match served {
        Some(result) => result,
        None => Err(shell::refusal(
            "no live CAD session on this carrier (start the canvas session first)",
        )),
    }
}
