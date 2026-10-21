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
use super::{lock, CadEnv, Shared, BRIDGE, CAD_ENV, WEBVIEW};

/// Build the probe window, its child webview and the GPU presenter.
///
/// Runs on the main thread during Tauri `setup`.
pub fn init(app: &mut tauri::App, shared: &Arc<Mutex<Shared>>) -> tauri::Result<()> {
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
        // The host-resolved locale, handed to the chrome **before** the document
        // is parsed: this is the single system default both the native dialog
        // labels and the React UI use (see `host::locale`).
        .initialization_script(host_locale_script())
        .on_page_load(|_webview, payload| {
            eprintln!(
                "[probe] page_load url={} event={:?}",
                payload.url(),
                payload.event()
            );
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

    // Window events: log geometry (WM-native evidence) and release the surface
    // before the window goes away.
    let events_shared = shared.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Resized(size) => {
            eprintln!(
                "[probe] WM event: window resized to {}x{}",
                size.width, size.height
            );
            // The child webview does not follow the window by itself (see
            // `sync_webview_size`): apply the new inner size now, alongside the
            // GTK layout pass the resize already triggered.
            crate::gtk_bridge::sync_webview_size((size.width, size.height), "window-resize");
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
            // waiting for the next frame.
            crate::gtk_bridge::sync_webview_size(
                (new_inner_size.width, new_inner_size.height),
                "window-scale",
            );
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
            request_stop(&events_shared);
        }
        _ => {}
    });

    let evidence_window = window.clone();
    let no_gpu = lock(shared).no_gpu;
    lock(shared).window = Some(window.clone());
    lock(shared).app_handle = Some(app.handle().clone());

    // ---- CAD platform composition (main thread; `Rc`, kept in a TLS) ----
    let launch = lock(shared).launch.clone();
    if !launch.probe_only {
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

    if no_gpu {
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
    let wake_pending = lock(shared).wake_pending.clone();
    let pump = crate::redraw::RedrawPump::spawn(app.handle(), wake_pending);
    lock(shared).redraw_pump = Some(pump);

    // Env-gated, evidence-only lifecycle driver (no effect on normal runs).
    crate::evidence::maybe_spawn(&evidence_window, child.as_ref());
    crate::evidence::maybe_spawn_tool_evidence(child.as_ref());
    crate::evidence::maybe_spawn_box_evidence(child.as_ref());
    crate::evidence::maybe_spawn_hover_evidence(child.as_ref());
    // Env-gated, evidence-only 3D-view driver: switches the real session to the
    // 3D view (`SWITCH2D3D`), orbits it with a synthetic primary drag and reports
    // the real `cad_status` reads. Independent of every other driver and OFF on a
    // normal run (`YACR_PROBE_VIEW3D` is unset).
    crate::evidence::maybe_spawn_view3d(child.as_ref());
    crate::evidence::maybe_spawn_lifecycle(&evidence_window, app.handle());
    // Env-gated, evidence-only modal-popup pipeline driver: opens, proves the host
    // gate refuses input/commands, closes and proves the gate releases. No effect
    // on a normal run (`YACR_PROBE_POPUP_EVIDENCE` is unset).
    crate::evidence::maybe_spawn_popup_evidence(app.handle());
    Ok(())
}
