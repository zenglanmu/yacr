//! The wasm-side CAD session behind the exported canvas API.
//!
//! One process-global session (`wasm` is single-threaded, so a `thread_local`
//! `RefCell` is enough; no `Mutex`). It owns exactly one `CanvasPresenter`, so
//! "one presenter per canvas" holds by construction: `start` refuses to build a
//! second session while one is live.
//!
//! The pipeline mirrors the headless render contract:
//! `include_bytes!` fixture → `AcadrustImporter` → `cad_app::render_scene::build_scene`
//! → `Renderer::prepare_upload_batches` + `commit_upload` → `Renderer::render`
//! → `CanvasPresenter::present_texture(Renderer::frame_texture())`.
//!
//! Pan/zoom only touch `cad_app::Camera`: the scene batches stay resident (no
//! re-parse, no re-upload), and each navigation step renders and presents
//! immediately so the canvas is never a frame behind the camera.

use std::cell::RefCell;
use std::sync::Arc;

use cad_app::render_scene::build_scene;
use cad_domain::{DatabaseId, DocumentId, Point3, TaskStamp};
use cad_import_acadrust::{AcadrustImporter, ImportLimits, ImportRequest, Importer};
use cad_render_wgpu::canvas::CanvasPresenter;
use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// A committed, writer-produced AC1032 DWG with four LINE entities
/// (`fixtures/manifest`). Not a vendor sample; wasm has no filesystem, so the
/// bytes are compiled in. This is phase-1 scope (ADR 0024 §5): no DWG opening,
/// no proxy/resource decoding in wasm.
const SYNTHETIC_DWG: &[u8] = include_bytes!("../../../fixtures/dwg/synthetic-four-lines.dwg");

/// Stable names of the steps a failure can be attributed to. They are the only
/// stable vocabulary in the status `step` field, so a host can key on them
/// (browser console messages, Linux exit codes) without parsing prose.
const STEP_VIEWPORT: &str = "viewport";
const STEP_CANVAS: &str = "canvas-element";
const STEP_WEBGL2: &str = "webgl2";
const STEP_IMPORT: &str = "import";
const STEP_SCENE: &str = "scene";
const STEP_DEVICE: &str = "device";
const STEP_UPLOAD: &str = "upload";
const STEP_CAMERA: &str = "camera";
const STEP_RENDER: &str = "render";
const STEP_PRESENT: &str = "present";
const STEP_NAVIGATION: &str = "navigation";
const STEP_STATE: &str = "state";

/// The stable refusal marker for a `dom-canvas` host without usable WebGL2
/// (ADR 0024 D5). Only the `webgl2` step carries it; the host emits this line
/// verbatim and exits with code 6 instead of showing a blank canvas.
const WEBGL2_MARKER: &str = "yacr: unsupported-runtime: webgl2";

/// The document identity of the single session (one fixture, one database).
const SESSION_DOCUMENT: u128 = 1;
const SESSION_DATABASE: u128 = 1;

/// The process-global session state.
enum State {
    /// Nothing has started yet.
    Idle,
    /// `start` is in flight (it awaits the canvas presenter). A concurrent
    /// `start` is refused here, so one canvas never gets two presenters.
    Starting,
    /// `start` failed before a session existed; the payload is the exact status
    /// JSON the failure produced.
    Failed(String),
    /// A live session on one canvas.
    Live(Box<Session>),
}

thread_local! {
    // `wasm` is single-threaded, so one `RefCell` slot is the whole session
    // registry. `const` init keeps the first access allocation-free.
    static STATE: RefCell<State> = const { RefCell::new(State::Idle) };
}

/// The one CAD session: presenter + renderer + camera.
struct Session {
    presenter: CanvasPresenter,
    renderer: Renderer,
    /// The application camera. The render camera is derived from it through
    /// `camera2d_params()` on every frame, so the two cannot drift.
    camera: cad_app::Camera,
    metrics: Metrics,
    /// Resident draw batches after the initial upload (evidence for the status).
    batches: usize,
    /// Draw submissions of the last presented frame.
    draws: u64,
    /// The last navigation/frame failure, kept so `status()` keeps reporting it
    /// while the session stays usable (a surface `Outdated`/`Lost` frame is
    /// retryable and must not destroy the session).
    last_error: Option<(&'static str, String)>,
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
    // This is the real WebGL2 capability check (instance → surface → adapter →
    // device → configure), not a feature sniff: a host that fails here must emit
    // the `webgl2` marker instead of showing a blank canvas (ADR 0024 D5).
    let presenter = CanvasPresenter::create(canvas, physical)
        .await
        .map_err(|error| fail(STEP_WEBGL2, error.to_string()))?;
    // Everything below is synchronous, so no `RefCell` borrow and no DOM borrow
    // is held across an await point.
    let mut session =
        build_session(presenter, metrics).map_err(|(step, message)| fail(step, message))?;
    session
        .render_and_present()
        .map_err(|(step, message)| fail(step, message))?;
    let status = live_status(&session);
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            *state = State::Live(Box::new(session));
        }
    });
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
                "the canvas session is still starting; await start() first",
            ),
            State::Failed(status) => status.clone(),
            State::Live(session) => live_status(session),
        },
        Err(_) => failure_status(STEP_STATE, "the session state is already borrowed"),
    })
}

// ---------------------------------------------------------------------------
// Session construction
// ---------------------------------------------------------------------------

fn build_session(
    presenter: CanvasPresenter,
    metrics: Metrics,
) -> Result<Session, (&'static str, String)> {
    let drawing = AcadrustImporter::new()
        .import(
            &ImportRequest {
                document: DocumentId(SESSION_DOCUMENT),
                database: DatabaseId(SESSION_DATABASE),
                bytes: Arc::from(SYNTHETIC_DWG),
                limits: ImportLimits::default(),
                generation: 0,
            },
            &|| false,
        )
        .map_err(|error| (STEP_IMPORT, error.to_string()))?;
    let stamp = TaskStamp::new(DocumentId(SESSION_DOCUMENT), 0);
    let delta =
        build_scene(&drawing.database, stamp).map_err(|error| (STEP_SCENE, error.to_string()))?;
    // The camera is fitted once from the drawing bounds, exactly like the
    // engine's initial fit; after that only pan/zoom move it.
    let camera = fit_camera(&drawing.database, [metrics.css.0, metrics.css.1]);

    // The renderer shares the presenter's device/queue: the frame texture it
    // renders is the one the presenter blits.
    let mut renderer = Renderer::new(BackendPreference::WebGl2);
    let (device, queue) = presenter.device_queue();
    renderer
        .initialize_with_device(device, queue)
        .map_err(|error| (STEP_DEVICE, error.to_string()))?;
    let staged = renderer
        .prepare_upload_batches(&delta.added)
        .map_err(|error| (STEP_UPLOAD, error.to_string()))?;
    renderer
        .commit_upload(staged, 0)
        .map_err(|error| (STEP_UPLOAD, error.to_string()))?;
    renderer
        .upload_masks(&delta.masks)
        .map_err(|error| (STEP_UPLOAD, error.to_string()))?;
    let batches = renderer.batch_count();
    if batches == 0 {
        // Never present an empty frame as success: the fixture has four lines, so
        // zero batches means the scene pipeline failed, not that the drawing is
        // empty.
        return Err((
            STEP_SCENE,
            "the synthetic fixture produced no draw batches; refusing to present a blank canvas"
                .to_string(),
        ));
    }
    Ok(Session {
        presenter,
        renderer,
        camera,
        metrics,
        batches,
        draws: 0,
        last_error: None,
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
// Status JSON
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
        "batches": session.batches,
        "draws": session.draws,
        "presented": session.presenter.presented_frames(),
        "configure_count": session.presenter.configure_count(),
        "world_per_px": session.world_per_px(),
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
        "batches": 0,
        "draws": 0,
        "presented": 0,
    });
    if step == STEP_WEBGL2 {
        status["marker"] = serde_json::json!(WEBGL2_MARKER);
    }
    status.to_string()
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

/// The `Err` value for one failure: a message that names the step, echoed on the
/// browser console so a caller that swallows the rejection still sees why.
fn rejection(step: &str, message: &str) -> JsValue {
    let text = format!("{step}: {message}");
    web_sys::console::error_1(&JsValue::from_str(&text));
    JsValue::from_str(&text)
}

/// Claim the single start slot.
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
                "the canvas session is still starting; await start() first",
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
