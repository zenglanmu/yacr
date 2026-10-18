//! `YACR_PROBE_EVIDENCE`: the synthetic-input + resize/minimize/resize
//! lifecycle driver for the real-window probe run. Split out of `evidence.rs`
//! verbatim (names, env vars, log prefixes and scheduling unchanged).

use std::time::Instant;

use super::schedule;

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
