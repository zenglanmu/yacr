//! The mechanical FeatureManager tree at the session boundary.
//!
//! These contracts pin the **routing** (the session entry and the generic
//! `host_read` envelope answer the same projection) and the no-document empty
//! answer. The projection's own rules (feature order, body statuses, the bound)
//! are covered by the `cad-host` tests; nothing here evaluates a feature.

use super::super::*;
use super::test_support::*;
use crate::host_read::{HostReadAnswer, HostReadRequest};
use crate::mechanical::{MechanicalTreeResponse, MECHANICAL_TREE_KIND};

fn demo_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn empty_session() -> CadSession {
    let mut options = options();
    options.document_at_start = StartDocument::Empty;
    CadSession::new(
        options,
        platform(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        })),
        RenderSessionHandle::default(),
    )
    .expect("empty session builds")
}

#[test]
fn the_session_projects_the_tree_and_the_generic_seam_answers_the_same_rows() {
    let session = demo_session();
    let direct = session.mechanical_tree().expect("the session answers");
    let answer = session
        .host_read(&HostReadRequest::MechanicalTree)
        .expect("the seam answers");
    let HostReadAnswer::MechanicalTree(response) = answer else {
        panic!("a mechanical-tree request must answer a mechanical tree");
    };
    assert_eq!(response, direct);
    assert_eq!(
        response.total,
        response.rows.len(),
        "the demo document is under the bound"
    );
    assert!(!response.truncated);
    for row in &response.rows {
        assert!(
            row.kind == "feature" || row.kind == "body",
            "row kind is the closed vocabulary"
        );
        assert!(!row.status.is_empty());
    }
}

#[test]
fn a_session_with_no_document_answers_an_empty_tree() {
    let session = empty_session();
    assert!(
        session.core.controller.borrow().drawing().is_none(),
        "the fixture really starts with no document"
    );
    let expected = MechanicalTreeResponse::default();
    assert_eq!(session.mechanical_tree().expect("no document"), expected);
    assert_eq!(
        session
            .host_read(&HostReadRequest::MechanicalTree)
            .expect("no document is not an error"),
        HostReadAnswer::MechanicalTree(expected),
    );
}

#[test]
fn the_request_kind_is_the_documented_spelling() {
    let value = serde_json::to_value(HostReadRequest::MechanicalTree).expect("serialize");
    assert_eq!(value["kind"], MECHANICAL_TREE_KIND);
    assert_eq!(MECHANICAL_TREE_KIND, "mechanicalTree");
}
