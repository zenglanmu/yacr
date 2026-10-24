//! The wasm-side CAD session behind the exported canvas API.
//!
//! One process-global session (`wasm` is single-threaded, so a `thread_local`
//! `RefCell` is enough; no `Mutex`). It owns exactly one `CanvasPresenter`, so
//! "one presenter per canvas" holds by construction: `start` refuses to build a
//! second session while one is live, and `open_bytes` **replaces** the session
//! (the previous presenter is dropped before the replacement is created).
//!
//! The pipeline mirrors the headless render contract:
//! DWG bytes (compiled-in synthetic fixture, or caller-supplied via `open_bytes`)
//! → `AcadrustImporter` → `cad_app::render_scene::build_scene`
//! → `Renderer::prepare_upload_batches` in bounded chunks + one `commit_upload`
//! → `Renderer::render`
//! → `CanvasPresenter::present_texture(Renderer::frame_texture())`.
//!
//! Opening a file is two staged halves on purpose:
//!
//! 1. [`import::load`] — geometry only: import the DWG bytes, build the display
//!    representation, check it is not empty, fit the camera, then release the
//!    imported database. No GPU object is touched, so a bad, unsupported, empty
//!    or over-budget file is reported while the live session (canvas included)
//!    stays exactly as it was. Both memory budgets ([`crate::budget`]) are applied
//!    on this side of the boundary: input bytes before parsing, wasm heap after
//!    the build.
//! 2. [`upload::attach`] — GPU only: initialize the renderer on the (new)
//!    presenter's device, upload the resident batches in bounded chunks (with one
//!    atomic commit, so the first presented frame is always complete), and build
//!    the [`Session`].
//!
//! Pan/zoom only touch `cad_app::Camera`: the scene batches stay resident (no
//! re-parse, no re-upload), and each navigation step renders and presents
//! immediately so the canvas is never a frame behind the camera.
//!
//! Module layout: [`session_state`] holds the registry, the failure vocabulary and
//! the frame path; [`import`] the geometry half; [`upload`] the chunked GPU half;
//! [`surface`] the two carriers a session can be opened on.
//!
//! # One session implementation, two carriers
//!
//! The pipeline above is identical for both carriers, and so is the code: only
//! `Surface` changes (see [`surface`]).
//!
//! - **main thread** — `start`/`open_bytes` resolve a page `<canvas id=...>` and
//!   create the presenter from an `HtmlCanvasElement`.
//! - **worker** — `start_offscreen`/`open_bytes_offscreen` take the
//!   `OffscreenCanvas` the page transferred with `transferControlToOffscreen()` and
//!   create the presenter from it. The Worker owns this session outright; the page
//!   creates none (that is why the page shell does not instantiate the module on
//!   that path), and the offscreen entry points are refused outside a Worker
//!   (`step: "isolation"`) so two owners can never coexist.
//!
//! Whether the worker carrier is possible is a **browser** fact (see
//! [`crate::capability`]): Chromium 141 gives WebGL2 on a worker-side
//! `OffscreenCanvas`, **WebKitGTK 2.52.6 returns `null` from
//! `getContext("webgl2")` there and therefore cannot do worker rendering at all**.
//! On that engine the offscreen carrier fails with the `webgl2` refusal (marker
//! `yacr: unsupported-runtime: webgl2`) — the main-thread carrier is the honest
//! answer, and the host says so in its status line.

pub(crate) mod import;
pub(crate) mod session_state;
pub(crate) mod surface;
pub(crate) mod upload;

use wasm_bindgen::prelude::*;

use crate::budget;
use import::{load, Loaded};
use session_state::{
    begin_replace, begin_start, document_failure, ensure_not_starting, fail, install_live,
    last_import_ms, record_import, wasm_heap_bytes, with_live, Failure, Metrics, Session, State,
    NO_MEASUREMENT, STATE, STEP_INPUT, STEP_NAVIGATION, STEP_PRESENT, STEP_STATE, STEP_VIEWPORT,
    STEP_WEBGL2, WEBGL2_MARKER,
};
use surface::Surface;

/// A committed, writer-produced AC1032 DWG with four LINE entities
/// (`fixtures/manifest`). Not a vendor sample; wasm has no filesystem, so the
/// bytes are compiled in. `start` keeps loading it so an existing host boot path
/// (browser shell, desktop transport probe) is unaffected by `open_bytes`.
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../fixtures/dwg/synthetic-four-lines.dwg");

/// The stable label of [`SYNTHETIC_DWG`]: `status()`/`document()` report it as
/// the open file name, exactly like a caller-supplied file name.
const SYNTHETIC_LABEL: &str = "synthetic-four-lines.dwg";

// ---------------------------------------------------------------------------
// Exported entry points
// ---------------------------------------------------------------------------

pub(crate) async fn start_impl(
    canvas_id: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
) -> Result<String, JsValue> {
    start_with(Surface::Page(canvas_id.to_string()), css_w, css_h, dpr).await
}

/// Worker entry: the same session on an `OffscreenCanvas` transferred into this
/// Worker.
///
/// Identical pipeline to [`start_impl`] — the only difference is the carrier
/// ([`Surface::Offscreen`]), so there is one session implementation with two
/// carriers rather than a second copy of the import/build/upload path. Refused with
/// `step: "isolation"` when called outside a Worker.
pub(crate) async fn start_offscreen_impl(
    canvas: web_sys::OffscreenCanvas,
    css_w: f64,
    css_h: f64,
    dpr: f64,
) -> Result<String, JsValue> {
    start_with(Surface::Offscreen(canvas), css_w, css_h, dpr).await
}

async fn start_with(surface: Surface, css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    // Claim the start slot first: a concurrent `start` must not build a second
    // presenter for the same canvas.
    begin_start()?;
    let metrics = Metrics::new(css_w, css_h, dpr)
        .map_err(|message| fail(Failure::new(STEP_VIEWPORT, message)))?;
    let physical = metrics
        .physical()
        .map_err(|message| fail(Failure::new(STEP_VIEWPORT, message)))?;
    // Resolve the carrier before the geometry load: a missing element, a wrong
    // element type or an `OffscreenCanvas` outside a Worker is a cheap, explicit
    // refusal that must not cost a parse.
    let carrier = surface.resolve().map_err(fail)?;
    let loaded = load_within_budget(
        SYNTHETIC_DWG,
        SYNTHETIC_LABEL,
        [metrics.css.0, metrics.css.1],
    )
    .map_err(fail)?;
    // This is the real WebGL2 capability check (instance → surface → adapter →
    // device → configure), not a feature sniff: a host that fails here must emit
    // the `webgl2` marker instead of showing a blank canvas (ADR 0024 D5). On
    // WebKitGTK 2.52.6 this is also where a worker-side `OffscreenCanvas` fails,
    // because `getContext("webgl2")` returns `null` there.
    let presenter = carrier.presenter(physical).await.map_err(fail)?;
    // Everything below is synchronous, so no `RefCell` borrow and no DOM borrow
    // is held across an await point.
    let mut session = upload::attach(presenter, metrics, SYNTHETIC_LABEL, loaded).map_err(fail)?;
    session.render_and_present().map_err(fail)?;
    session.heap_after_open = wasm_heap_bytes();
    record_import(session.import_ms);
    let status = live_status(&session);
    install_live(session);
    web_sys::console::log_1(&JsValue::from_str(&status));
    Ok(status)
}

pub(crate) async fn open_bytes_impl(
    canvas_id: &str,
    label: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
    bytes: &[u8],
) -> Result<String, JsValue> {
    open_bytes_with(
        Surface::Page(canvas_id.to_string()),
        label,
        css_w,
        css_h,
        dpr,
        bytes,
    )
    .await
}

/// Worker entry for [`open_bytes_impl`]: caller-supplied DWG bytes on the
/// transferred `OffscreenCanvas`.
///
/// Same staged halves, same budgets, same chunked upload — the carrier is the only
/// difference.
pub(crate) async fn open_bytes_offscreen_impl(
    canvas: web_sys::OffscreenCanvas,
    label: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
    bytes: &[u8],
) -> Result<String, JsValue> {
    open_bytes_with(Surface::Offscreen(canvas), label, css_w, css_h, dpr, bytes).await
}

async fn open_bytes_with(
    surface: Surface,
    label: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
    bytes: &[u8],
) -> Result<String, JsValue> {
    // Refuse while another start/open is in flight, before doing any work.
    ensure_not_starting()?;
    let label = label.trim();
    if label.is_empty() {
        return Err(document_failure(Failure::new(
            STEP_INPUT,
            "open_bytes needs a non-empty label (the source file name)",
        )));
    }
    let metrics = Metrics::new(css_w, css_h, dpr)
        .map_err(|message| document_failure(Failure::new(STEP_VIEWPORT, message)))?;
    let physical = metrics
        .physical()
        .map_err(|message| document_failure(Failure::new(STEP_VIEWPORT, message)))?;
    // The carrier is resolved (not created) before any state change: the previous
    // presenter is still live at this point.
    let carrier = surface.resolve().map_err(document_failure)?;
    // Stage 1, before any state change: a bad, unsupported, empty or over-budget
    // file leaves a live session — and its canvas pixels — exactly as they were,
    // and the open document stays navigable.
    let loaded = load_within_budget(bytes, label, [metrics.css.0, metrics.css.1])
        .map_err(document_failure)?;
    // Stage 2: replace. The previous renderer and canvas presenter are dropped
    // here, before the replacement presenter is created, so two presenters are
    // never live at once and no surface is leaked. From this point a failure can
    // only be a surface/device/upload failure, and it is fatal and named.
    begin_replace()?;
    let presenter = carrier.presenter(physical).await.map_err(fail)?;
    let mut session = upload::attach(presenter, metrics, label, loaded).map_err(fail)?;
    session.render_and_present().map_err(fail)?;
    session.heap_after_open = wasm_heap_bytes();
    record_import(session.import_ms);
    let status = live_status(&session);
    install_live(session);
    web_sys::console::log_1(&JsValue::from_str(&status));
    Ok(status)
}

pub(crate) fn resize_impl(css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    with_live(|session| {
        let metrics =
            Metrics::new(css_w, css_h, dpr).map_err(|m| Failure::new(STEP_VIEWPORT, m))?;
        let physical = metrics
            .physical()
            .map_err(|m| Failure::new(STEP_VIEWPORT, m))?;
        session
            .presenter
            .resize(physical)
            .map_err(|error| Failure::new(STEP_PRESENT, error.to_string()))?;
        session.metrics = metrics;
        // A resize that only reconfigures the surface would leave the old frame
        // stretched; render at the new size instead.
        session.render_and_present()?;
        Ok(live_status(session))
    })
}

pub(crate) fn zoom_at_impl(factor: f64, x: f64, y: f64) -> Result<String, JsValue> {
    with_live(|session| {
        if !(factor.is_finite() && x.is_finite() && y.is_finite()) {
            return Err(Failure::new(
                STEP_NAVIGATION,
                format!("zoom arguments must be finite: factor={factor}, cursor=({x}, {y})"),
            ));
        }
        let size = [session.metrics.css.0, session.metrics.css.1];
        session
            .camera
            .zoom_at(factor, [x, y], size)
            .map_err(|error| Failure::new(STEP_NAVIGATION, error.to_string()))?;
        session.render_and_present()?;
        Ok(live_status(session))
    })
}

pub(crate) fn pan_impl(dx: f64, dy: f64) -> Result<String, JsValue> {
    with_live(|session| {
        if !(dx.is_finite() && dy.is_finite()) {
            return Err(Failure::new(
                STEP_NAVIGATION,
                format!("pan delta must be finite: ({dx}, {dy})"),
            ));
        }
        // A zero delta is a no-op: it must not present a redundant frame.
        if dx != 0.0 || dy != 0.0 {
            session.pan_css([dx, dy])?;
            session.render_and_present()?;
        }
        Ok(live_status(session))
    })
}

pub(crate) fn status_impl() -> String {
    STATE.with(|cell| match cell.try_borrow() {
        Ok(state) => match &*state {
            State::Idle => failure_status(&Failure::new(
                STEP_STATE,
                "the canvas session has not started; call start() first",
            )),
            State::Starting => failure_status(&Failure::new(
                STEP_STATE,
                "the canvas session is starting or being replaced; await that call first",
            )),
            State::Failed(failure) => failure_status(failure),
            State::Live(session) => live_status(session),
        },
        Err(_) => failure_status(&Failure::new(
            STEP_STATE,
            "the session state is already borrowed",
        )),
    })
}

/// What the open file is, as far as this session knows it.
///
/// `{ok, label, batches, draws, presented, bytes}` plus the measured `import_ms`,
/// `wasm_heap_bytes`, `heap_after_open`, `upload_chunks`,
/// `peak_batches_per_chunk` and the `db_retained` retention flag. With no live
/// session the object is `ok: false` with null/zero values — never a made-up file.
pub(crate) fn document_impl() -> String {
    STATE.with(|cell| {
        let document = match cell.try_borrow() {
            Ok(state) => match &*state {
                State::Live(session) => serde_json::json!({
                    "ok": session.last_error.is_none(),
                    "label": session.label,
                    "batches": session.batches,
                    "draws": session.draws,
                    "presented": session.presenter.presented_frames(),
                    "bytes": session.bytes,
                    "import_ms": session.import_ms,
                    "wasm_heap_bytes": wasm_heap_bytes(),
                    "heap_after_open": session.heap_after_open,
                    "upload_chunks": session.upload_chunks,
                    "peak_batches_per_chunk": session.peak_batches_per_chunk,
                    "db_retained": session.db_retained,
                }),
                _ => empty_document(),
            },
            Err(_) => empty_document(),
        };
        document.to_string()
    })
}

/// The import+build+upload duration of the last completed open, in milliseconds,
/// measured with `web_time::Instant`.
///
/// Returns [`NO_MEASUREMENT`] (a negative number) until one open has completed —
/// there is no default, estimated or cached-elsewhere number. This is a
/// measurement surface for the evidence run, **not** a performance claim.
pub(crate) fn last_import_ms_impl() -> f64 {
    last_import_ms()
}

// ---------------------------------------------------------------------------
// Budgets around the geometry load
// ---------------------------------------------------------------------------

/// Geometry load guarded by both named memory budgets.
///
/// 1. [`budget::check_input_bytes`] **before** parsing: cheap refusal of an absurd
///    input, so the importer never copies or parses it.
/// 2. [`budget::check_heap_after_build`] **after** the scene build and **before**
///    any GPU object: the imported database has been released and the display
///    representation is resident, so this is the footprint the upload would grow
///    from. If it is over budget, the freshly built scene is dropped here (so the
///    refusal does not keep the footprint it refused) and the live session — if
///    any — is left untouched.
///
/// A refusal always carries the budget name, its limit and the measured value, both
/// in the message and as a structured `budget` object in the status.
fn load_within_budget(bytes: &[u8], label: &str, logical: [f64; 2]) -> Result<Loaded, Failure> {
    budget::check_input_bytes(label, bytes.len()).map_err(Failure::from_refusal)?;
    let loaded = load(bytes, label, logical)?;
    if let Err(refusal) = budget::check_heap_after_build(label, wasm_heap_bytes()) {
        drop(loaded);
        return Err(Failure::from_refusal(refusal));
    }
    Ok(loaded)
}

// ---------------------------------------------------------------------------
// Status / document JSON
// ---------------------------------------------------------------------------

fn live_status(session: &Session) -> String {
    let info = session.presenter.adapter_info();
    let (width, height) = session.presenter.surface_size();
    let mut status = serde_json::json!({
        "ok": session.last_error.is_none(),
        "backend": info.backend,
        "adapter_name": info.name,
        "format": format!("{:?}", info.format),
        // Which carrier draws this frame, and therefore where the session runs.
        // `target` is the presenter's own diagnostic tag and `isolation` is derived
        // from it (an `OffscreenCanvas` presenter can only exist inside a Worker —
        // `surface::Surface::resolve` refuses the page case), so a host cannot be
        // told `isolation: worker` by a main-thread session.
        "target": info.target.as_str(),
        "isolation": if info.target.is_worker() {
            crate::capability::ISOLATION_WORKER
        } else {
            crate::capability::ISOLATION_MAIN_THREAD
        },
        "physical": [width, height],
        "css": [session.metrics.css.0, session.metrics.css.1],
        "dpr": session.metrics.dpr,
        "label": session.label,
        "bytes": session.bytes,
        "batches": session.batches,
        "draws": session.draws,
        "presented": session.presenter.presented_frames(),
        "configure_count": session.presenter.configure_count(),
        "world_per_px": session.world_per_px(),
        "import_ms": session.import_ms,
        "upload_chunks": session.upload_chunks,
        "peak_batches_per_chunk": session.peak_batches_per_chunk,
        "heap_after_open": session.heap_after_open,
        "db_retained": session.db_retained,
    });
    if let Some(failure) = &session.last_error {
        apply_failure(&mut status, failure);
    }
    status.to_string()
}

fn failure_status(failure: &Failure) -> String {
    let mut status = serde_json::json!({
        "ok": false,
        "backend": null,
        "adapter_name": null,
        "format": null,
        // No session, so no carrier: explicit nulls, never a guessed isolation.
        "target": null,
        "isolation": null,
        "physical": null,
        "label": null,
        "bytes": 0,
        "batches": 0,
        "draws": 0,
        "presented": 0,
        "upload_chunks": 0,
        "peak_batches_per_chunk": 0,
        "heap_after_open": null,
        "db_retained": false,
    });
    apply_failure(&mut status, failure);
    status.to_string()
}

/// Add the stable failure vocabulary to a status object.
///
/// `step` + `error` are always present; a `webgl2` failure carries the
/// `yacr: unsupported-runtime: webgl2` marker, and a `budget` failure carries the
/// structured `{name, limit_bytes, measured_bytes}` of the budget that tripped —
/// so a refusal is never a blank canvas and never a bare prose string.
fn apply_failure(status: &mut serde_json::Value, failure: &Failure) {
    status["step"] = serde_json::json!(failure.step);
    status["error"] = serde_json::json!(failure.message);
    if failure.step == STEP_WEBGL2 {
        status["marker"] = serde_json::json!(WEBGL2_MARKER);
    }
    if let Some(refusal) = &failure.budget {
        status["budget"] = serde_json::json!({
            "name": refusal.name,
            "limit_bytes": refusal.limit_bytes,
            "measured_bytes": refusal.measured_bytes,
        });
    }
}

/// The `document()` object with no live session: explicit nulls, no invented
/// file, and the wasm heap (which exists whether or not a document does).
fn empty_document() -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "label": null,
        "batches": 0,
        "draws": 0,
        "presented": 0,
        "bytes": 0,
        "import_ms": NO_MEASUREMENT,
        "wasm_heap_bytes": wasm_heap_bytes(),
        "heap_after_open": null,
        "upload_chunks": 0,
        "peak_batches_per_chunk": 0,
        "db_retained": false,
    })
}
