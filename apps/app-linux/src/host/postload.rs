//! Env-gated, bounded post-load pan evidence driver (default OFF). Pure
//! decision helpers plus the main-thread tick driver; never a fake pass.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{lock, Shared, CAD, WEBVIEW};

/// A compact summary of the latest **actual** render (for the post-load driver's
/// readiness/final-frame admission). Not a replay of stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActualRenderSummary {
    pub(super) render_seq: u64,
    pub(super) view_revision: u64,
    pub(super) device_epoch: u64,
}

/// Bounded post-load pan driver phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PostloadPanPhase {
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
pub(super) enum PostloadPanAction {
    None,
    StartGesture,
    PollReport,
    BeginFinal,
    Finish,
    Fail,
}

/// Parsed structured gesture report (from the RUNID-tracked eval-response marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct PanReport {
    pub(super) run_id: u64,
    pub(super) done: bool,
    pub(super) moves: u64,
    pub(super) downs: u64,
    pub(super) ups: u64,
    pub(super) elapsed_ms: i64,
    pub(super) failed: bool,
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
pub(super) struct PostloadPanState {
    pub(super) started: Instant,
    pub(super) overall_deadline: Instant,
    pub(super) phase: PostloadPanPhase,
    /// View revision before the gesture (the pan must move away from it).
    pub(super) start_view_revision: u64,
    /// Actual-render sequence observed when the gesture report completed (the
    /// final frame must be a strictly newer actual render).
    pub(super) end_render_seq: u64,
    /// Next structured report poll.
    pub(super) poll_at: Option<Instant>,
    /// Bound for the final frame after the gesture completes.
    pub(super) final_deadline: Option<Instant>,
    /// Host-admitted gesture inputs during the run.
    pub(super) admitted_moves: u64,
    pub(super) admitted_ups: u64,
    /// A `resize` input arrived while the gesture/final wait was active: any
    /// resulting view change is not attributable to the pan → explicit failure.
    pub(super) concurrent_resize: bool,
    /// Latest parsed report (RUNID-tracked).
    pub(super) report: Option<PanReport>,
    /// How many gestures this run performs (default 1 = unchanged semantics;
    /// `YACR_PROBE_POSTLOAD_PAN_GESTURES`).
    pub(super) gestures_target: u32,
    /// Gestures already settled in this run.
    pub(super) gestures_done: u32,
    /// The unique run id captured from the gesture response; a report with a
    /// different id is rejected.
    pub(super) expected_run_id: Option<u64>,
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
    pub(super) fn new() -> Self {
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
pub(super) fn label_file_name(label: &str) -> &str {
    label.rsplit(['/', '\\']).next().unwrap_or(label)
}

/// Admission for starting the gesture: the **requested** drawing — not the
/// `yacr-demo` placeholder — has been imported (a verified import transition with
/// no pending load and no error, and its document label matches the requested
/// file name), it is **presented**, AND the latest actual render matches the
/// current view revision and device epoch. Never an entity count or a fixed delay.
pub(super) fn postload_pan_import_ready(
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
pub(super) fn parse_pan_run_id(text: &str) -> Option<u64> {
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
pub(super) fn parse_pan_report(raw: &str) -> Option<PanReport> {
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
pub(super) fn pan_report_matches_run(expected: Option<u64>, report: &PanReport) -> bool {
    expected.is_none_or(|id| id == report.run_id)
}

/// Whether the gesture is proven complete: the report is done without failure,
/// the JS loop advanced (moves) and ended (`up`), the elapsed time covers the
/// gesture duration, AND the host actually admitted moves and an up.
pub(super) fn postload_pan_gesture_complete(
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
pub(super) fn postload_pan_camera_changed(
    actual: Option<&ActualRenderSummary>,
    start_view_revision: u64,
) -> bool {
    actual.is_some_and(|actual| actual.view_revision != start_view_revision)
}

/// Admission for the final frame: the requested drawing is presented AND a **new**
/// actual render (render_seq beyond the gesture end) landed at a view different
/// from the pre-gesture one. A concurrent WM resize can never satisfy this.
pub(super) fn postload_pan_final_ok(
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
pub(super) fn postload_pan_step(
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

/// Bounded post-load pan evidence driver (env-gated, default OFF). Runs on the
/// main-thread tick: waits for the **requested** drawing (never the demo
/// placeholder) to be imported/presented with an actual render at the current
/// view/device, starts one synthetic DOM pan gesture, waits for its **structured
/// completion report** (RUNID-tracked; a view change alone is not completion),
/// then waits (bounded) for the final rendered+presented frame and exits
/// naturally. Any timeout, missing moves/up, or a concurrent resize is an
/// explicit bounded failure, never a fake pass.
pub(super) fn drive_postload_pan(shared: &Arc<Mutex<Shared>>) {
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
