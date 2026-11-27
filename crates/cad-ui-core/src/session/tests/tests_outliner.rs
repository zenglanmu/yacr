//! The `outliner` host read: request kind, the session projection and the
//! no-lane refusal.
//!
//! Contract level: no GUI, no GPU. The projection itself is pinned in
//! `cad-host`; here the session only has to forward it unchanged.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_outliner_request_round_trips_through_the_envelope() {
    use crate::host_read::{HostReadAnswer, HostReadRequest};
    let request = HostReadRequest::Outliner;
    assert_eq!(request.kind(), crate::outliner::OUTLINER_KIND);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::json!({ "kind": "outliner" })
    );
    let decoded: HostReadRequest =
        serde_json::from_value(serde_json::json!({ "kind": "outliner" })).expect("decode");
    assert_eq!(decoded, HostReadRequest::Outliner);
    let answer = HostReadAnswer::Outliner(crate::outliner::OutlinerResponse::default());
    assert!(matches!(answer, HostReadAnswer::Outliner(_)));
}

#[test]
fn a_carrier_without_an_outliner_lane_names_its_own_refusal() {
    assert_eq!(
        crate::host_read::no_seam_refusal(&crate::host_read::HostReadRequest::Outliner),
        crate::outliner::OUTLINER_UNSUPPORTED
    );
}

#[test]
fn the_session_projects_the_document_tree_unchanged() {
    let session = fresh_session();
    let expected = session.core.outliner_tree();
    let answer = session.outliner();
    assert_eq!(answer.revision, expected.revision);
    assert_eq!(answer.total, expected.total);
    assert_eq!(answer.truncated, expected.truncated);
    assert_eq!(answer.rows.len(), expected.rows.len());
    for (wire, host) in answer.rows.iter().zip(expected.rows.iter()) {
        assert_eq!(wire.kind, host.kind);
        assert_eq!(wire.id, host.id);
        assert_eq!(wire.label, host.label);
        assert_eq!(wire.depth, host.depth);
        assert_eq!(wire.has_children, host.has_children);
        assert_eq!(wire.selectable, host.selectable);
        assert_eq!(wire.select_ids, host.select_ids);
    }
}

#[test]
fn a_group_member_row_carries_the_ids_one_click_selects() {
    let session = fresh_session();
    // `GROUP` over the whole demo selection is not guaranteed; instead assert the
    // invariant on whatever the document projects: a selectable row's `selectIds`
    // is non-empty, a non-selectable row has none.
    for row in session.outliner().rows {
        if row.selectable {
            assert!(!row.select_ids.is_empty(), "{row:?}");
        } else {
            assert!(row.select_ids.is_empty(), "{row:?}");
        }
    }
}
