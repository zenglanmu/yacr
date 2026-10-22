//! Wire contract pins for the WP1b command/state migration (behaviour-invariant).
//!
//! Provenance (honest record, WP1 gate finding 1): the fixtures were produced
//! by a temporary `wire_golden_capture` harness that ran the pre-migration
//! bodies in `apps/app-linux/src/host/commands.rs` and was removed with that
//! file before the migration commit. It never reached git history, so "captured
//! before the move" is NOT independently verifiable from the repository; treat
//! these fixtures as regression pins for the current wire format. The
//! independent equivalence argument is: the migration reused the command
//! bodies nearly verbatim (8 bodies byte-identical; rename hunks are
//! import/state wiring) and the registration set is asserted equal against the
//! pre-migration `entry.rs` list (`fixtures/registered_commands.txt`).
//!
//! Coverage boundary (WP1 gate finding 2): NOT covered — 11 of the 14 commands
//! (only `probe_status`/`cad_status`/`probe_input` are pinned), the telemetry
//! and popup seam paths (`YACR_PROBE_CAD_LOG` `pulled=` token, the
//! `popup.host_unavailable` refusal), canonical form vs actual IPC encoding
//! (tests call the `*_impl` bodies directly), and lock ordering (reviewed
//! separately in the WP1 gate; out of scope here).
//! The additive `cad_open_popup` ack `payload` field (WP3) is covered by shape
//! contract tests only (`cad_ui_core::popup::tests`); it has no byte golden here.

use std::sync::Arc;

use cad_desktop_shell::commands::{
    cad_status_impl, probe_input_impl, probe_status_impl, COMMAND_NAMES,
};
use cad_desktop_shell::state::{lock, new_shared, Shared};
use cad_ui_core::launch::LaunchOptions;
use cad_ui_core::protocol::ProbeInput;
use cad_ui_core::session::CadPopup;
use cad_ui_core::shell::{HostError, HostErrorKind};
use cad_ui_core::unsupported_status;
use serde_json::{json, Value};

fn probe_launch() -> LaunchOptions {
    LaunchOptions {
        probe_only: true,
        ..LaunchOptions::default()
    }
}

fn assert_golden(name: &str, value: &Value) {
    let actual = serde_json::to_string_pretty(value).expect("serialize") + "\n";
    let expected = match name {
        "probe_status_fresh.json" => include_str!("fixtures/probe_status_fresh.json"),
        "probe_status_populated.json" => include_str!("fixtures/probe_status_populated.json"),
        "probe_status_stopped.json" => include_str!("fixtures/probe_status_stopped.json"),
        "cad_status_probe_mode.json" => include_str!("fixtures/cad_status_probe_mode.json"),
        "cad_status_no_snapshot.json" => include_str!("fixtures/cad_status_no_snapshot.json"),
        "cad_status_projected_popup.json" => {
            include_str!("fixtures/cad_status_projected_popup.json")
        }
        "cad_status_clean_session.json" => {
            include_str!("fixtures/cad_status_clean_session.json")
        }
        "probe_input_acks.json" => include_str!("fixtures/probe_input_acks.json"),
        other => panic!("unknown golden {other}"),
    };
    assert_eq!(actual, expected, "wire golden mismatch: {name}");
}

fn decode_input(value: Value) -> ProbeInput {
    serde_json::from_value(value).expect("input decodes")
}

fn dump_status(name: &str, shared: &Arc<std::sync::Mutex<Shared>>) {
    let value = serde_json::to_value(probe_status_impl(shared)).expect("status");
    assert_golden(name, &value);
}

fn dump_cad_status(name: &str, shared: &Arc<std::sync::Mutex<Shared>>) {
    let value = serde_json::to_value(cad_status_impl(shared)).expect("status");
    assert_golden(name, &value);
}

#[test]
fn probe_status_is_byte_identical_to_the_pre_migration_host() {
    dump_status(
        "probe_status_fresh.json",
        &new_shared(LaunchOptions::default()),
    );

    let shared = new_shared(LaunchOptions::default());
    {
        let mut state = lock(&shared);
        state.shell.status.backend = Some("vulkan".into());
        state.shell.status.adapter = Some("Quadro P620".into());
        state.shell.status.width = 1280;
        state.shell.status.height = 720;
        state.shell.status.frames = 42;
        state.shell.status.error = Some("present failed".into());
    }
    dump_status("probe_status_populated.json", &shared);

    let stopped = new_shared(LaunchOptions::default());
    {
        let mut state = lock(&stopped);
        state.shell.status.stopped = true;
        state.shell.status.width = 0;
        state.shell.status.height = 0;
    }
    dump_status("probe_status_stopped.json", &stopped);
}

#[test]
#[allow(deprecated)] // the frozen `cad_status` wire label is built by hand here
fn cad_status_is_byte_identical_to_the_pre_migration_host() {
    dump_cad_status("cad_status_probe_mode.json", &new_shared(probe_launch()));
    dump_cad_status(
        "cad_status_no_snapshot.json",
        &new_shared(LaunchOptions::default()),
    );

    let projected = new_shared(LaunchOptions::default());
    {
        let mut state = lock(&projected);
        state.shell.cad_status = Some(unsupported_status(Some("open failed".into())));
        state.shell.cad_host_error = Some(HostError {
            kind: HostErrorKind::Present,
            text: "cad present failed: boom".into(),
        });
        state.shell.popup = Some(CadPopup {
            kind: "about".into(),
            label: "popup-1".into(),
            modal: true,
        });
    }
    dump_cad_status("cad_status_projected_popup.json", &projected);

    let clean = new_shared(LaunchOptions::default());
    {
        let mut state = lock(&clean);
        let mut snapshot = unsupported_status(None);
        snapshot.document_label = Some("/fixtures/complex-test.dwg".into());
        snapshot.entity_count = Some(7);
        snapshot.import_count = 1;
        snapshot.frame_count = 12;
        snapshot.can_open = true;
        snapshot.can_fit = true;
        state.shell.cad_status = Some(snapshot);
    }
    dump_cad_status("cad_status_clean_session.json", &clean);
}

#[test]
#[allow(deprecated)] // the frozen `cad_status` wire label is built by hand here
fn probe_input_acks_are_byte_identical_to_the_pre_migration_host() {
    let mut cases = Vec::new();

    let waiting = new_shared(LaunchOptions::default());
    cases.push(json!({
        "case": "cad_waiting",
        "ack": probe_input_impl(
            &waiting,
            decode_input(json!({"sequence": 1, "kind": "pointer_down", "x": 0.25, "width": 1000.0})),
        ),
    }));

    let probe = new_shared(probe_launch());
    cases.push(json!({
        "case": "probe_marker_move",
        "ack": probe_input_impl(
            &probe,
            decode_input(json!({"sequence": 1, "kind": "pointer_move", "x": 0.75, "width": 1000.0})),
        ),
    }));
    cases.push(json!({
        "case": "probe_marker_up",
        "ack": probe_input_impl(
            &probe,
            decode_input(json!({"sequence": 2, "kind": "pointer_up", "x": 0.75, "width": 1000.0})),
        ),
    }));

    let resize = new_shared(LaunchOptions::default());
    cases.push(json!({
        "case": "resize",
        "ack": probe_input_impl(
            &resize,
            decode_input(json!({
                "sequence": 1, "kind": "resize", "width": 960.0, "height": 553.0,
                "x": 0.0, "y": 203.0, "dpr": 1.0
            })),
        ),
    }));

    let ready = new_shared(LaunchOptions::default());
    lock(&ready).shell.cad_ready = true;
    cases.push(json!({
        "case": "cad_ready_pointer_down",
        "ack": probe_input_impl(
            &ready,
            decode_input(json!({"sequence": 1, "kind": "pointer_down", "x": 0.4, "width": 1000.0})),
        ),
    }));

    let modal = new_shared(LaunchOptions::default());
    {
        let mut state = lock(&modal);
        state.shell.popup = Some(CadPopup {
            kind: "about".into(),
            label: "popup-1".into(),
            modal: true,
        });
    }
    cases.push(json!({
        "case": "modal_gate",
        "ack": probe_input_impl(
            &modal,
            decode_input(json!({"sequence": 1, "kind": "pointer_move", "x": 0.5, "width": 1000.0})),
        ),
    }));

    let stale = new_shared(probe_launch());
    let _ = probe_input_impl(
        &stale,
        decode_input(json!({"sequence": 5, "kind": "pointer_move", "x": 0.5, "width": 1000.0})),
    );
    cases.push(json!({
        "case": "sequence_regression",
        "ack": probe_input_impl(
            &stale,
            decode_input(json!({"sequence": 4, "kind": "pointer_move", "x": 0.6, "width": 1000.0})),
        ),
    }));

    assert_golden("probe_input_acks.json", &Value::Array(cases));
}

#[test]
fn the_registered_command_set_equals_the_pre_migration_set() {
    let expected: Vec<&str> = include_str!("fixtures/registered_commands.txt")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let expected: std::collections::BTreeSet<&str> = expected.into_iter().collect();
    let names: std::collections::BTreeSet<&str> = COMMAND_NAMES.iter().copied().collect();
    assert_eq!(names, expected, "registered command names changed");

    // And the `generate_handler!` list really is that set, not just the constant.
    let source = include_str!("../src/commands.rs");
    let start = source
        .find("generate_handler![")
        .expect("one generate_handler! invocation");
    let body = &source[start + "generate_handler![".len()..];
    let end = body.find(']').expect("terminated list");
    let registered: std::collections::BTreeSet<&str> = body[..end]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    assert_eq!(registered, expected, "generate_handler! list changed");
}
