//! The `dom-canvas` BIM query/summary wire (ADR-0042 D11).
//!
//! The browser carrier owns its session on the calling thread, so the two
//! request-carrying reads go straight to `CadSession::bim_query`/
//! `bim_summary` through [`DirectReads`] — the same session method the desktop
//! owner thread would run. What these contracts pin: the argument object decodes
//! (camelCase, `filter`/`offset`/`limit`), the answer is the frozen camelCase page
//! bound to the fixture's database, and an unknown `value.kind` is refused by
//! marker.
//!
//! The synthetic DWG fixture carries **no** BIM data, which is the honest empty
//! answer: `rows: []`, `total: 0`, `databaseId`/`revision` from the real document.

use cad_ui_core::bim::{BimFilterWire, BIM_UNSUPPORTED, BIM_UNSUPPORTED_VALUE};
use cad_ui_core::session::{bim_reads, QueueReads};

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
fn the_live_session_answers_the_bim_query_shape() {
    let session = session();
    let direct = DirectReads::new(&session);
    let mut sink = DirectSink::new(&session);
    let mut popups = cad_ui_core::session::NoPopups;

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
    // Every key the frontend parses, and no snake_case leak.
    for key in [
        "databaseId",
        "revision",
        "total",
        "offset",
        "limit",
        "rows",
        "stale",
    ] {
        assert!(answer.get(key).is_some(), "missing {key}: {answer}");
    }
    assert!(
        !answer.to_string().contains("database_id"),
        "camelCase only: {answer}"
    );

    let summary = dispatch("cad_bim_summary", args(), &mut sink, &direct, &mut popups)
        .expect("the live session answers");
    assert_eq!(summary["total"], 0);
    assert_eq!(summary["summary"]["total"], 0);
    assert_eq!(summary["summary"]["byCategory"], serde_json::json!([]));
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
    // `BimHost` seam the shared layer must answer by marker, never an empty page.
    let reads = QueueReads::new(None, None, None, None);
    let error = bim_reads::bim_query(&reads, BimFilterWire::default(), 0, 10)
        .expect_err("no BIM query lane on this host");
    assert_eq!(error, BIM_UNSUPPORTED);
    assert_eq!(error, "unsupported-runtime: bimQuery");
}
