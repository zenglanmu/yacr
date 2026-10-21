use std::collections::VecDeque;

use super::admission::{cad_present_admission, layout_needs_revalidate, CadPresentAdmission};
use super::commands::{
    admit_selected_text, admit_standard_view, admit_tool_parameter, admit_tool_text,
    drain_standard_views, parse_standard_view, parse_tool_parameter, CAD_SELECTED_TEXT_CAP,
    CAD_STANDARD_VIEW_CAP, CAD_TOOL_PARAMETER_CAP, CAD_TOOL_TEXT_CAP,
};
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

/// The properties-panel write channel's own contracts (kept in their own file
/// so this one stays under the repository's file-size limit).
mod properties;

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
fn tool_parameter_admission_is_typed_bounded_and_refuses_probe_mode() {
    // The `cad_set_tool_parameter` ack path: the value is only *queued* for the
    // owner thread (never applied off the main thread), refused while the CAD
    // session is disabled (probe mode), and bounded. Range/kind/Viewer validation
    // is the session's authority, applied on drain, not here.
    use cad_app::{ToolParameter, ToolValue};
    let mut queue: Vec<(ToolParameter, ToolValue)> = Vec::new();
    assert_eq!(
        admit_tool_parameter(
            &mut queue,
            false,
            ToolParameter::PolygonSides,
            ToolValue::Number(4.0)
        )
        .unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty(), "a probe-mode refusal queues nothing");

    // Accepted values are queued verbatim — no clamp, no range/kind check here.
    admit_tool_parameter(
        &mut queue,
        true,
        ToolParameter::PolygonSides,
        ToolValue::Number(3.0),
    )
    .expect("accepted");
    admit_tool_parameter(
        &mut queue,
        true,
        ToolParameter::PolygonSides,
        ToolValue::Number(4.0),
    )
    .expect("accepted");
    // A kind-mismatched value is *also* admitted verbatim here: the session
    // refuses it on drain, exactly like an out-of-range number.
    admit_tool_parameter(
        &mut queue,
        true,
        ToolParameter::PolygonSides,
        ToolValue::Flag(true),
    )
    .expect("accepted verbatim; the session refuses the mismatch on drain");
    assert_eq!(
        queue,
        vec![
            (ToolParameter::PolygonSides, ToolValue::Number(3.0)),
            (ToolParameter::PolygonSides, ToolValue::Number(4.0)),
            (ToolParameter::PolygonSides, ToolValue::Flag(true)),
        ]
    );

    // Bounded: filling to capacity then one more is an explicit refusal.
    queue.clear();
    for _ in 0..CAD_TOOL_PARAMETER_CAP {
        admit_tool_parameter(
            &mut queue,
            true,
            ToolParameter::PolygonSides,
            ToolValue::Number(6.0),
        )
        .expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_TOOL_PARAMETER_CAP);
    assert_eq!(
        admit_tool_parameter(
            &mut queue,
            true,
            ToolParameter::PolygonSides,
            ToolValue::Number(6.0)
        )
        .unwrap_err(),
        "CAD tool-parameter queue is full"
    );
}

#[test]
fn tool_text_admission_is_bounded_and_refuses_probe_mode_and_static_content() {
    // The `cad_set_tool_text` ack path (ADR 0019 decision A.1): the text is only
    // *queued* for the owner thread, the static content rules (non-empty, at most
    // 1024 characters) are refused before the queue, and the live state
    // (is a tool awaiting text? are its points complete?) is the session's
    // authority on drain — this admission deliberately knows no session state.
    let mut queue: Vec<String> = Vec::new();
    assert_eq!(
        admit_tool_text(&mut queue, false, "Hello".into()).unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty(), "a probe-mode refusal queues nothing");
    assert_eq!(
        admit_tool_text(&mut queue, true, String::new()).unwrap_err(),
        "tool.text_empty: cad_set_tool_text needs a non-empty text"
    );
    assert_eq!(
        admit_tool_text(&mut queue, true, "  \t".into()).unwrap_err(),
        "tool.text_empty: cad_set_tool_text needs a non-empty text"
    );
    let over_long = "a".repeat(cad_app::MAX_TOOL_TEXT_CHARS + 1);
    let refusal = admit_tool_text(&mut queue, true, over_long).unwrap_err();
    assert!(refusal.contains("tool.text_too_long"), "{refusal}");
    assert!(
        queue.is_empty(),
        "content refusals queue nothing: {queue:?}"
    );

    // Exactly at the bound is accepted, verbatim (no trimming, no truncation).
    let at_bound = "b".repeat(cad_app::MAX_TOOL_TEXT_CHARS);
    admit_tool_text(&mut queue, true, at_bound.clone()).expect("the bound is accepted");
    admit_tool_text(&mut queue, true, "  padded  ".into()).expect("accepted verbatim");
    assert_eq!(queue, vec![at_bound, "  padded  ".to_string()]);

    // Bounded: filling to capacity then one more is an explicit refusal.
    queue.clear();
    for _ in 0..CAD_TOOL_TEXT_CAP {
        admit_tool_text(&mut queue, true, "x".into()).expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_TOOL_TEXT_CAP);
    assert_eq!(
        admit_tool_text(&mut queue, true, "x".into()).unwrap_err(),
        "CAD tool-text queue is full"
    );
}

#[test]
fn selected_text_admission_is_bounded_and_refuses_probe_mode_and_static_content() {
    // The `cad_set_selected_text` ack path (ADR 0020 amendment B): the text is
    // only *queued* for the owner thread, the static content rules are refused
    // before the queue, and whether the current selection is one editable `Text`
    // is the session's authority on drain — this admission knows no selection.
    let mut queue: Vec<String> = Vec::new();
    assert_eq!(
        admit_selected_text(&mut queue, false, "Hello".into()).unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert!(queue.is_empty(), "a probe-mode refusal queues nothing");
    assert_eq!(
        admit_selected_text(&mut queue, true, String::new()).unwrap_err(),
        "tool.text_empty: cad_set_selected_text needs a non-empty text"
    );
    assert_eq!(
        admit_selected_text(&mut queue, true, "  \t".into()).unwrap_err(),
        "tool.text_empty: cad_set_selected_text needs a non-empty text"
    );
    let over_long = "a".repeat(cad_app::MAX_TOOL_TEXT_CHARS + 1);
    let refusal = admit_selected_text(&mut queue, true, over_long).unwrap_err();
    assert!(refusal.contains("tool.text_too_long"), "{refusal}");
    assert!(refusal.contains("cad_set_selected_text"), "{refusal}");
    assert!(
        queue.is_empty(),
        "content refusals queue nothing: {queue:?}"
    );

    // Exactly at the bound is accepted verbatim; the queue holds the text only
    // (no entity id — the host resolves the current selection).
    let at_bound = "b".repeat(cad_app::MAX_TOOL_TEXT_CHARS);
    admit_selected_text(&mut queue, true, at_bound.clone()).expect("the bound is accepted");
    admit_selected_text(&mut queue, true, "  padded  ".into()).expect("accepted verbatim");
    assert_eq!(queue, vec![at_bound, "  padded  ".to_string()]);

    // Bounded: filling to capacity then one more is an explicit refusal.
    queue.clear();
    for _ in 0..CAD_SELECTED_TEXT_CAP {
        admit_selected_text(&mut queue, true, "x".into()).expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_SELECTED_TEXT_CAP);
    assert_eq!(
        admit_selected_text(&mut queue, true, "x".into()).unwrap_err(),
        "CAD selected-text queue is full"
    );
}

#[test]
fn standard_view_wire_names_parse_case_insensitively_and_refuse_unknown_or_empty() {
    // The `cad_standard_view` wire form: the stable machine key of a standard
    // view, matched ASCII case-insensitively. Every one of the seven views is
    // reachable by name; an empty request and an unknown name are distinct,
    // named refusals (never a default view, never a clamp).
    use cad_app::StandardView;

    for (wire, expected) in [
        ("top", StandardView::Top),
        ("BOTTOM", StandardView::Bottom),
        ("Front", StandardView::Front),
        ("bAcK", StandardView::Back),
        ("left", StandardView::Left),
        ("RIGHT", StandardView::Right),
        ("isometric", StandardView::Isometric),
        ("  Isometric  ", StandardView::Isometric),
    ] {
        assert_eq!(parse_standard_view(wire), Ok(expected), "{wire:?}");
    }
    // Distinct wire names map to distinct variants: no key aliases another.
    let mut keys: Vec<&str> = StandardView::ALL.iter().map(|view| view.key()).collect();
    let count = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), count, "standard-view keys are unique");

    let empty = parse_standard_view("   ").unwrap_err();
    let unknown = parse_standard_view("diagonal").unwrap_err();
    assert!(empty.starts_with("view.standard_empty"), "{empty}");
    assert!(unknown.starts_with("view.standard_unknown"), "{unknown}");
    assert_ne!(empty, unknown);
    assert!(unknown.contains("diagonal"), "{unknown}");
    // The refusal lists the accepted vocabulary instead of guessing a view.
    assert!(unknown.contains("isometric"), "{unknown}");
}

#[test]
fn standard_view_admission_is_bounded_and_refuses_probe_mode_and_the_modal_gate() {
    // The `cad_standard_view` ack path: the typed view is only *queued* for the
    // owner thread. The gates are `admit_command`'s: the modal popup gate first,
    // then a disabled CAD session, then the bound.
    use cad_app::StandardView;

    let mut queue: VecDeque<StandardView> = VecDeque::new();
    assert_eq!(
        admit_standard_view(&mut queue, false, false, StandardView::Top).unwrap_err(),
        "CAD is disabled in probe mode"
    );
    assert_eq!(
        admit_standard_view(&mut queue, true, true, StandardView::Top).unwrap_err(),
        cad_ui_core::shell::POPUP_MODAL_OPEN
    );
    assert!(queue.is_empty(), "refusals queue nothing");

    admit_standard_view(&mut queue, true, false, StandardView::Isometric).expect("accepted");
    assert_eq!(queue.pop_front(), Some(StandardView::Isometric));

    for _ in 0..CAD_STANDARD_VIEW_CAP {
        admit_standard_view(&mut queue, true, false, StandardView::Front).expect("within capacity");
    }
    assert_eq!(queue.len(), CAD_STANDARD_VIEW_CAP);
    assert_eq!(
        admit_standard_view(&mut queue, true, false, StandardView::Top).unwrap_err(),
        "CAD standard-view queue is full"
    );
    // The modal gate is checked before the bound, exactly like `admit_command`:
    // a full queue behind an open modal reports the modal reason.
    assert_eq!(
        admit_standard_view(&mut queue, true, true, StandardView::Top).unwrap_err(),
        cad_ui_core::shell::POPUP_MODAL_OPEN
    );
}

#[test]
fn drained_standard_views_apply_in_order_and_empty_the_queue() {
    // Two queued views apply in admission order (the last one wins on the
    // camera) and the queue is emptied. A real refusal is returned, never
    // swallowed, and does not stop the remaining views.
    use cad_app::StandardView;

    let mut queue = VecDeque::from([StandardView::Front, StandardView::Isometric]);
    let mut applied = Vec::new();
    let refusals = drain_standard_views(&mut queue, |view| {
        applied.push(view);
        Ok(())
    });
    assert!(refusals.is_empty(), "{refusals:?}");
    assert_eq!(
        applied,
        vec![StandardView::Front, StandardView::Isometric],
        "admission order is preserved"
    );
    assert!(queue.is_empty(), "everything queued was drained");

    let mut queue = VecDeque::from([StandardView::Back, StandardView::Top]);
    let mut applied = Vec::new();
    let refusals = drain_standard_views(&mut queue, |view| {
        applied.push(view);
        if view == StandardView::Back {
            Err("view.refused: test refusal".into())
        } else {
            Ok(())
        }
    });
    assert_eq!(applied, vec![StandardView::Back, StandardView::Top]);
    assert_eq!(refusals, vec!["view.refused: test refusal".to_string()]);
    assert!(queue.is_empty());
}

#[test]
fn tool_parameter_wire_shape_parses_the_key_and_exactly_one_value() {
    // The exact `cad_set_tool_parameter` wire shape: a stable machine-key string
    // plus exactly one of `number`/`flag`. An unknown key or a malformed value
    // pairing is refused before anything is queued.
    use cad_app::{ToolParameter, ToolValue};

    assert_eq!(
        parse_tool_parameter("polygonSides", Some(6.0), None).unwrap(),
        (ToolParameter::PolygonSides, ToolValue::Number(6.0))
    );
    assert_eq!(
        parse_tool_parameter("alignScale", None, Some(true)).unwrap(),
        (ToolParameter::AlignScale, ToolValue::Flag(true))
    );
    assert_eq!(
        parse_tool_parameter("arrayFillAngle", Some(90.0), None).unwrap(),
        (ToolParameter::ArrayFillAngle, ToolValue::Number(90.0))
    );

    // An unknown key is refused with the key named.
    assert!(parse_tool_parameter("nope", Some(1.0), None)
        .unwrap_err()
        .contains("nope"));
    // Neither value, or both, is refused.
    assert!(parse_tool_parameter("polygonSides", None, None).is_err());
    assert!(parse_tool_parameter("polygonSides", Some(1.0), Some(true)).is_err());
}

#[test]
fn the_tool_status_projects_awaiting_text_on_the_wire_for_the_react_contract() {
    // The committed React lane reads `tool.awaiting_text` (`ui/src/text.mjs`,
    // `ui/src/cad.mjs`) — the exact snake_case wire name, not `awaitingText`.
    // This pins the real serialized payload, because a rename here would silently
    // never open the text prompt.
    use cad_ui_core::session::{CadTool, CadToolParams};
    let tool = CadTool {
        key: "draw.text".into(),
        points: 1,
        ready: false,
        open_ended: false,
        awaiting_text: true,
        text_prefill: Some("current".into()),
        params: CadToolParams {
            sides: None,
            fillet_radius: None,
            chamfer_distance_a: None,
            chamfer_distance_b: None,
            array_rows: None,
            array_columns: None,
            array_row_spacing: None,
            array_column_spacing: None,
            array_items: None,
            array_fill_angle: None,
            array_rotate_items: None,
            align_scale: None,
            hatch_pattern_type: None,
            hatch_spacing: None,
            hatch_angle: None,
            hatch_double: None,
            hatch_style: None,
            text_height: Some(2.5),
        },
    };
    let value = serde_json::to_value(&tool).expect("the tool serializes");
    assert_eq!(
        value.get("awaiting_text").and_then(|flag| flag.as_bool()),
        Some(true),
        "the wire carries `awaiting_text`: {value}"
    );
    assert!(
        value.get("awaitingText").is_none(),
        "the camelCase spelling must not be emitted: {value}"
    );
    // The TEXTEDIT prefill uses the same snake_case wire convention.
    assert_eq!(
        value.get("text_prefill").and_then(|text| text.as_str()),
        Some("current"),
        "the wire carries `text_prefill`: {value}"
    );
    assert!(
        value.get("textPrefill").is_none(),
        "the camelCase spelling must not be emitted: {value}"
    );
    assert_eq!(
        value
            .get("params")
            .and_then(|params| params.get("textHeight"))
            .and_then(|height| height.as_f64()),
        Some(2.5),
        "the TEXT height uses the params block's camelCase convention: {value}"
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

#[path = "settings_tests.rs"]
mod settings_tests;
