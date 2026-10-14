//! Env-gated, evidence-only lifecycle driver for the real-window probe run.
//!
//! Enabled only when `YACR_PROBE_EVIDENCE` is truthy; a normal run is
//! unaffected. On a background thread it schedules (relative to driver start):
//! - synthetic input injected through the real webview (`Webview::eval`),
//!   explicitly **synthetic, not a real mouse**;
//! - a window resize;
//! - a minimize, then an unminimize (restore);
//! - a final resize.
//!
//! The thread only calls public `Window`/`Webview` dispatch methods. The GPU
//! presenter still runs on the main-thread tick, and all GPU work stays inside
//! `SurfaceProbePresenter`; this module touches no GPU state.

use std::time::{Duration, Instant};

/// Whether the evidence driver is enabled.
pub fn enabled() -> bool {
    matches!(
        std::env::var("YACR_PROBE_EVIDENCE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Schedule the evidence lifecycle if enabled. Never panics on spawn failure.
pub fn maybe_spawn(window: &tauri::Window, webview: Option<&tauri::Webview>) {
    if !enabled() {
        return;
    }
    let Some(webview) = webview else {
        eprintln!("[probe] evidence driver: no child webview; synthetic input skipped");
        return;
    };
    eprintln!("[probe] evidence driver enabled (YACR_PROBE_EVIDENCE)");
    let window = window.clone();
    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-evidence".into())
        .spawn(move || run(window, webview))
    {
        eprintln!("[probe] evidence driver failed to spawn: {error}");
    }
}

fn run(window: tauri::Window, webview: tauri::Webview) {
    let start = Instant::now();
    schedule(
        &start,
        8,
        "synthetic input (eval; NOT a real mouse)",
        || webview.eval(SYNTHETIC_INPUT_JS),
    );
    schedule(&start, 18, "resize grow to 1200x800", || {
        let before = window.inner_size().map(|size| (size.width, size.height));
        let result = window.set_size(tauri::PhysicalSize::new(1200, 800));
        eprintln!("[probe] resize grow before={before:?} result={result:?}");
        result
    });
    schedule(&start, 28, "minimize", || window.minimize());
    schedule(&start, 34, "unminimize (restore)", || window.unminimize());
    schedule(&start, 40, "resize shrink to 800x500", || {
        let before = window.inner_size().map(|size| (size.width, size.height));
        let result = window.set_size(tauri::PhysicalSize::new(800, 500));
        eprintln!("[probe] resize shrink before={before:?} result={result:?}");
        result
    });
}

/// Sleep until `seconds` after `start`, then run `action` and log the result.
fn schedule(
    start: &Instant,
    seconds: u64,
    label: &str,
    action: impl FnOnce() -> tauri::Result<()>,
) {
    let target = Duration::from_secs(seconds);
    let elapsed = start.elapsed();
    if target > elapsed {
        std::thread::sleep(target - elapsed);
    }
    match action() {
        Ok(()) => eprintln!("[probe] evidence step: {label} -> ok"),
        Err(error) => eprintln!("[probe] evidence step: {label} -> error: {error}"),
    }
}

/// Whether the lifecycle driver is enabled and its mode.
fn lifecycle_mode() -> Option<String> {
    match std::env::var("YACR_PROBE_LIFECYCLE") {
        Ok(value) => {
            let value = value.trim().to_lowercase();
            if value.is_empty() || value == "0" || value == "false" {
                None
            } else {
                Some(value)
            }
        }
        Err(_) => None,
    }
}

/// Schedule the evidence-only **lifecycle** driver if enabled. Unlike the input
/// driver this drives real window/resize/minimize/close/exit plus a forced
/// overlay unrealize→realize; it is env-gated and never runs on a normal path.
pub fn maybe_spawn_lifecycle(window: &tauri::Window, app: &tauri::AppHandle) {
    let Some(mode) = lifecycle_mode() else {
        return;
    };
    eprintln!("[probe] lifecycle driver enabled mode={mode}");
    let window = window.clone();
    let app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-lifecycle".into())
        .spawn(move || run_lifecycle(window, app, mode))
    {
        eprintln!("[probe] lifecycle driver failed to spawn: {error}");
    }
}

fn run_lifecycle(window: tauri::Window, app: tauri::AppHandle, mode: String) {
    let start = Instant::now();

    // `hold`: minimize once and do NOTHING else (no Tauri unminimize/set_focus/
    // show). The external harness then restores the window via a real WM/OS path
    // (_NET_ACTIVE_WINDOW / task switch), so the restore is not an app API call.
    if mode == "hold" {
        let w_min = window.clone();
        schedule(
            &start,
            8,
            "minimize (hold; no app-side restore)",
            move || w_min.minimize(),
        );
        let w_close = window.clone();
        schedule(&start, 45, "window.close()", move || w_close.close());
        return;
    }

    let do_resize = matches!(mode.as_str(), "full" | "resize" | "close" | "exit");
    let do_minimize = matches!(mode.as_str(), "full" | "minimize");
    let do_unrealize = matches!(mode.as_str(), "full" | "unrealize");

    if do_resize {
        let window = window.clone();
        schedule(&start, 8, "resize grow to 1200x800", move || {
            let before = window.inner_size().map(|size| (size.width, size.height));
            let result = window.set_size(tauri::PhysicalSize::new(1200, 800));
            eprintln!("[probe] resize grow before={before:?} result={result:?}");
            result
        });
    }
    if do_minimize {
        let w_min = window.clone();
        schedule(&start, 12, "minimize", move || w_min.minimize());
        let w_unmin = window.clone();
        schedule(&start, 15, "unminimize (restore)", move || {
            w_unmin.unminimize()
        });
        if mode == "minimize" {
            // Diagnostic: try the usual extra restore remedies.
            let w_focus = window.clone();
            schedule(&start, 17, "set_focus", move || w_focus.set_focus());
            let w_show = window.clone();
            schedule(&start, 19, "show", move || w_show.show());
        }
    }
    if do_resize {
        let window = window.clone();
        schedule(&start, 19, "resize shrink to 900x560", move || {
            let before = window.inner_size().map(|size| (size.width, size.height));
            let result = window.set_size(tauri::PhysicalSize::new(900, 560));
            eprintln!("[probe] resize shrink before={before:?} result={result:?}");
            result
        });
    }
    if do_unrealize {
        schedule_main(&start, 24, &app, "force overlay unrealize", || {
            crate::host::force_unrealize()
        });
        schedule_main(&start, 26, &app, "force overlay realize/map", || {
            crate::host::force_realize()
        });
    }
    if mode == "exit" {
        let app = app.clone();
        schedule(&start, 32, "app.exit(0)", move || {
            app.exit(0);
            Ok(())
        });
    } else {
        let window = window.clone();
        schedule(&start, 32, "window.close()", move || window.close());
    }
}

/// Sleep until `seconds` after `start`, then run a main-thread `action` and log
/// its boolean result (a dispatch that merely posts is not a pass).
fn schedule_main(
    start: &Instant,
    seconds: u64,
    app: &tauri::AppHandle,
    label: &str,
    action: impl FnOnce() -> bool + Send + 'static,
) {
    let target = Duration::from_secs(seconds);
    let elapsed = start.elapsed();
    if target > elapsed {
        std::thread::sleep(target - elapsed);
    }
    let label = label.to_string();
    let label_in = label.clone();
    match app.run_on_main_thread(move || {
        let ok = action();
        eprintln!("[probe] lifecycle step: {label_in} -> ok={ok}");
    }) {
        Ok(()) => {}
        Err(error) => eprintln!("[probe] lifecycle step: {label} -> dispatch error: {error}"),
    }
}

/// Synthetic input script. It dispatches DOM pointer/wheel events that the
/// frontend forwards over the real IPC bridge. Pointer capture is shimmed
/// because these events are not backed by a real device.
const SYNTHETIC_INPUT_JS: &str = r#"
(function () {
  try {
    var el = document.querySelector('.native-input');
    if (!el) { return 'no-element'; }
    el.setPointerCapture = function () {};
    el.hasPointerCapture = function () { return false; };
    el.releasePointerCapture = function () {};
    function fire(type, x) {
      el.dispatchEvent(new PointerEvent(type, {
        pointerId: 1, pointerType: 'mouse', isPrimary: true,
        clientX: x, clientY: 140, bubbles: true, cancelable: true
      }));
    }
    fire('pointerdown', 60);
    fire('pointermove', 200);
    fire('pointermove', 340);
    fire('pointerup', 380);
    el.dispatchEvent(new WheelEvent('wheel', {
      deltaX: 0, deltaY: 240, deltaMode: 0,
      clientX: 220, clientY: 140, bubbles: true, cancelable: true
    }));
    return 'injected';
  } catch (e) { return 'error:' + e; }
})()
"#;
