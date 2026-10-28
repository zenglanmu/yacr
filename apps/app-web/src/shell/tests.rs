//! Host-target contract tests for the `dom-canvas` shell surface.
//!
//! The projection is asserted against a **real** session
//! (`cad_ui_core::session::CadSession` built from the synthetic fixture's bytes with
//! a render handle that has no device attached), so "the status is the session's own
//! snapshot" is proven rather than described. What this does not prove is anything
//! about pixels: the GPU half lives in `crate::page` and its evidence is the browser
//! run (`docs/validation-dom-canvas-flip-2026-10-28.md`).

use cad_ui_core::session::{unsupported_status, CadSession};

use super::*;
use crate::live::tests::fixture_session;

/// The keys the chrome's `parseCadStatus` requires on **every** payload
/// (`apps/desktop-ui/src/cad/cad-status.ts`). A missing one is a hard
/// `CAD_STATUS_INVALID`, so this list is the actual contract, not a sample.
const REQUIRED_KEYS: [&str; 21] = [
    "documentLabel",
    "entityCount",
    "loading",
    "importProgress",
    "error",
    "canNew",
    "canOpen",
    "canSave",
    "canPlot",
    "canFit",
    "backend",
    "adapter",
    "camera",
    "selection",
    "gridEnabled",
    "snapGridEnabled",
    "snapSpacing",
    "orthoEnabled",
    "cursorWorld",
    "importCount",
    "frameCount",
];

/// The carrier facts of a live session (the presenter's own numbers are injected:
/// this test has no GPU).
fn carrier() -> Carrier {
    Carrier {
        canvas_session: Some(CanvasSession {
            label: "synthetic-four-lines.dwg".to_string(),
            batches: Some(4),
            draws: None,
        }),
    }
}

/// The live session's status as the wire sees it.
fn live_wire(session: &CadSession) -> Value {
    let status = session.snapshot(
        Some(BACKEND_WEBGL2.to_string()),
        Some("test adapter".to_string()),
        7,
    );
    status_value(&status, &carrier())
}

fn parse(wire: &str) -> Value {
    serde_json::from_str(wire).expect("the shell status must be valid JSON")
}

#[test]
fn the_status_is_the_frozen_wire_shape_plus_the_carrier_keys() {
    let session = fixture_session();
    let wire = live_wire(&session);
    let object = wire.as_object().expect("a JSON object");
    for key in REQUIRED_KEYS {
        assert!(
            object.contains_key(key),
            "missing required key {key}: {wire}"
        );
    }
    assert_eq!(object["surfaceMode"], json!(SURFACE_MODE_DOM_CANVAS));
    assert_eq!(object["palettesAvailable"], json!(false));
    assert_eq!(object["standardViews"], json!([]));
    assert!(
        object["snapSpacing"]
            .as_f64()
            .is_some_and(|value| value > 0.0),
        "snapSpacing must stay a usable positive number: {wire}"
    );
}

#[test]
fn a_live_session_reports_its_own_document_tool_and_selection() {
    let session = fixture_session();
    let wire = live_wire(&session);
    // The document facts are the session's own (`cad-db` counts), not a host copy.
    assert_eq!(wire["documentLabel"], json!("synthetic-four-lines.dwg"));
    assert_eq!(
        wire["entityCount"],
        json!(4),
        "the imported fixture has four LINE entities: {wire}"
    );
    assert_eq!(wire["tool"], json!(null), "no capture is active yet");
    assert_eq!(wire["selection"]["count"], json!(0));
    assert_eq!(wire["commandLog"], json!([]));
    assert_eq!(wire["error"], json!(null));
    assert_eq!(wire["loading"], json!(false));
    assert_eq!(wire["backend"], json!(BACKEND_WEBGL2));
    assert_eq!(wire["adapter"], json!("test adapter"));
    assert_eq!(wire["frameCount"], json!(7));
    assert_eq!(wire["importCount"], json!(1));
    assert_eq!(
        wire["canvasSession"],
        json!({"label": "synthetic-four-lines.dwg", "batches": 4, "draws": null})
    );

    // A command really moves the session, and the next status is its new state.
    session.command("LINE").expect("LINE is admitted");
    let wire = live_wire(&session);
    let tool = wire["tool"]["key"]
        .as_str()
        .unwrap_or_else(|| panic!("a draw capture owns input: {wire}"));
    assert!(tool.starts_with("draw."), "{tool}");
    assert!(
        wire["commandLog"]
            .as_array()
            .is_some_and(|log| !log.is_empty()),
        "the router echo is the session's own command log: {wire}"
    );
    assert_eq!(wire["canFit"], json!(true));
}

#[test]
fn every_capability_the_carrier_lacks_is_false_and_every_one_it_has_is_true() {
    let session = fixture_session();
    let wire = live_wire(&session);
    // No filesystem, no save dialog, no plot export: the carrier advertises none of
    // them, so none of them may be `true`.
    for key in ["canNew", "canOpen", "canSave", "canPlot"] {
        assert_eq!(wire[key], json!(false), "{key} must be false: {wire}");
    }
    // …and the one it has: a live document can be refit.
    assert_eq!(wire["canFit"], json!(true));
}

#[test]
fn no_session_reports_nothing_invented() {
    let wire = parse(&status_json(&empty_status(), &Carrier::default()));
    assert_eq!(wire["documentLabel"], json!(null));
    assert_eq!(wire["entityCount"], json!(null));
    assert_eq!(wire["backend"], json!(null));
    assert_eq!(wire["camera"], json!(null));
    assert_eq!(wire["frameCount"], json!(0));
    assert_eq!(wire["importCount"], json!(0));
    assert_eq!(wire["canFit"], json!(false));
    // Absent, not null: "no session" must not look like "a session without a label".
    assert!(wire.get("canvasSession").is_none(), "{wire}");
    // The carrier keys are always present (the chrome dispatches on them).
    assert_eq!(wire["surfaceMode"], json!(SURFACE_MODE_DOM_CANVAS));
}

#[test]
fn a_session_error_is_reported_instead_of_looking_healthy() {
    let wire = parse(&status_json(
        &unsupported_status(Some("webgl2: no context".to_string())),
        &Carrier::default(),
    ));
    assert_eq!(wire["error"], json!("webgl2: no context"));
}

#[test]
fn the_vocabulary_is_the_router_whole_vocabulary() {
    let commands: Value = parse(&commands_json());
    let names = commands.as_array().expect("an array of names");
    let admitted = cad_ui_core::session::admissible_names();
    assert_eq!(
        names.len(),
        admitted.len(),
        "the full vocabulary, unfiltered"
    );
    for name in &admitted {
        assert!(
            names.iter().any(|value| value.as_str() == Some(name)),
            "{name} is missing from the carrier's vocabulary"
        );
    }
    // The selectors the old carrier advertised are part of it (nothing regressed for
    // a chrome that hard-codes them), and so are the ones it refused before.
    for name in [
        "FIT", "ZOOMIN", "ZOOMOUT", "LINE", "PAN", "SAVE", "OSNAP", "GRID",
    ] {
        assert!(
            names.iter().any(|value| value.as_str() == Some(name)),
            "{name}"
        );
    }
    // `OPEN` **is** a router selector (the desktop command table has it); on this
    // carrier the router reaches the platform's file contract and is refused there
    // with the stable `unsupported-filesystem` reason, which is a real answer — so
    // advertising it is not an over-claim. The page shell's picker intercepts the
    // name before it ever gets here (`apps/desktop-ui/src/host/web-adapter.ts`).
    assert!(names.iter().any(|value| value.as_str() == Some("OPEN")));
    let session = fixture_session();
    let refusal = session
        .command("OPEN")
        .expect_err("this carrier has no filesystem");
    assert!(
        refusal.starts_with("unsupported-filesystem"),
        "the refusal names the real limitation: {refusal}"
    );
}

#[test]
fn every_alias_row_points_at_an_admitted_selector() {
    let rows: Value = parse(&aliases_json());
    let rows = rows.as_array().expect("an array of rows");
    let desktop = cad_ui_core::session::alias_pairs();
    assert_eq!(rows.len(), desktop.len(), "the whole desktop alias table");
    for row in rows {
        let alias = row["alias"].as_str().expect("an alias");
        let command = row["command"].as_str().expect("a target");
        assert!(
            desktop.contains(&(alias, command)),
            "{alias} -> {command} is not a desktop alias row"
        );
        assert!(
            cad_ui_core::session::canonical_command_name(command).is_some(),
            "{command} is not an admitted selector"
        );
    }
}

#[test]
fn the_acknowledgement_is_the_frozen_wire_object() {
    assert_eq!(parse(&accepted_json()), json!({"accepted": true}));
}

#[test]
fn a_gate_refusal_keeps_the_stable_prefix_and_the_gates_own_words() {
    let refusal = refusal("unsupported CAD command: FLOOPY");
    assert!(
        refusal.starts_with(SHELL_COMMAND_REFUSAL),
        "a caller must be able to key on the prefix: {refusal}"
    );
    assert!(
        refusal.ends_with("unsupported CAD command: FLOOPY"),
        "the gate's own sentence is kept unchanged: {refusal}"
    );
    assert!(SHELL_COMMAND_REFUSAL.starts_with("unsupported-runtime: "));
}
