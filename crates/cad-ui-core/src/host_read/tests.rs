//! Contract for the generic host-read envelope (ADR-0042 D11).
//!
//! What must hold: the `kind` tag is the single documented spelling, a known
//! kind round-trips, a request that omits the page window gets the documented
//! default, an unknown kind is refused **by name** (never coerced into BIM), and
//! a carrier with no seam answers the kind's own refusal.

use super::*;

use cad_app::cad_query::bim::BimSummary;
use cad_app::cad_query::{QueryPage, QueryRequest};
use cad_domain::{DatabaseId, DocumentId, RequestId, Revision};

use crate::bim::{query_response, BimQueryResponse};
use crate::param::{ParamListResponse, ParamListRow, PARAM_LIST_KIND, PARAM_LIST_UNSUPPORTED};

fn bim_request(offset: usize, limit: usize) -> HostReadRequest {
    HostReadRequest::BimQuery {
        filter: BimFilterWire::default(),
        offset,
        limit,
    }
}

fn response() -> BimQueryResponse {
    let page = QueryPage {
        request: QueryRequest {
            document: DocumentId(1),
            revision: Revision(7),
            request: RequestId(1),
            offset: 0,
            limit: 100,
        },
        rows: Vec::new(),
        total: 0,
    };
    query_response(page, &BimSummary::default(), Some(DatabaseId(1)))
}

#[test]
fn a_known_kind_round_trips() {
    let request = bim_request(4, 8);
    assert_eq!(request.kind(), BIM_QUERY_KIND);

    let value = serde_json::to_value(&request).expect("serialize");
    // The tag spelling and the constant are one fact, not two literals.
    assert_eq!(value["kind"], BIM_QUERY_KIND);
    assert_eq!(value["offset"], 4);
    assert_eq!(value["limit"], 8);

    let decoded: HostReadRequest = serde_json::from_value(value).expect("decode");
    assert_eq!(decoded, request);
}

#[test]
fn a_request_without_a_page_window_gets_the_documented_defaults() {
    let mut value = serde_json::to_value(bim_request(0, 0)).expect("serialize");
    let object = value.as_object_mut().expect("an envelope object");
    object.remove("offset");
    object.remove("limit");

    let decoded: HostReadRequest = serde_json::from_value(value).expect("decode");
    assert_eq!(
        decoded,
        bim_request(0, crate::bim::BIM_DEFAULT_LIMIT),
        "offset defaults to 0 and limit to BIM_DEFAULT_LIMIT"
    );
}

#[test]
fn the_answer_round_trips_with_its_kind() {
    let answer = HostReadAnswer::BimQuery(response());

    let value = serde_json::to_value(&answer).expect("serialize");
    assert_eq!(value["kind"], BIM_QUERY_KIND);
    // The inner value is the unchanged `cad_bim_query` answer: the envelope only
    // adds the tag, it never renames a wire field.
    let response_value = serde_json::to_value(response()).expect("serialize");
    for key in [
        "databaseId",
        "revision",
        "offset",
        "limit",
        "rows",
        "summary",
    ] {
        assert_eq!(value[key], response_value[key], "field {key}");
    }

    let decoded: HostReadAnswer = serde_json::from_value(value).expect("decode");
    assert_eq!(decoded, answer);
}

#[test]
fn a_parameter_list_request_serializes_as_just_its_kind() {
    let request = HostReadRequest::ParamList;
    assert_eq!(request.kind(), PARAM_LIST_KIND);
    assert_eq!(PARAM_LIST_KIND, "paramList");

    let value = serde_json::to_value(&request).expect("serialize");
    assert_eq!(value, serde_json::json!({ "kind": PARAM_LIST_KIND }));

    let decoded: HostReadRequest = serde_json::from_value(value).expect("decode");
    assert_eq!(decoded, request);
}

#[test]
fn a_parameter_list_answer_round_trips_with_its_kind() {
    let response = ParamListResponse {
        active: Some("mix".to_string()),
        rows: vec![ParamListRow {
            seq: 1,
            name: "r".to_string(),
            kind: "length".to_string(),
            unit: Some("mm".to_string()),
            value: 5.0,
            status: "overridden".to_string(),
        }],
        total: 1,
    };
    let answer = HostReadAnswer::ParamList(response.clone());
    let value = serde_json::to_value(&answer).expect("serialize");
    assert_eq!(value["kind"], PARAM_LIST_KIND);
    assert_eq!(value["active"], "mix");
    assert_eq!(value["total"], 1);
    assert_eq!(value["rows"][0]["status"], "overridden");
    assert_eq!(value["rows"][0]["seq"], 1);
    let decoded: HostReadAnswer = serde_json::from_value(value).expect("decode");
    assert_eq!(decoded, answer);
}

#[test]
fn an_unknown_host_read_kind_is_refused_by_name() {
    // A future/foreign kind is a mismatch, not "this carrier has no lane": it is
    // named and never coerced into the one kind this build knows.
    let error = serde_json::from_str::<HostReadRequest>(r#"{"kind":"bimSummary"}"#)
        .expect_err("an unknown kind is not a query");
    let text = error.to_string();
    assert!(text.starts_with(HOST_READ_UNKNOWN_KIND), "{text}");
    assert_eq!(HOST_READ_UNKNOWN_KIND, "unsupported-runtime: hostRead.kind");

    // `kind` is required, so a bare object cannot silently become BIM either.
    serde_json::from_str::<HostReadRequest>("{}").expect_err("no kind, no request");
}

#[test]
fn no_seam_refuses_with_the_kinds_own_marker() {
    let error = HostReadSeam::default()
        .read(&bim_request(0, 10))
        .expect_err("no seam installed");
    assert_eq!(error, BIM_UNSUPPORTED);
    assert_eq!(error, "unsupported-runtime: bimQuery");

    // Same answer through the `HostReads` default: a target with no lane needs no
    // impl and still refuses by name.
    struct Vanilla;
    impl HostReads for Vanilla {}
    assert_eq!(
        Vanilla.host_read(&bim_request(0, 10)).expect_err("no lane"),
        BIM_UNSUPPORTED
    );
}

#[test]
fn no_seam_refuses_a_parameter_list_with_its_own_marker() {
    let error = HostReadSeam::default()
        .read(&HostReadRequest::ParamList)
        .expect_err("no seam installed");
    assert_eq!(error, PARAM_LIST_UNSUPPORTED);
    assert_eq!(error, "unsupported-runtime: paramList");
    assert_ne!(error, BIM_UNSUPPORTED, "BIM's word is not a parameter word");
}
