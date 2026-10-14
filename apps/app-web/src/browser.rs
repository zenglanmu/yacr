//! Browser host assembly (wasm32 only).
//!
//! The platform-common orchestration (command routing, state push, the
//! open/plot/save funnels and the navigation/draw/pick sinks) lives in
//! `cad_host::HostCore`; this module is the browser composition root: it builds
//! the engine-owned render session first, the core over it, installs the Slint
//! adapter and the CAD render bridge, then attaches the single `EngineAdapter`.
//! It keeps only the genuinely browser-specific adapters (the Open/Switch-backend
//! interception, the browser pointer semantics and the synchronous open
//! heartbeat).
//!
//! P2-B ownership: the engine owns the render session and the command/interaction
//! dispatch; the `UiHandle` and the `CadView` facade live here (never in the
//! host), and the one `EngineAdapter` is the only callback path — the legacy
//! `UiCommandSink` is a detached refusal, so there is no double dispatch.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::request_queue;

use cad_app::host::HostController;
use cad_domain::{CadError, CadResult, ViewportId};
use cad_host::{HostCore, HostOptions, HostPlatform};
use cad_platform::HostCapabilities;
use cad_ui_slint::{
    install_existing_cad_bridge, CadView, EngineAdapter, IncomingDocument, LocaleResolution,
    RenderSessionHandle, UiAdapter, UiConfiguration, UiHandle,
};

mod async_open;
mod client;
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

use input::{DetachedCommandSink, WebViewInput};
use platform::{WebConfigStore, WebFiles, WebSurface};

pub use async_open::{
    poll_and_apply as async_open_poll, worker_available as async_open_worker_available,
};
pub use documents::open_document;
pub use fonts::{font_report, load_current_fonts};

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

/// The storage spelling of a backend preference, matching
/// [`cad_ui_slint::web::stored_preference`]'s parser.
fn storage_preference_name(preference: cad_ui_slint::web::BackendPreference) -> &'static str {
    match preference {
        cad_ui_slint::web::BackendPreference::Auto => "auto",
        cad_ui_slint::web::BackendPreference::WebGpu => "webgpu",
        cad_ui_slint::web::BackendPreference::WebGl2 => "webgl2",
    }
}

/// Persist the selected browser backend preference under the shared storage key.
///
/// Uses `js_sys::Reflect` (like `emit_config_changed`) so no extra `web-sys`
/// feature is required. A missing/blocked storage or a failed write is a real
/// error, never a silent success.
pub(super) fn persist_backend_preference(
    preference: cad_ui_slint::web::BackendPreference,
) -> CadResult<()> {
    // Reach `localStorage` through the global object (the same reflection style
    // as `emit_config_changed`), so no extra `web-sys` feature is required.
    let global = js_sys::global();
    let storage = js_sys::Reflect::get(&global, &"localStorage".into())
        .ok()
        .filter(|value| !value.is_null() && !value.is_undefined())
        .ok_or_else(no_local_storage)?;
    let set_item = js_sys::Reflect::get(&storage, &"setItem".into())
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
        .ok_or_else(no_local_storage)?;
    set_item
        .call2(
            &storage,
            &JsValue::from_str(cad_ui_slint::web::BACKEND_STORAGE_KEY),
            &JsValue::from_str(storage_preference_name(preference)),
        )
        .map(|_| ())
        .map_err(|_| CadError::Invariant("cannot persist backend preference".into()))
}

/// Reload the page so the render session is rebuilt with the new backend.
///
/// The page reload itself is the switch (spec §6): a failure to reload is a real
/// error rather than a claimed switch.
pub(super) fn reload_page() -> CadResult<()> {
    let global = js_sys::global();
    let location = js_sys::Reflect::get(&global, &"location".into())
        .map_err(|_| CadError::Invariant("no location".into()))?;
    let reload = js_sys::Reflect::get(&location, &"reload".into())
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
        .ok_or_else(|| CadError::Invariant("location.reload unavailable".into()))?;
    reload
        .call0(&location)
        .map(|_| ())
        .map_err(|_| CadError::Invariant("reload failed".into()))
}

fn no_local_storage() -> CadError {
    CadError::Invariant("localStorage unavailable".into())
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

/// Browser composition state, cloned freely (each field is an `Rc`-backed handle
/// over owner-thread state).
///
/// The `HostCore` owns the shared orchestration state and the render session; the
/// `UiHandle` and the `CadView` facade are retained here (the host no longer sees
/// them); the `EngineAdapter` is the single command→UI binding; the browser open
/// queue and the session's incoming slot travel alongside.
#[derive(Clone)]
struct HostRuntime {
    core: HostCore,
    handle: UiHandle,
    view: CadView,
    binding: Rc<EngineAdapter>,
    files: Rc<WebFiles>,
    incoming: IncomingDocument,
}

impl HostRuntime {
    fn viewport(&self) -> ViewportId {
        self.core.controller.borrow().viewport_id
    }

    /// The two channels the shared `HostCore` updates do not cover.
    fn push_web_overlay(&self) {
        push_web_overlay(&self.handle, &self.core);
    }

    /// Apply pending engine updates to the shell, then re-apply the browser
    /// overlay so a command cannot clobber it.
    fn flush_updates(&self) -> CadResult<()> {
        self.binding.flush_updates()?;
        self.push_web_overlay();
        Ok(())
    }

    /// One waker-driven pump: web-safe metrics → bounded queue pump (with
    /// completion drain) → flush. File IO is drained by the app heartbeat.
    ///
    /// The metrics refresh is the web-surface-aware path (`core.metrics`, which
    /// subtracts the CSS safe area); the adapter's `refresh_metrics` uses the raw
    /// shell rect, so it is deliberately not used here to preserve safe-area
    /// canvas metrics.
    fn pump_once(&self) -> CadResult<()> {
        if let Err(error) = self.core.metrics() {
            if !matches!(error, CadError::Cancelled) {
                return Err(error);
            }
        }
        self.binding.pump_requests()?;
        self.flush_updates()
    }
}

thread_local! {
    /// Coalesced single-shot wake schedule state (logical, so it is host-testable).
    static WAKE_SCHEDULE: RefCell<request_queue::WakeSchedule> =
        RefCell::new(request_queue::WakeSchedule::default());
    /// The single live `requestAnimationFrame` callback handle: the actual strong
    /// owner of the Rust `Closure`. There is exactly one slot (this is the whole
    /// queue's wake, not a per-stream registry). The handle is taken out and
    /// dropped when invoked, outside any `RefCell` borrow and before pumping.
    static LIVE_RAF_CALLBACK: RefCell<Option<Closure<dyn FnMut()>>> =
        RefCell::new(None);
}

/// Coalesced single-shot queue wake.
///
/// Claims at most one live animation-frame callback (the host only invokes this
/// on an empty→nonempty edge or while work remains queued). It captures no state,
/// so it cannot form a strong reference cycle back to the engine adapter.
///
/// Ownership follows the official wasm-bindgen `requestAnimationFrame` pattern:
/// the Rust `Closure` is kept alive as a **strong owner** in `LIVE_RAF_CALLBACK`
/// and is *not* `forget`-ten (with wasm-bindgen 0.2.129 `Closure::once` +
/// `forget` leaks the outer `FnMut` trampoline; only the captured `FnOnce`
/// payload is consumed by the call). Instead the handle is released by an
/// explicit `Drop` when invoked (or on a schedule failure). Dropping the
/// handle is safe during its own JS invocation because wasm-bindgen refcounts the
/// callback (`cnt++`) for the duration of the call and destroys it in `finally`
/// after the last unref.
///
/// Exactly one slot exists; no per-stream RAF registry. It never pumps
/// synchronously from the waker and adds no permanent timer.
fn schedule_queue_pump() {
    let claimed = WAKE_SCHEDULE.with(|schedule| schedule.borrow_mut().claim());
    if !claimed {
        return;
    }
    // Take the previous (already-executed) handle out and release it *outside*
    // any borrow — dropping a `Closure` must never run while a `RefCell` borrow
    // is held. This happens before creating/storing the new callback so it can
    // never take the freshly scheduled slot.
    let previous = LIVE_RAF_CALLBACK.with(|slot| slot.borrow_mut().take());
    drop(previous);

    let Some(window) = web_sys::window() else {
        // No window to schedule on; let the heartbeat be the only driver.
        WAKE_SCHEDULE.with(|schedule| schedule.borrow_mut().on_schedule_failed());
        return;
    };
    let callback: Closure<dyn FnMut()> = Closure::once(move || {
        // Release this callback before a pump can schedule and store its successor.
        // wasm-bindgen defers destruction until the invocation's final unref.
        let current = LIVE_RAF_CALLBACK.with(|slot| slot.borrow_mut().take());
        drop(current);
        // Clear the scheduled flag, then pump: a pending rearm during the pump
        // may schedule a *new* callback, and this body never touches that slot.
        WAKE_SCHEDULE.with(|schedule| schedule.borrow_mut().on_executed());
        pump_once();
    });
    // Per the HTML/WHATWG rendering spec (and the wasm-bindgen guide), this only
    // registers the callback and returns; it never invokes it synchronously.
    match window.request_animation_frame(callback.as_ref().unchecked_ref()) {
        Ok(_) => {
            // Keep the real handle alive as the strong owner (no `forget`).
            LIVE_RAF_CALLBACK.with(|slot| *slot.borrow_mut() = Some(callback));
        }
        Err(error) => {
            // Drop the real handle here (reclaim) and free the slot for a retry;
            // never pump synchronously and never use `once_into_js` (a failed
            // schedule has no way to release that JS-owned cycle).
            drop(callback);
            WAKE_SCHEDULE.with(|schedule| schedule.borrow_mut().on_schedule_failed());
            cad_ui_slint::web::console_error(&format!("queue wake scheduling failed: {error:?}"));
        }
    }
}

/// Run one pump against the live runtime, releasing the runtime slot first so no
/// `RefCell` borrow is held while the pump runs business/UI callbacks.
fn pump_once() {
    let Some(rt) = with_runtime(|rt| rt.clone()) else {
        return;
    };
    if let Err(error) = rt.pump_once() {
        cad_ui_slint::web::console_error(&format!("queue pump failed: {error}"));
    }
}

/// Read `performance.now()` in milliseconds through the global object.
///
/// Reflect-based (no extra `web-sys` feature). `None` when the Performance API
/// is unavailable or does not return a finite number; the caller maps that to an
/// explicit error rather than a fabricated zero.
fn performance_now_ms() -> Option<f64> {
    let global = js_sys::global();
    let performance = js_sys::Reflect::get(&global, &"performance".into()).ok()?;
    if performance.is_null() || performance.is_undefined() {
        return None;
    }
    let now = js_sys::Reflect::get(&performance, &"now".into()).ok()?;
    let now = now.dyn_into::<js_sys::Function>().ok()?;
    now.call0(&performance).ok()?.as_f64()
}

thread_local! {
    static HOST: RefCell<Option<HostRuntime>> = const { RefCell::new(None) };
}

fn with_runtime<T>(f: impl FnOnce(&HostRuntime) -> T) -> Option<T> {
    HOST.with(|slot| slot.borrow().as_ref().map(f))
}

/// The UI handle for the running host, if it has started.
///
/// Exposed so wasm exports (locale switch, config writes) can reach the Slint
/// shell without duplicating the thread-local lookup.
pub fn current_handle() -> Option<UiHandle> {
    with_runtime(|rt| rt.handle.clone())
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

/// Run one command through the shared funnel, then apply the pending updates and
/// the web overlay. Used by the browser-specific sinks (`ViewInput`, the touch
/// router) that drive `HostCore::execute` directly rather than the adapter.
pub(super) fn run_command(core: &HostCore, command: cad_app::Command) -> CadResult<()> {
    let result = core.execute(command);
    flush_pending();
    result
}

/// Apply any pending engine updates and the browser overlay to the running shell.
///
/// A no-op before `start` completes. Errors are logged, never fatal: a later
/// heartbeat retries.
pub(super) fn flush_pending() {
    with_runtime(|rt| {
        if let Err(error) = rt.flush_updates() {
            cad_ui_slint::web::console_error(&format!("host update flush failed: {error}"));
        }
    });
}

/// The two channels the shared `HostCore` updates do not cover.
fn push_web_overlay(handle: &UiHandle, core: &HostCore) {
    state_push::push_import_state(handle, core);
    state_push::push_resource_sections(handle, core);
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
    let files = Rc::new(WebFiles::new());
    let platform = HostPlatform {
        files: files.clone(),
        config_store: Rc::new(WebConfigStore),
        surface: Rc::new(WebSurface),
        // The browser resolves fonts/images through its own stack, so the shared
        // core's installation strategy is an explicit refusal (never called by
        // the web open path, which uses `browser::documents`).
        resources: Rc::new(cad_host::UnsupportedResources),
        // Browser capability gates: open/New/Save/Plot are all wired (New builds
        // a blank database, Save/Plot synthesize a download target in the
        // gesture). The open runs on the calling thread (no worker).
        capabilities: HostCapabilities {
            open: true,
            new: true,
            save: true,
            plot: true,
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
    };

    // The engine owns the render session from the start: create it with the
    // browser's IncomingDocument slot (shared with the synchronous open path),
    // then build the core over it. `HostCore::new` syncs the demo drawing into
    // that same slot; no second session is created here.
    let incoming: IncomingDocument = Rc::new(RefCell::new(None));
    let render = RenderSessionHandle::with_incoming(incoming.clone(), chosen);
    let core = HostCore::new(host_options, platform, render.clone())?;

    // P3: the owner-thread queue's pump time budget needs a monotonic millisecond
    // clock. The browser supplies `performance.now()`; installing happens before
    // the wrapper/attach/first pump. A missing Performance API is an explicit
    // error, never a fabricated zero clock (the host validates the first sample).
    if performance_now_ms().is_none() {
        return Err(CadError::Unsupported(
            "performance.now is unavailable for the request queue clock".into(),
        ));
    }
    core.set_request_clock(request_queue::request_clock_from(Rc::new(|| {
        performance_now_ms().unwrap_or(f64::NAN)
    })))?;

    let _ = core.controller.borrow_mut().fit();
    let (document_id, viewport_id) = {
        let c = core.controller.borrow();
        (c.document_id, c.viewport_id)
    };

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

    // The legacy sink is a detached refusal: `EngineAdapter::attach` below
    // installs the single engine dispatch, so this slot is never reached and the
    // two paths cannot double-send.
    let mut adapter = UiAdapter::new(configuration, DetachedCommandSink, true)?;
    let scale = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0);
    adapter.fit_window_to_logical(web_viewport_size(), scale as f32);
    let handle = adapter.handle();

    // Install Slint's presenter hooks around the engine-owned session.
    let view = install_existing_cad_bridge(handle.clone(), adapter.window(), render.clone())?;

    // The single engine→UI binding: the browser-scoped wrapper intercepts the
    // browser-only SwitchBackend/OpenDrawing commands and delegates the rest.
    let client = Rc::new(client::WebEngineClient::new(core.clone()));
    let binding = Rc::new(EngineAdapter::attach(&mut adapter, client, view.clone())?);

    // P3 queue wake: a coalesced single-shot animation-frame pump, registered on
    // the one binding. The waker captures no state (so it cannot form a strong
    // reference cycle back to the adapter); the app heartbeat keeps pumping as
    // the backstop so it is never the only queue driver.
    let waker: Rc<dyn Fn()> = Rc::new(schedule_queue_pump);
    binding.set_request_waker(Some(waker))?;

    // Initial fit uses the actual CAD content rectangle, not the entire shell.
    // Subsequent resize/layout changes preserve the user's camera. The surface is
    // the shell's CAD rect; the safe-area insets (CSS `env()`, reported by the JS
    // host) are folded in by the core's `metrics()`.
    if let Some((size, _)) = handle.cad_surface_size() {
        SURFACE.with(|cell| cell.set((size, scale)));
        core.metrics()?;
        core.controller.borrow_mut().fit()?;
    }
    let _ = handle.set_backend_index(backend_index(preference));

    // Startup status is catalog-only: the same key the JS host later re-issues
    // with the real adapter label once the renderer is ready.
    let backend_label = backend_status(chosen);
    core.set_catalog_status("host.renderer_ready", &[("backend", &backend_label)])?;

    // Browser-only input semantics (tap-vs-drag, wheel sign) remain for the
    // pre-attach window and the pinch path; the engine owns pointer/scroll once
    // the client is attached.
    adapter.set_view_input(Rc::new(WebViewInput::new(
        core.clone(),
        handle.clone(),
        viewport_id,
    )));
    // Draw/edit preview: the engine publishes the capture through `HostUpdates`;
    // the legacy browser draw-command mapping is retained as a compatibility
    // facade (never read once a client is attached).
    draw::install(&adapter, core.clone(), viewport_id);

    // The first flush derives every panel from the real application state.
    let runtime = HostRuntime {
        core: core.clone(),
        handle: handle.clone(),
        view: view.clone(),
        binding: binding.clone(),
        files: files.clone(),
        incoming: incoming.clone(),
    };
    runtime.flush_updates().map_err(|error| {
        cad_ui_slint::web::console_error(&format!("initial state flush failed: {error}"));
        error
    })?;

    // Host policy and the persisted user preference: the shared core injects the
    // allowed-paths defaults and merges the stored projection. A rejected/limited
    // preference never fails startup; a failure is logged, never fatal.
    for diagnostic in core.load_startup() {
        cad_ui_slint::web::console_error(&format!("host startup: {diagnostic}"));
    }
    runtime.flush_updates()?;
    // Baseline the font identity on the current document so startup does not
    // trigger a spurious font load; a later real open changes it.
    fonts::prime_font_identity(&runtime);

    HOST.with(|slot| {
        *slot.borrow_mut() = Some(runtime);
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
        let (surface, _) = rt
            .handle
            .cad_surface_size()
            .unwrap_or(([width, height], 1.0));
        SURFACE.with(|cell| cell.set((surface, scale)));
        rt.core.metrics()?;
        rt.view.request_redraw();
        rt.flush_updates()
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
            rt.view.request_redraw();
            rt.flush_updates()?;
        }
        Ok(())
    })
    .unwrap_or(Ok(()))
}

/// Report renderer state for diagnostics and headless tests.
pub fn renderer_report() -> String {
    let report = with_runtime(|rt| {
        let controller = rt.core.controller.borrow();
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
/// The handle rebuilds its own chrome and re-derives the mirrored panel labels;
/// the host's catalog mirror and the browser overlay (import/resource labels)
/// must follow, so this wraps both in one call. Localization is UI-side: the
/// engine's `HostUpdates` carry catalog keys, never localized text.
pub fn apply_locale(tag: &str) -> CadResult<LocaleResolution> {
    let handle = current_handle().ok_or(CadError::Cancelled)?;
    let resolution = handle.set_locale(tag)?;
    messages::set_locale(tag);
    with_runtime(|rt| rt.push_web_overlay());
    Ok(resolution)
}
