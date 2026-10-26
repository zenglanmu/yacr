//! Contract tests for `YACR_PROBE_UI_DIALOGS`.
//!
//! They pin the parts an external evidence run depends on and that cannot be
//! checked without a display: the env gate and its parsing, the scheduled
//! timeline, the exact log-line prefixes, and the log bodies built from real
//! `cad_status` reads. Nothing here claims a GUI run happened.

use std::collections::BTreeSet;

use cad_ui_core::session::{
    CadCommandLogEntry, CadCommandLogKind, CadCommandLogPoint, CadMeasurementEcho, CadSelection,
    CadSelectionRow,
};

use super::super::ui_dialogs_report::{
    command_log_line, decode_payload, flat, measurement_value, selection_detail,
    selection_rows_complete, xid_detail, Evidence,
};
use super::*;

// ---------------------------------------------------------------- env gate ----

#[test]
fn env_gate_is_off_by_default_and_accepts_only_documented_values() {
    assert!(!enabled_value(None));
    for value in ["", "0", "false", "no", "TRUE", "on", "2"] {
        assert!(!enabled_value(Some(value)), "{value:?} must not enable");
    }
    for value in ["1", "true", "yes"] {
        assert!(enabled_value(Some(value)), "{value:?} must enable");
    }
}

// ------------------------------------------------------------ start seconds ----

#[test]
fn start_seconds_uses_the_default_when_unset_or_blank() {
    assert_eq!(parse_start_seconds(None), (DEFAULT_START_SECONDS, None));
    assert_eq!(parse_start_seconds(Some("")), (DEFAULT_START_SECONDS, None));
    assert_eq!(
        parse_start_seconds(Some("   ")),
        (DEFAULT_START_SECONDS, None)
    );
}

#[test]
fn start_seconds_accepts_zero_and_positive_seconds() {
    assert_eq!(parse_start_seconds(Some("0")), (0, None));
    assert_eq!(parse_start_seconds(Some("12")), (12, None));
    assert_eq!(parse_start_seconds(Some(" 7 ")), (7, None));
}

#[test]
fn start_seconds_reports_an_unparseable_value_instead_of_hiding_it() {
    let (seconds, notice) = parse_start_seconds(Some("soon"));
    assert_eq!(seconds, DEFAULT_START_SECONDS);
    let notice = notice.expect("an unparseable AT must be reported");
    assert!(notice.contains("YACR_PROBE_UI_DIALOGS_AT"), "{notice}");
    assert!(notice.contains("\"soon\""), "{notice}");

    let (seconds, notice) = parse_start_seconds(Some("-1"));
    assert_eq!(seconds, DEFAULT_START_SECONDS);
    assert!(notice.is_some(), "a negative AT must be reported");
}

// ------------------------------------------------------------- hold seconds ----

#[test]
fn hold_seconds_defaults_to_thirty() {
    assert_eq!(parse_hold_seconds(None), (DEFAULT_HOLD_SECONDS, None));
    assert_eq!(parse_hold_seconds(Some("")), (DEFAULT_HOLD_SECONDS, None));
    assert_eq!(DEFAULT_HOLD_SECONDS, 30);
}

#[test]
fn hold_seconds_is_floored_so_the_popup_handle_read_and_grab_have_a_window() {
    for raw in ["0", "1", "9"] {
        let (seconds, notice) = parse_hold_seconds(Some(raw));
        assert_eq!(seconds, MIN_HOLD_SECONDS, "{raw} must be floored");
        let notice = notice.expect("a floored hold must be reported");
        assert!(notice.contains("floor"), "{notice}");
        assert!(notice.contains(&MIN_HOLD_SECONDS.to_string()), "{notice}");
    }
    assert_eq!(parse_hold_seconds(Some("10")), (MIN_HOLD_SECONDS, None));
}

#[test]
fn hold_seconds_accepts_a_longer_explicit_hold() {
    assert_eq!(parse_hold_seconds(Some("12")), (12, None));
    assert_eq!(parse_hold_seconds(Some(" 90 ")), (90, None));
}

#[test]
fn hold_seconds_reports_an_unparseable_value() {
    let (seconds, notice) = parse_hold_seconds(Some("long"));
    assert_eq!(seconds, DEFAULT_HOLD_SECONDS);
    let notice = notice.expect("an unparseable HOLD must be reported");
    assert!(notice.contains("YACR_PROBE_UI_DIALOGS_HOLD"), "{notice}");
}

// ---------------------------------------------------------------- timeline ----

#[test]
fn timeline_is_strictly_increasing() {
    let timeline = Timeline::new(3, 30);
    let seconds = [
        timeline.properties_key(),
        timeline.properties_read(),
        timeline.select_click(),
        timeline.select_read(),
        timeline.command_dist(),
        timeline.pick_a(),
        timeline.pick_b(),
        timeline.command_log_read(),
        timeline.options(),
        timeline.popup_handle(),
        timeline.popup_handle_retry(),
        timeline.close(),
        timeline.close_read(),
        timeline.summary(),
    ];
    assert_eq!(seconds[0], 3, "the start second is the env override");
    for pair in seconds.windows(2) {
        assert!(
            pair[1] > pair[0],
            "timeline must increase: {} then {}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn timeline_close_follows_the_hold_after_the_options_open() {
    for hold in [MIN_HOLD_SECONDS, 30, 90] {
        let timeline = Timeline::new(5, hold);
        assert_eq!(timeline.close(), timeline.options() + hold);
        assert!(timeline.close() > timeline.popup_handle_retry());
        assert_eq!(timeline.close_read(), timeline.close() + 2);
        assert_eq!(timeline.summary(), timeline.close() + 4);
        assert_eq!(timeline.hold(), hold);
    }
}

#[test]
fn timeline_describe_names_every_scheduled_second() {
    let timeline = Timeline::new(3, 30);
    let detail = timeline.describe();
    for (name, second) in [
        ("properties_key", timeline.properties_key()),
        ("properties_read", timeline.properties_read()),
        ("select_click", timeline.select_click()),
        ("select_read", timeline.select_read()),
        ("command_DIST", timeline.command_dist()),
        ("pick1", timeline.pick_a()),
        ("pick2", timeline.pick_b()),
        ("commandlog", timeline.command_log_read()),
        ("options", timeline.options()),
        ("popup_handle", timeline.popup_handle()),
        ("popup_handle_retry", timeline.popup_handle_retry()),
        ("options_close", timeline.close()),
        ("options_popup_after_close", timeline.close_read()),
        ("summary", timeline.summary()),
        ("hold", timeline.hold()),
    ] {
        let expected = format!("{name}={second}s");
        assert!(detail.contains(&expected), "{detail} is missing {expected}");
    }
}

// ------------------------------------------------------------- log prefixes ----

#[test]
fn step_lines_all_share_the_driver_prefix() {
    assert_eq!(
        step_line(7, "selection count=1"),
        "[probe] ui-dialogs: t=7s selection count=1"
    );
    for second in [0_u64, 3, 57, 61] {
        let line = step_line(second, "probe");
        assert!(line.starts_with("[probe] ui-dialogs: t="), "{line}");
        assert!(line.contains(&format!("t={second}s ")), "{line}");
    }
}

#[test]
fn labels_are_distinct_and_greppable() {
    let labels = [
        L_PROPERTIES_KEY,
        L_PROPERTIES_READ,
        L_MAIN_HANDLE,
        L_SELECT_CLICK,
        L_SELECT_READ,
        L_COMMAND_DIST,
        L_COMMAND_OPTIONS,
        L_PICK_A,
        L_PICK_B,
        L_COMMAND_LOG,
        L_POPUP_HANDLE,
        L_POPUP_RETRY,
        L_CLOSE,
        L_CLOSE_READ,
        L_SUMMARY,
    ];
    let unique: BTreeSet<&&str> = labels.iter().collect();
    assert_eq!(unique.len(), labels.len(), "two labels collide");
    for label in labels {
        assert!(!label.is_empty());
        assert_eq!(label.trim(), label, "{label:?} has outer whitespace");
        assert!(
            !label.contains(':'),
            "{label:?} would break the prefix grep"
        );
        assert!(
            !label.starts_with("FAILED"),
            "{label:?} shadows the failure marker"
        );
        assert!(!label.contains("  "), "{label:?} has doubled whitespace");
    }
    // The anchors external scripts grep for.
    assert!(L_PROPERTIES_KEY.contains("ctrl+1"));
    assert!(L_COMMAND_LOG.starts_with("commandlog"));
    assert!(L_SUMMARY == "summary");
}

// ------------------------------------------------------- selection log body ----

fn row(key: &str, value: Option<&str>, group: &str) -> CadSelectionRow {
    CadSelectionRow {
        key: key.to_string(),
        value: value.map(str::to_string),
        group: group.to_string(),
        mixed: value.is_none(),
        editable: false,
        kind: "text".to_string(),
        options: Vec::new(),
    }
}

fn single_line_selection() -> CadSelection {
    CadSelection {
        count: 1,
        empty: false,
        rows: vec![
            row("id", Some("1F"), "entity"),
            row("type", Some("LINE"), "entity"),
            row("layer", Some("0"), "entity"),
            row("color", Some("ByLayer"), "display"),
            row("linetype", Some("ByLayer"), "display"),
            row("lineweight", Some("ByLayer"), "display"),
            row("transparency", Some("ByLayer"), "display"),
        ],
        mixed_keys: Vec::new(),
        text: None,
        text_multiline: false,
    }
}

#[test]
fn selection_detail_lists_the_seven_palette_rows() {
    let selection = single_line_selection();
    let detail = selection_detail(&selection);
    assert_eq!(
        detail,
        "selection count=1 groups=entity,display \
         rows=id=1F;type=LINE;layer=0;color=ByLayer;linetype=ByLayer;lineweight=ByLayer;\
         transparency=ByLayer"
    );
    assert!(selection_rows_complete(&selection));
}

#[test]
fn selection_detail_marks_an_absent_row_a_mixed_value_and_flattens_whitespace() {
    let mut selection = single_line_selection();
    selection.rows.retain(|row| row.key != "transparency");
    if let Some(layer) = selection.rows.iter_mut().find(|row| row.key == "layer") {
        layer.value = Some("Layer 1".to_string());
    }
    if let Some(color) = selection.rows.iter_mut().find(|row| row.key == "color") {
        color.value = None;
        color.mixed = true;
    }
    let detail = selection_detail(&selection);
    assert!(detail.contains("layer=Layer_1"), "{detail}");
    assert!(detail.contains("color=mixed"), "{detail}");
    assert!(detail.contains("transparency=-"), "{detail}");
    assert!(
        !selection_rows_complete(&selection),
        "a missing key must not read as complete"
    );
}

#[test]
fn selection_detail_reports_an_empty_selection_honestly() {
    let detail = selection_detail(&CadSelection::empty());
    assert!(
        detail.starts_with("selection count=0 groups=- rows="),
        "{detail}"
    );
    for key in [
        "id",
        "type",
        "layer",
        "color",
        "linetype",
        "lineweight",
        "transparency",
    ] {
        assert!(
            detail.contains(&format!("{key}=-")),
            "{detail} is missing {key}"
        );
    }
    assert!(!selection_rows_complete(&CadSelection::empty()));
}

// ------------------------------------------------------- command-log body ----

#[test]
fn xid_detail_keeps_the_popup_evidence_shape() {
    assert_eq!(xid_detail("main window", 0x42), "main window xid=0x42");
    assert_eq!(xid_detail("popup", 0), "popup xid=0x0");
}

#[test]
fn measurement_value_uses_the_chromes_four_decimals() {
    assert_eq!(measurement_value(3.162_277_660_168_379), "3.1623");
    assert_eq!(measurement_value(0.0), "0.0000");
    assert_eq!(measurement_value(-1.5), "-1.5000");
}

#[test]
fn command_log_line_carries_the_measurement_echo() {
    let entry = CadCommandLogEntry {
        sequence: 4,
        kind: CadCommandLogKind::Result,
        command: None,
        code: None,
        measurement: Some(CadMeasurementEcho {
            algorithm: "distance".to_string(),
            value: 3.162_277_660_168_379,
            unit: Some("mm".to_string()),
            precision: Some("analytic".to_string()),
            points: vec![[0.0, 0.0, 0.0], [3.0, 1.0, 0.0]],
        }),
        point: None,
    };
    assert_eq!(
        command_log_line(&entry),
        "commandlog seq=4 kind=result command=- code=- algorithm=distance value=3.1623 \
         unit=mm precision=analytic points=0.000,0.000,0.000;3.000,1.000,0.000 point=-"
    );
}

#[test]
fn command_log_line_carries_the_point_echo() {
    // ID reports a coordinate, not a scalar measurement: the line must name the
    // position, its space and the snap that produced it, and keep `algorithm=-`
    // (nothing was measured).
    let entry = CadCommandLogEntry {
        sequence: 6,
        kind: CadCommandLogKind::Result,
        command: None,
        code: None,
        measurement: None,
        point: Some(CadCommandLogPoint {
            position: [12.5, -3.0, 0.0],
            space: "model",
            snap: Some("endpoint"),
            precision: Some("analytic"),
            unit: Some("mm".to_string()),
            pixel_distance: Some(2.5),
        }),
    };
    assert_eq!(
        command_log_line(&entry),
        "commandlog seq=6 kind=result command=- code=- algorithm=- value=- unit=- precision=- \
         points=- point=12.500,-3.000,0.000@model#endpoint"
    );
}

#[test]
fn command_log_line_shows_dashes_where_an_entry_has_no_echo() {
    let entry = CadCommandLogEntry {
        sequence: 3,
        kind: CadCommandLogKind::Command,
        command: Some("DIST".to_string()),
        code: None,
        measurement: None,
        point: None,
    };
    assert_eq!(
        command_log_line(&entry),
        "commandlog seq=3 kind=command command=DIST code=- algorithm=- value=- unit=- \
         precision=- points=- point=-"
    );

    let error = CadCommandLogEntry {
        sequence: 5,
        kind: CadCommandLogKind::Error,
        command: None,
        code: Some("tool_not_ready".to_string()),
        measurement: None,
        point: None,
    };
    let line = command_log_line(&error);
    assert!(line.contains("kind=error"), "{line}");
    assert!(line.contains("code=tool_not_ready"), "{line}");
    assert!(line.contains("value=-"), "{line}");
}

// -------------------------------------------------------------- eval payload ----

#[test]
fn decode_payload_unquotes_the_eval_callback_payload() {
    assert_eq!(decode_payload("\"mounted\""), "mounted");
    assert_eq!(decode_payload("  \"a b\\nc\"  "), "a b\nc");
    assert_eq!(decode_payload("not-json"), "not-json");
    assert_eq!(flat("a  b\nc"), "a_b_c");
}

// ------------------------------------------------------------------ summary ----

#[test]
fn summary_lists_only_the_parts_that_really_happened() {
    let mut evidence = Evidence::default();
    evidence.set_document(Some("synthetic-four-lines.dwg".to_string()));
    evidence.set_main_xid("0x3a00007".to_string());
    evidence.set_palette(true, Some("1024x768".to_string()));
    evidence.set_selection(1, true);
    evidence.set_command_log(3, true, Some("3.1623".to_string()));
    evidence.set_popup(
        "options:yacr-options:true".to_string(),
        Some("options"),
        Some("0x3a00042".to_string()),
    );
    evidence.set_close_ack();
    evidence.set_popup_after_close("-".to_string());

    let summary = evidence.summary(30);
    assert!(
        summary.contains("ui_dialogs_trigger=probe_driver_not_a_real_click"),
        "{summary}"
    );
    assert!(summary.contains("ui_dialogs_synthetic=1"), "{summary}");
    assert!(
        summary.contains("document=synthetic-four-lines.dwg"),
        "{summary}"
    );
    assert!(summary.contains("main_xid=0x3a00007"), "{summary}");
    assert!(summary.contains("selection_count=1"), "{summary}");
    assert!(summary.contains("commandlog_dist_command=1"), "{summary}");
    assert!(
        summary.contains("commandlog_dist_value=3.1623"),
        "{summary}"
    );
    assert!(summary.contains("popup_xid=0x3a00042"), "{summary}");
    assert!(summary.contains("popup_native_handle=options"), "{summary}");
    assert!(summary.contains("ui_dialogs_hold_s=30"), "{summary}");
    assert_eq!(
        verified_list(&summary),
        "properties_palette_mounted,selection_rows,commandline_submitted,\
         commandline_dist_result,options_dialog_open,options_dialog_xid,\
         options_dialog_closed"
    );
    assert!(summary.contains("ui_dialogs_failed=-"), "{summary}");
}

#[test]
fn a_degraded_run_cannot_read_as_a_pass() {
    let mut evidence = Evidence::default();
    evidence.set_palette(true, Some("1024x768".to_string()));
    let summary = evidence.summary(10);
    assert!(
        summary.contains("ui_dialogs_verified=properties_palette_mounted"),
        "{summary}"
    );
    for absent in [
        "selection_rows",
        "commandline_submitted",
        "options_dialog_open",
        "options_dialog_xid",
        "options_dialog_closed",
    ] {
        assert!(!summary.contains(absent), "{summary} claims {absent}");
    }
    assert!(summary.contains("ui_dialogs_hold_s=10"), "{summary}");
}

#[test]
fn a_closed_first_popup_read_is_not_a_failure_because_one_retry_is_scheduled() {
    let mut evidence = Evidence::default();
    evidence.set_popup("-".to_string(), None, None);
    assert!(!evidence.popup_opened());
    let summary = evidence.summary(10);
    assert!(summary.contains("options_popup=-"), "{summary}");
    assert!(
        summary.contains("ui_dialogs_failed=-"),
        "a first read before the retry must not fail the run: {summary}"
    );
    assert!(
        !summary.contains("options_dialog_xid"),
        "{summary} claims an XID it never read"
    );

    // The retry read is the one that judges.
    evidence.fail_popup_never_opened("-");
    let summary = evidence.summary(10);
    assert!(
        summary.contains("options_popup=the_Options_dialog_never_opened_(last_read_-)"),
        "{summary}"
    );
}

#[test]
fn palette_absence_a_wrong_selection_and_a_null_popup_are_recorded_as_failures() {
    let mut evidence = Evidence::default();
    evidence.set_palette(false, None);
    evidence.set_selection(2, false);
    evidence.set_command_log(1, false, None);
    evidence.set_popup("-".to_string(), None, None);
    evidence.fail_popup_never_opened("-");
    evidence.set_popup_after_close("options:yacr-options:true".to_string());

    let summary = evidence.summary(10);
    assert!(summary.contains("properties_palette=0"), "{summary}");
    assert!(
        summary.contains("properties=the_palette_is_not_mounted_after_Ctrl+1"),
        "{summary}"
    );
    assert!(
        summary.contains("selection=expected_count=1,_read_2"),
        "{summary}"
    );
    assert!(
        summary.contains("commandlog=no_admitted_DIST_entry_in_commandLog"),
        "{summary}"
    );
    assert!(
        summary.contains("options_popup=the_Options_dialog_never_opened_(last_read_-)"),
        "{summary}"
    );
    assert!(
        summary.contains("popup_after_close=expected_-,_read_options:yacr-options:true"),
        "{summary}"
    );
    assert!(summary.contains("ui_dialogs_verified=-"), "{summary}");
    assert!(!evidence.popup_opened());
}

/// The `ui_dialogs_verified=` value of a summary line (the field is printed
/// between `popup_after_close=… ` and ` ui_dialogs_hold_s=`).
fn verified_list(summary: &str) -> &str {
    let start = summary
        .find("ui_dialogs_verified=")
        .expect("summary has no ui_dialogs_verified field")
        + "ui_dialogs_verified=".len();
    let rest = &summary[start..];
    let end = rest
        .find(" ui_dialogs_hold_s=")
        .expect("verified list is not followed by ui_dialogs_hold_s");
    &rest[..end]
}
