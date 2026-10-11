//! Browser host assembly (wasm32 only).
//!
//! The platform-common orchestration (command routing, state push, the
//! open/plot/save funnels and the navigation/draw/pick sinks) lives in
//! `cad_host::HostCore`; this module is the browser composition root: it builds
//! the core over the web platform impls, installs the Slint adapter and the
//! CAD render bridge, and keeps only the genuinely browser-specific adapters
//! (the Open/Switch-backend interception, the browser pointer semantics and the
//! synchronous open heartbeat).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_app::host::HostController;
use cad_domain::{CadError, CadResult, ViewportId};
use cad_host::{HostCore, HostOptions, HostPlatform};
use cad_platform::HostCapabilities;
use cad_ui_slint::{
    install_with_preference, CadView, IncomingDocument, LocaleResolution, UiAdapter,
    UiConfiguration, UiHandle,
};

mod async_open;
pub mod config;
mod documents;
mod draw;
mod fonts;
mod input;
mod messages;
mod persistence;
mod pick;
mod platform;
pub mod shell;
mod state_push;

use input::{WebSink, WebViewInput};
use platform::{WebConfigStore, WebFiles, WebSurface};

pub use async_open::{
    poll_and_apply as async_open_poll, worker_available as async_open_worker_available,
};
pub use documents::open_document;
pub use fonts::{font_report, load_current_fonts};

type SharedHandle = Rc<RefCell<Option<UiHandle>>>;

/// The authoritative viewport camera as a recovery snapshot's camera.
fn viewport_camera(controller: &HostController) -> ([f64; 3], f64) {
    controller
        .application
        .workspace
        .viewports
        .get(&controller.viewport_id)
        .map(|vp| {
            (
                [vp.camera.target.x, vp.camera.target.y, vp.camera.target.z],
                vp.world_per_px(),
            )
        })
        .unwrap_or(([0.0, 0.0, 0.0], 1.0))
}

pub(super) fn preference_for(
    choice: cad_app::BackendChoice,
) -> cad_ui_slint::web::BackendPreference {
    match choice {
        cad_app::BackendChoice::Auto => cad_ui_slint::web::BackendPreference::Auto,
        cad_app::BackendChoice::WebGpu => cad_ui_slint::web::BackendPreference::WebGpu,
        cad_app::BackendChoice::WebGl2 => cad_ui_slint::web::BackendPreference::WebGl2,
    }
}

fn backend_index(preference: cad_ui_slint::web::BackendPreference) -> i32 {
    match preference {
        cad_ui_slint::web::BackendPreference::Auto => 0,
        cad_ui_slint::web::BackendPreference::WebGpu => 1,
        cad_ui_slint::web::BackendPreference::WebGl2 => 2,
    }
}

/// Logical viewport size from `window.innerWidth/innerHeight` (CSS pixels).
fn web_viewport_size() -> [f64; 2] {
    if let Some(host) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("canvas-host"))
    {
        return [
            host.client_width().max(1) as f64,
            host.client_height().max(1) as f64,
        ];
    }
    let window = web_sys::window();
    let width = window
        .as_ref()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64());
    let height = window
        .as_ref()
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64());
    [
        width.unwrap_or(1280.0).max(320.0),
        height.unwrap_or(800.0).max(240.0),
    ]
}

/// Human-facing name plus the probe results, for the status line and tests.
fn backend_status(preference: cad_ui_slint::web::BackendPreference) -> String {
    let webgpu_api = cad_ui_slint::web::webgpu_api_present();
    let webgl2 = cad_ui_slint::web::webgl2_available();
    format!(
        "{}（navigator.gpu={webgpu_api}, webgl2={webgl2}）",
        preference_name(preference)
    )
}

fn preference_name(preference: cad_ui_slint::web::BackendPreference) -> &'static str {
    match preference {
        cad_ui_slint::web::BackendPreference::Auto => "Auto",
        cad_ui_slint::web::BackendPreference::WebGpu => "WebGPU",
        cad_ui_slint::web::BackendPreference::WebGl2 => "WebGL2",
    }
}

/// Runtime handed to the JS exports after `start_web` completes setup.
///
/// The `HostCore` is a clone handle over the shared orchestration state; the
/// browser-specific open queue and the render bridge's incoming slot travel
/// alongside it.
struct HostRuntime {
    core: HostCore,
    files: Rc<WebFiles>,
    incoming: IncomingDocument,
}

impl HostRuntime {
    fn viewport(&self) -> ViewportId {
        self.core.controller.borrow().viewport_id
    }
}

thread_local! {
    static HOST: RefCell<Option<HostRuntime>> = const { RefCell::new(None) };
}

fn with_runtime<T>(f: impl FnOnce(&HostRuntime) -> T) -> Option<T> {
    HOST.with(|slot| slot.borrow().as_ref().map(f))
}

/// The UI handle for the running host, if it has started.
///
/// Exposed so wasm exports (locale switch) can reach the Slint shell
/// without duplicating the thread-local lookup.
pub fn current_handle() -> Option<UiHandle> {
    with_runtime(|rt| rt.core.handle.borrow().as_ref().cloned()).flatten()
}

thread_local! {
    /// Last logical CAD surface size + DPI scale reported by the JS host, and the
    /// measured CSS safe-area insets `[top, right, bottom, left]`. Retained so a
    /// later inset update (or a rotation) re-applies the same surface.
    static SURFACE: Cell<([f64; 2], f64)> = const { Cell::new(([0.0, 0.0], 1.0)) };
    static SAFE_INSETS: Cell<[f64; 4]> = const { Cell::new([0.0; 4]) };
}

/// The CSS safe-area insets the JS host last reported.
pub(super) fn safe_insets() -> [f64; 4] {
    SAFE_INSETS.with(Cell::get)
}

/// The last reported surface `(logical size, DPI scale)`.
pub(super) fn surface_snapshot() -> ([f64; 2], f64) {
    SURFACE.with(Cell::get)
}

/// The canvas logical size for a reported surface, after safe-area subtraction.
///
/// `None` for a degenerate surface/inset (never a clamped guess). The DPI scale
/// does not affect the logical size; callers that need physical pixels use the
/// authoritative viewport.
pub(super) fn inset_canvas_size(surface: [f64; 2]) -> Option<[f64; 2]> {
    cad_app::input::inset_canvas_metrics([0.0, 0.0], surface, safe_insets(), 1.0)
        .map(|canvas| canvas.size_logical)
}

/// Run one command through the shared funnel, then re-apply the web overlay.
///
/// `HostCore::execute` already pushes every generic panel; the browser overlay
/// (the synchronous-open progress panel and the resources font summary) is
/// re-applied after it so a command cannot clobber them.
pub(super) fn run_command(core: &HostCore, command: cad_app::Command) -> CadResult<()> {
    let result = core.execute(command);
    push_web_overlay(core);
    result
}

/// Push every derived panel state and the browser overlay in one place.
pub(super) fn push_state(core: &HostCore) {
    if let Err(error) = core.push() {
        cad_ui_slint::web::console_error(&format!("host state push failed: {error}"));
    }
    push_web_overlay(core);
}

/// The two channels the shared `HostCore::push` does not cover.
fn push_web_overlay(core: &HostCore) {
    state_push::push_import_state(core);
    state_push::push_resource_sections(core);
}

/// Mirror the authoritative application viewport into the render camera.
fn sync_view_camera(controller: &HostController, view: &CadView, viewport: &ViewportId) {
    if let Some(vp) = controller.application.workspace.viewports.get(viewport) {
        // Space + observation mode + full camera in one call (F04/F13).
        view.sync_session(&controller.session.active_space, vp);
    }
}

/// Start the browser host. Must be called after the DOM is ready.
pub async fn start() -> CadResult<()> {
    let preference = cad_ui_slint::web::stored_preference();
    start_with_preference(preference).await
}

pub async fn start_with_preference(
    preference: cad_ui_slint::web::BackendPreference,
) -> CadResult<()> {
    let chosen = match cad_ui_slint::web::select_backend(preference).await {
        Ok(chosen) => chosen,
        Err(e) => {
            // A forced backend that cannot initialize must report and fall
            // back instead of leaving a blank page (spec §6).
            cad_ui_slint::web::console_error(&format!("{e}"));
            if preference == cad_ui_slint::web::BackendPreference::Auto {
                return Err(e);
            }
            cad_ui_slint::web::select_backend(cad_ui_slint::web::BackendPreference::WebGl2).await?
        }
    };

    let locale = cad_ui_slint::web::stored_locale().unwrap_or_else(|| "zh-CN".to_string());
    let handle_slot: SharedHandle = Rc::new(RefCell::new(None));
    let files = Rc::new(WebFiles::new());
    let platform = HostPlatform {
        files: files.clone(),
        config_store: Rc::new(WebConfigStore),
        surface: Rc::new(WebSurface),
        // The browser resolves fonts/images through its own stack, so the shared
        // core's installation strategy is an explicit refusal (never called by
        // the web open path, which uses `browser::documents`).
        resources: Rc::new(cad_host::UnsupportedResources),
        // Browser capability gates: file open is wired; New/Save/Plot stay off
        // this phase, and the open runs on the calling thread (no worker).
        capabilities: HostCapabilities {
            open: true,
            new: false,
            save: false,
            plot: false,
            import_worker: false,
        },
    };
    let host_options = HostOptions {
        fonts: Vec::new(),
        fonts_dir: None,
        images_dir: None,
        headless: false,
        drawing: None,
        size: web_viewport_size(),
        locale: locale.clone(),
    };
    let core = HostCore::new(host_options, handle_slot, platform)?;
    let controller = core.controller.clone();
    let (drawing, document_id, viewport_id) = {
        let mut c = controller.borrow_mut();
        let _ = c.fit();
        (c.drawing(), c.document_id, c.viewport_id)
    };
    let incoming: IncomingDocument = Rc::new(RefCell::new(drawing));

    let configuration = UiConfiguration {
        compact: false,
        locale: locale.clone(),
        safe_insets: [0.0; 4],
        application_title: "yacr CAD (Web)".into(),
        document: document_id,
        viewport: viewport_id,
        logical_size: web_viewport_size(),
    };
    // Mirror the catalog the handle will use so panel empty/mixed labels are
    // derived from the same locale (the handle does not expose its catalog).
    messages::set_locale(&configuration.locale);

    let sink = WebSink { core: core.clone() };
    let layout_sink = sink.clone();
    let mut adapter = UiAdapter::new(configuration, sink, true)?;
    // The web sink is the layout-switch sink too: a layout click runs through the
    // browser funnel (`run_command`), so the shared push cannot clobber the web
    // overlay (font summary / import panel / `images_modeled: false`).
    adapter.set_layout_switch_sink(Box::new(layout_sink));
    let scale = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0);
    adapter.fit_window_to_logical(web_viewport_size(), scale as f32);
    let handle = adapter.handle();
    core.set_handle(handle.clone());
    // Initial fit uses the actual CAD content rectangle, not the entire shell.
    // Subsequent resize/layout changes preserve the user's camera. The surface is
    // the shell's CAD rect; the safe-area insets (CSS `env()`, reported by the JS
    // host) are folded in by the core's `metrics()`.
    if let Some((size, _)) = handle.cad_surface_size() {
        SURFACE.with(|cell| cell.set((size, scale)));
        core.metrics()?;
        controller.borrow_mut().fit()?;
    }
    let _ = handle.set_backend_index(backend_index(preference));

    let view = install_with_preference(handle.clone(), adapter.window(), incoming.clone(), chosen)?;
    {
        let c = controller.borrow();
        if let Some(vp) = c.application.workspace.viewports.get(&viewport_id) {
            view.sync_session(&c.session.active_space, vp);
        }
    }
    view.request_redraw();
    core.set_view(view.clone());
    adapter.set_view_input(Rc::new(WebViewInput {
        core: core.clone(),
        viewport: viewport_id,
        last: Cell::new([0.0, 0.0]),
        dragging: Cell::new(false),
        down: Cell::new(None),
    }));
    // Measurement/annotation canvas picks only become real points when the host
    // installs this mapper; the shared core resolves a canvas-local logical point
    // to a world point on the viewport work plane.
    adapter.set_canvas_pick_mapper(Rc::new(core.clone()));
    // Draw/edit tools: the shell captures points and emits an intent; the browser
    // sink maps it to one drawing command (line/circle/move/trim). The preview
    // sink is the shared core's.
    draw::install(&adapter, core.clone(), viewport_id);

    let backend_label = backend_status(chosen);
    let _ = handle.set_status(format!("就绪（{backend_label}）"));
    // The first push derives every panel from the real application state.
    push_state(&core);

    // Host policy and the persisted user preference: the shared core injects the
    // allowed-paths defaults and merges the stored projection. A rejected/limited
    // preference never fails startup; a failure is logged, never fatal.
    for diagnostic in core.load_startup() {
        cad_ui_slint::web::console_error(&format!("host startup: {diagnostic}"));
    }

    HOST.with(|slot| {
        *slot.borrow_mut() = Some(HostRuntime {
            core: core.clone(),
            files: files.clone(),
            incoming,
        });
    });

    // Hands the browser event loop its winit app. On wasm this may not
    // return: winit uses a thrown exception as its control-flow handoff,
    // which `main.js` catches explicitly.
    adapter.run()
}

/// Browser resize is presentation-only and never dirties or reparses the drawing.
pub fn resize(width: f64, height: f64, scale: f64) -> CadResult<()> {
    if !width.is_finite()
        || !height.is_finite()
        || !scale.is_finite()
        || width <= 0.0
        || height <= 0.0
        || scale <= 0.0
    {
        return Err(cad_domain::CadError::InvalidInput(
            "invalid browser viewport size".into(),
        ));
    }
    with_runtime(|rt| {
        let handle = rt
            .core
            .handle
            .borrow()
            .as_ref()
            .cloned()
            .ok_or(CadError::Cancelled)?;
        handle.resize_browser_surface([width, height], scale)?;
        let (surface, _) = handle.cad_surface_size().unwrap_or(([width, height], 1.0));
        SURFACE.with(|cell| cell.set((surface, scale)));
        rt.core.metrics()?;
        rt.core.sync_view()?;
        if let Some(view) = rt.core.view.borrow().as_ref() {
            view.request_redraw();
        }
        Ok(())
    })
    .unwrap_or(Ok(()))
}

/// Report the CSS `env(safe-area-inset-*)` values measured by the JS host.
///
/// The insets are logical CSS pixels ordered `[top, right, bottom, left]`.
/// Degenerate input (non-finite or negative) is refused rather than clamped, and
/// the retained surface is re-applied so a soft-keyboard/notch change updates the
/// authoritative viewport without moving the camera.
#[cfg(target_arch = "wasm32")]
pub fn set_safe_insets(top: f64, right: f64, bottom: f64, left: f64) -> CadResult<()> {
    let insets = [top, right, bottom, left];
    if insets.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(CadError::InvalidInput(
            "safe insets must be finite and non-negative".into(),
        ));
    }
    SAFE_INSETS.with(|cell| cell.set(insets));
    with_runtime(|rt| {
        let (surface, _) = surface_snapshot();
        if surface[0] > 0.0 && surface[1] > 0.0 {
            rt.core.metrics()?;
            if let Some(view) = rt.core.view.borrow().as_ref() {
                view.request_redraw();
            }
        }
        Ok(())
    })
    .unwrap_or(Ok(()))
}

/// Report renderer state for diagnostics and headless tests.
pub fn renderer_report() -> String {
    let report = with_runtime(|rt| {
        let controller = rt.core.controller.borrow();
        let view = rt.core.view.borrow();
        let view = view.as_ref();
        format!(
            "chosen={:?} adapter={:?} caps={:?} error={:?} entities={} wpp={:.6} status={} surface={:?} cad_frames={} lifecycle={:?} center={:?}",
            view.map(|v| v.preference()),
            view.and_then(|v| v.active_backend()),
            view.and_then(|v| v.capabilities()),
            view.and_then(|v| v.last_error().or_else(|| v.view_diagnostic())),
            rt.incoming
                .borrow()
                .as_ref()
                .map(|d| d.entity_count())
                .unwrap_or(0),
            view.map(|v| v.camera().world_per_px).unwrap_or(0.0),
            controller.status(),
            rt.core
                .handle
                .borrow()
                .as_ref()
                .and_then(|h| h.cad_surface_size()),
            view.map(|v| v.frames_rendered()).unwrap_or(0),
            view.map(|v| v.lifecycle()),
            viewport_camera(&controller).0,
        )
    });
    report.unwrap_or_else(|| "host not started".to_string())
}

/// Redacted diagnostics model JSON for the drawer and headless inspection.
///
/// Encodes the current document's import report through the shared
/// `cad_diagnostics` model (every reason kept) with the redacted encoder. Before
/// any import report this is an explicit empty model, never fabricated rows.
pub fn diagnostics_report() -> String {
    with_runtime(|rt| {
        let controller = rt.core.controller.borrow();
        let model = controller
            .last_import_report
            .as_ref()
            .map(|report| state_push::diagnostics_model_from_import(&report.diagnostics))
            .unwrap_or_default();
        match cad_diagnostics::DiagnosticPackage::encode_model_redacted(
            &model,
            env!("CARGO_PKG_VERSION"),
        ) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(error) => format!("{{\"error\":\"{error}\"}}"),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"host not started\"}".to_string())
}

/// Apply a locale to the shell and re-push catalog-derived panel state.
///
/// The handle rebuilds its own chrome; the host's catalog mirror and the panel
/// empty/mixed labels must follow, so this wraps both in one call.
pub fn apply_locale(tag: &str) -> CadResult<LocaleResolution> {
    let handle = current_handle().ok_or(CadError::Cancelled)?;
    let resolution = handle.set_locale(tag)?;
    messages::set_locale(tag);
    with_runtime(|rt| {
        // The shared catalog source is the core's, so a runtime switch must reach
        // it too — otherwise shared-catalog text (completed/failed, fit.empty,
        // panel empty/mixed labels) stays in the startup locale.
        rt.core.set_locale(tag);
        push_state(&rt.core);
    });
    Ok(resolution)
}
