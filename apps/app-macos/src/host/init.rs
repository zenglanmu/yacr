//! Tauri setup: build the transparent top-level window, its child WKWebView and
//! the native CAD child `NSView`.
//!
//! Transparent-composition contract (the same direction as Linux/Windows):
//! - window `transparent(true)` (AppKit `opaque = false` + clear background);
//! - WKWebView `transparent(true)` (wry sets `drawsBackground = NO`), so the
//!   React chrome blends with the CAD child **below** it;
//! - the CAD `NSView` is installed **after** the webview exists, because the
//!   WKWebView is its ordering anchor; it is display-only (no responder
//!   overrides, no gesture recognizers) and is kept directly below the webview by
//!   [`crate::macos_surface`];
//! - no `set_ignore_cursor_events` equivalent and no second window.
//!
//! Unlike Windows there is **no runtime pre-check**: WKWebView and Metal ship
//! with the OS, so there is no "missing runtime" refusal to make.

use std::sync::{Arc, Mutex};

use cad_host::HostOptions;
use cad_platform::ConfigStore;
use cad_ui_core::i18n::{host_locale_script, host_locale_tag};
use cad_ui_core::unsupported_status;

use super::popup::install_renderer_crash_hook;
use super::{
    close_presenter, lock, lock_telemetry, register_seams, request_stop, CadEnv, RenderTelemetry,
    Shared, SURFACE, WEBVIEW_LABEL, WINDOW_LABEL,
};

/// Build the window, the child webview, the lifecycle hooks and the CAD surface.
///
/// Runs on the main thread during Tauri `setup`.
pub fn init(
    app: &mut tauri::App,
    shared: &Arc<Mutex<Shared>>,
    telemetry: &Arc<Mutex<RenderTelemetry>>,
) -> Result<(), Box<dyn std::error::Error>> {
    crate::main_thread::record_main_thread();

    // Default: open maximized (the CAD partition should use the whole screen).
    // `YACR_PROBE_NO_MAXIMIZE` (acceptance/evidence only) opts out; its presence,
    // not its value, is the signal.
    let maximize = std::env::var_os("YACR_PROBE_NO_MAXIMIZE").is_none();
    let window = tauri::window::WindowBuilder::new(app, WINDOW_LABEL)
        .title("Yacr")
        .inner_size(
            f64::from(cad_ui_core::launch::INITIAL_WIDTH),
            f64::from(cad_ui_core::launch::INITIAL_HEIGHT),
        )
        .min_inner_size(320.0, 240.0)
        .resizable(true)
        // The DOM chrome has translucent panels over the CAD region; the window
        // must be non-opaque so those pixels reveal the native child view below
        // the WKWebView (same composition direction as the Windows/Linux hosts).
        .transparent(true)
        // Create hidden: the webview and CAD view must exist before the first
        // composited frame, otherwise a white/clear flash is unavoidable.
        .visible(false)
        .maximized(maximize)
        .build()?;

    // Use the window's real size for the child webview, not the initial
    // constants (the window manager may already have adjusted the window).
    let initial_size = window.inner_size().unwrap_or(tauri::PhysicalSize::new(
        cad_ui_core::launch::INITIAL_WIDTH,
        cad_ui_core::launch::INITIAL_HEIGHT,
    ));

    let webview = window.add_child(
        tauri::webview::WebviewBuilder::new(
            WEBVIEW_LABEL,
            tauri::WebviewUrl::App("index.html".into()),
        )
        .transparent(true)
        // wry's macOS **child** webview sets `ViewMinYMargin` (fixed size), so
        // `auto_resize()` is a no-op there; the window event below keeps the DOM
        // viewport equal to the client area explicitly. The CAD child view is
        // framed independently from the accepted DOM rect.
        // The host-resolved locale, handed to the chrome **before** the document
        // is parsed: the single system default for the React UI and the native
        // dialog labels (see `cad_ui_core::i18n`).
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
                    cad_desktop_shell::popup::on_reload(&page_load_shared);
                }
            }
        }),
        tauri::PhysicalPosition::new(0, 0),
        tauri::PhysicalSize::new(initial_size.width.max(1), initial_size.height.max(1)),
    )?;

    // Renderer-crash lifecycle trigger ④ has no Tauri macOS hook; the seam logs
    // the documented gap instead of fabricating a crash notification.
    install_renderer_crash_hook(&webview, shared);

    // Window events: log geometry (native evidence) and release the surface
    // before the window goes away.
    let events_shared = shared.clone();
    let events_telemetry = telemetry.clone();
    let events_webview = webview.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Resized(size) => {
            // wry's macOS child webview is created with `ViewMinYMargin` (fixed),
            // so it does not follow the window on its own. Keep the single DOM
            // viewport equal to the window client area; the CAD child view has
            // its own geometry authority (the accepted React rect).
            if let Err(error) =
                events_webview.set_size(tauri::PhysicalSize::new(size.width, size.height))
            {
                eprintln!("[probe] NSWindow event: child webview resize failed: {error}");
            }
            eprintln!(
                "[probe] NSWindow event: window resized to {}x{}",
                size.width, size.height
            );
        }
        tauri::WindowEvent::Moved(position) => {
            eprintln!(
                "[probe] NSWindow event: window moved to {},{}",
                position.x, position.y
            );
        }
        tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            eprintln!("[probe] NSWindow event: scale factor {scale_factor}");
        }
        tauri::WindowEvent::Focused(focused) => {
            eprintln!("[probe] NSWindow event: focused={focused}");
        }
        tauri::WindowEvent::CloseRequested { .. } => {
            eprintln!("[probe] NSWindow event: close requested; closing surface first");
            close_presenter();
        }
        tauri::WindowEvent::Destroyed => {
            eprintln!("[probe] NSWindow event: destroyed; requesting stop");
            request_stop(&events_shared, &events_telemetry);
        }
        _ => {}
    });

    // Register the app-side seams before any IPC command can run.
    register_seams(app.handle(), shared, telemetry);

    let no_gpu = lock_telemetry(telemetry).no_gpu;
    lock(shared).window = Some(window.clone());

    // ---- CAD platform composition (main thread; `Rc`, kept in a TLS) ----
    let launch = lock(shared).launch.clone();
    if !launch.probe_only {
        // Same resolved tag as the injected `window.__YACR_HOST_LOCALE__`:
        // native labels and chrome cannot pick different languages.
        match crate::platform::ReactFiles::new(host_locale_tag(), app.handle().clone()) {
            Ok(files) => {
                let (config, preference) = crate::platform::config_paths();
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
                super::CAD_ENV.with(|cell| *cell.borrow_mut() = Some(env));
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
        eprintln!("[probe] YACR_PROBE_NO_GPU: CAD child surface skipped (React-only baseline)");
    } else {
        // Measurement-only GPU frame-time opt-in (ADR 0013 class 1). This is the
        // only place the flag is read; off by default (product configuration).
        let gpu_time = cad_render_wgpu::gpu_time::GpuTimeConfig {
            enabled: std::env::var_os("YACR_PROBE_GPU_TIME").is_some(),
        };
        if gpu_time.enabled {
            eprintln!(
                "[probe] YACR_PROBE_GPU_TIME: measurement-only GPU frame-time opt-in enabled \
                 (the readback perturbs the frame; not a product configuration)"
            );
        }
        // The child view is a sibling **below** the WKWebView; there is no
        // top-level fallback, so an install failure is explicit.
        match crate::macos_surface::MacosCadSurface::install(&window, gpu_time) {
            Ok(surface) => {
                SURFACE.with(|cell| *cell.borrow_mut() = Some(surface));
                eprintln!("[probe] macos CAD child installed (display-only, below WKWebView)");
            }
            Err(error) => {
                eprintln!("[probe] macos CAD child install FAILED: {error} (no fallback)");
                lock(shared).shell.status.error =
                    Some(format!("macos CAD surface install failed: {error}"));
            }
        }
    }

    // Keep the single event loop awake so the host renders continuously.
    let wake_pending = lock_telemetry(telemetry).wake_pending.clone();
    let pump = crate::redraw::RedrawPump::spawn(app.handle(), wake_pending);
    lock_telemetry(telemetry).redraw_pump = Some(pump);

    // Only now is the composited first frame possible; showing earlier is what
    // produces the creation flash.
    window.show()?;
    let _ = window.set_focus();
    Ok(())
}
