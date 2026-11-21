//! `cad_select_entities` on the `dom-canvas` carrier.
//!
//! The browser carrier owns its session on the calling thread, so the BIM panel's
//! click action applies inline: [`DirectSink::select_entities`] calls
//! `CadSession::select_entities`, which resolves the ids against the open document
//! and replaces the selection through the ordinary selection command. What these
//! contracts pin:
//!
//! * the argument field is the camelCase `entityIds` (the shared macro's
//!   `rename_all = "camelCase"`, the same decoding Tauri applies);
//! * an acknowledged request really changes `cad_status.selection`, it is never a
//!   fake `{accepted:true}`;
//! * an unknown id refuses the **whole** request by marker and leaves the previous
//!   selection untouched.
//!
//! This runs on the host target over the compiled-in synthetic fixture (four
//! `LINE`s): a contract, not browser or pixels evidence.

use cad_host::SELECTION_UNKNOWN_ENTITY;

use super::matrix::{LIVE, MODAL, PROBE};
use super::*;

/// The per-command equivalence cases for `cad_select_entities`, driven by the
/// matrix in [`super::matrix`] through both transports.
pub(super) const SELECTION_CASES: &[(&str, &str, &str, (bool, bool))] = &[
    (
        "selection/valid",
        "cad_select_entities",
        r#"{"entityIds":[1,2]}"#,
        LIVE,
    ),
    (
        "selection/empty",
        "cad_select_entities",
        r#"{"entityIds":[]}"#,
        LIVE,
    ),
    (
        "selection/missing-arg",
        "cad_select_entities",
        r#"{}"#,
        LIVE,
    ),
    (
        "selection/snake-case",
        "cad_select_entities",
        r#"{"entity_ids":[1]}"#,
        LIVE,
    ),
    (
        "selection/probe-mode",
        "cad_select_entities",
        r#"{"entityIds":[1]}"#,
        PROBE,
    ),
    (
        "selection/modal",
        "cad_select_entities",
        r#"{"entityIds":[1]}"#,
        MODAL,
    ),
];

/// The fixture's real entity ids, as the wire carries them (`u64`).
fn fixture_ids(session: &CadSession) -> Vec<u64> {
    session
        .command("SELECTALL")
        .expect("SELECTALL is a host command");
    let ids: Vec<u64> = session
        .api_snapshot()
        .selection
        .iter()
        .map(|id| id.parse().expect("a decimal entity id"))
        .collect();
    assert_eq!(ids.len(), 4, "the synthetic fixture holds four lines");
    ids
}

#[test]
fn the_argument_field_is_the_camel_case_entity_ids() {
    let session = session();
    let ids = fixture_ids(&session);
    let mut sink = DirectSink::new(&session);
    let direct = DirectReads::new(&session);
    let mut popups = NoPopups;

    // The wire name is `entityIds`; the snake_case spelling is a malformed request
    // (decoding refuses before the sink), exactly like every other camelCase field.
    let error = dispatch(
        "cad_select_entities",
        &format!(r#"{{"entity_ids":[{}]}}"#, ids[0]),
        &mut sink,
        &direct,
        &mut popups,
    )
    .expect_err("`entity_ids` is not the wire field");
    assert!(
        error.contains("entityIds"),
        "the decoder names the missing camelCase field: {error}"
    );

    // …and the camelCase spelling decodes and reaches the session.
    let ack = dispatch(
        "cad_select_entities",
        &format!(r#"{{"entityIds":[{}]}}"#, ids[0]),
        &mut sink,
        &direct,
        &mut popups,
    )
    .expect("the camelCase request is served");
    assert_eq!(ack, serde_json::json!({"accepted": true}));
}

#[test]
fn an_acknowledged_selection_really_changes_the_host_status() {
    let session = session();
    let ids = fixture_ids(&session);
    assert_eq!(
        session.snapshot(None, None, 0).selection.count,
        4,
        "SELECTALL really filled the selection first"
    );

    let mut sink = DirectSink::new(&session);
    let direct = DirectReads::new(&session);
    let mut popups = NoPopups;
    dispatch(
        "cad_select_entities",
        &format!(r#"{{"entityIds":[{},{}]}}"#, ids[0], ids[2]),
        &mut sink,
        &direct,
        &mut popups,
    )
    .expect("both ids exist in the fixture");

    assert_eq!(
        session.snapshot(None, None, 0).selection.count,
        2,
        "the acknowledged request replaced the selection"
    );
    assert_eq!(
        session.api_snapshot().selection,
        vec![ids[0].to_string(), ids[2].to_string()],
        "the selection is exactly the requested ids, in order"
    );
}

#[test]
fn an_unknown_id_refuses_the_whole_request_and_keeps_the_selection() {
    let session = session();
    let ids = fixture_ids(&session);
    let selection_before = session.api_snapshot().selection;
    let unknown = 99_999_999_u64;

    let mut sink = DirectSink::new(&session);
    let direct = DirectReads::new(&session);
    let mut popups = NoPopups;
    let error = dispatch(
        "cad_select_entities",
        &format!(r#"{{"entityIds":[{},{}]}}"#, ids[0], unknown),
        &mut sink,
        &direct,
        &mut popups,
    )
    .expect_err("an id outside the fixture is refused");
    assert_eq!(error, format!("{SELECTION_UNKNOWN_ENTITY}: {unknown}"));
    assert_eq!(
        session.snapshot(None, None, 0).selection.count,
        4,
        "a refused request never partially selects"
    );
    assert_eq!(session.api_snapshot().selection, selection_before);
    assert_eq!(
        session.snapshot(None, None, 0).error,
        Some(format!("{SELECTION_UNKNOWN_ENTITY}: {unknown}")),
        "the host's real verdict reaches `cad_status.error`"
    );
}

#[test]
fn an_empty_request_is_refused_and_does_not_clear_the_selection() {
    let session = session();
    let ids = fixture_ids(&session);
    let mut sink = DirectSink::new(&session);
    let direct = DirectReads::new(&session);
    let mut popups = NoPopups;
    let error = dispatch(
        "cad_select_entities",
        r#"{"entityIds":[]}"#,
        &mut sink,
        &direct,
        &mut popups,
    )
    .expect_err("an empty request is malformed");
    assert_eq!(error, cad_ui_core::session::SELECTION_EMPTY);
    assert_eq!(
        session.api_snapshot().selection.len(),
        ids.len(),
        "the selection is untouched"
    );
}
