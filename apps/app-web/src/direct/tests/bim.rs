//! The `dom-canvas` BIM query wire (ADR-0042 D11).
//!
//! The browser carrier owns its session on the calling thread, so the one
//! request-carrying read goes straight to `CadSession::bim_query` through
//! [`DirectReads`] — the same session method the desktop owner thread would run.
//! What these contracts pin: the argument object decodes (camelCase,
//! `filter`/`offset`/`limit`), **one** call answers the frozen camelCase page
//! bound to the fixture's database plus the summary of the same filtered set, and
//! an unknown `value.kind` is refused by marker.
//!
//! The synthetic DWG fixture carries **no** BIM data, which is the honest empty
//! answer: `rows: []`, `total: 0`, an empty summary, `databaseId`/`revision` from
//! the real document.
//!
//! The former `cad_bim_summary` command is gone: the summary rides the query
//! answer, so there is no second dispatch arm to test.

use cad_ui_core::bim::{BimFilterWire, BIM_UNSUPPORTED, BIM_UNSUPPORTED_VALUE};
use cad_ui_core::session::{host_read, QueueReads};

use super::*;

fn args() -> &'static str {
    r#"{
        "filter": {
            "categories": ["IfcWall"],
            "levels": [],
            "types": [],
            "materials": []
        },
        "offset": 0,
        "limit": 100
    }"#
}

#[test]
fn one_query_answer_carries_the_page_and_its_summary() {
    let session = session();
    let direct = DirectReads::new(&session);
    let mut sink = DirectSink::new(&session);
    let mut popups = cad_ui_core::session::NoPopups;

    // One dispatch: rows and summary arrive together, so the panel needs one
    // round trip per filter change.
    let answer = dispatch("cad_bim_query", args(), &mut sink, &direct, &mut popups)
        .expect("the live session answers");
    assert_eq!(answer["total"], 0, "the DWG fixture has no BIM data");
    assert_eq!(answer["offset"], 0);
    assert_eq!(answer["limit"], 100);
    assert_eq!(answer["stale"], false);
    assert_eq!(answer["rows"], serde_json::json!([]));
    // The identity is the real open document, not a fabricated one.
    assert!(
        answer["databaseId"].as_u64().unwrap_or(0) > 0,
        "the page is bound to the fixture's database: {answer}"
    );
    // The summary block: same shape the standalone command used to answer, over
    // the same filtered set.
    let summary = answer
        .get("summary")
        .unwrap_or_else(|| panic!("the query answer carries the summary: {answer}"));
    assert_eq!(summary["total"], answer["total"]);
    assert_eq!(summary["byCategory"], serde_json::json!([]));
    assert_eq!(summary["byLevel"], serde_json::json!([]));
    assert_eq!(summary["byType"], serde_json::json!([]));
    assert_eq!(summary["byMaterial"], serde_json::json!([]));

    // Every key the frontend parses, and no snake_case leak.
    for key in [
        "databaseId",
        "revision",
        "total",
        "offset",
        "limit",
        "rows",
        "stale",
        "summary",
    ] {
        assert!(answer.get(key).is_some(), "missing {key}: {answer}");
    }
    assert!(
        !answer.to_string().contains("database_id"),
        "camelCase only: {answer}"
    );
    assert!(
        !answer.to_string().contains("by_category"),
        "camelCase only: {answer}"
    );
}

#[test]
fn the_summary_command_no_longer_exists() {
    // The one registry is the authority: `cad_bim_query` is served, the former
    // `cad_bim_summary` is not a name any transport registers.
    assert!(cad_ui_core::session::SHARED_WIRE_COMMAND_NAMES.contains(&"cad_bim_query"));
    assert!(!cad_ui_core::session::WIRE_COMMAND_NAMES.contains(&"cad_bim_summary"));
    assert!(!crate::direct::WIRE_COMMANDS.contains(&"cad_bim_summary"));

    let session = session();
    let direct = DirectReads::new(&session);
    let mut sink = DirectSink::new(&session);
    let mut popups = cad_ui_core::session::NoPopups;
    let error = dispatch("cad_bim_summary", args(), &mut sink, &direct, &mut popups)
        .expect_err("an unregistered command is refused");
    assert!(
        error.contains("cad_bim_summary"),
        "the refusal names the unknown command: {error}"
    );
}

#[test]
fn a_property_filter_is_decoded_on_this_transport() {
    let session = session();
    let direct = DirectReads::new(&session);
    let mut sink = DirectSink::new(&session);
    let mut popups = cad_ui_core::session::NoPopups;
    let answer = dispatch(
        "cad_bim_query",
        r#"{
            "filter": {
                "categories": [],
                "levels": [],
                "types": [],
                "materials": [],
                "property": {
                    "pset": "Pset_WallCommon",
                    "property": "IsExternal",
                    "op": "eq",
                    "value": { "kind": "bool", "value": true }
                },
                "evidence": "declared"
            },
            "offset": 0,
            "limit": 10
        }"#,
        &mut sink,
        &direct,
        &mut popups,
    )
    .expect("the filter decodes and the session answers");
    assert_eq!(answer["total"], 0);
    assert_eq!(answer["limit"], 10);
    assert_eq!(answer["summary"]["total"], 0);
}

#[test]
fn an_unknown_value_kind_is_refused_by_marker() {
    let session = session();
    let direct = DirectReads::new(&session);
    let mut sink = DirectSink::new(&session);
    let mut popups = cad_ui_core::session::NoPopups;
    let error = dispatch(
        "cad_bim_query",
        r#"{
            "filter": {
                "property": {
                    "pset": "Pset_WallCommon",
                    "property": "IsExternal",
                    "op": "eq",
                    "value": { "kind": "date", "value": "2026-11-21" }
                }
            },
            "offset": 0,
            "limit": 10
        }"#,
        &mut sink,
        &direct,
        &mut popups,
    )
    .expect_err("an unknown kind is not a query");
    assert!(error.starts_with(BIM_UNSUPPORTED_VALUE), "{error}");
}

#[test]
fn a_desktop_mirror_without_the_seam_refuses_by_name() {
    // The desktop transport has no session on the IPC thread: without the host's
    // generic host-read seam the shared layer must answer by marker, never an
    // empty page.
    let reads = QueueReads::new(None, None, None, None);
    let error = host_read::bim_query(&reads, BimFilterWire::default(), 0, 10)
        .expect_err("no host-read lane on this host");
    assert_eq!(error, BIM_UNSUPPORTED);
    assert_eq!(error, "unsupported-runtime: bimQuery");
}

#[test]
fn the_generic_host_read_answers_what_the_query_command_answers() {
    use cad_ui_core::host_read::{HostReadAnswer, HostReadRequest, HostReads};

    // The dispatcher's `cad_bim_query` arm now builds the generic request; the
    // direct session read must answer the identical JSON, or the refactor changed
    // behaviour.
    let session = session();
    let direct = DirectReads::new(&session);
    let mut sink = DirectSink::new(&session);
    let mut popups = cad_ui_core::session::NoPopups;

    let wire = dispatch("cad_bim_query", args(), &mut sink, &direct, &mut popups)
        .expect("the live session answers");
    let answer = direct
        .host_read(&HostReadRequest::BimQuery {
            filter: BimFilterWire {
                categories: vec!["IfcWall".to_string()],
                ..BimFilterWire::default()
            },
            offset: 0,
            limit: 100,
        })
        .expect("the direct read answers");
    let HostReadAnswer::BimQuery(response) = answer;
    assert_eq!(
        wire,
        serde_json::to_value(&response).expect("serialize"),
        "the generic entry must not reshape the command's answer"
    );
}
