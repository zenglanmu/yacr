//! Tauri setup: build the transparent top-level window, its child WebView2 and
//! the native CAD child HWND.
//!
//! Transparent-composition contract (design F.1):
//! - window `transparent(true)` **and** `background_color(rgba 0,0,0,0)`;
//! - WebView2 `transparent(true)` + `background_color(rgba 0,0,0,0)`, then a
//!   startup read-back of `ICoreWebView2Controller2::DefaultBackgroundColor` so a
//!   silent wry clamp (any non-zero alpha becomes `255`) cannot masquerade as a
//!   working transparent layer;
//! - `WS_EX_NOREDIRECTIONBITMAP` requested **at creation** through Tauri's
//!   `WindowBuilder::no_redirection_bitmap(true)` (tauri ≥ 2.12) and verified by
//!   reading the created window's ex-style back. Win32 honours this style only at
//!   creation, so there is no post-creation `SetWindowLongPtrW` path any more;
//! - **no** `WS_EX_LAYERED`, no `set_ignore_cursor_events`.
//!
//! The CAD child HWND is created **after** the WebView2 container exists so the
//! explicit `SetWindowPos(cad, container, …)` has a sibling to sit below.

use std::sync::{Arc, Mutex, OnceLock};

use cad_host::HostOptions;
use cad_platform::ConfigStore;
use cad_ui_core::i18n::{host_locale_script, host_locale_tag};
use cad_ui_core::unsupported_status;
use windows::core::Interface;

use super::popup::install_renderer_crash_hook;
use super::{
    close_presenter, lock, lock_telemetry, register_seams, request_stop, CadEnv, RenderTelemetry,
    Shared, SURFACE, WEBVIEW_LABEL, WINDOW_LABEL,
};
use crate::support::TRANSPARENT_BACKGROUND;

/// Build the window, the child webview, the WebView2 hooks and the CAD surface.
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
        .transparent(true)
        .background_color(tauri::window::Color(
            TRANSPARENT_BACKGROUND.0,
            TRANSPARENT_BACKGROUND.1,
            TRANSPARENT_BACKGROUND.2,
            TRANSPARENT_BACKGROUND.3,
        ))
        // Create hidden: the ex-style and the webview must exist before the
        // first composited frame, otherwise a white/black flash is unavoidable.
        .visible(false)
        // `WS_EX_NOREDIRECTIONBITMAP` is a **creation-time-only** ex-style: it can
        // only be requested while the window is created. This used to be a
        // post-creation `SetWindowLongPtrW`, which always fails
        // `ERROR_INVALID_PARAMETER`; tauri 2.12 exposes the builder option, so the
        // flag is set where Win32 honours it. The read-back below verifies it.
        .no_redirection_bitmap(true)
        .maximized(maximize)
        .build()?;

    let hwnd = window
        .hwnd()
        .map_err(|error| format!("window hwnd unavailable: {error}"))?;
    // Verification, not application: the flag was requested at creation above.
    // A runtime that ignored it is reported explicitly (the window is still
    // usable — with a possible creation flash — so this stays diagnostic).
    match crate::win32_surface::no_redirection_bitmap_state(hwnd) {
        crate::win32_surface::NoRedirectionBitmapState::Set => eprintln!(
            "[probe] win32 ex-style: WS_EX_NOREDIRECTIONBITMAP set \
             (creation-time, tauri no_redirection_bitmap)"
        ),
        crate::win32_surface::NoRedirectionBitmapState::Absent {
            ex_style,
            last_error,
        } => eprintln!(
            "[probe] win32 ex-style: WS_EX_NOREDIRECTIONBITMAP FAILED: WIN32_ERROR({last_error}) \
             (absent from the created window, ex-style {ex_style:#x}; the runtime ignored the \
             creation-time request)"
        ),
    }

    // Use the window's real size for the child webview, not the initial
    // constants (the WM may already have adjusted the window).
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
        // Alpha is exactly 0 by contract; wry clamps any other value to 255.
        .background_color(tauri::webview::Color(
            TRANSPARENT_BACKGROUND.0,
            TRANSPARENT_BACKGROUND.1,
            TRANSPARENT_BACKGROUND.2,
            TRANSPARENT_BACKGROUND.3,
        ))
        // Keep the WebView2 controller bounds proportional to the window client
        // area; the CAD child is sized independently from the accepted DOM rect.
        .auto_resize()
        // The host-resolved locale, handed to the chrome **before** the document
        // is parsed: the single system default for the React UI and the native
        // dialog labels (see `cad_ui_core::i18n`).
        .initialization_script(initialization_script())
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

    // Anti-clamp read-back and trigger ③ (renderer crash) both need the live
    // WebView2 controller, which only exists after the child is built.
    verify_transparent_background(&webview, shared);
    install_renderer_crash_hook(&webview, shared);

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
        }
        tauri::WindowEvent::Moved(position) => {
            eprintln!(
                "[probe] WM event: window moved to {},{}",
                position.x, position.y
            );
        }
        tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            eprintln!("[probe] WM event: scale factor {scale_factor}");
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

    // Register the app-side seams before any IPC command can run.
    register_seams(app.handle(), shared, telemetry);

    let no_gpu = lock_telemetry(telemetry).no_gpu;
    lock(shared).window = Some(window.clone());

    // ---- CAD platform composition (main thread; `Rc`, kept in a TLS) ----
    let launch = lock(shared).launch.clone();
    if !launch.probe_only {
        // Same resolved tag as the injected `window.__YACR_HOST_LOCALE__`:
        // native labels and chrome cannot pick different languages.
        match crate::platform::ReactFiles::new(host_locale_tag()) {
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
        // The child HWND is a sibling **below** the wry container; there is no
        // top-level fallback, so an install failure is explicit.
        match crate::win32_surface::Win32CadSurface::install(&window, gpu_time) {
            Ok(surface) => {
                SURFACE.with(|cell| *cell.borrow_mut() = Some(surface));
                eprintln!("[probe] win32 CAD child installed (display-only, below WebView2)");
            }
            Err(error) => {
                eprintln!(
                    "[probe] win32 CAD child install FAILED: {error} (no top-level fallback)"
                );
                lock(shared).shell.status.error =
                    Some(format!("win32 CAD surface install failed: {error}"));
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

/// Read the WebView2 background alpha back and refuse to call a clamped value
/// transparent.
///
/// wry maps any non-zero alpha to `255` without an error, so the pinned `0` is
/// asserted at runtime, not just in the unit test. A mismatch is repaired once
/// (`SetDefaultBackgroundColor`) and re-read; if it still disagrees, the host
/// records an explicit composition error instead of pretending the DOM layer is
/// transparent.
fn verify_transparent_background(webview: &tauri::Webview, shared: &Arc<Mutex<Shared>>) {
    let task_shared = shared.clone();
    let result = webview.with_webview(move |platform| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2Controller2, COREWEBVIEW2_COLOR,
        };

        let expected = COREWEBVIEW2_COLOR {
            A: TRANSPARENT_BACKGROUND.3,
            R: TRANSPARENT_BACKGROUND.0,
            G: TRANSPARENT_BACKGROUND.1,
            B: TRANSPARENT_BACKGROUND.2,
        };
        let controller2 = match platform.controller().cast::<ICoreWebView2Controller2>() {
            Ok(controller2) => controller2,
            Err(error) => {
                record_composition_error(
                    &task_shared,
                    format!("webview2 background read-back unavailable: {error}"),
                );
                return;
            }
        };
        let mut color = COREWEBVIEW2_COLOR::default();
        let alpha = match unsafe { controller2.DefaultBackgroundColor(&mut color) } {
            Ok(()) => color.A,
            Err(error) => {
                record_composition_error(
                    &task_shared,
                    format!("webview2 background read-back failed: {error}"),
                );
                return;
            }
        };
        if alpha == expected.A {
            eprintln!(
                "[probe] webview2 background alpha read-back={alpha} (transparent contract holds)"
            );
            return;
        }
        eprintln!(
            "[probe] webview2 background alpha read-back={alpha} expected={} (clamp suspected; repairing)",
            expected.A
        );
        if let Err(error) = unsafe { controller2.SetDefaultBackgroundColor(expected) } {
            record_composition_error(
                &task_shared,
                format!("webview2 background clamp repair failed: {error}"),
            );
            return;
        }
        let mut repaired = COREWEBVIEW2_COLOR::default();
        match unsafe { controller2.DefaultBackgroundColor(&mut repaired) } {
            Ok(()) if repaired.A == expected.A => {
                eprintln!("[probe] webview2 background alpha repaired to 0");
            }
            Ok(()) => record_composition_error(
                &task_shared,
                format!(
                    "webview2 background alpha stayed {} after repair (expected 0)",
                    repaired.A
                ),
            ),
            Err(error) => record_composition_error(
                &task_shared,
                format!("webview2 background re-read failed: {error}"),
            ),
        }
    });
    if let Err(error) = result {
        record_composition_error(
            shared,
            format!("webview2 with_webview FAILED (background read-back): {error}"),
        );
    }
}

/// Record an explicit composition failure (diagnostic + shell status).
fn record_composition_error(shared: &Arc<Mutex<Shared>>, text: String) {
    eprintln!("[probe] {text}");
    let mut state = lock(shared);
    if state.shell.status.error.is_none() {
        state.shell.status.error = Some(text);
    }
}

/// Whether `YACR_PROBE_DOM_OVERLAY` is present (diagnostic only).
///
/// V0 discriminator: does the DOM/WebView2 chain actually alpha-blend the chrome
/// in this environment? The VM's "flat" result and the laptop's see-through result
/// may be two different sub-failures; this paints the CAD canvas host magenta at
/// 50 % so a capture answers the DOM half independently of the native surface.
fn dom_overlay_requested() -> bool {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    *REQUESTED.get_or_init(|| std::env::var_os("YACR_PROBE_DOM_OVERLAY").is_some())
}

/// The webview initialization scripts: the single host-resolved locale script,
/// plus the diagnostic DOM overlay when its knob is present.
fn initialization_script() -> String {
    let mut script = host_locale_script();
    if dom_overlay_requested() {
        eprintln!(
            "[probe] dom-overlay active: .workspace__canvas painted rgba(255,0,255,0.5) \
             (diagnostic; not acceptance evidence)"
        );
        script.push_str(DOM_OVERLAY_SCRIPT);
    }
    script
}

/// V0 overlay probe (diagnostic only, never a product path).
///
/// Inline style only: no product CSS, no i18n key, no behaviour change — the
/// canvas host is painted half-transparent magenta once the document exists.
const DOM_OVERLAY_SCRIPT: &str = r#"
(() => {
  const paint = () => {
    const el = document.querySelector('.workspace__canvas');
    if (el) {
      el.style.background = 'rgba(255,0,255,0.5)';
    }
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', paint);
  } else {
    paint();
  }
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The knob's payload is pinned: the CAD canvas host selector and the exact
    /// half-transparent magenta fill, so a capture difference is attributable to
    /// the DOM/blend path and nothing else.
    #[test]
    fn dom_overlay_script_paints_the_canvas_host_half_transparent_magenta() {
        assert!(DOM_OVERLAY_SCRIPT.contains(".workspace__canvas"));
        assert!(DOM_OVERLAY_SCRIPT.contains("rgba(255,0,255,0.5)"));
        assert!(
            DOM_OVERLAY_SCRIPT.contains("DOMContentLoaded"),
            "it must survive being injected before the document exists"
        );
    }
}
