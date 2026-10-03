//! Browser host assembly (wasm32 only).
//! Feature logic lives in `browser/`; this module owns the shared runtime and
//! assembles the application, UI and render view without changing wasm exports.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cad_app::host::HostController;
use cad_domain::{CadError, CadResult, ViewportId};
use cad_ui_slint::{
    install_with_preference, CadView, IncomingDocument, LocaleResolution, UiAdapter,
    UiConfiguration, UiHandle,
};

mod annotations;
mod async_open;
mod documents;
mod draw;
mod fonts;
mod input;
mod messages;
mod persistence;
mod pick;
pub mod shell;
mod state_push;

pub use annotations::{
    confirm_annotation_export, drop_pending_recovery_snapshot, export_annotations_json,
    import_annotations_json, pending_recovery_is_valid, pending_recovery_snapshot,
    restore_pending_recovery_snapshot,
};
pub use async_open::{
    poll_and_apply as async_open_poll, worker_available as async_open_worker_available,
};
pub use documents::{open_document, open_document_decided, open_needs_decision};
pub use fonts::{font_report, load_current_fonts};
use input::{WebSink, WebViewInput};
use persistence::restore_startup_recovery;

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

fn preference_for(choice: cad_app::BackendChoice) -> cad_ui_slint::web::BackendPreference {
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
struct HostRuntime {
    controller: Rc<RefCell<HostController>>,
    handle: UiHandle,
    view: CadView,
    incoming: IncomingDocument,
    viewport: ViewportId,
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
    with_runtime(|rt| rt.handle.clone())
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

    let controller = Rc::new(RefCell::new(HostController::with_demo_document(
        web_viewport_size(),
    )?));
    let (drawing, document_id, viewport_id) = {
        let mut c = controller.borrow_mut();
        let _ = c.fit();
        (c.drawing(), c.document_id, c.viewport_id)
    };
    let incoming: IncomingDocument = Rc::new(RefCell::new(drawing));
    let shared_handle: SharedHandle = Rc::new(RefCell::new(None));
    let view_slot: Rc<RefCell<Option<CadView>>> = Rc::new(RefCell::new(None));

    let configuration = UiConfiguration {
        compact: false,
        locale: cad_ui_slint::web::stored_locale().unwrap_or_else(|| "zh-CN".to_string()),
        safe_insets: [0.0; 4],
        application_title: "yacr CAD (Web)".into(),
        document: document_id,
        viewport: viewport_id,
        logical_size: web_viewport_size(),
    };
    // Mirror the catalog the handle will use so panel empty/mixed labels are
    // derived from the same locale (the handle does not expose its catalog).
    messages::set_locale(&configuration.locale);

    let sink = WebSink {
        controller: controller.clone(),
        handle: shared_handle.clone(),
        view: view_slot.clone(),
        viewport: viewport_id,
    };
    let mut adapter = UiAdapter::new(configuration, sink, true)?;
    let scale = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0);
    adapter.fit_window_to_logical(web_viewport_size(), scale as f32);
    let handle = adapter.handle();
    *shared_handle.borrow_mut() = Some(handle.clone());
    let _ = handle.set_backend_index(backend_index(preference));

    let view = install_with_preference(handle.clone(), adapter.window(), incoming.clone(), chosen)?;
    {
        let c = controller.borrow();
        if let Some(vp) = c.application.workspace.viewports.get(&viewport_id) {
            view.sync_session(&c.session.active_space, vp);
        }
    }
    view.request_redraw();
    *view_slot.borrow_mut() = Some(view.clone());
    adapter.set_view_input(Rc::new(WebViewInput {
        controller: controller.clone(),
        handle: shared_handle.clone(),
        view: view_slot.clone(),
        viewport: viewport_id,
        last: Cell::new([0.0, 0.0]),
        dragging: Cell::new(false),
        down: Cell::new(None),
    }));
    // Measurement/annotation canvas picks only become real points when the host
    // installs this mapper; without it the adapter reports "pick unwired".
    adapter.set_canvas_pick_mapper(Rc::new(pick::WebCanvasPickMapper {
        controller: controller.clone(),
        handle: shared_handle.clone(),
        viewport: viewport_id,
    }));
    // Draw/edit tools: the shell captures points and emits an intent; the host
    // maps it to one drawing command (line/circle/move/trim). Without this the
    // confirm path reports "not wired" instead of fabricating a command.
    draw::install(
        &adapter,
        controller.clone(),
        shared_handle.clone(),
        view_slot.clone(),
        viewport_id,
    );

    let backend_label = backend_status(chosen);
    // Restore any persisted recovery snapshot for the starting document;
    // only a matching fingerprint is applied, otherwise it is reported and
    // left for an explicit restore/discard.
    restore_startup_recovery(&controller, &handle, &view_slot);
    if !cad_ui_slint::web::has_recovery_snapshot() {
        let _ = handle.set_status(format!("就绪（{backend_label}）"));
    }
    // The first push derives every panel from the real application state.
    state_push::push_panel_state(&controller, &handle, &view_slot);

    HOST.with(|slot| {
        *slot.borrow_mut() = Some(HostRuntime {
            controller: controller.clone(),
            handle: handle.clone(),
            view: view.clone(),
            incoming,
            viewport: viewport_id,
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
        rt.handle.resize_browser_surface([width, height], scale)?;
        let mut controller = rt.controller.borrow_mut();
        let (size, _) = rt
            .handle
            .cad_surface_size()
            .unwrap_or(([width, height], 1.0));
        if let Some(vp) = controller
            .application
            .workspace
            .viewports
            .get_mut(&rt.viewport)
        {
            vp.logical_size = size;
        }
        sync_view_camera(&controller, &rt.view, &rt.viewport);
        rt.view.request_redraw();
        Ok(())
    })
    .unwrap_or(Ok(()))
}

/// Report renderer state for diagnostics and headless tests.
pub fn renderer_report() -> String {
    let report = with_runtime(|rt| {
        let controller = rt.controller.borrow();
        format!(
            "chosen={:?} adapter={:?} caps={:?} error={:?} entities={} wpp={:.6} status={} surface={:?} cad_frames={} lifecycle={:?} center={:?}",
            rt.view.preference(),
            rt.view.active_backend(),
            rt.view.capabilities(),
            rt.view.last_error().or_else(|| rt.view.view_diagnostic()),
            rt.incoming
                .borrow()
                .as_ref()
                .map(|d| d.entity_count())
                .unwrap_or(0),
            rt.view.camera().world_per_px,
            controller.status(),
            rt.handle.cad_surface_size(),
            rt.view.frames_rendered(),
            rt.view.lifecycle(),
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
        let controller = rt.controller.borrow();
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
        let slot = state_push::view_slot(&rt.view);
        state_push::push_panel_state(&rt.controller, &rt.handle, &slot);
    });
    Ok(resolution)
}
