//! Tauri setup: build the probe window, the child webview and the GPU bridge.

use std::sync::{Arc, Mutex};

use cad_host::HostOptions;
use cad_platform::ConfigStore;
use cad_ui_core::unsupported_status;

use super::dom::ERROR_HOOK_JS;
use super::locale::{host_locale_script, host_locale_tag};
use super::options::{
    config_paths, start_maximized, INITIAL_HEIGHT, INITIAL_WIDTH, WEBVIEW_LABEL, WINDOW_LABEL,
};
use super::presenter::{close_presenter, release_presenter, request_stop};
use super::telemetry::log_top_level_native_handle;
use super::{
    lock, lock_telemetry, surface_plan, CadEnv, RenderTelemetry, Shared, BRIDGE, CAD_ENV, WEBVIEW,
};

/// Build the probe window, its child webview and the GPU presenter.
///
/// Runs on the main thread during Tauri `setup`.
///
/// What is built depends on the installed `surfaceMode` (`surface_plan()`, ADR
/// 0024 D1/D2): `native-below` builds the native CAD composition and the GTK
/// overlay exactly as before; `dom-canvas` builds **only** the window and the
/// WebView, because the wasm session inside the WebView is the sole CAD authority
/// (iron rule 8) and there is no native surface to compose.
pub fn init(
    app: &mut tauri::App,
    shared: &Arc<Mutex<Shared>>,
    telemetry: &Arc<Mutex<RenderTelemetry>>,
) -> tauri::Result<()> {
    let plan = surface_plan();
    // `YACR_PROBE_OPAQUE` is a diagnostic-only escape hatch (window visual test).
    let transparent = std::env::var_os("YACR_PROBE_OPAQUE").is_none();
    // Default: open maximized (the CAD partition should use the whole screen).
    // `YACR_PROBE_NO_MAXIMIZE` (acceptance/evidence only) opts out; it is never
    // the default.
    let maximize = start_maximized(std::env::var_os("YACR_PROBE_NO_MAXIMIZE").is_some());
    let window = tauri::window::WindowBuilder::new(app, WINDOW_LABEL)
        .title("Yacr")
        .inner_size(f64::from(INITIAL_WIDTH), f64::from(INITIAL_HEIGHT))
        .min_inner_size(320.0, 240.0)
        .resizable(true)
        .transparent(transparent)
        .maximized(maximize)
        .build()?;

    // Top-level handle check: the probe surface is created on this exact window.
    log_top_level_native_handle(&window);

    // Use the window's real size for the child webview, not the initial
    // constants (the WM may already have adjusted the window).
    let initial_size = window
        .inner_size()
        .unwrap_or(tauri::PhysicalSize::new(INITIAL_WIDTH, INITIAL_HEIGHT));

    // Transparent, full-window child webview overlaying the probe surface.
    let child = {
        let webview = tauri::webview::WebviewBuilder::new(
            WEBVIEW_LABEL,
            tauri::WebviewUrl::App("index.html".into()),
        )
        .transparent(transparent)
        // No `.auto_resize()`: it is inert for this webview on Linux. Tauri packs
        // a child webview into the window's `GtkBox`, so `wry`'s `set_bounds`
        // matches neither its `GtkFixed` parent branch nor its raw-X11-child
        // branch and does nothing; the host then reparents the widget into its
        // own `GtkOverlay`, leaving nothing to size it. `crate::gtk_bridge::sync_webview_size`
        // is the real mechanism (window resize/scale events, the overlay's own
        // layout pass and the DOM layout tick).
        .initialization_script(ERROR_HOOK_JS)
        .initialization_script(crate::stacking_regression::initialization_script(
            crate::stacking_regression::enabled(),
        ))
        // The host-resolved locale, handed to the chrome **before** the document
        // is parsed: this is the single system default both the native dialog
        // labels and the React UI use (see `host::locale`).
        .initialization_script(host_locale_script())
        .on_page_load({
            let page_load_shared = shared.clone();
            move |_webview, payload| {
                eprintln!(
                    "[probe] page_load url={} event={:?}",
                    payload.url(),
                    payload.event()
                );
                // Trigger ②: a reload/navigation replaced the document, so the
                // in-DOM overlay no longer exists and the modal gate must not
                // stay set. The initial page load is a harmless no-op.
                if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                    // A reload also ends the wasm session that reported the
                    // descriptor: forget its name so a stale one cannot outlive
                    // it (`host::canvas_session`).
                    crate::host::canvas_session::on_session_reset("reload");
                    cad_desktop_shell::popup::on_reload(&page_load_shared);
                }
            }
        });
        let child = window.add_child(
            webview,
            tauri::PhysicalPosition::new(0, 0),
            tauri::PhysicalSize::new(initial_size.width.max(1), initial_size.height.max(1)),
        )?;
        // Keep a handle for the bounded DOM diagnostics (main thread only).
        WEBVIEW.with(|cell| *cell.borrow_mut() = Some(child.clone()));
        Some(child)
    };

    // Trigger ③ of the popup lifecycle: a renderer crash must clear the gate and
    // leave a visible diagnostic.
    if let Some(webview) = child.as_ref() {
        super::popup::install_renderer_crash_hook(webview, shared);
    }

    // Window events: log geometry (WM-native evidence) and release the surface
    // before the window goes away.
    let events_shared = shared.clone();
    let events_telemetry = telemetry.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Resized(size) => {
            eprintln!(
                "[probe] WM event: window resized to {}x{}",
                size.width, size.height
            );
            // The child webview does not follow the window by itself (see
            // `sync_webview_size`): apply the new inner size now, alongside the
            // GTK layout pass the resize already triggered. Only the
            // `native-below` carrier reparents the webview; `dom-canvas` leaves it
            // in Tauri's own `GtkBox`, which GTK sizes.
            if plan.gtk_bridge {
                crate::gtk_bridge::sync_webview_size((size.width, size.height), "window-resize");
            }
        }
        tauri::WindowEvent::Moved(position) => {
            eprintln!(
                "[probe] WM event: window moved to {},{}",
                position.x, position.y
            );
        }
        tauri::WindowEvent::ScaleFactorChanged {
            scale_factor,
            new_inner_size,
            ..
        } => {
            eprintln!("[probe] WM event: scale factor {scale_factor}");
            // Allocations are logical px: re-apply at the new scale instead of
            // waiting for the next frame (`native-below` only, see above).
            if plan.gtk_bridge {
                crate::gtk_bridge::sync_webview_size(
                    (new_inner_size.width, new_inner_size.height),
                    "window-scale",
                );
            }
        }
        tauri::WindowEvent::Focused(focused) => {
            eprintln!("[probe] WM event: focused={focused}");
        }
        tauri::WindowEvent::CloseRequested { .. } => {
            eprintln!("[probe] WM event: close requested; closing surface first");
            close_presenter();
        }
        tauri::WindowEvent::Destroyed => {
            eprintln!("[probe] WM event: destroyed; requesting stop");
            request_stop(&events_shared, &events_telemetry);
        }
        _ => {}
    });

    let evidence_window = window.clone();
    // Register the app-side seams before any IPC command can run.
    super::register_seams(app.handle(), shared, telemetry);

    let no_gpu = lock_telemetry(telemetry).no_gpu;
    lock(shared).window = Some(window.clone());
    lock_telemetry(telemetry).app_handle = Some(app.handle().clone());

    // ---- CAD platform composition (main thread; `Rc`, kept in a TLS) ----
    //
    // `dom-canvas` (X11/XWayland) must **not** create this: it is the native CAD
    // identity (platform files/surface/host options for a `CadSession`), and the
    // tick would then build a second `cad-db` authority next to the wasm session
    // (ADR 0024 D2 / iron rule 8). Only the window/WebView/platform seams remain.
    let launch = lock(shared).launch.clone();
    if !plan.native_cad_session {
        eprintln!(
            "[probe] surfaceMode=dom-canvas: native CAD composition skipped (the wasm session in \
             the WebView is the only CAD authority)"
        );
    } else if !launch.probe_only {
        // Same resolved tag as the injected `window.__YACR_HOST_LOCALE__`:
        // native labels and chrome cannot pick different languages.
        match crate::platform::ReactFiles::new(host_locale_tag()) {
            Ok(files) => {
                let (config, preference) = config_paths();
                let config_store: std::rc::Rc<dyn ConfigStore> = match (config, preference) {
                    (Some(config), Some(preference)) => {
                        std::rc::Rc::new(crate::platform::ReactConfigStore::new(config, preference))
                    }
                    _ => std::rc::Rc::new(crate::platform::ReactConfigStore::non_persistent()),
                };
                let env = CadEnv {
                    files: std::rc::Rc::new(files),
                    config: config_store,
                    surface: crate::platform::ReactSurface::new(),
                    options: HostOptions {
                        fonts: Vec::new(),
                        fonts_dir: launch.fonts_dir.clone(),
                        images_dir: launch.images_dir.clone(),
                        headless: false,
                        drawing: launch.drawing.clone(),
                        size: launch.size,
                    },
                };
                CAD_ENV.with(|cell| *cell.borrow_mut() = Some(env));
            }
            Err(error) => {
                let text = format!("{error:?}");
                eprintln!("[probe] CAD platform init FAILED: {text}");
                // A platform-composition failure is a CadSession-level failure:
                // it lives in the snapshot, not the host transient error.
                lock(shared).shell.cad_status = Some(unsupported_status(Some(text)));
            }
        }
    }

    if !plan.gtk_bridge {
        // `dom-canvas`: there is no native surface to put below the DOM, so the
        // GTK overlay, the windowed `GtkDrawingArea`, the app-owned Wayland
        // subsurface shim and the wgpu presenter are all skipped. The WebView
        // stays where Tauri packed it (the window's `GtkBox`), which GTK sizes
        // itself — no reparent and no host-side sizing (`sync_webview_size`
        // reports `skipped=not-registered` if anything still calls it).
        eprintln!(
            "[probe] surfaceMode=dom-canvas: GTK overlay drawing area, subsurface shim and wgpu \
             presenter skipped (in-DOM canvas carrier)"
        );
    } else if no_gpu {
        eprintln!("[probe] YACR_PROBE_NO_GPU: GPU bridge skipped (React-only baseline)");
    } else {
        // Measurement-only GPU frame-time opt-in (ADR 0013 class 1). This is the
        // **only** place the flag is read; it is threaded explicitly to device
        // creation and the render session. Off by default (product config).
        let gpu_time = cad_render_wgpu::gpu_time::GpuTimeConfig {
            enabled: std::env::var_os("YACR_PROBE_GPU_TIME").is_some(),
        };
        if gpu_time.enabled {
            eprintln!(
                "[probe] YACR_PROBE_GPU_TIME: measurement-only GPU frame-time opt-in enabled \
                 (the readback perturbs the frame; not a product configuration)"
            );
        }
        // GTK overlay bridge: an independent native child surface (windowed
        // GtkDrawingArea) with the React webview reparented as a transparent
        // sibling. **No top-level fallback** — failure is explicit.
        match crate::gtk_bridge::install(&window, WINDOW_LABEL, gpu_time) {
            Ok(owner) => {
                owner.log_tree();
                let release = std::rc::Rc::new(release_presenter);
                crate::gtk_bridge::install_unrealize_hooks(&owner, release);
                BRIDGE.with(|cell| *cell.borrow_mut() = Some(owner));
                if let Some(webview) = child.as_ref() {
                    if let Err(error) = crate::gtk_bridge::reparent_webview(webview, WINDOW_LABEL) {
                        eprintln!("[probe] bridge reparent FAILED: {error}");
                        lock(shared).shell.status.error =
                            Some(format!("gtk bridge reparent failed: {error}"));
                    }
                }
            }
            Err(error) => {
                eprintln!("[probe] bridge install FAILED: {error} (no top-level fallback)");
                lock(shared).shell.status.error =
                    Some(format!("gtk bridge install failed: {error}"));
            }
        }
    }
    // Keep the single event loop awake so the probe renders continuously.
    let wake_pending = lock_telemetry(telemetry).wake_pending.clone();
    let pump = crate::redraw::RedrawPump::spawn(app.handle(), wake_pending);
    lock_telemetry(telemetry).redraw_pump = Some(pump);

    // Env-gated, evidence-only lifecycle driver (no effect on normal runs).
    //
    // The CAD drivers below drive the **native** CAD session (synthetic pointer
    // input through the real webview, tool/edit/draw phases, popup payload reads)
    // through `cad_status`/`cad_set_*`. `dom-canvas` has no native session, so on
    // that carrier they are not spawned at all: a driver run there would report
    // refusals from a path that does not exist, and its frames would prove nothing
    // (the window-level lifecycle driver stays unconditional).
    if plan.native_cad_session {
        crate::evidence::maybe_spawn(&evidence_window, child.as_ref());
        crate::evidence::maybe_spawn_tool_evidence(child.as_ref());
        crate::evidence::maybe_spawn_box_evidence(child.as_ref());
        crate::evidence::maybe_spawn_hover_evidence(child.as_ref());
        // Env-gated, evidence-only 3D-view driver: switches the real session to the
        // 3D view (`SWITCH2D3D`), orbits it with a synthetic primary drag and
        // reports the real `cad_status` reads. Independent of every other driver
        // and OFF on a normal run (`YACR_PROBE_VIEW3D` is unset).
        crate::evidence::maybe_spawn_view3d(child.as_ref());
        // Env-gated, evidence-only modal-popup pipeline driver: opens, proves the
        // host gate refuses input/commands, closes and proves the gate releases. No
        // effect on a normal run (`YACR_PROBE_POPUP_EVIDENCE` is unset).
        crate::evidence::maybe_spawn_popup_evidence(app.handle());
        // Env-gated, evidence-only UI-dialogs driver: drives the React chrome
        // through its own Ctrl+1 handler and command field (Ctrl+1 -> Properties,
        // click-select -> populated rows, DIST -> command echo, OPTIONS -> the real
        // dialog) so an external grab can capture them. No effect on a normal run
        // (`YACR_PROBE_UI_DIALOGS` is unset).
        crate::evidence::maybe_spawn_ui_dialogs(app.handle(), child.as_ref());
    } else {
        eprintln!(
            "[probe] surfaceMode=dom-canvas: native CAD evidence drivers not spawned (no native \
             session)"
        );
    }
    crate::evidence::maybe_spawn_lifecycle(&evidence_window, app.handle());
    Ok(())
}
