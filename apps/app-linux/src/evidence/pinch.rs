//! `YACR_PROBE_PINCH`: the trackpad-pinch evidence driver.
//!
//! Evidence-only, default OFF, and independent of every other probe driver. It
//! answers the one question compile/static checks cannot: **does a trackpad pinch
//! reach the CAD session, and does the page stay unscaled?**
//!
//! The gesture is a **synthetic GDK `TOUCHPAD_PINCH` sequence** dispatched with
//! `gtk_main_do_event` on the real WebView widget, so it exercises the real GTK
//! dispatch -> `GtkGestureZoom` -> `NativePinch` queue -> owner tick ->
//! `CadSession::handle_inputs` path. It is **not a real touchpad** (the kernel/
//! libinput/compositor gesture layer is bypassed), and the report line says so.
//! A real trackpad run is still required for a product verdict.
//!
//! What is measured (all real reads, nothing synthesized):
//!
//! * `page_scale` from `window.visualViewport.scale` before/after — a page zoom
//!   (the defect) changes it; the contract says it must stay `1`;
//! * `camera.scale` from `cad_status.camera` before/after — the 2D orthographic
//!   projection scale the host actually applied (the CAD zoom);
//! * `mode` and the host's `error` string, so a refusal is reported as a refusal.
//!
//! Steps: record -> inject begin/update/update/end -> record -> report. The
//! injection is a no-op when the webview/widget/window is not ready (the step log
//! names it) rather than pretending a pinch happened.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use gtk::prelude::*;

/// The raw `eval_with_callback` payloads, in dispatch order (`before`, `after`).
///
/// `eval_with_callback` reports on the GTK/main thread later than the dispatch,
/// so the record steps log their own line when it arrives and the final report
/// reads what really came back instead of re-running a racy eval.
static SAMPLES: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());

/// Whether the pinch evidence driver is enabled (`YACR_PROBE_PINCH`).
pub fn pinch_evidence_enabled() -> bool {
    std::env::var_os("YACR_PROBE_PINCH").is_some()
}

/// Spawn the driver against the live webview (evidence only; default OFF).
///
/// The driver sleeps on its own thread, exactly like the other evidence passes;
/// every DOM/GTK action is marshalled to where it belongs (`eval` for the page,
/// `with_webview` for the widget).
pub fn maybe_spawn_pinch(webview: Option<&tauri::Webview>) {
    if !pinch_evidence_enabled() {
        return;
    }
    let Some(webview) = webview.cloned() else {
        eprintln!("[probe] pinch evidence: no webview; driver not spawned");
        return;
    };
    std::thread::spawn(move || run(&webview));
}

/// One recorded sample: the page scale and the host's real camera block.
#[derive(Debug, Default)]
struct Sample {
    page_scale: Option<f64>,
    dpr: Option<f64>,
    width: Option<f64>,
    camera_scale: Option<f64>,
    mode: Option<String>,
    error: Option<String>,
    done: bool,
}

fn run(webview: &tauri::Webview) {
    let start = Instant::now();
    eprintln!(
        "[probe] pinch evidence: start (synthetic GDK TOUCHPAD_PINCH, NOT a real touchpad; \
         verify a real gesture separately)"
    );
    step(&start, 8000, "before", || record(webview, "before"));
    // The injection runs on the GTK main thread; the pauses between phases are
    // on the driver thread and only exist so the gesture is not one burst.
    step(&start, 9000, "inject", || inject(webview));
    step(&start, 11000, "after", || record(webview, "after"));
    step(&start, 12000, "report", report);
}

/// Sleep to `millis` after `start`, then run one driver action.
fn step(start: &Instant, millis: u64, label: &str, action: impl FnOnce()) {
    let target = Duration::from_millis(millis);
    let elapsed = start.elapsed();
    if target > elapsed {
        std::thread::sleep(target - elapsed);
    }
    eprintln!("[probe] pinch step: {label} -> dispatched");
    action();
}

/// Read `visualViewport.scale` + the real `cad_status` camera into
/// `window.__yacr_pinch[<name>]`, then print the decoded sample.
fn record(webview: &tauri::Webview, name: &'static str) {
    let record_js = RECORD_JS.replace("__NAME__", name);
    let report_js = format!("JSON.stringify((window.__yacr_pinch || {{}}).{name} || null)");
    let Ok(()) = webview.eval(&record_js) else {
        eprintln!("[probe] pinch evidence: {name} record dispatch failed");
        return;
    };
    // `cad_status` resolves asynchronously; read the record after it settles.
    std::thread::sleep(Duration::from_millis(400));
    let Ok(()) = webview.eval_with_callback(&report_js, move |payload| {
        let text = match parse_sample(&payload) {
            Some(sample) => describe(&sample),
            None => format!("no sample (payload={payload:?})"),
        };
        eprintln!("[probe] pinch evidence: {name} {text}");
        if let Ok(mut samples) = SAMPLES.lock() {
            samples.push((name, payload));
        }
    }) else {
        eprintln!("[probe] pinch evidence: {name} report dispatch failed");
        return;
    };
    // Let the callback run before the next step; the report reads the store.
    std::thread::sleep(Duration::from_millis(400));
}

/// Dispatch the synthetic pinch on the GTK main thread, through the real widget.
fn inject(webview: &tauri::Webview) {
    let result = webview.with_webview(|platform| {
        let widget = platform.inner().upcast::<gtk::Widget>();
        // The event must target the widget that owns the pinch gesture (WebKit's
        // internal view), not the outer `WebKitWebView`: an event on an ancestor's
        // window only runs the controllers on that ancestor's chain, so the
        // gesture would never see it.
        let Some((target, window)) = pinch_event_widget(&widget) else {
            eprintln!("[probe] pinch evidence: no realized pinch widget; injection skipped");
            return;
        };
        use gdk::glib::translate::ToGlibPtr;
        let mut user_data: gtk::glib::ffi::gpointer = std::ptr::null_mut();
        unsafe {
            gdk::ffi::gdk_window_get_user_data(window.to_glib_none().0, &mut user_data);
        }
        eprintln!(
            "[probe] pinch evidence: event widget={} owns_window={} window_type={:?} \
             user_data_is_target={}",
            target.type_().name(),
            target.has_window(),
            window.window_type(),
            std::ptr::eq(user_data, target.as_ptr().cast()),
        );
        let x = f64::from(target.allocated_width()) / 2.0;
        let y = f64::from(target.allocated_height()) / 2.0;
        // A real touchpad event carries the seat's pointer device; GTK's gesture
        // controller refuses an event without a source device, so the synthetic
        // one must carry it too (otherwise the injection is silently a no-op).
        let Some(device) = window
            .display()
            .default_seat()
            .and_then(|seat| seat.pointer())
        else {
            eprintln!("[probe] pinch evidence: no pointer device on the seat; injection skipped");
            return;
        };
        // BEGIN, two UPDATEs with an increasing absolute scale, END. GTK turns
        // the absolute `scale` into the incremental deltas `GtkGestureZoom`
        // reports, which is what the host receives as `InputKind::Pinch`.
        for (phase, scale, pause_ms) in [
            (gdk::ffi::GDK_TOUCHPAD_GESTURE_PHASE_BEGIN, 1.0, 60),
            (gdk::ffi::GDK_TOUCHPAD_GESTURE_PHASE_UPDATE, 1.25, 60),
            (gdk::ffi::GDK_TOUCHPAD_GESTURE_PHASE_UPDATE, 1.6, 60),
            (gdk::ffi::GDK_TOUCHPAD_GESTURE_PHASE_END, 1.6, 0),
        ] {
            dispatch_pinch(&window, &device, phase as i8, x, y, scale);
            if pause_ms > 0 {
                std::thread::sleep(Duration::from_millis(pause_ms));
            }
        }
        eprintln!(
            "[probe] pinch evidence: injected 4 synthetic phases at ({x:.1},{y:.1}) \
             scale=1.6 fingers=2"
        );
    });
    if let Err(error) = result {
        eprintln!("[probe] pinch evidence: with_webview failed: {error}");
    }
}

/// The widget that actually receives touchpad pinch events, and its window.
///
/// WebKit stores its zoom gesture as GObject data on its internal view widget
/// (`WebKitWebViewBase`). That widget is the event target; the outer
/// `WebKitWebView` may not own a `GdkWindow` at all, so using its window would
/// dispatch into the ancestor chain and miss the gesture.
fn pinch_event_widget(root: &gtk::Widget) -> Option<(gtk::Widget, gdk::Window)> {
    let key = std::ffi::CString::new("wk-view-zoom-gesture").ok()?;
    let mut found: Option<gtk::Widget> = None;
    walk(root, &mut |widget| {
        // SAFETY: `widget` is a live GObject on this thread and `key` is a valid
        // NUL-terminated string; `g_object_get_data` only reads the association.
        let value = unsafe {
            gtk::glib::gobject_ffi::g_object_get_data(widget.as_ptr().cast(), key.as_ptr())
        };
        if value.is_null() {
            return false;
        }
        found = Some(widget.clone());
        true
    });
    let target = found?;
    let window = target.window()?;
    Some((target, window))
}

/// Depth-first walk of `root` and its `GtkContainer` children.
fn walk(root: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget) -> bool) -> bool {
    if visit(root) {
        return true;
    }
    let Some(container) = root.downcast_ref::<gtk::Container>() else {
        return false;
    };
    container.children().iter().any(|child| walk(child, visit))
}

/// Build one `GDK_TOUCHPAD_PINCH` event and hand it to GTK's dispatcher.
///
/// `gdk::Event` exposes read-only accessors for this subtype, so the fields are
/// written through the raw union. The window reference the event owns is
/// balanced (`g_object_ref` here, `gdk_event_free` on drop).
fn dispatch_pinch(
    window: &gdk::Window,
    device: &gdk::Device,
    phase: i8,
    x: f64,
    y: f64,
    scale: f64,
) {
    use gdk::glib::translate::{ToGlibPtr, ToGlibPtrMut};
    let mut event = gdk::Event::new(gdk::EventType::TouchpadPinch);
    unsafe {
        let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
        let win: *mut gdk::ffi::GdkWindow = window.to_glib_none().0;
        gdk::glib::gobject_ffi::g_object_ref(win as *mut _);
        let pinch = std::ptr::addr_of_mut!((*raw).touchpad_pinch);
        (*pinch).window = win;
        (*pinch).send_event = 1;
        (*pinch).phase = phase;
        (*pinch).n_fingers = 2;
        (*pinch).time = 0;
        (*pinch).x = x;
        (*pinch).y = y;
        (*pinch).dx = 0.0;
        (*pinch).dy = 0.0;
        (*pinch).angle_delta = 0.0;
        (*pinch).scale = scale;
        (*pinch).x_root = x;
        (*pinch).y_root = y;
        (*pinch).state = 0;
    }
    // The device lives in the event's private header (the touchpad struct itself
    // has no device field). Without it GTK refuses to route the pinch to a
    // gesture.
    event.set_device(Some(device));
    event.set_source_device(Some(device));
    eprintln!(
        "[probe] pinch evidence: dispatch window={} device={} source_device={} seat={}",
        event.window().is_some(),
        event.device().is_some(),
        event.source_device().is_some(),
        event.seat().is_some(),
    );
    gtk::main_do_event(&mut event);
}

/// Print the final report: page scale (must be unchanged) and camera scale.
fn report() {
    let samples = SAMPLES
        .lock()
        .map(|samples| samples.clone())
        .unwrap_or_default();
    let find = |name: &str| {
        samples
            .iter()
            .find(|(label, _)| *label == name)
            .and_then(|(_, payload)| parse_sample(payload))
    };
    let (Some(before), Some(after)) = (find("before"), find("after")) else {
        eprintln!(
            "[probe] pinch evidence: report missing samples (stored={:?})",
            samples.iter().map(|(name, _)| *name).collect::<Vec<_>>()
        );
        return;
    };
    let page_changed = !same_number(before.page_scale, after.page_scale);
    let camera_changed = !same_number(before.camera_scale, after.camera_scale);
    eprintln!(
        "[probe] pinch evidence: report synthetic=1 page_scale {} -> {} (changed={page_changed}) \
         camera_scale {} -> {} (changed={camera_changed}) mode {} -> {} error_before={:?} \
         error_after={:?}",
        fmt(before.page_scale),
        fmt(after.page_scale),
        fmt(before.camera_scale),
        fmt(after.camera_scale),
        before.mode.as_deref().unwrap_or("null"),
        after.mode.as_deref().unwrap_or("null"),
        before.error,
        after.error,
    );
    if page_changed {
        eprintln!(
            "[probe] pinch evidence: FAIL page scaled by the synthetic pinch \
             (the WebKit gesture was not neutralised)"
        );
    }
}

/// Parse one `window.__yacr_pinch[<name>]` JSON object into a [`Sample`].
fn parse_sample(payload: &str) -> Option<Sample> {
    decode_json(payload).and_then(|value| parse_object(&value))
}

fn parse_object(value: &serde_json::Value) -> Option<Sample> {
    let object = value.as_object()?;
    let camera = object.get("camera").and_then(|camera| camera.as_object());
    Some(Sample {
        page_scale: object.get("page_scale").and_then(serde_json::Value::as_f64),
        dpr: object.get("dpr").and_then(serde_json::Value::as_f64),
        width: object.get("width").and_then(serde_json::Value::as_f64),
        camera_scale: camera
            .and_then(|camera| camera.get("scale"))
            .and_then(serde_json::Value::as_f64),
        mode: object
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        error: object
            .get("error")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        done: object
            .get("done")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

/// `eval_with_callback` may hand back a JSON-encoded string or the raw text.
fn decode_json(payload: &str) -> Option<serde_json::Value> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('"') {
        let inner: String = serde_json::from_str(trimmed).ok()?;
        return serde_json::from_str(&inner).ok();
    }
    serde_json::from_str(trimmed).ok()
}

fn same_number(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => (left - right).abs() < 1e-9,
        (None, None) => true,
        _ => false,
    }
}

fn fmt(value: Option<f64>) -> String {
    value.map_or_else(|| "null".to_string(), |value| format!("{value:.6}"))
}

fn describe(sample: &Sample) -> String {
    format!(
        "page_scale={} dpr={} width={} camera_scale={} mode={} error={:?} done={}",
        fmt(sample.page_scale),
        fmt(sample.dpr),
        fmt(sample.width),
        fmt(sample.camera_scale),
        sample.mode.as_deref().unwrap_or("null"),
        sample.error,
        sample.done,
    )
}

/// Record `visualViewport.scale` and the real `cad_status` camera under `__NAME__`.
const RECORD_JS: &str = r#"
(function () {
  var d = window.__yacr_pinch = window.__yacr_pinch || {};
  var name = '__NAME__';
  var viewport = window.visualViewport;
  d[name] = {
    page_scale: viewport ? viewport.scale : null,
    dpr: window.devicePixelRatio,
    width: window.innerWidth,
    camera: null,
    mode: null,
    error: null,
    done: false
  };
  var internals = window.__TAURI_INTERNALS__;
  if (!internals || typeof internals.invoke !== 'function') return 'no-tauri-internals';
  internals.invoke('cad_status').then(function (status) {
    d[name].camera = status && status.camera
      ? { scale: status.camera.scale, center: status.camera.center }
      : null;
    d[name].mode = status && status.view ? status.view.mode : null;
    d[name].error = (status && status.error) || null;
    d[name].done = true;
  }, function (error) {
    d[name].error = String(error);
    d[name].done = true;
  });
  return 'ok';
})()
"#;
