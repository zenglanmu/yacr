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
//! → `Renderer::prepare_upload_batches` + `commit_upload` → `Renderer::render`
//! → `CanvasPresenter::present_texture(Renderer::frame_texture())`.
//!
//! Opening a file is two staged halves on purpose:
//!
//! 1. [`load`] — geometry only: import the DWG bytes, build the display
//!    representation, check it is not empty, fit the camera. No GPU object is
//!    touched, so a bad or unsupported file is reported while the live session
//!    (canvas included) stays exactly as it was.
//! 2. [`attach`] — GPU only: initialize the renderer on the (new) presenter's
//!    device, upload the resident batches and build the [`Session`].
//!
//! Pan/zoom only touch `cad_app::Camera`: the scene batches stay resident (no
//! re-parse, no re-upload), and each navigation step renders and presents
//! immediately so the canvas is never a frame behind the camera.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use cad_app::render_scene::build_scene;
use cad_domain::{DatabaseId, DocumentId, Point3, TaskStamp};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};
use cad_render_wgpu::canvas::CanvasPresenter;
use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};
use cad_scene::SceneDelta;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// A committed, writer-produced AC1032 DWG with four LINE entities
/// (`fixtures/manifest`). Not a vendor sample; wasm has no filesystem, so the
/// bytes are compiled in. `start` keeps loading it so an existing host boot path
/// (browser shell, desktop transport probe) is unaffected by `open_bytes`.
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../fixtures/dwg/synthetic-four-lines.dwg");

/// The stable label of [`SYNTHETIC_DWG`]: `status()`/`document()` report it as
/// the open file name, exactly like a caller-supplied file name.
const SYNTHETIC_LABEL: &str = "synthetic-four-lines.dwg";

/// Stable names of the steps a failure can be attributed to. They are the only
/// stable vocabulary in the status `step` field, so a host can key on them
/// (browser console messages, Linux exit codes) without parsing prose.
const STEP_VIEWPORT: &str = "viewport";
const STEP_CANVAS: &str = "canvas-element";
const STEP_WEBGL2: &str = "webgl2";
const STEP_IMPORT: &str = "import";
/// The file parsed but produced nothing drawable. Never a blank-canvas success.
const STEP_EMPTY_SCENE: &str = "empty-scene";
const STEP_SCENE: &str = "scene";
const STEP_DEVICE: &str = "device";
const STEP_UPLOAD: &str = "upload";
const STEP_CAMERA: &str = "camera";
const STEP_RENDER: &str = "render";
const STEP_PRESENT: &str = "present";
const STEP_NAVIGATION: &str = "navigation";
/// A rejected argument of `open_bytes` (an empty label, for example).
const STEP_INPUT: &str = "input";
const STEP_STATE: &str = "state";

/// The stable refusal marker for a `dom-canvas` host without usable WebGL2
/// (ADR 0024 D5). Only the `webgl2` step carries it; the host emits this line
/// verbatim and exits with code 6 instead of showing a blank canvas.
const WEBGL2_MARKER: &str = "yacr: unsupported-runtime: webgl2";

/// The document identity of the single session (one file, one database).
const SESSION_DOCUMENT: u128 = 1;
const SESSION_DATABASE: u128 = 1;

/// The value [`last_import_ms_impl`] returns until one open has completed.
///
/// A negative number is not a measurement: it says "nothing has been measured
/// yet", so a host can never mistake it for a suspiciously fast import.
const NO_MEASUREMENT: f64 = -1.0;

/// The process-global session state.
enum State {
    /// Nothing has started yet.
    Idle,
    /// `start`/`open_bytes` is in flight (it awaits the canvas presenter). A
    /// concurrent `start` or `open_bytes` is refused here, so one canvas never
    /// gets two presenters.
    Starting,
    /// A start failed before a session existed; the payload is the exact status
    /// JSON the failure produced.
    Failed(String),
    /// A live session on one canvas.
    Live(Box<Session>),
}

thread_local! {
    // `wasm` is single-threaded, so one `RefCell` slot is the whole session
    // registry. `const` init keeps the first access allocation-free.
    static STATE: RefCell<State> = const { RefCell::new(State::Idle) };
    /// The last completed import+build+upload duration (`web_time::Instant`), in
    /// milliseconds. Evidence surface only; see [`last_import_ms_impl`].
    static LAST_IMPORT_MS: Cell<f64> = const { Cell::new(NO_MEASUREMENT) };
}

/// The one CAD session: presenter + renderer + camera.
struct Session {
    presenter: CanvasPresenter,
    renderer: Renderer,
    /// The application camera. The render camera is derived from it through
    /// `camera2d_params()` on every frame, so the two cannot drift.
    camera: cad_app::Camera,
    metrics: Metrics,
    /// The source file name this session was opened from (`status`/`document`).
    label: String,
    /// The source DWG length in bytes (evidence: what was actually opened).
    bytes: usize,
    /// Resident draw batches after the initial upload (evidence for the status).
    batches: usize,
    /// Draw submissions of the last presented frame.
    draws: u64,
    /// The last navigation/frame failure, kept so `status()` keeps reporting it
    /// while the session stays usable (a surface `Outdated`/`Lost` frame is
    /// retryable and must not destroy the session).
    last_error: Option<(&'static str, String)>,
    /// Import + display-representation build + GPU upload of this file, measured
    /// with `web_time::Instant`. A measurement of this open, not a performance
    /// claim.
    import_ms: f64,
}

/// Canvas sizing in CSS pixels plus the device pixel ratio.
#[derive(Debug, Clone, Copy)]
struct Metrics {
    css: (f64, f64),
    dpr: f64,
}

impl Metrics {
    fn new(css_w: f64, css_h: f64, dpr: f64) -> Result<Self, String> {
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
    fn physical(self) -> Result<(u32, u32), String> {
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

/// The geometry half of an open: what [`load`] produced, before any GPU object
/// exists.
struct Loaded {
    /// The display representation batches (`added` is what gets uploaded).
    delta: SceneDelta,
    /// The camera fitted to the drawing bounds at the requested CSS size.
    camera: cad_app::Camera,
    /// Source DWG length in bytes.
    bytes: usize,
    /// Import + scene build duration in milliseconds.
    import_ms: f64,
}

// ---------------------------------------------------------------------------
// Exported entry points
// ---------------------------------------------------------------------------

pub(crate) async fn start_impl(
    canvas_id: &str,
    css_w: f64,
    css_h: f64,
    dpr: f64,
) -> Result<String, JsValue> {
    // Claim the start slot first: a concurrent `start` must not build a second
    // presenter for the same canvas.
    begin_start()?;
    let metrics =
        Metrics::new(css_w, css_h, dpr).map_err(|message| fail(STEP_VIEWPORT, message))?;
    let physical = metrics
        .physical()
        .map_err(|message| fail(STEP_VIEWPORT, message))?;
    let canvas = canvas_element(canvas_id).map_err(|message| fail(STEP_CANVAS, message))?;
    let loaded = load(
        SYNTHETIC_DWG,
        SYNTHETIC_LABEL,
        [metrics.css.0, metrics.css.1],
    )
    .map_err(|(step, message)| fail(step, message))?;
    // This is the real WebGL2 capability check (instance → surface → adapter →
    // device → configure), not a feature sniff: a host that fails here must emit
    // the `webgl2` marker instead of showing a blank canvas (ADR 0024 D5).
    let presenter = CanvasPresenter::create(canvas, physical)
        .await
        .map_err(|error| fail(STEP_WEBGL2, error.to_string()))?;
    // Everything below is synchronous, so no `RefCell` borrow and no DOM borrow
    // is held across an await point.
    let mut session = attach(presenter, metrics, SYNTHETIC_LABEL, loaded)
        .map_err(|(step, message)| fail(step, message))?;
    session
        .render_and_present()
        .map_err(|(step, message)| fail(step, message))?;
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
    // Refuse while another start/open is in flight, before doing any work.
    ensure_not_starting()?;
    let label = label.trim();
    if label.is_empty() {
        return Err(document_failure(
            STEP_INPUT,
            "open_bytes needs a non-empty label (the source file name)",
        ));
    }
    let metrics = Metrics::new(css_w, css_h, dpr)
        .map_err(|message| document_failure(STEP_VIEWPORT, &message))?;
    let physical = metrics
        .physical()
        .map_err(|message| document_failure(STEP_VIEWPORT, &message))?;
    let canvas =
        canvas_element(canvas_id).map_err(|message| document_failure(STEP_CANVAS, &message))?;
    // Stage 1, before any state change: a bad, unsupported or empty file leaves a
    // live session — and its canvas pixels — exactly as they were.
    let loaded = load(bytes, label, [metrics.css.0, metrics.css.1])
        .map_err(|(step, message)| document_failure(step, &message))?;
    // Stage 2: replace. The previous renderer and canvas presenter are dropped
    // here, before the replacement presenter is created, so two presenters are
    // never live at once and no surface is leaked. From this point a failure can
    // only be a surface/device/upload failure, and it is fatal and named.
    begin_replace()?;
    let presenter = CanvasPresenter::create(canvas, physical)
        .await
        .map_err(|error| fail(STEP_WEBGL2, error.to_string()))?;
    let mut session =
        attach(presenter, metrics, label, loaded).map_err(|(step, message)| fail(step, message))?;
    session
        .render_and_present()
        .map_err(|(step, message)| fail(step, message))?;
    record_import(session.import_ms);
    let status = live_status(&session);
    install_live(session);
    web_sys::console::log_1(&JsValue::from_str(&status));
    Ok(status)
}

pub(crate) fn resize_impl(css_w: f64, css_h: f64, dpr: f64) -> Result<String, JsValue> {
    with_live(|session| {
        let metrics = Metrics::new(css_w, css_h, dpr).map_err(|m| (STEP_VIEWPORT, m))?;
        let physical = metrics.physical().map_err(|m| (STEP_VIEWPORT, m))?;
        session
            .presenter
            .resize(physical)
            .map_err(|error| (STEP_PRESENT, error.to_string()))?;
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
            return Err((
                STEP_NAVIGATION,
                format!("zoom arguments must be finite: factor={factor}, cursor=({x}, {y})"),
            ));
        }
        let size = [session.metrics.css.0, session.metrics.css.1];
        session
            .camera
            .zoom_at(factor, [x, y], size)
            .map_err(|error| (STEP_NAVIGATION, error.to_string()))?;
        session.render_and_present()?;
        Ok(live_status(session))
    })
}

pub(crate) fn pan_impl(dx: f64, dy: f64) -> Result<String, JsValue> {
    with_live(|session| {
        if !(dx.is_finite() && dy.is_finite()) {
            return Err((
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
            State::Idle => failure_status(
                STEP_STATE,
                "the canvas session has not started; call start() first",
            ),
            State::Starting => failure_status(
                STEP_STATE,
                "the canvas session is starting or being replaced; await that call first",
            ),
            State::Failed(status) => status.clone(),
            State::Live(session) => live_status(session),
        },
        Err(_) => failure_status(STEP_STATE, "the session state is already borrowed"),
    })
}

/// What the open file is, as far as this session knows it.
///
/// `{ok, label, batches, draws, presented, bytes}` plus the measured
/// `import_ms` and the `wasm_heap_bytes` evidence field. With no live session the
/// object is `ok: false` with null/zero values — never a made-up file.
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
    LAST_IMPORT_MS.with(Cell::get)
}

// ---------------------------------------------------------------------------
// Geometry load and GPU attach
// ---------------------------------------------------------------------------

/// Geometry load: DWG `bytes` → imported database → display representation.
///
/// Touches no GPU object and no session state, which is what lets `open_bytes`
/// report a bad file without disturbing the session that is already open. `label`
/// only names the file in diagnostics.
fn load(bytes: &[u8], label: &str, logical: [f64; 2]) -> Result<Loaded, (&'static str, String)> {
    let started = web_time::Instant::now();
    if bytes.is_empty() {
        return Err((STEP_IMPORT, format!("{label}: the file is empty (0 bytes)")));
    }
    let drawing = AcadrustImporter::new()
        .import(
            &ImportRequest {
                document: DocumentId(SESSION_DOCUMENT),
                database: DatabaseId(SESSION_DATABASE),
                bytes: Arc::from(bytes),
                limits: ImportLimits::default(),
                generation: 0,
            },
            &|| false,
        )
        .map_err(|error| (STEP_IMPORT, format!("{label}: {error}")))?;
    let stamp = TaskStamp::new(DocumentId(SESSION_DOCUMENT), 0);
    let delta = build_scene(&drawing.database, stamp)
        .map_err(|error| (STEP_SCENE, format!("{label}: {error}")))?;
    if delta.added.is_empty() {
        // A parsed drawing with nothing drawable must not become a blank canvas
        // that looks like a successful open.
        return Err((
            STEP_EMPTY_SCENE,
            format!("{label}: the drawing produced no draw batches"),
        ));
    }
    // The camera is fitted once from the drawing bounds, exactly like the
    // engine's initial fit; after that only pan/zoom move it.
    let camera = fit_camera(&drawing.database, logical);
    let import_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(Loaded {
        delta,
        camera,
        bytes: bytes.len(),
        import_ms,
    })
}

/// GPU half of an open: initialize the renderer on `presenter`'s device, upload
/// the resident batches, and build the session.
fn attach(
    presenter: CanvasPresenter,
    metrics: Metrics,
    label: &str,
    loaded: Loaded,
) -> Result<Session, (&'static str, String)> {
    // The renderer shares the presenter's device/queue: the frame texture it
    // renders is the one the presenter blits.
    let mut renderer = Renderer::new(BackendPreference::WebGl2);
    let (device, queue) = presenter.device_queue();
    renderer
        .initialize_with_device(device, queue)
        .map_err(|error| (STEP_DEVICE, error.to_string()))?;
    let upload_started = web_time::Instant::now();
    let staged = renderer
        .prepare_upload_batches(&loaded.delta.added)
        .map_err(|error| (STEP_UPLOAD, error.to_string()))?;
    renderer
        .commit_upload(staged, 0)
        .map_err(|error| (STEP_UPLOAD, error.to_string()))?;
    renderer
        .upload_masks(&loaded.delta.masks)
        .map_err(|error| (STEP_UPLOAD, error.to_string()))?;
    let upload_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
    let batches = renderer.batch_count();
    if batches == 0 {
        // `load` already refused an empty delta; this catches a renderer that
        // admits every batch and keeps none, so a blank canvas is never success.
        return Err((
            STEP_EMPTY_SCENE,
            format!("{label}: the uploaded scene holds no draw batches"),
        ));
    }
    Ok(Session {
        presenter,
        renderer,
        camera: loaded.camera,
        metrics,
        label: label.to_string(),
        bytes: loaded.bytes,
        batches,
        draws: 0,
        last_error: None,
        import_ms: loaded.import_ms + upload_ms,
    })
}

/// The initial camera: the default 2D top view moved to the drawing centre with
/// the world-per-pixel scale that frames the bounds.
///
/// The arithmetic is the engine's `fit_camera` rule (bounding-box centre, the
/// larger of the two axis ratios, 1.05 margin), and the projection goes through
/// the validated `Projection::orthographic` constructor. Non-finite bounds keep
/// the default top view rather than producing a NaN scale.
fn fit_camera(database: &cad_db::DrawingDatabase, logical: [f64; 2]) -> cad_app::Camera {
    let mut camera = cad_app::Camera::top_view_2d();
    let Some((min, max)) = database.bounds() else {
        return camera;
    };
    let extent_x = (max.x - min.x).max(1e-6);
    let extent_y = (max.y - min.y).max(1e-6);
    let center = Point3 {
        x: (min.x + max.x) * 0.5,
        y: (min.y + max.y) * 0.5,
        z: 0.0,
    };
    let world_per_px = (extent_x / logical[0].max(1.0)).max(extent_y / logical[1].max(1.0)) * 1.05;
    if !(center.x.is_finite() && center.y.is_finite()) {
        return camera;
    }
    // Shift both eye and target, so the view direction and the eye distance of
    // the top view are preserved (the 2D path only reads the target centre and
    // the projection scale).
    let offset = Point3 {
        x: center.x - camera.target.x,
        y: center.y - camera.target.y,
        z: 0.0,
    };
    camera.target = Point3 {
        x: camera.target.x + offset.x,
        y: camera.target.y + offset.y,
        z: camera.target.z,
    };
    camera.eye = Point3 {
        x: camera.eye.x + offset.x,
        y: camera.eye.y + offset.y,
        z: camera.eye.z,
    };
    if let Ok(projection) = cad_app::Projection::orthographic(world_per_px) {
        camera.projection = projection;
    }
    camera
}

// ---------------------------------------------------------------------------
// Frame path and navigation
// ---------------------------------------------------------------------------

impl Session {
    /// The world-per-pixel scale the renderer will use this frame.
    fn world_per_px(&self) -> Option<f64> {
        self.camera.camera2d_params().ok().map(|p| p.world_per_px)
    }

    /// Pan by a CSS-pixel screen delta.
    ///
    /// Mirrors the desktop path: the pointer delta becomes the world delta
    /// `[dx × wpp, -dy × wpp]` (screen y grows down, world y grows up) and the
    /// camera target+eye shift by its negation, so the model follows the pointer.
    fn pan_css(&mut self, delta: [f64; 2]) -> Result<(), (&'static str, String)> {
        let world_per_px = self
            .camera
            .camera2d_params()
            .map_err(|error| (STEP_CAMERA, error.to_string()))?
            .world_per_px;
        let shift = Point3 {
            x: -delta[0] * world_per_px,
            y: delta[1] * world_per_px,
            z: 0.0,
        };
        if !(shift.x.is_finite() && shift.y.is_finite()) {
            return Err((
                STEP_NAVIGATION,
                "pan produced a non-finite world shift".to_string(),
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

    /// Render the resident scene with the current camera and present it.
    ///
    /// Camera-only: nothing is parsed, rebuilt or re-uploaded here.
    fn render_and_present(&mut self) -> Result<(), (&'static str, String)> {
        let params = self
            .camera
            .camera2d_params()
            .map_err(|error| (STEP_CAMERA, error.to_string()))?;
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
            .map_err(|error| (STEP_RENDER, error.to_string()))?;
        self.draws = stats.draw_calls;
        let texture = self.renderer.frame_texture().ok_or_else(|| {
            (
                STEP_RENDER,
                "the renderer reported a frame but holds no frame texture".to_string(),
            )
        })?;
        self.presenter
            .present_texture(texture)
            .map_err(|error| (STEP_PRESENT, error.to_string()))?;
        self.last_error = None;
        Ok(())
    }
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
    });
    if let Some((step, message)) = &session.last_error {
        status["step"] = serde_json::json!(step);
        status["error"] = serde_json::json!(message);
        if *step == STEP_WEBGL2 {
            status["marker"] = serde_json::json!(WEBGL2_MARKER);
        }
    }
    status.to_string()
}

fn failure_status(step: &str, message: &str) -> String {
    let mut status = serde_json::json!({
        "ok": false,
        "step": step,
        "error": message,
        "backend": null,
        "adapter_name": null,
        "format": null,
        "physical": null,
        "label": null,
        "bytes": 0,
        "batches": 0,
        "draws": 0,
        "presented": 0,
    });
    if step == STEP_WEBGL2 {
        status["marker"] = serde_json::json!(WEBGL2_MARKER);
    }
    status.to_string()
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
    })
}

/// The size of the module's **wasm linear memory**, in bytes, or `null` when the
/// host does not expose it.
///
/// This is the module's own heap (`WebAssembly.Memory`), **not** process RSS:
/// there is no process to measure in a browser tab, and the page owns memory this
/// module cannot see. Record it as "wasm heap", never as resident memory.
fn wasm_heap_bytes() -> Option<f64> {
    let memory: js_sys::WebAssembly::Memory = wasm_bindgen::memory().dyn_into().ok()?;
    let buffer: js_sys::ArrayBuffer = js_sys::WebAssembly::Memory::buffer(&memory)
        .dyn_into()
        .ok()?;
    Some(f64::from(js_sys::ArrayBuffer::byte_length(&buffer)))
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/// Record a fatal startup failure and turn it into the `Err` value the host sees.
fn fail(step: &'static str, message: String) -> JsValue {
    let status = failure_status(step, &message);
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            *state = State::Failed(status);
        }
    });
    rejection(step, &message)
}

/// Report an `open_bytes` failure that must not destroy a live session.
///
/// With a live session the failure is recorded on it — `status()`/`document()`
/// then report the still-open file plus the failing step, and the canvas keeps
/// its pixels — and nothing is disposed. Without one, the failure is an ordinary
/// startup failure.
fn document_failure(step: &'static str, message: &str) -> JsValue {
    let recorded = STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match &mut *state {
            State::Live(session) => {
                session.last_error = Some((step, message.to_string()));
                true
            }
            _ => false,
        },
        Err(_) => false,
    });
    if recorded {
        return rejection(step, message);
    }
    fail(step, message.to_string())
}

/// The `Err` value for one failure: a message that names the step, echoed on the
/// browser console so a caller that swallows the rejection still sees why.
fn rejection(step: &str, message: &str) -> JsValue {
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
fn begin_start() -> Result<(), JsValue> {
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
fn ensure_not_starting() -> Result<(), JsValue> {
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
fn begin_replace() -> Result<(), JsValue> {
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
fn install_live(session: Session) {
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            *state = State::Live(Box::new(session));
        }
    });
}

/// Publish one completed import measurement (see [`last_import_ms_impl`]).
fn record_import(import_ms: f64) {
    LAST_IMPORT_MS.with(|cell| cell.set(import_ms));
}

/// Run `action` on the live session, translating its failure into the exported
/// error shape while keeping the session alive.
fn with_live<T>(
    action: impl FnOnce(&mut Session) -> Result<T, (&'static str, String)>,
) -> Result<T, JsValue> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => match &mut *state {
            State::Live(session) => match action(session) {
                Ok(value) => Ok(value),
                Err((step, message)) => {
                    // A failed frame or navigation step is retryable: keep the
                    // session and let `status()` keep reporting the failure.
                    session.last_error = Some((step, message.clone()));
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

fn canvas_element(canvas_id: &str) -> Result<web_sys::HtmlCanvasElement, String> {
    let window =
        web_sys::window().ok_or_else(|| "the host page has no window object".to_string())?;
    let document = window
        .document()
        .ok_or_else(|| "the host page has no document".to_string())?;
    let element = document
        .get_element_by_id(canvas_id)
        .ok_or_else(|| format!("no element with id {canvas_id:?} in the host page"))?;
    element
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| format!("element #{canvas_id} is not an HtmlCanvasElement"))
}
