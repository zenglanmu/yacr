use super::admission::{cad_present_admission, layout_needs_revalidate, CadPresentAdmission};
use super::options::{start_maximized, LaunchOptions};
use super::postload::{
    label_file_name, pan_report_matches_run, parse_pan_report, parse_pan_run_id,
    postload_pan_camera_changed, postload_pan_final_ok, postload_pan_gesture_complete,
    postload_pan_import_ready, postload_pan_step, resolve_process_exit, ActualRenderSummary,
    PanReport, PostloadPanAction, PostloadPanPhase,
};
use super::telemetry::{
    cad_frame_log_state, frame_completeness_label, opt_telemetry_u128, opt_telemetry_u64,
    should_log_render_diag, CadFrameLogState,
};
use cad_engine::render::FrameCompleteness;

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
    let failed =
        parse_pan_report("run=2 done=false moves=5 downs=1 ups=0 elapsed_ms=-1 failed=move:err")
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
