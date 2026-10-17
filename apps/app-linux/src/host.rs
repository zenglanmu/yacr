//! Tauri host wiring: window + child webview + the GPU probe lifecycle.
//!
//! The probe is driven from the **Tauri event loop** (`RunEvent::MainEventsCleared`)
//! on the main thread; there is no second event loop.
//!
//! Ownership and threading (safety corrections):
//! - The GPU presenter is **main-thread-local** (a `thread_local` `RefCell`), not
//!   in the `Arc<Mutex<Shared>>`: IPC commands run on worker threads and hold
//!   clones of that Arc, so a GPU object stored there could be dropped off the
//!   main thread. It is created, used and destroyed only on the main thread.
//! - The shared state is **GPU-free**: only status and the input tracker cross
//!   threads, and the shared lock is never held across GPU work.
//! - The surface is explicitly stopped and released on `CloseRequested` and on
//!   `ExitRequested`/`Exit` (surface before window), not only after `Destroyed`
//!   (which arrives after the window is already gone, with no tick afterwards).
//!
//! Lifecycle:
//! - resize: the surface follows the window's physical size; a zero-size window
//!   (minimized/unmapped) **pauses** instead of reconfiguring to zero;
//! - minimize/restore: detected from the window state each tick and resumed when
//!   a non-zero size returns;
//! - GPU failure: recorded in [`ProbeStatus::error`], never a fabricated frame.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cad_engine::render::{
    BackendPreference, FrameCompleteness, RenderDiagnostics, RenderSessionHandle,
};
use cad_host::{HostOptions, HostPlatform};
use cad_platform::{ConfigStore, HostCapabilities, PlatformFiles};
use cad_render_wgpu::ProbeStageTimings;
use cad_render_wgpu::PulledState;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

use cad_ui_core::layout::{surface_metrics, LayoutError, LayoutTracker};
use cad_ui_core::protocol::{InputKind, ProbeInput, ProbeInputAck, ProbeStatus};
use cad_ui_core::session::{CadSession, CadStatus};
use cad_ui_core::shell::{
    admit_command, bind_allowed, clear_host_error, note_host_error, projected_status,
    queue_cad_input, HostErrorKind, ShellState,
};
use cad_ui_core::unsupported_status;

/// Label of the top-level probe window.
pub const WINDOW_LABEL: &str = "probe-window";
/// Label of the child webview that hosts the React frontend.
pub const WEBVIEW_LABEL: &str = "probe-webview";
/// Initial window size in physical pixels.
pub const INITIAL_WIDTH: u32 = 960;
/// Initial window size in physical pixels.
pub const INITIAL_HEIGHT: u32 = 600;
/// Minimum interval between presented frames (bounds CPU/GPU work to ~60 fps).
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Fixed animation step applied to the marker on frames without input.
pub const ANIMATION_STEP: f32 = 0.01;

/// CLI/startup options for the React host (owned by the main thread).
///
/// `--open <path>` (or `YACR_TEST_DWG`) is the real fixture funnel: the path is
/// read through the same `PlatformFiles` open path, never a fake headless open.
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// A drawing to open at startup (real file, read through the open funnel).
    pub drawing: Option<PathBuf>,
    /// Explicit CAD font directory (`--fonts-dir`).
    pub fonts_dir: Option<PathBuf>,
    /// Explicit raster-image directory (`--images-dir`).
    pub images_dir: Option<PathBuf>,
    /// Initial logical window size.
    pub size: [f64; 2],
    /// `--probe` / `YACR_PROBE_ONLY`: keep the old probe marker path and report
    /// CAD as unsupported (no CAD session).
    pub probe_only: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            drawing: None,
            fonts_dir: None,
            images_dir: None,
            size: [f64::from(INITIAL_WIDTH), f64::from(INITIAL_HEIGHT)],
            probe_only: false,
        }
    }
}

impl LaunchOptions {
    /// Parse CLI arguments (the real fixture funnel; never a fake open).
    ///
    /// Recognised: `--open <path>`, `--fonts-dir <path>`, `--images-dir <path>`
    /// and `--probe`. A **bare positional** argument (does not start with `-`) is
    /// a drawing path, exactly as if passed to `--open`, so a desktop entry can
    /// launch `yacr <drawing>`. When both are present `--open` wins. Only the
    /// first positional is opened (multi-document open is NOT IMPLEMENTED); every
    /// dropped or unsupported argument is reported on stderr instead of being
    /// silently ignored. Parsing is pure: the returned [`LaunchOptions`] and the
    /// diagnostics are unit-testable, and [`from_args`] emits them verbatim.
    ///
    /// [`from_args`]: LaunchOptions::from_args
    pub fn parse_args<I: Iterator<Item = String>>(args: I) -> (Self, Vec<String>) {
        let mut options = Self::default();
        let mut diagnostics = Vec::new();
        let mut positionals: Vec<String> = Vec::new();
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--open" => options.drawing = args.next().map(PathBuf::from),
                "--fonts-dir" => options.fonts_dir = args.next().map(PathBuf::from),
                "--images-dir" => options.images_dir = args.next().map(PathBuf::from),
                "--probe" => options.probe_only = true,
                _ => {
                    if arg.starts_with('-') {
                        // Unknown flags stay ignored exactly as before: never a
                        // path and never a new failure.
                    } else if arg.contains("://") {
                        // A URI scheme (e.g. `file://`) is NOT a filesystem path.
                        // Explicit refusal, never a silent mis-parse.
                        diagnostics.push(format!(
                            "error: unsupported argument `{arg}` (URI scheme is NOT IMPLEMENTED; pass a plain filesystem path or use --open <path>)"
                        ));
                    } else {
                        positionals.push(arg);
                    }
                }
            }
        }

        if let Some((first, extras)) = positionals.split_first() {
            if options.drawing.is_some() {
                diagnostics.push(format!(
                    "[probe] --open takes precedence; positional drawing argument ignored: {first}"
                ));
            } else {
                diagnostics.push(format!("[probe] positional drawing argument: {first}"));
                options.drawing = Some(PathBuf::from(first));
            }
            if !extras.is_empty() {
                diagnostics.push(format!(
                    "[probe] extra positional argument(s) ignored (multi-document open is NOT IMPLEMENTED): {}",
                    extras.join(" ")
                ));
            }
        }

        (options, diagnostics)
    }

    /// [`LaunchOptions::parse_args`] plus emitting each diagnostic verbatim on
    /// stderr (English lines; the `[probe]`/`error:` prefixes match the existing
    /// style and stay greppable).
    pub fn from_args<I: Iterator<Item = String>>(args: I) -> Self {
        let (options, diagnostics) = Self::parse_args(args);
        for diagnostic in &diagnostics {
            eprintln!("{diagnostic}");
        }
        options
    }
}

/// Main-thread-only CAD platform pieces (built once in `init`).
///
/// Holds `Rc` handles, so it lives in a thread-local and is never shared.
struct CadEnv {
    files: std::rc::Rc<dyn PlatformFiles>,
    config: std::rc::Rc<dyn ConfigStore>,
    surface: crate::platform::ReactSurface,
    options: HostOptions,
}

thread_local! {
    /// The GTK overlay bridge (GTK widgets + GPU presenter), main-thread only.
    static BRIDGE: RefCell<Option<crate::gtk_bridge::GtkOverlayOwner>> =
        const { RefCell::new(None) };
    /// The child webview, kept for bounded DOM diagnostics (main thread only).
    static WEBVIEW: RefCell<Option<tauri::Webview>> = const { RefCell::new(None) };
    /// The CAD session (`!Send`): created, driven and dropped on the main thread.
    static CAD: RefCell<Option<CadSession>> = const { RefCell::new(None) };
    /// The main-thread CAD platform pieces (see [`CadEnv`]).
    static CAD_ENV: RefCell<Option<CadEnv>> = const { RefCell::new(None) };
}

/// Initialization script: capture uncaught JS errors into `window.__yacr_diag`
/// so the diagnostic eval can report them (bounded diagnostics only).
const ERROR_HOOK_JS: &str = r#"
(function () {
  try {
    window.__yacr_diag = window.__yacr_diag || { errors: [] };
    window.addEventListener('error', function (e) {
      try { window.__yacr_diag.errors.push('error: ' + (e.message || e.error || String(e))); } catch (_) {}
    });
    window.addEventListener('unhandledrejection', function (e) {
      try { window.__yacr_diag.errors.push('rejection: ' + String(e.reason)); } catch (_) {}
    });
  } catch (_) {}
})();
"#;

/// Diagnostic eval: report DOM mount state, bounding rects, computed styles and
/// counts. Returns an object; the callback receives it serialized as JSON.
const DOM_DIAG_JS: &str = r#"
(function () {
  try {
    var d = window.__yacr_diag || { errors: [] };
    function info(el) {
      if (!el) return null;
      var r = el.getBoundingClientRect();
      var cs = getComputedStyle(el);
      return {
        rect: [r.x, r.y, r.width, r.height],
        display: cs.display, visibility: cs.visibility, opacity: cs.opacity,
        background: cs.backgroundColor, color: cs.color,
        children: el.children.length, htmlLen: el.innerHTML.length
      };
    }
    return {
      readyState: document.readyState,
      href: location.href,
      title: document.title,
      bodyTextLen: (document.body && document.body.innerText || '').length,
      buttons: document.querySelectorAll('button').length,
      inputs: document.querySelectorAll('input').length,
      selects: document.querySelectorAll('select').length,
      root: info(document.getElementById('root')),
      main: info(document.querySelector('main.shell')),
      ribbon: info(document.querySelector('.ribbon')),
      commandBar: info(document.querySelector('.commandline')),
      errors: d.errors
    };
  } catch (e) {
    return { evalError: String(e) };
  }
})()
"#;

fn run_dom_diag(run: u32) {
    WEBVIEW.with(|cell| {
        if let Some(webview) = cell.borrow().as_ref() {
            let result = webview.eval_with_callback(DOM_DIAG_JS, move |json| {
                eprintln!("[probe] DOM diag #{run}: {json}");
            });
            if let Err(error) = result {
                eprintln!("[probe] DOM diag #{run} dispatch failed: {error}");
            }
        } else {
            eprintln!("[probe] DOM diag #{run}: no webview handle");
        }
    });
}

/// Outcome of one bind step for the CAD render session.
enum BindStep {
    /// No bridge is installed yet (deferred, not a failure).
    NoBridge,
    /// Skipped: the bridge is closing/failed, or this generation already failed.
    Skipped,
    /// A bind was attempted for `generation` with `result`.
    Attempted(u64, Result<(), String>),
}

/// The telemetry state of one CAD frame attempt (never UI text).
///
/// `NoSession`/`Waiting`/`Error` are explicitly **not** `Presented`, so a log
/// consumer can never mistake a non-present for a new frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CadFrameLogState {
    Presented,
    Waiting,
    Error,
    NoSession,
}

impl CadFrameLogState {
    fn label(self) -> &'static str {
        match self {
            Self::Presented => "presented",
            Self::Waiting => "waiting",
            Self::Error => "error",
            Self::NoSession => "no-session",
        }
    }
}

/// Whether the render-gate diagnostic line should be emitted now: on a change, or
/// at least once per second while unchanged (never per frame).
fn should_log_render_diag(unchanged: bool, recent: bool) -> bool {
    !(unchanged && recent)
}

/// A host CAD frame at or above this wall time (µs) is logged even without an
/// upload observation: `50 ms` (well above the 16 ms cadence, so it never spams).
const CAD_STAGE_SLOW_US: u64 = 50_000;

/// A compact summary of the latest **actual** render (for the post-load driver's
/// readiness/final-frame admission). Not a replay of stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActualRenderSummary {
    render_seq: u64,
    view_revision: u64,
    device_epoch: u64,
}

/// Bounded post-load pan driver phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostloadPanPhase {
    /// Waiting for the **requested** drawing (not the demo placeholder) to be
    /// imported, presented, and actually rendered at the current view/device.
    WaitingReady,
    /// Gesture dispatched; waiting for its structured completion report.
    Gesture,
    /// Gesture proven complete; waiting for the final rendered+presented frame.
    AwaitingFinal,
    Finished,
    Failed,
}

/// What the bounded post-load pan driver should do next (pure decision).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostloadPanAction {
    None,
    StartGesture,
    PollReport,
    BeginFinal,
    Finish,
    Fail,
}

/// Parsed structured gesture report (from the RUNID-tracked eval-response marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PanReport {
    run_id: u64,
    done: bool,
    moves: u64,
    downs: u64,
    ups: u64,
    elapsed_ms: i64,
    failed: bool,
}

/// Minimum JS-side moves proving the loop actually advanced.
const POSTLOAD_PAN_MIN_JS_MOVES: u64 = 60;
/// Minimum HOST-admitted moves/ups proving the gesture reached the IPC path.
const POSTLOAD_PAN_MIN_ADMITTED_MOVES: u64 = 1;
const POSTLOAD_PAN_MIN_ADMITTED_UPS: u64 = 1;
/// The JS gesture duration (mirrors `POSTLOAD_PAN_JS` `DURATION`).
const POSTLOAD_PAN_GESTURE_MS: i64 = 8000;

/// Bounded post-load pan driver state (`YACR_PROBE_POSTLOAD_PAN`). Default OFF and
/// independent of the input/lifecycle drivers.
struct PostloadPanState {
    started: Instant,
    overall_deadline: Instant,
    phase: PostloadPanPhase,
    /// View revision before the gesture (the pan must move away from it).
    start_view_revision: u64,
    /// Actual-render sequence observed when the gesture report completed (the
    /// final frame must be a strictly newer actual render).
    end_render_seq: u64,
    /// Next structured report poll.
    poll_at: Option<Instant>,
    /// Bound for the final frame after the gesture completes.
    final_deadline: Option<Instant>,
    /// Host-admitted gesture inputs during the run.
    admitted_moves: u64,
    admitted_ups: u64,
    /// A `resize` input arrived while the gesture/final wait was active: any
    /// resulting view change is not attributable to the pan → explicit failure.
    concurrent_resize: bool,
    /// Latest parsed report (RUNID-tracked).
    report: Option<PanReport>,
    /// How many gestures this run performs (default 1 = unchanged semantics;
    /// `YACR_PROBE_POSTLOAD_PAN_GESTURES`).
    gestures_target: u32,
    /// Gestures already settled in this run.
    gestures_done: u32,
    /// The unique run id captured from the gesture response; a report with a
    /// different id is rejected.
    expected_run_id: Option<u64>,
}

/// Total bound for the whole driver (import + present + gesture + final frame).
const POSTLOAD_PAN_OVERALL: Duration = Duration::from_secs(90);
/// Grace after the ~8 s gesture for the final frame to be rendered AND presented.
const POSTLOAD_PAN_FINAL_GRACE: Duration = Duration::from_secs(10);
/// Extra overall budget allowed per additional gesture (`N - 1` more settle
/// commits); the default `N == 1` therefore keeps the historical 90 s bound.
const POSTLOAD_PAN_GESTURE_BUDGET: Duration = Duration::from_secs(15);
/// Upper bound on `YACR_PROBE_POSTLOAD_PAN_GESTURES` (bounded evidence runs only).
const POSTLOAD_PAN_MAX_GESTURES: u32 = 10;
/// Structured report poll interval.
const POSTLOAD_PAN_POLL: Duration = Duration::from_millis(250);

impl PostloadPanState {
    fn new() -> Self {
        let started = Instant::now();
        let gestures_target = std::env::var("YACR_PROBE_POSTLOAD_PAN_GESTURES")
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .unwrap_or(1)
            .clamp(1, POSTLOAD_PAN_MAX_GESTURES);
        Self {
            started,
            overall_deadline: started
                + POSTLOAD_PAN_OVERALL
                + POSTLOAD_PAN_GESTURE_BUDGET * (gestures_target - 1),
            gestures_target,
            gestures_done: 0,
            phase: PostloadPanPhase::WaitingReady,
            start_view_revision: 0,
            end_render_seq: 0,
            poll_at: None,
            final_deadline: None,
            admitted_moves: 0,
            admitted_ups: 0,
            concurrent_resize: false,
            report: None,
            expected_run_id: None,
        }
    }
}

/// The file-name component of a document label (a path or a bare name).
fn label_file_name(label: &str) -> &str {
    label.rsplit(['/', '\\']).next().unwrap_or(label)
}

/// Admission for starting the gesture: the **requested** drawing — not the
/// `yacr-demo` placeholder — has been imported (a verified import transition with
/// no pending load and no error, and its document label matches the requested
/// file name), it is **presented**, AND the latest actual render matches the
/// current view revision and device epoch. Never an entity count or a fixed delay.
fn postload_pan_import_ready(
    loading: bool,
    error: Option<&str>,
    document_label: Option<&str>,
    import_count: u64,
    requested_file_name: &str,
) -> bool {
    if loading || error.is_some() || import_count == 0 {
        return false;
    }
    document_label.is_some_and(|label| label_file_name(label) == requested_file_name)
}

/// Decode a Tauri/GTK eval-callback payload.
///
/// `eval_with_callback` hands back the JS result **JSON-encoded**: a string
/// result arrives as `"run=1 … failed="` (quotes and escapes). Decode that with
/// `serde_json::from_str::<String>` when it is a JSON string, and accept a raw
/// marker unchanged. Returns `None` for neither.
fn decode_eval_payload(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.starts_with('"') {
        return serde_json::from_str::<String>(trimmed).ok();
    }
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Parse the unique `run=N` id from a gesture/report marker (decoding a quoted
/// eval payload first). `None` when absent/malformed.
fn parse_pan_run_id(text: &str) -> Option<u64> {
    let decoded = decode_eval_payload(text)?;
    for field in decoded.split_whitespace() {
        if let Some(value) = field.strip_prefix("run=") {
            return value.parse().ok();
        }
    }
    None
}

/// Parse the structured report marker `run=N done=… moves=N downs=N ups=N
/// elapsed_ms=N failed=…`, decoding the JSON-quoted eval payload first
/// (`"run=1 …"`). `None` when the marker is absent/malformed or has no `run=`.
fn parse_pan_report(raw: &str) -> Option<PanReport> {
    let text = decode_eval_payload(raw)?;
    if !text.contains("run=") {
        return None;
    }
    let mut report = PanReport::default();
    let mut seen_run = false;
    for field in text.split_whitespace() {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        match key {
            "run" => {
                report.run_id = value.parse().ok()?;
                seen_run = true;
            }
            "done" => report.done = value == "true",
            "moves" => report.moves = value.parse().unwrap_or(0),
            "downs" => report.downs = value.parse().unwrap_or(0),
            "ups" => report.ups = value.parse().unwrap_or(0),
            "elapsed_ms" => report.elapsed_ms = value.parse().unwrap_or(-1),
            // A non-empty `failed=` means the JS recorded a failure.
            "failed" => report.failed = !value.is_empty(),
            _ => {}
        }
    }
    seen_run.then_some(report)
}

/// Whether a parsed report belongs to the expected unique run id (`None` expected
/// accepts the first report; a mismatched id is a stale/foreign report).
fn pan_report_matches_run(expected: Option<u64>, report: &PanReport) -> bool {
    expected.is_none_or(|id| id == report.run_id)
}

/// Whether the gesture is proven complete: the report is done without failure,
/// the JS loop advanced (moves) and ended (`up`), the elapsed time covers the
/// gesture duration, AND the host actually admitted moves and an up.
fn postload_pan_gesture_complete(
    report: Option<&PanReport>,
    admitted_moves: u64,
    admitted_ups: u64,
) -> bool {
    report.is_some_and(|report| {
        report.done
            && !report.failed
            && report.ups >= 1
            && report.moves >= POSTLOAD_PAN_MIN_JS_MOVES
            && report.elapsed_ms >= POSTLOAD_PAN_GESTURE_MS
            && admitted_moves >= POSTLOAD_PAN_MIN_ADMITTED_MOVES
            && admitted_ups >= POSTLOAD_PAN_MIN_ADMITTED_UPS
    })
}

/// Whether the gesture actually changed the camera away from the start view.
fn postload_pan_camera_changed(
    actual: Option<&ActualRenderSummary>,
    start_view_revision: u64,
) -> bool {
    actual.is_some_and(|actual| actual.view_revision != start_view_revision)
}

/// Admission for the final frame: the requested drawing is presented AND a **new**
/// actual render (render_seq beyond the gesture end) landed at a view different
/// from the pre-gesture one. A concurrent WM resize can never satisfy this.
fn postload_pan_final_ok(
    presented: bool,
    actual: Option<&ActualRenderSummary>,
    start_view_revision: u64,
    end_render_seq: u64,
) -> bool {
    presented
        && actual.is_some_and(|actual| {
            actual.render_seq > end_render_seq && actual.view_revision != start_view_revision
        })
}

/// Pure step decision for the bounded post-load pan driver.
#[allow(clippy::too_many_arguments)]
fn postload_pan_step(
    phase: PostloadPanPhase,
    ready: bool,
    gesture_complete: bool,
    final_ok: bool,
    concurrent_resize: bool,
    overall_timed_out: bool,
    final_timed_out: bool,
) -> PostloadPanAction {
    match phase {
        PostloadPanPhase::WaitingReady => {
            if ready {
                PostloadPanAction::StartGesture
            } else if overall_timed_out {
                PostloadPanAction::Fail
            } else {
                PostloadPanAction::None
            }
        }
        PostloadPanPhase::Gesture => {
            if concurrent_resize {
                PostloadPanAction::Fail
            } else if gesture_complete {
                PostloadPanAction::BeginFinal
            } else if overall_timed_out {
                PostloadPanAction::Fail
            } else {
                PostloadPanAction::PollReport
            }
        }
        PostloadPanPhase::AwaitingFinal => {
            if concurrent_resize {
                PostloadPanAction::Fail
            } else if final_ok {
                PostloadPanAction::Finish
            } else if final_timed_out || overall_timed_out {
                PostloadPanAction::Fail
            } else {
                PostloadPanAction::None
            }
        }
        PostloadPanPhase::Finished | PostloadPanPhase::Failed => PostloadPanAction::None,
    }
}

/// Emit the engine render-gate diagnostic on a meaningful state change or a ~1 s
/// heartbeat (never per frame). The caller checks `YACR_PROBE_CAD_LOG`.
fn log_render_diagnostics(shared: &Arc<Mutex<Shared>>, diagnostics: &RenderDiagnostics) {
    let now = Instant::now();
    {
        let state = lock(shared);
        let unchanged = state.last_render_diag.as_ref() == Some(diagnostics);
        let recent = state
            .last_render_diag_log
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1));
        if !should_log_render_diag(unchanged, recent) {
            return;
        }
    }
    {
        let mut state = lock(shared);
        state.last_render_diag = Some(diagnostics.clone());
        state.last_render_diag_log = Some(now);
    }
    let runtime = &diagnostics.runtime;
    let upload = runtime
        .upload
        .as_ref()
        .map(|upload| {
            format!(
                "{}/{}/{}",
                upload.scene_revision, upload.group, upload.offset
            )
        })
        .unwrap_or_else(|| "-".to_string());
    eprintln!(
        "[probe] pan-wait gate={:?} lifecycle={} input_rev={} requested_token={} prepared_token={:?} prepared_identity={:?} view_rev={} ready_rev={:?} ready_base={:?} applied_rev={:?} applied_base={:?} resident_batches={} prep_busy={} upload={} upload_base_changed={:?} uploads_started={} upload_rev_resets={} ready_none_clears={} last_upload_reset={:?} suppressed={} grid_refresh_pending={} grid_refresh={} suffix_commits={} pulled_reuses={} pulled_rebuilds={} frames_rendered={} status={:?}",
        diagnostics.gate,
        diagnostics.lifecycle,
        diagnostics.input_revision,
        diagnostics.requested_token,
        diagnostics.prepared_token,
        diagnostics.prepared_identity,
        diagnostics.view_revision,
        diagnostics.ready_revision,
        diagnostics.ready_base_revision,
        runtime.applied_revision,
        runtime.applied_base,
        runtime.resident_base_batches,
        diagnostics.preparation_busy,
        upload,
        runtime.upload.as_ref().map(|upload| upload.base_changed),
        runtime.uploads_started,
        runtime.upload_revision_resets,
        runtime.ready_none_clears,
        runtime.last_upload_reset,
        runtime.overlays_suppressed,
        diagnostics.grid_refresh_pending,
        diagnostics.grid_refresh,
        runtime.overlay_suffix_commits,
        runtime.pulled_prefix_reuses,
        runtime.pulled_prefix_rebuilds,
        runtime.frames_rendered,
        diagnostics.result_status,
    );
}

/// Bounded post-load pan evidence driver (env-gated, default OFF). Runs on the
/// main-thread tick: waits for the **requested** drawing (never the demo
/// placeholder) to be imported/presented with an actual render at the current
/// view/device, starts one synthetic DOM pan gesture, waits for its **structured
/// completion report** (RUNID-tracked; a view change alone is not completion),
/// then waits (bounded) for the final rendered+presented frame and exits
/// naturally. Any timeout, missing moves/up, or a concurrent resize is an
/// explicit bounded failure, never a fake pass.
fn drive_postload_pan(shared: &Arc<Mutex<Shared>>) {
    let now = Instant::now();
    let Some((
        phase,
        overall_deadline,
        final_deadline,
        poll_at,
        start_view_revision,
        end_render_seq,
        admitted_moves,
        admitted_ups,
        concurrent_resize,
        report,
    )) = ({
        let state = lock(shared);
        state.postload_pan.as_ref().map(|pan| {
            (
                pan.phase,
                pan.overall_deadline,
                pan.final_deadline,
                pan.poll_at,
                pan.start_view_revision,
                pan.end_render_seq,
                pan.admitted_moves,
                pan.admitted_ups,
                pan.concurrent_resize,
                pan.report,
            )
        })
    })
    else {
        return;
    };

    // The driver cannot gate without a requested drawing: fail boundedly instead
    // of running on whatever the app happens to show.
    let requested = lock(shared).launch.drawing.clone();
    if requested.is_none() && phase == PostloadPanPhase::WaitingReady {
        eprintln!(
            "[probe] postload-pan FAILED (no requested drawing from --open/YACR_TEST_DWG); explicit failure, no fake pass"
        );
        if let Some(pan) = lock(shared).postload_pan.as_mut() {
            pan.phase = PostloadPanPhase::Failed;
        }
        exit_postload_pan(shared, 3);
        return;
    }
    let requested_file_name = requested
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    // The session status (already computed this tick) shows the imported document.
    let status = lock(shared).shell.cad_status.clone();
    let import_ready = status.as_ref().is_some_and(|status| {
        postload_pan_import_ready(
            status.loading,
            status.error.as_deref(),
            status.document_label.as_deref(),
            status.import_count,
            &requested_file_name,
        )
    });

    // Readiness facts are main-thread engine state.
    let (presented, view_revision, device_epoch) = CAD.with(|cell| {
        let borrowed = cell.borrow();
        let Some(session) = borrowed.as_ref() else {
            return (false, 0, 0);
        };
        let handle = session.render_session();
        (
            handle.current_drawing_presented(),
            handle.view_revision(),
            handle.device_epoch(),
        )
    });
    let actual = lock(shared).last_actual_render;

    // Structured report poll (RUNID-tracked marker via the eval response).
    if phase == PostloadPanPhase::Gesture && poll_at.is_some_and(|at| now >= at) {
        if let Some(pan) = lock(shared).postload_pan.as_mut() {
            pan.poll_at = Some(now + POSTLOAD_PAN_POLL);
        }
        run_postload_pan_report(shared);
    }

    let ready = import_ready
        && presented
        && actual.is_some_and(|actual| {
            actual.view_revision == view_revision && actual.device_epoch == device_epoch
        });
    let gesture_complete =
        postload_pan_gesture_complete(report.as_ref(), admitted_moves, admitted_ups);
    let final_ok = postload_pan_final_ok(
        presented,
        actual.as_ref(),
        start_view_revision,
        end_render_seq,
    );
    let action = postload_pan_step(
        phase,
        ready,
        gesture_complete,
        final_ok,
        concurrent_resize,
        now >= overall_deadline,
        final_deadline.is_some_and(|deadline| now >= deadline),
    );
    match action {
        PostloadPanAction::None | PostloadPanAction::PollReport => {}
        PostloadPanAction::StartGesture => {
            eprintln!(
                "[probe] postload-pan start view_rev={view_revision} device_epoch={device_epoch} presented={presented} actual_render_seq={:?} document={:?} import_count={:?} (synthetic DOM gesture; NOT a real mouse)",
                actual.map(|actual| actual.render_seq),
                status.as_ref().and_then(|status| status.document_label.clone()),
                status.as_ref().map(|status| status.import_count),
            );
            run_postload_pan_gesture(shared);
            let mut state = lock(shared);
            if let Some(pan) = state.postload_pan.as_mut() {
                pan.phase = PostloadPanPhase::Gesture;
                pan.start_view_revision = view_revision;
                pan.poll_at = Some(now + POSTLOAD_PAN_POLL);
                // Per-gesture accounting (identical to the historical single-gesture
                // run, where these start at 0 anyway).
                pan.admitted_moves = 0;
                pan.admitted_ups = 0;
            }
        }
        PostloadPanAction::BeginFinal => {
            // The gesture is proven complete. Require the pan to have actually
            // changed the camera (a concurrent resize would have been caught by
            // `concurrent_resize`).
            if !postload_pan_camera_changed(actual.as_ref(), start_view_revision) {
                eprintln!(
                    "[probe] postload-pan FAILED (gesture completed but the camera did not change; start_view_rev={start_view_revision}) — explicit failure, no fake pass"
                );
                if let Some(pan) = lock(shared).postload_pan.as_mut() {
                    pan.phase = PostloadPanPhase::Failed;
                }
                exit_postload_pan(shared, 3);
                return;
            }
            let report_text = report
                .map(|report| {
                    format!(
                        "run={} moves={} downs={} ups={} elapsed_ms={} failed={}",
                        report.run_id,
                        report.moves,
                        report.downs,
                        report.ups,
                        report.elapsed_ms,
                        report.failed
                    )
                })
                .unwrap_or_else(|| "-".to_string());
            eprintln!(
                "[probe] postload-pan gesture-complete {report_text} admitted_moves={admitted_moves} admitted_ups={admitted_ups} view_rev={view_revision}"
            );
            let mut state = lock(shared);
            if let Some(pan) = state.postload_pan.as_mut() {
                pan.phase = PostloadPanPhase::AwaitingFinal;
                pan.end_render_seq = actual.map(|actual| actual.render_seq).unwrap_or(0);
                pan.final_deadline = Some(now + POSTLOAD_PAN_FINAL_GRACE);
            }
        }
        PostloadPanAction::Finish => {
            // Repeated-drag evidence: re-arm instead of exiting while gestures
            // remain. `gestures_target == 1` (the default) never takes this branch,
            // so the historical semantics are unchanged.
            let (done, target) = {
                let state = lock(shared);
                state
                    .postload_pan
                    .as_ref()
                    .map_or((1, 1), |pan| (pan.gestures_done + 1, pan.gestures_target))
            };
            if done < target {
                eprintln!(
                    "[probe] postload-pan settle {done}/{target} view_rev={view_revision} device_epoch={device_epoch} actual_render_seq={:?} settled; re-arming for the next gesture",
                    actual.map(|actual| actual.render_seq)
                );
                if let Some(pan) = lock(shared).postload_pan.as_mut() {
                    pan.gestures_done = done;
                    pan.phase = PostloadPanPhase::WaitingReady;
                    pan.poll_at = None;
                    pan.final_deadline = None;
                    pan.report = None;
                    pan.end_render_seq = 0;
                    pan.start_view_revision = 0;
                    pan.admitted_moves = 0;
                    pan.admitted_ups = 0;
                }
                return;
            }
            let elapsed_ms = lock(shared)
                .postload_pan
                .as_ref()
                .map(|pan| pan.started.elapsed().as_millis())
                .unwrap_or(0);
            eprintln!(
                "[probe] postload-pan end view_rev={view_revision} device_epoch={device_epoch} actual_render_seq={:?} start_view_rev={start_view_revision} elapsed_ms={elapsed_ms} natural_exit=0",
                actual.map(|actual| actual.render_seq)
            );
            if let Some(pan) = lock(shared).postload_pan.as_mut() {
                pan.phase = PostloadPanPhase::Finished;
            }
            exit_postload_pan(shared, 0);
        }
        PostloadPanAction::Fail => {
            let reason = if concurrent_resize {
                "concurrent resize during the pan invalidated the report"
            } else if phase == PostloadPanPhase::Gesture {
                "gesture did not complete (no moves/up or not enough progress) before the bound"
            } else if phase == PostloadPanPhase::WaitingReady {
                "requested drawing was not imported+presented+rendered before the bound"
            } else {
                "final rendered+presented frame did not land before the bound"
            };
            eprintln!(
                "[probe] postload-pan FAILED (bounded) phase={phase:?} reason={reason} presented={presented} view_rev={view_revision} device_epoch={device_epoch} concurrent_resize={concurrent_resize} admitted_moves={admitted_moves} admitted_ups={admitted_ups} — explicit failure, no fake pass"
            );
            if let Some(pan) = lock(shared).postload_pan.as_mut() {
                pan.phase = PostloadPanPhase::Failed;
            }
            exit_postload_pan(shared, 3);
        }
    }
}

/// Exit the process for the post-load driver.
///
/// The requested status is **recorded** in [`Shared`] so the entry point can
/// propagate it to the OS exit status after the event loop returns
/// (`AppHandle::exit` alone was observed to leave the OS status 0); the handle's
/// `exit` is still called to stop the loop promptly. `0` only on a proven pass.
fn exit_postload_pan(shared: &Arc<Mutex<Shared>>, code: i32) {
    let app = {
        let mut state = lock(shared);
        state.postload_pan_exit = Some(code);
        state.app_handle.clone()
    };
    if let Some(app) = app {
        app.exit(code);
    }
}

/// The process exit status the entry point must use after the event loop
/// returns, if the bounded post-load driver requested one.
pub fn requested_exit_code(shared: &Arc<Mutex<Shared>>) -> Option<i32> {
    lock(shared).postload_pan_exit
}

/// The OS exit status implied by a recorded requested code (pure): an absent
/// request is a normal `0`, so a manual/default run and a passing driver both
/// exit 0 and only an explicit failure propagates non-zero.
pub(crate) fn resolve_process_exit(requested: Option<i32>) -> i32 {
    requested.unwrap_or(0)
}

/// Dispatch the synthetic DOM pan gesture through the real webview (`eval`).
fn run_postload_pan_gesture(shared: &Arc<Mutex<Shared>>) {
    let shared = shared.clone();
    WEBVIEW.with(|cell| {
        if let Some(webview) = cell.borrow().as_ref() {
            let result =
                webview.eval_with_callback(crate::evidence::POSTLOAD_PAN_JS, move |json| {
                    eprintln!("[probe] postload-pan gesture: {json}");
                    if let Some(run_id) = parse_pan_run_id(&json) {
                        if let Some(pan) = lock(&shared).postload_pan.as_mut() {
                            pan.expected_run_id = Some(run_id);
                        }
                    }
                });
            if let Err(error) = result {
                eprintln!("[probe] postload-pan gesture dispatch failed: {error}");
            }
        } else {
            eprintln!("[probe] postload-pan: no webview handle; gesture skipped");
        }
    });
}

/// Poll the structured gesture report and store the parsed RUNID/counts.
///
/// The callback payload is JSON-encoded by Tauri/GTK; `parse_pan_report` decodes
/// it. A report whose run id does not match the gesture's unique id is rejected.
fn run_postload_pan_report(shared: &Arc<Mutex<Shared>>) {
    let shared = shared.clone();
    WEBVIEW.with(|cell| {
        if let Some(webview) = cell.borrow().as_ref() {
            let result =
                webview.eval_with_callback(crate::evidence::POSTLOAD_PAN_REPORT_JS, move |json| {
                    eprintln!("[probe] postload-pan report: {json}");
                    let Some(report) = parse_pan_report(&json) else {
                        return;
                    };
                    let mut state = lock(&shared);
                    if let Some(pan) = state.postload_pan.as_mut() {
                        if pan_report_matches_run(pan.expected_run_id, &report) {
                            pan.report = Some(report);
                        }
                    }
                });
            if let Err(error) = result {
                eprintln!("[probe] postload-pan report dispatch failed: {error}");
            }
        }
    });
}

/// Classify a CAD frame result for the env-gated telemetry log.
fn cad_frame_log_state(result: &Option<Result<Option<u64>, String>>) -> CadFrameLogState {
    match result {
        Some(Ok(Some(_))) => CadFrameLogState::Presented,
        Some(Ok(None)) => CadFrameLogState::Waiting,
        Some(Err(_)) => CadFrameLogState::Error,
        None => CadFrameLogState::NoSession,
    }
}

/// Admission for one CAD presentation attempt, derived from the **bridge
/// lifecycle state** (not from `cad_ready`, which only means the render session
/// was registered).
///
/// This is the CAD present gate's explicit table: a bridge that is still coming
/// up (or suspended for re-creation) must make the host **wait** — never a
/// spurious `Present` error — while a terminal bridge is never presented on and
/// its diagnostics are preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CadPresentAdmission {
    /// `AwaitingMap`/`Suspended`: no presenter by design; wait (`Ok(None)`), do
    /// not touch/clear any prior error.
    Waiting,
    /// `Ready`: attempt the existing resize / native-health / render path.
    Ready,
    /// `Closing`/`Failed`: never present; preserve the terminal diagnostics.
    Terminal,
}

/// Map a bridge lifecycle state to the CAD presentation admission (pure).
fn cad_present_admission(state: &crate::gtk_bridge::BridgeState) -> CadPresentAdmission {
    use crate::gtk_bridge::BridgeState;
    match state {
        BridgeState::AwaitingMap | BridgeState::Suspended => CadPresentAdmission::Waiting,
        BridgeState::Ready => CadPresentAdmission::Ready,
        BridgeState::Closing | BridgeState::Failed(_) => CadPresentAdmission::Terminal,
    }
}

/// Whether the cached CAD rect must be **re-validated** (instead of a strict
/// `accept`): a new GPU generation, or a change in the margin-receiving container
/// bounds — the latter even when the DOM sequence did not change (the container
/// resized under an unchanged rect). Pure, so the decision is unit-testable.
fn layout_needs_revalidate(
    generation_revalidate: bool,
    last_bounds: Option<(f32, f32)>,
    bounds: (f32, f32),
) -> bool {
    generation_revalidate || last_bounds != Some(bounds)
}

/// A rendered-frame completeness label for the telemetry log (technical, not UI).
fn frame_completeness_label(completeness: FrameCompleteness) -> &'static str {
    match completeness {
        FrameCompleteness::CurrentScene => "current_scene",
        FrameCompleteness::BaseOnlyPendingOverlays => "base_only_pending_overlays",
    }
}

/// Render an optional telemetry number, using `-` for an absent value.
fn opt_telemetry_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Render an optional telemetry duration (µs), using `-` for an absent value.
fn opt_telemetry_u128(value: Option<u128>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// State shared with the IPC commands. Deliberately **GPU-free**: only status,
/// the input tracker and the CAD admission state cross threads.
///
/// The UI-framework-free half lives in [`ShellState`] (`cad-ui-core`) under the
/// explicit `shell` field; this struct adds the Tauri/GTK-specific probe fields
/// beside it. The halves are accessed explicitly (`state.shell.status`,
/// `state.shell.cad_commands`, …) — there is deliberately no `Deref` bridge, so
/// a future Win/macOS host cannot inherit one by accident.
pub struct Shared {
    /// The UI-framework-free shared state (status, input, CAD admission/errors).
    pub shell: ShellState,
    /// The most recent frame's per-stage timings (evidence).
    last_timings: Option<ProbeStageTimings>,
    /// The redraw pump's single-pending flag: the tick clears it, so the pump
    /// never queues more than one outstanding wake.
    wake_pending: Arc<AtomicBool>,
    /// The redraw pump that keeps the single Tauri loop awake.
    redraw_pump: Option<crate::redraw::RedrawPump>,
    /// Wall-clock time of the last rendered frame (frame-rate throttle).
    last_frame: Option<Instant>,
    /// Wall-clock time of the last periodic status log line.
    last_log: Option<Instant>,
    /// The probe window handle the tick polls for size/state. Kept here because
    /// the public `Manager` trait does not expose plain windows by label.
    window: Option<tauri::Window>,
    /// When the probe started; drives the bounded DOM-diagnostic schedule.
    diag_start: Instant,
    /// How many bounded DOM diagnostics have run (max 2).
    diag_runs: u32,
    /// `YACR_PROBE_NO_GPU`: skip the GPU presenter (WM/lifecycle baseline).
    no_gpu: bool,
    /// React-reported CAD layout (logical rect -> viewport insets), seq/generation guarded.
    layout: LayoutTracker,
    /// Set when a new GPU generation starts: the cached CAD rect must be
    /// re-validated against the current top geometry (not blindly re-applied).
    layout_revalidate: bool,
    /// The margin-receiving container bounds (logical px) last used for a layout
    /// decision. A change re-validates the cached DOM rect even without a new
    /// sequence (the container resized under an unchanged rect).
    layout_bounds: Option<(f32, f32)>,
    /// Last emitted engine render-gate diagnostic (dedup for the cad-log line).
    last_render_diag: Option<cad_engine::render::RenderDiagnostics>,
    /// Wall-clock time of the last emitted render-gate diagnostic (≤1 Hz).
    last_render_diag_log: Option<Instant>,
    /// The most recent **actual** render (render_seq/view/device), for the
    /// post-load pan driver's readiness/final-frame admission.
    last_actual_render: Option<ActualRenderSummary>,
    /// Observable state of the accelerated pulled organisation from the most
    /// recent **actual** render (`cad_render_wgpu::PulledState`). Surfaced in the
    /// `cad_status` evidence line so a skipped accelerated organisation is visible
    /// next to the status the frontend receives.
    last_pulled_state: Option<PulledState>,
    /// Bounded post-load pan evidence driver state (`YACR_PROBE_POSTLOAD_PAN`).
    postload_pan: Option<PostloadPanState>,
    /// App handle for the post-load driver's bounded natural exit.
    app_handle: Option<tauri::AppHandle>,
    /// The bounded post-load driver's requested process exit status, propagated
    /// to the OS by the entry point after the event loop returns (`AppHandle::exit`
    /// alone was observed to leave the OS status 0).
    postload_pan_exit: Option<i32>,
    /// Startup/CLI options (drawing, font/image dirs, size, probe mode).
    pub launch: LaunchOptions,
}

/// Managed Tauri state: a shared handle to [`Shared`].
pub struct ProbeShared(pub Arc<Mutex<Shared>>);

/// Create the shared probe state with the parsed launch options.
pub fn new_shared(launch: LaunchOptions) -> Arc<Mutex<Shared>> {
    let cad_mode = !launch.probe_only;
    Arc::new(Mutex::new(Shared {
        shell: ShellState::new(cad_mode),
        last_timings: None,
        wake_pending: Arc::new(AtomicBool::new(false)),
        redraw_pump: None,
        last_frame: None,
        last_log: None,
        window: None,
        diag_start: Instant::now(),
        diag_runs: 0,
        no_gpu: std::env::var_os("YACR_PROBE_NO_GPU").is_some(),
        layout: LayoutTracker::new(),
        layout_revalidate: false,
        layout_bounds: None,
        last_render_diag: None,
        last_render_diag_log: None,
        last_actual_render: None,
        last_pulled_state: None,
        postload_pan: crate::evidence::postload_pan_enabled().then(PostloadPanState::new),
        app_handle: None,
        postload_pan_exit: None,
        launch,
    }))
}

fn lock(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Resolve the durable config/preference paths (`$XDG_CONFIG_HOME` or `~/.config`).
///
/// `None` when no home is resolvable: the store then refuses persistence
/// explicitly (the host falls back to in-memory defaults).
fn config_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    match dir {
        Some(dir) => {
            let app = dir.join("yacr");
            (
                Some(app.join("config.json")),
                Some(app.join("preference.json")),
            )
        }
        None => (None, None),
    }
}

/// Whether the window starts maximized.
///
/// Default is maximized; the acceptance-only `YACR_PROBE_NO_MAXIMIZE` env opts
/// out (its presence, not its value, is passed here).
fn start_maximized(no_maximize_env_present: bool) -> bool {
    !no_maximize_env_present
}

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
        .auto_resize()
        .initialization_script(ERROR_HOOK_JS)
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
        let locale = std::env::var("YACR_LOCALE").unwrap_or_else(|_| "zh-CN".to_string());
        match crate::platform::ReactFiles::new(&locale) {
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
        // GTK overlay bridge: an independent native child surface (windowed
        // GtkDrawingArea) with the React webview reparented as a transparent
        // sibling. **No top-level fallback** — failure is explicit.
        match crate::gtk_bridge::install(&window, WINDOW_LABEL) {
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
    crate::evidence::maybe_spawn_lifecycle(&evidence_window, app.handle());
    Ok(())
}

/// Ask the tick to stop and release the presenter (idempotent).
pub fn request_stop(shared: &Arc<Mutex<Shared>>) {
    let mut state = lock(shared);
    state.shell.stop_requested = true;
    state.redraw_pump = None;
}

/// Enter the terminal closing state and drop the surface **on the main thread**
/// (B1): sets `Closing` so a later `try_init_gpu` can never run, then releases.
pub fn close_presenter() {
    BRIDGE.with(|cell| {
        if let Some(owner) = cell.borrow_mut().as_mut() {
            owner.close();
        }
    });
}

/// Stop and release the GPU presenter **on the main thread**, before the window
/// is torn down. Safe to call repeatedly.
pub fn release_presenter() {
    BRIDGE.with(|cell| {
        if let Some(owner) = cell.borrow_mut().as_mut() {
            owner.release_gpu();
        }
    });
}

/// Evidence-only: force the overlay to unrealize on the main thread, running the
/// registered unrealize hook (GPU released before native teardown). The owner is
/// temporarily moved out of the thread-local cell so GTK's **synchronous**
/// unrealize signal cannot re-enter an already-borrowed `RefCell`; the GPU is
/// released here, before the native unrealize, then the owner is put back.
/// Returns whether a bridge existed.
pub fn force_unrealize() -> bool {
    let owner = BRIDGE.with(|cell| cell.borrow_mut().take());
    match owner {
        Some(mut owner) => {
            owner.release_gpu();
            owner.force_unrealize();
            BRIDGE.with(|cell| *cell.borrow_mut() = Some(owner));
            true
        }
        None => false,
    }
}

/// Evidence-only: re-realize/re-map the overlay after a forced unrealize so the
/// next tick can create a new GPU generation. Uses the same move-out pattern so
/// any synchronous GTK signal cannot re-enter a borrowed `RefCell`.
pub fn force_realize() -> bool {
    let owner = BRIDGE.with(|cell| cell.borrow_mut().take());
    match owner {
        Some(mut owner) => {
            owner.force_realize();
            BRIDGE.with(|cell| *cell.borrow_mut() = Some(owner));
            true
        }
        None => false,
    }
}

/// One probe tick. Runs on the main thread. The shared lock is held only for
/// bookkeeping, never across GPU work.
pub fn tick(shared: &Arc<Mutex<Shared>>) {
    // ---- short critical section: bookkeeping only ----
    let (size, marker) = {
        let mut state = lock(shared);
        state.wake_pending.store(false, Ordering::SeqCst);

        if state.shell.stopped {
            return;
        }
        if state.shell.stop_requested {
            drop(state);
            release_presenter();
            let mut state = lock(shared);
            state.shell.stopped = true;
            state.shell.status.stopped = true;
            return;
        }

        let now = Instant::now();
        if let Some(last) = state.last_frame {
            if now.duration_since(last) < FRAME_INTERVAL {
                return;
            }
        }
        state.last_frame = Some(now);
        state.shell.ticks += 1;

        // Periodic (~1 Hz) status line for evidence; includes the last frame's
        // per-stage timings and the frame index.
        if state
            .last_log
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
        {
            state.last_log = Some(now);
            let stages = state
                .last_timings
                .map(|t| {
                    format!(
                        "write={} acquire={} configure={} submit={} scope={} present={} total={}",
                        t.write_buffer_us,
                        t.acquire_us,
                        t.configure_us,
                        t.submit_us,
                        t.scope_pop_us,
                        t.present_us,
                        t.total_us
                    )
                })
                .unwrap_or_else(|| "none".to_string());
            eprintln!(
                "[probe] status backend={:?} adapter={:?} size={}x{} frames={} ticks={} present_mode={:?} stopped={} marker={:.3} stages_us[{}] error={:?}",
                state.shell.status.backend,
                state.shell.status.adapter,
                state.shell.status.width,
                state.shell.status.height,
                state.shell.status.frames,
                state.shell.ticks,
                state.shell.present_mode,
                state.shell.status.stopped,
                state.shell.input.marker_offset(),
                stages,
                state.shell.status.error,
            );
        }

        // Bounded DOM diagnostics at ~6s and ~14s (2 runs max); they only
        // dispatch an eval, so they never block the loop.
        if state.diag_runs < 2 {
            let due = Duration::from_secs(6 + 8 * u64::from(state.diag_runs));
            if state.diag_start.elapsed() >= due {
                state.diag_runs += 1;
                let run = state.diag_runs;
                drop(state);
                run_dom_diag(run);
                return;
            }
        }

        let Some(window) = state.window.clone() else {
            return;
        };
        let minimized = window.is_minimized().unwrap_or(false);
        let size = window
            .inner_size()
            .unwrap_or(tauri::PhysicalSize::new(0, 0));

        if minimized || size.width == 0 || size.height == 0 {
            // Zero/occluded surface: pause instead of reconfiguring to zero.
            if !state.shell.paused {
                eprintln!(
                    "[probe] paused (minimized={minimized}) size={}x{}",
                    size.width, size.height
                );
                state.shell.paused = true;
            }
            state.shell.status.width = size.width;
            state.shell.status.height = size.height;
            return;
        }
        if state.shell.paused {
            eprintln!("[probe] resumed size={}x{}", size.width, size.height);
            state.shell.paused = false;
        }
        // Report the live size even when no GPU presenter is active.
        state.shell.status.width = size.width;
        state.shell.status.height = size.height;

        // Input wins over the fixed animation on the frame it arrives.
        let marker = if state.shell.input.take_dirty() {
            state.shell.input.marker_offset()
        } else {
            state.shell.input.advance_animation(ANIMATION_STEP);
            state.shell.input.marker_offset()
        };
        (size, marker)
    };

    // ---- deferred bridge GPU init (main thread, no shared lock held) ----
    // `LayoutTracker::confirmed` is the single source of truth for the CAD-rect
    // gate; the bridge keeps no second copy.
    let layout_confirmed = lock(shared).layout.confirmed();
    let init_result = BRIDGE.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .and_then(|owner| owner.try_init_gpu(layout_confirmed))
    });
    if let Some(result) = init_result {
        let mut state = lock(shared);
        match result {
            Ok(()) => {
                let mut surface = None;
                BRIDGE.with(|cell| {
                    if let Some(owner) = cell.borrow().as_ref() {
                        if let Some(adapter) = owner.adapter() {
                            state.shell.status.backend = Some(adapter.backend.clone());
                            state.shell.status.adapter = Some(adapter.name.clone());
                        }
                        if let Some(mode) = owner.present_mode() {
                            state.shell.present_mode = Some(format!("{mode:?}"));
                        }
                        surface = owner.surface_size();
                    }
                });
                // A new GPU generation: keep the DOM input sequence monotonic and
                // re-validate the cached CAD rect against the current top geometry
                // (do not blindly re-apply it; there is no cross-generation reset).
                state.layout.note_generation();
                state.layout_revalidate = true;
                // Report the actual child-surface size, not the window size.
                if let Some((w, h)) = surface {
                    state.shell.status.width = w;
                    state.shell.status.height = h;
                }
            }
            Err(error) => {
                state.shell.status.error = Some(error);
            }
        }
        return;
    }

    // ---- React CAD layout -> native margins (main thread, no shared lock) ----
    // The reference bounds for the CAD viewport insets are the **logical size of
    // the GTK container that receives the margins** (the overlay the React WebView
    // fills), in the frontend's DOM/CSS coordinate space. The Tauri window
    // `inner_size()` is WRONG here: it includes CSD/decorations and is larger than
    // the DOM viewport, so using it inflated every inset (native capture A:
    // 1012x652 vs the real 960x553 viewport → margins end=86/bottom=183 instead of
    // the correct end=34/bottom=84).
    let container_bounds = BRIDGE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|owner| owner.logical_container_bounds())
    });
    // Defer explicitly while the container allocation is not settled: never
    // accept a fake empty layout (zero bounds) and never disturb a confirmed one.
    let last_rect = lock(shared).shell.input.last_rect();
    if let (Some(bounds), Some(rect)) = (container_bounds, last_rect) {
        let (revalidate, decision) = {
            let mut state = lock(shared);
            // Re-validate the cached rect when a GPU generation changed **or** the
            // margin-receiving container bounds changed — the latter even without a
            // new DOM sequence (the container resized under an unchanged rect).
            let revalidate =
                layout_needs_revalidate(state.layout_revalidate, state.layout_bounds, bounds);
            state.layout_revalidate = false;
            let decision = if revalidate {
                state.layout.revalidate(&rect, bounds.0, bounds.1)
            } else {
                state.layout.accept(&rect, bounds.0, bounds.1)
            };
            (revalidate, decision)
        };
        match decision {
            Ok(margins) => {
                lock(shared).layout_bounds = Some(bounds);
                // Keep the CAD surface source in sync with the accepted DOM
                // rect. The host `tick` re-reads this surface every cycle
                // (`HostCore::tick` -> `metrics`), so `viewport.logical_size`
                // always matches the rect the renderer draws into. Leaving it
                // at the creation-time size made `Camera::zoom_at` anchor off
                // the cursor after any DOM-rect change (window resize, panel
                // toggle, DPR change).
                let scale = lock(shared)
                    .window
                    .as_ref()
                    .and_then(|window| window.scale_factor().ok())
                    .unwrap_or(1.0);
                CAD_ENV.with(|cell| {
                    if let Some(env) = cell.borrow().as_ref() {
                        env.surface.set_metrics(surface_metrics(&rect, scale));
                    }
                });
                BRIDGE.with(|cell| {
                    if let Some(owner) = cell.borrow_mut().as_mut() {
                        owner.apply_layout(margins);
                    }
                });
                let geometry =
                    BRIDGE.with(|cell| cell.borrow().as_ref().map(|owner| owner.layout_geometry()));
                let surface =
                    BRIDGE.with(|cell| cell.borrow().as_ref().and_then(|o| o.surface_size()));
                let top = lock(shared)
                    .window
                    .as_ref()
                    .and_then(|window| window.inner_size().ok());
                // Distinguish every geometry explicitly (evidence only; emitted
                // only on a layout change, never per frame): the DOM CAD rect,
                // the margin-receiving container, the drawing area + its
                // translated origin, the integer scale and the physical extent.
                eprintln!(
                    "[probe] geometry top={}x{} container={:.0}x{:.0} dom={}x{} dom_xy=({},{}) drawing_area={}x{} origin=({},{}) scale={} native_surface={}x{} margins top={} bottom={} start={} end={} revalidated={}",
                    top.map(|s| s.width).unwrap_or(0),
                    top.map(|s| s.height).unwrap_or(0),
                    bounds.0,
                    bounds.1,
                    rect.width,
                    rect.height,
                    rect.x,
                    rect.y,
                    geometry.map(|g| g.drawing_area.0).unwrap_or(0),
                    geometry.map(|g| g.drawing_area.1).unwrap_or(0),
                    geometry.and_then(|g| g.origin).map(|o| o.0).unwrap_or(-1),
                    geometry.and_then(|g| g.origin).map(|o| o.1).unwrap_or(-1),
                    geometry.map(|g| g.scale).unwrap_or(0),
                    surface.map(|s| s.0).unwrap_or(0),
                    surface.map(|s| s.1).unwrap_or(0),
                    margins.top,
                    margins.bottom,
                    margins.start,
                    margins.end,
                    revalidate,
                );
            }
            Err(LayoutError::StaleSequence) => {}
            Err(error) => {
                if revalidate {
                    // The cached rect no longer fits the current container: wait
                    // for fresh metrics instead of applying stale margins.
                    eprintln!(
                        "[probe] layout revalidation deferred (waiting for fresh metrics): {error:?}"
                    );
                } else {
                    lock(shared).shell.status.error = Some(format!("layout rejected: {error:?}"));
                }
            }
        }
    }

    // ---- CAD session: lazy creation once the DOM rect is confirmed ----
    let cad_mode = lock(shared).shell.cad_mode;
    if cad_mode && CAD.with(|cell| cell.borrow().is_none()) {
        let rect = lock(shared).shell.input.last_rect();
        if let Some(rect) = rect {
            let scale = lock(shared)
                .window
                .as_ref()
                .and_then(|window| window.scale_factor().ok())
                .unwrap_or(1.0)
                .max(0.1);
            let created = CAD_ENV.with(|cell| {
                let env = cell.borrow();
                let env = env.as_ref()?;
                // The surface must report before `CadSession::new` (the open
                // funnel validates canvas metrics against it).
                env.surface.set_metrics(surface_metrics(&rect, scale));
                let platform = HostPlatform {
                    files: env.files.clone(),
                    config_store: env.config.clone(),
                    surface: std::rc::Rc::new(env.surface.clone()),
                    resources: std::rc::Rc::new(cad_host::DesktopResources),
                    capabilities: HostCapabilities {
                        open: true,
                        new: false,
                        save: false,
                        plot: false,
                        import_worker: true,
                    },
                };
                Some(CadSession::new(
                    env.options.clone(),
                    platform,
                    RenderSessionHandle::new(BackendPreference::WebGpu),
                ))
            });
            match created {
                Some(Ok(session)) => {
                    CAD.with(|cell| *cell.borrow_mut() = Some(session));
                    eprintln!("[probe] cad session created");
                }
                Some(Err(error)) => {
                    let text = format!("{error:?}");
                    eprintln!("[probe] cad session creation FAILED: {text}");
                    // A CadSession-level (core/open) failure lives in the
                    // snapshot, never in the host transient error.
                    lock(shared).shell.cad_status = Some(unsupported_status(Some(text)));
                }
                None => {}
            }
        }
    }

    // ---- CAD session bind (main thread): only a successful bind marks ready ----
    //
    // The device is attached through the presenter's `device_queue`. With no
    // presenter yet this stores the session and returns `Ok` (device deferred,
    // not a failure). A bind is attempted **at most once per presenter
    // generation**: a failure is recorded against the generation, kept visible,
    // and the session stays not-ready (waiting) until the generation changes (a
    // new device) — no per-tick initialization loop. A bridge that is not
    // installed yet, or is closing/failed, defers without a failure.
    if cad_mode && !lock(shared).shell.cad_ready {
        let render = CAD.with(|cell| cell.borrow().as_ref().map(|s| s.render_session()));
        if let Some(render) = render {
            let failed_generation = lock(shared).shell.cad_bind_failed_generation;
            let step = BRIDGE.with(|cell| {
                let mut slot = cell.borrow_mut();
                let Some(owner) = slot.as_mut() else {
                    return BindStep::NoBridge;
                };
                if owner.failure().is_some()
                    || matches!(owner.state(), crate::gtk_bridge::BridgeState::Closing)
                {
                    return BindStep::Skipped;
                }
                let generation = owner.generation();
                if !bind_allowed(false, failed_generation, generation) {
                    return BindStep::Skipped;
                }
                BindStep::Attempted(generation, owner.set_render_session(Some(render)))
            });
            match step {
                BindStep::Attempted(generation, Ok(())) => {
                    let mut state = lock(shared);
                    state.shell.cad_ready = true;
                    state.shell.cad_bind_failed_generation = None;
                    clear_host_error(&mut state.shell, HostErrorKind::Bind);
                    eprintln!("[probe] cad render session bound (generation {generation})");
                }
                BindStep::Attempted(generation, Err(error)) => {
                    let mut state = lock(shared);
                    state.shell.cad_bind_failed_generation = Some(generation);
                    note_host_error(
                        &mut state.shell,
                        HostErrorKind::Bind,
                        format!("cad bind failed: {error}"),
                    );
                    eprintln!(
                        "[probe] cad bind FAILED (generation {generation}); fail-stop until a new generation: {error}"
                    );
                }
                BindStep::NoBridge | BindStep::Skipped => {}
            }
        }
    }

    // ---- CAD commands + ordered inputs (batches copied, lock released) ----
    let cad_log = std::env::var_os("YACR_PROBE_CAD_LOG").is_some();
    // Telemetry: the latest input sequence in this batch and its IPC-to-drain
    // age (measured from the Rust `Instant` recorded at admission).
    let mut last_input_seq: Option<u64> = None;
    let mut input_age_us: Option<u128> = None;
    if cad_mode {
        let (commands, inputs) = {
            let mut state = lock(shared);
            (
                std::mem::take(&mut state.shell.cad_commands),
                std::mem::take(&mut state.shell.cad_inputs),
            )
        };
        let queued = commands.len() + inputs.len();
        last_input_seq = inputs.last().map(|input| input.sequence);
        input_age_us = last_input_seq.and_then(|sequence| {
            let state = lock(shared);
            state
                .shell
                .cad_input_recv
                .iter()
                .rev()
                .find(|(seq, _)| *seq == sequence)
                .map(|(_, received)| received.elapsed().as_micros())
        });
        let inputs_started = Instant::now();
        if queued > 0 {
            CAD.with(|cell| {
                if let Some(session) = cell.borrow().as_ref() {
                    for command in &commands {
                        // Admission already returned `{accepted:true}`. The real
                        // business outcome is the CadSession snapshot error; it
                        // is not duplicated into the host transient error.
                        if let Err(error) = session.command(command) {
                            eprintln!("[probe] cad command rejected: {error}");
                        }
                    }
                    for input in &inputs {
                        if let Err(error) = session.handle_input(input) {
                            eprintln!("[probe] cad input rejected: {error}");
                        }
                    }
                }
            });
        }
        let inputs_us = inputs_started.elapsed().as_micros();
        // One owner-thread tick (host file/open/save/metrics/preparation + pump).
        // A tick failure is a host transient error, cleared only by a later
        // successful tick.
        let tick_started = Instant::now();
        let tick = CAD.with(|cell| cell.borrow().as_ref().map(|session| session.tick()));
        let tick_us = tick_started.elapsed().as_micros();
        match tick {
            Some(Ok(())) => clear_host_error(&mut lock(shared).shell, HostErrorKind::Tick),
            Some(Err(error)) => note_host_error(
                &mut lock(shared).shell,
                HostErrorKind::Tick,
                format!("cad tick failed: {error}"),
            ),
            None => {}
        }
        if cad_log && queued > 0 {
            eprintln!("[probe] cad-stage queued={queued} input_us={inputs_us} tick_us={tick_us}");
        }
    }

    // ---- GPU work, without holding the shared lock ----
    let surface = BRIDGE.with(|cell| cell.borrow().as_ref().and_then(|o| o.surface_size()));
    if cad_mode {
        let (logical, scale) = {
            let rect = lock(shared).shell.input.last_rect();
            let scale = lock(shared)
                .window
                .as_ref()
                .and_then(|window| window.scale_factor().ok())
                .unwrap_or(1.0)
                .max(0.1);
            (
                rect.map(|rect| [f64::from(rect.width), f64::from(rect.height)])
                    .unwrap_or([0.0, 0.0]),
                scale,
            )
        };
        // Draw one CAD frame through the bound render session (main thread, no
        // shared lock held across the engine/GPU work).
        let frame_started = Instant::now();
        let frame_result: Option<Result<Option<u64>, String>> = CAD.with(|cell| {
            let borrowed = cell.borrow();
            let session = borrowed.as_ref()?;
            Some(BRIDGE.with(|bridge| {
                let mut slot = bridge.borrow_mut();
                let Some(owner) = slot.as_mut() else {
                    return Ok(None);
                };
                // Explicit bridge-state admission: only a `Ready` bridge presents.
                // `AwaitingMap`/`Suspended` (globals/presenter still coming up, or
                // suspended for re-creation) wait — no present attempt, no error;
                // `Closing`/`Failed` are terminal and are never presented on, and
                // their diagnostics are preserved. This removes the spurious
                // `Present` error the former unconditional `frame_cad` call
                // produced while the bridge was still coming up.
                match cad_present_admission(owner.state()) {
                    CadPresentAdmission::Waiting | CadPresentAdmission::Terminal => {
                        return Ok(None);
                    }
                    CadPresentAdmission::Ready => {}
                }
                // `Ready` is only set after a successful presenter init, so this is
                // a genuine error, never a wait (no global "no presenter" success).
                if owner.failure().is_some() {
                    return Ok(None);
                }
                let (w, h) = owner.desired_size();
                if w == 0 || h == 0 {
                    return Ok(None);
                }
                owner.resize_to(w, h)?;
                match owner.frame_cad(logical, scale) {
                    Ok(Some(stats)) => {
                        session.note_frame(stats.frame_index);
                        Ok(Some(stats.frame_index))
                    }
                    // No new frame and no reusable retained frame: explicit
                    // waiting; no blank/fake grid is presented.
                    Ok(None) => Ok(None),
                    Err(error) => Err(error),
                }
            }))
        });
        let (backend, adapter) = BRIDGE.with(|cell| {
            let bridge = cell.borrow();
            bridge
                .as_ref()
                .and_then(|owner| owner.adapter())
                .map(|info| (Some(info.backend.clone()), Some(info.name.clone())))
                .unwrap_or((None, None))
        });
        match &frame_result {
            Some(Ok(Some(frames))) => {
                let mut state = lock(shared);
                state.shell.status.frames = *frames;
                state.shell.status.error = None;
                // A good present clears only a previous present error (never a
                // bind error, and never the CadSession business error).
                clear_host_error(&mut state.shell, HostErrorKind::Present);
            }
            // Explicit waiting: no present happened, so nothing is cleared.
            Some(Ok(None)) => {}
            Some(Err(error)) => {
                let mut state = lock(shared);
                note_host_error(
                    &mut state.shell,
                    HostErrorKind::Present,
                    format!("cad present failed: {error}"),
                );
            }
            None => {}
        }
        let frame_us = frame_started.elapsed().as_micros();
        // Engine render-gate diagnostics (diagnostics only; env-gated by
        // `YACR_PROBE_CAD_LOG`). Emitted on a state change or a ~1 s heartbeat, so
        // a waiting/upload stall can be diagnosed without per-frame log spam.
        if cad_log {
            let diagnostics = CAD.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|session| session.render_session().diagnostics())
            });
            if let Some(diagnostics) = diagnostics {
                log_render_diagnostics(shared, &diagnostics);
            }
        }
        // Actual-render observation: taken **once** per real renderer invocation
        // (a retained/waiting frame can never replay stale stats). Always recorded
        // for the post-load driver's admission; logged only under CAD_LOG.
        let actual = CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|session| session.render_session().take_render_stats_observation())
        });
        if let Some(actual) = actual {
            if cad_log {
                eprintln!(
                    "[probe] actual-render seq={} device_epoch={} view_rev={} applied_rev={:?} applied_base={:?} cpu_ms={:.3} gpu_ms={:?} plan_ms={:.3} uniform_ms={:.3} encode_ms={:.3} submit_ms={:.3} wait_ms={:.3} draw_calls={} verts={} tris={} instanced={} mesh={} legacy_opaque={} arc={} transparent={} images={} accelerated={} instance_streams={} instanced_batches={} tier={} over_budget={} prog={:?}/{:?} complete={} uploads_started={} rev_resets={} ready_none={} pulled={}",
                    actual.render_seq,
                    actual.device_epoch,
                    actual.view_revision,
                    actual.applied_revision,
                    actual.applied_base,
                    actual.cpu_ms,
                    actual.gpu_ms,
                    actual.plan_ms,
                    actual.uniform_ms,
                    actual.encode_ms,
                    actual.submit_ms,
                    actual.wait_ms,
                    actual.draw_calls,
                    actual.vertices,
                    actual.triangles,
                    actual.instanced_draws,
                    actual.mesh_draws,
                    actual.legacy_opaque_draws,
                    actual.arc_draws,
                    actual.transparent_batches,
                    actual.image_batches,
                    actual.accelerated,
                    actual.instance_streams,
                    actual.instanced_batches,
                    actual.tier,
                    actual.over_budget,
                    actual.progressive_cursor,
                    actual.progressive_total,
                    actual.progressive_complete,
                    actual.uploads_started,
                    actual.upload_revision_resets,
                    actual.ready_none_clears,
                    actual.pulled_state,
                );
            }
            lock(shared).last_actual_render = Some(ActualRenderSummary {
                render_seq: actual.render_seq,
                view_revision: actual.view_revision,
                device_epoch: actual.device_epoch,
            });
            lock(shared).last_pulled_state = Some(actual.pulled_state.clone());
        }
        let snapshot = CAD.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|session| session.snapshot(backend, adapter, 0))
        });
        if let Some(snapshot) = &snapshot {
            lock(shared).shell.cad_status = Some(snapshot.clone());
        }
        // Bounded CAD stage telemetry (diagnostics only): ONE aggregated line
        // when an upload work unit staged/committed, or the host frame was slow
        // (> 50 ms). Both observations are taken (consumed) at most once, so a
        // stale value is never replayed; no per-batch logging.
        if cad_log {
            let stages = BRIDGE.with(|cell| {
                cell.borrow_mut()
                    .as_mut()
                    .and_then(|owner| owner.take_frame_cad_stages())
            });
            let upload = CAD.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|session| session.render_session().take_upload_timing_observation())
            });
            let frame_total = stages
                .map(|stages| stages.apply_us + stages.render_us + stages.present_us)
                .unwrap_or(0);
            if upload.is_some() || frame_total >= CAD_STAGE_SLOW_US {
                let (apply_us, render_us, present_us, presented) = stages
                    .map(|stages| {
                        (
                            stages.apply_us,
                            stages.render_us,
                            stages.present_us,
                            stages.presented,
                        )
                    })
                    .unwrap_or((0, 0, 0, false));
                let upload_text = match &upload {
                    Some(upload) => {
                        let commit = match &upload.commit {
                            Some(commit) => format!(
                                "rebase={} append={} gc={} pulled={} pick={} lineweight={} total={}",
                                commit.rebase_us,
                                commit.append_us,
                                commit.gc_us,
                                commit.pulled_us,
                                commit.pick_us,
                                commit.lineweight_us,
                                commit.total_us
                            ),
                            None => "-".to_string(),
                        };
                        format!(
                            "seq={} scene_rev={} base_changed={} committed={} group={} batches={} prepare_us={} append_us={} commit_us={} total_us={} commit[{}]",
                            upload.seq,
                            upload.scene_revision,
                            upload.base_changed,
                            upload.committed,
                            upload.group,
                            upload.batches,
                            upload.prepare_us,
                            upload.append_us,
                            upload.commit_us,
                            upload.total_us,
                            commit
                        )
                    }
                    None => "-".to_string(),
                };
                eprintln!(
                    "[probe] cad-stages apply_us={apply_us} render_us={render_us} present_us={present_us} presented={presented} frame_total_us={frame_total} upload={upload_text}"
                );
            }
        }
        // Env-gated, default-OFF bounded post-load pan evidence driver (runs after
        // the session status so it can gate on the imported document identity).
        drive_postload_pan(shared);
        // Consume the presented-frame observation exactly once, and only for a
        // real present (never for a waiting/error frame), so the telemetry can
        // never replay a stale frame.
        let observation = if matches!(&frame_result, Some(Ok(Some(_)))) {
            BRIDGE.with(|cell| {
                cell.borrow_mut()
                    .as_mut()
                    .and_then(|owner| owner.take_last_cad_frame_observation())
            })
        } else {
            None
        };
        if let Some(observation) = observation {
            // The native presenter's real CAD stage timings (this is what makes
            // the periodic `status … stages_us[…]` meaningful in CAD mode).
            lock(shared).last_timings = Some(observation.timings);
        }
        if cad_log {
            let view_current = CAD.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|session| session.render_session().view_revision())
            });
            let state_tag = cad_frame_log_state(&frame_result);
            if let (Some(obs), CadFrameLogState::Presented) = (observation, state_tag) {
                let camera = snapshot.as_ref().and_then(|status| status.camera);
                let (cx, cy, cscale) = camera
                    .map(|c| (c.center[0], c.center[1], c.scale))
                    .unwrap_or((f64::NAN, f64::NAN, f64::NAN));
                let t = obs.timings;
                eprintln!(
                    "[probe] cad-frame presented view_current={} view_rendered={} complete={} device_epoch={} texture_rev={} size={}x{} frame_id={} frame_us={} input_seq={} input_age_us={} camera_center={cx:.3},{cy:.3} camera_scale={cscale:.6} stages_us[write={} acquire={} configure={} submit={} scope={} present={} total={}]",
                    view_current.unwrap_or(0),
                    obs.rendered_view_revision,
                    frame_completeness_label(obs.completeness),
                    obs.device_epoch,
                    obs.texture_revision,
                    obs.size.0,
                    obs.size.1,
                    obs.frame_id,
                    frame_us,
                    opt_telemetry_u64(last_input_seq),
                    opt_telemetry_u128(input_age_us),
                    t.write_buffer_us,
                    t.acquire_us,
                    t.configure_us,
                    t.submit_us,
                    t.scope_pop_us,
                    t.present_us,
                    t.total_us,
                );
            } else {
                eprintln!(
                    "[probe] cad-frame {} frame_us={} view_current={}",
                    state_tag.label(),
                    frame_us,
                    view_current.unwrap_or(0),
                );
            }
        }
    } else {
        // ---- probe frame (marker path; `--probe` mode only) ----
        let outcome: Option<Result<(u64, ProbeStageTimings), String>> = BRIDGE.with(|cell| {
            let mut slot = cell.borrow_mut();
            let owner = slot.as_mut()?;
            if owner.failure().is_some() {
                return None;
            }
            let (w, h) = owner.desired_size();
            if w == 0 || h == 0 {
                return None;
            }
            if let Err(error) = owner.resize_to(w, h) {
                return Some(Err(error));
            }
            owner.frame(marker)
        });
        if let Some(outcome) = outcome {
            let mut state = lock(shared);
            match outcome {
                Ok((frames, timings)) => {
                    state.shell.status.frames = frames;
                    // Report the actual child-surface size when the surface
                    // exists, otherwise the window size (top-level).
                    if let Some((w, h)) = surface {
                        state.shell.status.width = w;
                        state.shell.status.height = h;
                    } else {
                        state.shell.status.width = size.width;
                        state.shell.status.height = size.height;
                    }
                    state.last_timings = Some(timings);
                    state.shell.status.error = None;
                }
                Err(error) => {
                    state.shell.status.error = Some(error);
                }
            }
        }
    }
}

/// Log the top-level native window handle the probe surface is created on.
///
/// Both supported backends are reported as first-class: the X11 XID line is
/// unchanged (capture scripts parse it), and a native Wayland toplevel logs the
/// actual `wl_surface`/`wl_display` pair as surface identity — it is never an
/// error. The Wayland *window* handle carries only the `wl_surface` (this
/// `raw-window-handle` version has no `display` field on it), so the paired
/// `wl_display` is read separately from the window's display handle instead of
/// being invented.
fn log_top_level_native_handle(window: &tauri::Window) {
    match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::Xlib(xlib) => eprintln!(
                "[probe] top-level X11 XID (Xlib) = 0x{:x} visual=0x{:x}",
                xlib.window, xlib.visual_id
            ),
            RawWindowHandle::Xcb(xcb) => {
                eprintln!("[probe] top-level X11 XID (Xcb) = 0x{:x}", xcb.window.get())
            }
            RawWindowHandle::Wayland(wayland) => match wayland_display_ptr(window) {
                Some(display) => eprintln!(
                    "[probe] top-level native Wayland surface={:p} display={:p}",
                    wayland.surface.as_ptr(),
                    display
                ),
                None => eprintln!(
                    "[probe] top-level native Wayland surface={:p} display=<unavailable>",
                    wayland.surface.as_ptr()
                ),
            },
            other => eprintln!("[probe] unexpected top-level raw handle: {other:?}"),
        },
        Err(error) => eprintln!("[probe] could not read the top-level window handle: {error}"),
    }
}

/// The raw `wl_display` pointer paired with the toplevel native window.
///
/// A Wayland window handle exposes only the `wl_surface`, so the display is read
/// from the window's display handle; a non-Wayland or unreadable display returns
/// `None` rather than a fabricated pointer.
fn wayland_display_ptr(window: &tauri::Window) -> Option<*mut std::ffi::c_void> {
    match window.display_handle() {
        Ok(handle) => match handle.as_raw() {
            RawDisplayHandle::Wayland(display) => Some(display.display.as_ptr()),
            _ => None,
        },
        Err(_) => None,
    }
}

/// `probe_status` command: return the current probe status snapshot.
#[tauri::command]
pub fn probe_status(shared: tauri::State<'_, ProbeShared>) -> ProbeStatus {
    lock(&shared.0).shell.status.clone()
}

/// Admission result of `cad_command`: queued, **not** completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CadCommandAck {
    /// Always `true` for an admitted command; it never reports completion.
    pub accepted: bool,
}

/// `cad_command` command: admit a known CAD command into the bounded main-thread
/// queue.
///
/// An unknown command is rejected immediately (never queued). `{accepted:true}`
/// means the command was **queued**, not that it completed; the real outcome
/// surfaces through `cad_status.error`/`loading`.
#[tauri::command]
pub fn cad_command(
    shared: tauri::State<'_, ProbeShared>,
    command: String,
) -> Result<CadCommandAck, String> {
    let mut state = lock(&shared.0);
    admit_command(&mut state.shell, &command)?;
    Ok(CadCommandAck { accepted: true })
}

/// `cad_status` command: the latest main-thread CAD snapshot, with the host-side
/// (transport) error merged in.
///
/// Before a session exists (or in probe mode) the snapshot is explicit (no
/// fabricated document/count/camera). A host bind/present/tick error is shown
/// even when the CadSession business error is `None`, and the business/open
/// error is preserved when the host error clears.
#[tauri::command]
pub fn cad_status(shared: tauri::State<'_, ProbeShared>) -> CadStatus {
    let status = {
        let state = lock(&shared.0);
        let host_error = state
            .shell
            .cad_host_error
            .as_ref()
            .map(|error| error.text.as_str());
        match state.shell.cad_status.clone() {
            Some(snapshot) => projected_status(snapshot, host_error),
            None => unsupported_status(None),
        }
    };
    // Env-gated, evidence-only JSON dump of the exact payload the frontend
    // receives. No behavior change: disabled unless `YACR_PROBE_CAD_LOG` is set.
    // The pulled organisation state is appended as a separate token (the JSON is
    // unchanged and stays first) so a skipped accelerated organisation is visible
    // next to the status payload.
    if std::env::var_os("YACR_PROBE_CAD_LOG").is_some() {
        let pulled = lock(&shared.0)
            .last_pulled_state
            .as_ref()
            .map(PulledState::to_string)
            .unwrap_or_else(|| "-".to_string());
        match serde_json::to_string(&status) {
            Ok(json) => eprintln!("[probe] cad_status {json} pulled={pulled}"),
            Err(error) => eprintln!("[probe] cad_status serialize failed: {error} pulled={pulled}"),
        }
    }
    status
}

/// `probe_input` command: sequence-gate one frontend input and route it.
///
/// In CAD mode a pointer/wheel input is queued for the main thread only when the
/// CAD session is ready; before that it is refused with an explicit `cad_waiting`
/// reason (never a pretend success), and a full queue is refused explicitly.
/// `resize` is applied by the tick through the CAD rect path.
#[tauri::command]
pub fn probe_input(shared: tauri::State<'_, ProbeShared>, input: ProbeInput) -> ProbeInputAck {
    let kind = input.kind;
    let sequence = input.sequence;
    let ack = lock(&shared.0).shell.input.apply(&input);
    // Evidence line: shows the real IPC path (invoke -> command -> tracker).
    // x/y/dx/dy/width are logged so the accepted input can be tied to real OS
    // coordinates (diagnosis only; identifiers stay English).
    eprintln!(
        "[probe] input kind={} seq={} accepted={} marker={:.3} terminal={} reason={:?} x={:?} y={:?} dx={:?} dy={:?} width={:?}",
        kind.as_str(),
        sequence,
        ack.accepted,
        ack.marker_offset,
        ack.terminal,
        ack.reason,
        input.x,
        input.y,
        input.dx,
        input.dy,
        input.width,
    );
    if !ack.accepted {
        return ack;
    }
    // Post-load pan driver accounting: count host-admitted gesture inputs, and
    // flag a concurrent resize (which would make any view change unattributable
    // to the pan).
    {
        let mut state = lock(&shared.0);
        if let Some(pan) = state.postload_pan.as_mut() {
            match kind {
                InputKind::PointerMove => pan.admitted_moves += 1,
                InputKind::PointerUp => pan.admitted_ups += 1,
                InputKind::Resize => {
                    if matches!(
                        pan.phase,
                        PostloadPanPhase::Gesture | PostloadPanPhase::AwaitingFinal
                    ) {
                        pan.concurrent_resize = true;
                    }
                }
                _ => {}
            }
        }
    }
    if matches!(kind, InputKind::Resize) {
        // The tick maps the accepted rect to GTK margins and canvas metrics.
        return ack;
    }
    let queued = {
        let mut state = lock(&shared.0);
        queue_cad_input(&mut state.shell, &input)
    };
    match queued {
        Ok(()) => ack,
        Err(reason) => ProbeInputAck {
            accepted: false,
            sequence,
            marker_offset: ack.marker_offset,
            terminal: ack.terminal,
            reason: Some(reason),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_options_parse_cli() {
        let opts = LaunchOptions::from_args(
            [
                "--open",
                "/tmp/a.dwg",
                "--fonts-dir",
                "/tmp/fonts",
                "--images-dir",
                "/tmp/img",
                "--probe",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(
            opts.drawing.as_deref(),
            Some(std::path::Path::new("/tmp/a.dwg"))
        );
        assert_eq!(
            opts.fonts_dir.as_deref(),
            Some(std::path::Path::new("/tmp/fonts"))
        );
        assert_eq!(
            opts.images_dir.as_deref(),
            Some(std::path::Path::new("/tmp/img"))
        );
        assert!(opts.probe_only);
        assert_eq!(opts.size, [960.0, 600.0]);
        // Unknown args are ignored; missing values leave the field unset.
        let empty = LaunchOptions::from_args(["--nope"].into_iter().map(String::from));
        assert!(empty.drawing.is_none() && !empty.probe_only);
    }

    #[test]
    fn positional_argument_opens_the_drawing() {
        let (opts, diagnostics) =
            LaunchOptions::parse_args(["/tmp/plan.dwg"].into_iter().map(String::from));
        assert_eq!(
            opts.drawing.as_deref(),
            Some(std::path::Path::new("/tmp/plan.dwg"))
        );
        assert_eq!(
            diagnostics,
            vec!["[probe] positional drawing argument: /tmp/plan.dwg".to_string()]
        );
    }

    #[test]
    fn open_flag_takes_precedence_over_positional() {
        let (opts, diagnostics) = LaunchOptions::parse_args(
            ["--open", "/tmp/real.dwg", "/tmp/ignored.dwg"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(
            opts.drawing.as_deref(),
            Some(std::path::Path::new("/tmp/real.dwg"))
        );
        assert_eq!(
            diagnostics,
            vec![
                "[probe] --open takes precedence; positional drawing argument ignored: /tmp/ignored.dwg"
                    .to_string()
            ]
        );

        // `--open` after the positional also wins (precedence is not order-based).
        let (opts, _) = LaunchOptions::parse_args(
            ["/tmp/ignored.dwg", "--open", "/tmp/real.dwg"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(
            opts.drawing.as_deref(),
            Some(std::path::Path::new("/tmp/real.dwg"))
        );
    }

    #[test]
    fn multiple_positionals_open_first_and_report_extras() {
        let (opts, diagnostics) = LaunchOptions::parse_args(
            ["/tmp/first.dwg", "/tmp/second.dwg", "/tmp/third.dwg"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(
            opts.drawing.as_deref(),
            Some(std::path::Path::new("/tmp/first.dwg"))
        );
        assert_eq!(
            diagnostics,
            vec![
                "[probe] positional drawing argument: /tmp/first.dwg".to_string(),
                "[probe] extra positional argument(s) ignored (multi-document open is NOT IMPLEMENTED): /tmp/second.dwg /tmp/third.dwg"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn file_uri_is_diagnosed_unsupported_and_not_a_path() {
        let (opts, diagnostics) =
            LaunchOptions::parse_args(["file:///tmp/plan.dwg"].into_iter().map(String::from));
        assert!(opts.drawing.is_none(), "a URI must never become a path");
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].starts_with("error: "));
        assert!(diagnostics[0].contains("file:///tmp/plan.dwg"));
        assert!(diagnostics[0].contains("NOT IMPLEMENTED"));

        // A URI alongside a real positional: the refusal is reported and the
        // plain path still opens.
        let (opts, diagnostics) = LaunchOptions::parse_args(
            ["file:///tmp/a.dwg", "/tmp/b.dwg"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(
            opts.drawing.as_deref(),
            Some(std::path::Path::new("/tmp/b.dwg"))
        );
        assert!(diagnostics.iter().any(|line| line.starts_with("error: ")));
        assert!(diagnostics
            .iter()
            .any(|line| line == "[probe] positional drawing argument: /tmp/b.dwg"));
    }

    #[test]
    fn unknown_flag_still_ignored_and_no_args_yields_none() {
        let (opts, diagnostics) =
            LaunchOptions::parse_args(["--frobnicate", "-x"].into_iter().map(String::from));
        assert!(opts.drawing.is_none());
        assert!(diagnostics.is_empty(), "unknown flags are silently ignored");

        let (opts, diagnostics) = LaunchOptions::parse_args(std::iter::empty::<String>());
        assert!(opts.drawing.is_none());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn positional_does_not_disturb_dir_options() {
        let (opts, _) = LaunchOptions::parse_args(
            [
                "--fonts-dir",
                "/tmp/fonts",
                "/tmp/plan.dwg",
                "--images-dir",
                "/tmp/img",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(
            opts.drawing.as_deref(),
            Some(std::path::Path::new("/tmp/plan.dwg"))
        );
        assert_eq!(
            opts.fonts_dir.as_deref(),
            Some(std::path::Path::new("/tmp/fonts"))
        );
        assert_eq!(
            opts.images_dir.as_deref(),
            Some(std::path::Path::new("/tmp/img"))
        );
    }

    #[test]
    fn start_maximized_defaults_true_and_opts_out_explicitly() {
        assert!(start_maximized(false), "the default is maximized");
        assert!(
            !start_maximized(true),
            "the acceptance-only env opts out of maximize"
        );
    }

    #[test]
    fn cad_frame_log_state_never_claims_a_present_for_unknown_or_waiting() {
        assert_eq!(cad_frame_log_state(&None), CadFrameLogState::NoSession);
        assert_eq!(
            cad_frame_log_state(&Some(Ok(None))),
            CadFrameLogState::Waiting
        );
        assert_eq!(
            cad_frame_log_state(&Some(Err("x".into()))),
            CadFrameLogState::Error
        );
        assert_eq!(
            cad_frame_log_state(&Some(Ok(Some(7)))),
            CadFrameLogState::Presented
        );
        // Unknown/waiting/error are never a present.
        assert_ne!(CadFrameLogState::NoSession, CadFrameLogState::Presented);
        assert_ne!(CadFrameLogState::Waiting, CadFrameLogState::Presented);
        assert_ne!(CadFrameLogState::Error, CadFrameLogState::Presented);
        assert_eq!(
            frame_completeness_label(FrameCompleteness::CurrentScene),
            "current_scene"
        );
        assert_eq!(
            frame_completeness_label(FrameCompleteness::BaseOnlyPendingOverlays),
            "base_only_pending_overlays"
        );
        assert_eq!(opt_telemetry_u64(None), "-");
        assert_eq!(opt_telemetry_u64(Some(3)), "3");
        assert_eq!(opt_telemetry_u128(None), "-");
        assert_eq!(opt_telemetry_u128(Some(42)), "42");
    }

    #[test]
    fn cad_present_admission_maps_each_bridge_state_without_fake_success() {
        use crate::gtk_bridge::BridgeState;
        // A bridge that is coming up / suspended for re-creation waits: no
        // present is attempted and no error is recorded.
        assert_eq!(
            cad_present_admission(&BridgeState::AwaitingMap),
            CadPresentAdmission::Waiting
        );
        assert_eq!(
            cad_present_admission(&BridgeState::Suspended),
            CadPresentAdmission::Waiting
        );
        // Only a Ready bridge may present.
        assert_eq!(
            cad_present_admission(&BridgeState::Ready),
            CadPresentAdmission::Ready
        );
        // Terminal states never present (their diagnostics stay untouched).
        assert_eq!(
            cad_present_admission(&BridgeState::Closing),
            CadPresentAdmission::Terminal
        );
        assert_eq!(
            cad_present_admission(&BridgeState::Failed("presenter init failed".into())),
            CadPresentAdmission::Terminal
        );
        // The distinctions are real: waiting/terminal are never "ready" (no state
        // is silently folded into a successful present).
        assert_ne!(CadPresentAdmission::Waiting, CadPresentAdmission::Ready);
        assert_ne!(CadPresentAdmission::Terminal, CadPresentAdmission::Ready);
    }

    #[test]
    fn render_diag_logs_on_change_or_heartbeat_but_never_per_frame() {
        // Changed state: log immediately.
        assert!(should_log_render_diag(false, false));
        assert!(should_log_render_diag(false, true));
        // Unchanged but the heartbeat elapsed: log again.
        assert!(should_log_render_diag(true, false));
        // Unchanged within the heartbeat window: suppress (no per-frame spam).
        assert!(!should_log_render_diag(true, true));
    }

    fn actual(render_seq: u64, view_revision: u64) -> ActualRenderSummary {
        ActualRenderSummary {
            render_seq,
            view_revision,
            device_epoch: 1,
        }
    }

    fn report(done: bool, moves: u64, ups: u64, elapsed_ms: i64, failed: bool) -> PanReport {
        PanReport {
            run_id: 1,
            done,
            moves,
            downs: 1,
            ups,
            elapsed_ms,
            failed,
        }
    }

    #[test]
    fn postload_pan_import_gate_rejects_demo_loading_and_wrong_document() {
        // The demo placeholder (loading, wrong label) is never admitted.
        assert!(!postload_pan_import_ready(
            true,
            None,
            Some("yacr-demo"),
            1,
            "complex-test.dwg"
        ));
        // The demo label at rest (loading false) still does not match the requested
        // drawing.
        assert!(!postload_pan_import_ready(
            false,
            None,
            Some("yacr-demo"),
            1,
            "complex-test.dwg"
        ));
        // Still importing.
        assert!(!postload_pan_import_ready(
            true,
            None,
            Some("/x/complex-test.dwg"),
            1,
            "complex-test.dwg"
        ));
        // Import error is never admitted.
        assert!(!postload_pan_import_ready(
            false,
            Some("open failed"),
            Some("/x/complex-test.dwg"),
            1,
            "complex-test.dwg"
        ));
        // No verified import transition yet.
        assert!(!postload_pan_import_ready(
            false,
            None,
            Some("/x/complex-test.dwg"),
            0,
            "complex-test.dwg"
        ));
        // A different imported document is not the requested one.
        assert!(!postload_pan_import_ready(
            false,
            None,
            Some("/x/other.dwg"),
            1,
            "complex-test.dwg"
        ));
        // The requested drawing, imported and at rest -> admitted.
        assert!(postload_pan_import_ready(
            false,
            None,
            Some("/home/u/fixtures/complex-test.dwg"),
            1,
            "complex-test.dwg"
        ));
        assert_eq!(label_file_name("/a/b/c.dwg"), "c.dwg");
        assert_eq!(label_file_name("c.dwg"), "c.dwg");
    }

    #[test]
    fn postload_pan_readiness_requires_presented_and_matching_actual_render() {
        // The gate is satisfied only when the requested drawing is presented and
        // the latest actual render matches the current view AND device epoch.
        assert!(postload_pan_import_ready(
            false,
            None,
            Some("/x/complex-test.dwg"),
            1,
            "complex-test.dwg"
        ));
        // A matching actual render is required for readiness (checked here via the
        // final_ok/camera helpers used alongside it).
        assert!(!postload_pan_camera_changed(Some(&actual(3, 5)), 5));
        assert!(postload_pan_camera_changed(Some(&actual(3, 6)), 5));
    }

    #[test]
    fn postload_pan_gesture_complete_requires_moves_up_elapsed_and_admission() {
        // No report yet.
        assert!(!postload_pan_gesture_complete(None, 1, 1));
        // Done but no moves/up (the observed bug): never complete.
        assert!(!postload_pan_gesture_complete(
            Some(&report(true, 0, 0, 8000, false)),
            1,
            1
        ));
        // Done, moves, but no up.
        assert!(!postload_pan_gesture_complete(
            Some(&report(true, 480, 0, 8000, false)),
            1,
            1
        ));
        // Too few JS moves (loop stalled).
        assert!(!postload_pan_gesture_complete(
            Some(&report(true, 5, 1, 8000, false)),
            1,
            1
        ));
        // Elapsed too short (a premature/aborted gesture).
        assert!(!postload_pan_gesture_complete(
            Some(&report(true, 480, 1, 3000, false)),
            1,
            0
        ));
        // A recorded JS failure is never completion.
        assert!(!postload_pan_gesture_complete(
            Some(&report(true, 480, 1, 8000, true)),
            1,
            1
        ));
        // The host never admitted a move -> not completion.
        assert!(!postload_pan_gesture_complete(
            Some(&report(true, 480, 1, 8000, false)),
            0,
            1
        ));
        // Full completion: JS progress + elapsed + host-admitted move and up.
        assert!(postload_pan_gesture_complete(
            Some(&report(true, 480, 1, 8000, false)),
            1,
            1
        ));
    }

    #[test]
    fn postload_pan_final_ok_requires_a_new_render_at_a_changed_view() {
        // Not presented.
        assert!(!postload_pan_final_ok(true, None, 5, 1));
        assert!(!postload_pan_final_ok(false, Some(&actual(3, 6)), 5, 2));
        // No NEW render (render_seq must be beyond the gesture end) — a WM resize
        // that only changed the view cannot satisfy this with a stale render.
        assert!(!postload_pan_final_ok(true, Some(&actual(2, 6)), 5, 2));
        // New render but the same view as the start (no pan effect).
        assert!(!postload_pan_final_ok(true, Some(&actual(3, 5)), 5, 2));
        // New render at a changed view + presented -> final frame admitted.
        assert!(postload_pan_final_ok(true, Some(&actual(3, 6)), 5, 2));
    }

    #[test]
    fn parse_pan_report_extracts_the_structured_marker() {
        let parsed =
            parse_pan_report("run=1 done=true moves=483 downs=1 ups=1 elapsed_ms=8012 failed=")
                .expect("parsed");
        assert_eq!(parsed.run_id, 1);
        assert!(parsed.done && !parsed.failed);
        assert_eq!((parsed.moves, parsed.ups), (483, 1));
        assert_eq!(parsed.elapsed_ms, 8012);
        // A failed gesture carries a non-empty `failed=`.
        let failed = parse_pan_report(
            "run=2 done=false moves=5 downs=1 ups=0 elapsed_ms=-1 failed=move:err",
        )
        .expect("parsed");
        assert!(failed.failed && !failed.done);
        assert_eq!(failed.run_id, 2);
        // No marker at all.
        assert_eq!(parse_pan_report("no-gesture"), None);
        assert_eq!(parse_pan_report("error:boom"), None);
    }

    #[test]
    fn parse_pan_report_decodes_the_json_quoted_callback_payload() {
        // The EXACT form captured from the Tauri/GTK callback: a JSON-quoted
        // string (this is what broke the driver).
        let quoted = r#""run=1 done=true moves=80 downs=1 ups=1 elapsed_ms=8049 failed=""#;
        let parsed = parse_pan_report(quoted).expect("quoted payload must parse");
        assert_eq!(parsed.run_id, 1);
        assert!(parsed.done && !parsed.failed);
        assert_eq!((parsed.moves, parsed.ups), (80, 1));
        assert_eq!(parsed.elapsed_ms, 8049);

        // A raw (unquoted) marker is still accepted.
        let parsed = parse_pan_report("run=2 done=true moves=483 ups=1 elapsed_ms=8012 failed=")
            .expect("raw marker must parse");
        assert_eq!(parsed.run_id, 2);

        // Escaped JSON is decoded (not parsed as a raw marker).
        let escaped = r#""run=3 done=true moves=9 ups=1 elapsed_ms=8001 failed=x\"y""#;
        let parsed = parse_pan_report(escaped).expect("escaped payload must parse");
        assert_eq!(parsed.run_id, 3);
        assert!(parsed.failed, "the escaped failed= is non-empty");

        // A JSON-quoted non-marker, and malformed `run=`, are rejected.
        assert_eq!(parse_pan_report(r#""no-gesture""#), None);
        assert_eq!(parse_pan_report("run=abc done=true"), None);
        assert_eq!(parse_pan_report(r#""run=""#), None);
    }

    #[test]
    fn pan_run_id_and_match_helpers_track_the_unique_run() {
        assert_eq!(
            parse_pan_run_id(r#""started run=1 rect=0,203,926,266 dur=8000""#),
            Some(1)
        );
        assert_eq!(
            parse_pan_run_id("started run=7 rect=0,203,1,1 dur=8000"),
            Some(7)
        );
        assert_eq!(parse_pan_run_id("no-element"), None);
        assert_eq!(parse_pan_run_id(r#""tiny-rect:1x1""#), None);

        let report = PanReport {
            run_id: 1,
            ..PanReport::default()
        };
        // No expected id yet: accept.
        assert!(pan_report_matches_run(None, &report));
        // Matching id: accept. A different id: reject (stale/foreign report).
        assert!(pan_report_matches_run(Some(1), &report));
        assert!(!pan_report_matches_run(Some(2), &report));
    }

    #[test]
    fn process_exit_status_maps_a_driver_failure_to_non_zero() {
        // A default/manual run or a passing driver exits 0.
        assert_eq!(resolve_process_exit(None), 0);
        assert_eq!(resolve_process_exit(Some(0)), 0);
        // An explicit bounded failure must reach the OS, never be silently 0.
        assert_eq!(resolve_process_exit(Some(3)), 3);
    }

    #[test]
    fn postload_pan_step_is_bounded_and_never_fakes_a_pass() {
        use PostloadPanAction as A;
        use PostloadPanPhase as P;
        // Waiting for the requested drawing: start, keep waiting, or bounded fail.
        assert_eq!(
            postload_pan_step(P::WaitingReady, true, false, false, false, false, false),
            A::StartGesture
        );
        assert_eq!(
            postload_pan_step(P::WaitingReady, false, false, false, false, false, false),
            A::None
        );
        assert_eq!(
            postload_pan_step(P::WaitingReady, false, false, false, false, true, false),
            A::Fail
        );
        // Gesture in flight: poll, begin final on completion, fail on a concurrent
        // resize or the overall bound.
        assert_eq!(
            postload_pan_step(P::Gesture, false, false, false, false, false, false),
            A::PollReport
        );
        assert_eq!(
            postload_pan_step(P::Gesture, false, true, false, false, false, false),
            A::BeginFinal
        );
        assert_eq!(
            postload_pan_step(P::Gesture, false, true, false, true, false, false),
            A::Fail
        );
        assert_eq!(
            postload_pan_step(P::Gesture, false, false, false, false, true, false),
            A::Fail
        );
        // Final wait: finish only on the final frame; otherwise bounded failure.
        assert_eq!(
            postload_pan_step(P::AwaitingFinal, false, false, true, false, false, false),
            A::Finish
        );
        assert_eq!(
            postload_pan_step(P::AwaitingFinal, false, false, false, false, false, true),
            A::Fail
        );
        assert_eq!(
            postload_pan_step(P::AwaitingFinal, false, false, false, true, false, false),
            A::Fail
        );
        // Terminal phases are inert.
        assert_eq!(
            postload_pan_step(P::Finished, true, true, true, true, true, true),
            A::None
        );
        assert_eq!(
            postload_pan_step(P::Failed, true, true, true, true, true, true),
            A::None
        );
    }

    #[test]
    fn layout_revalidates_when_the_container_bounds_change_without_a_new_sequence() {
        let a = (960.0_f32, 553.0_f32);
        let b = (1200.0_f32, 800.0_f32);
        // First decision: no prior bounds -> re-validate (never a strict accept
        // against unknown/zero bounds).
        assert!(layout_needs_revalidate(false, None, a));
        // A generation change always re-validates.
        assert!(layout_needs_revalidate(true, Some(a), a));
        // The container resized under an unchanged DOM rect (no new sequence):
        // still re-validate, so the cached rect is re-checked against the new
        // container instead of replaying stale insets.
        assert!(layout_needs_revalidate(false, Some(a), b));
        // Same container + same generation: a strict `accept` (which rejects a
        // stale sequence and therefore does not re-apply/re-flood).
        assert!(!layout_needs_revalidate(false, Some(a), a));
    }
}
