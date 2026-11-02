//! The `references` block of `cad_status`: authoritative XREF metadata projected
//! 1:1 onto the frozen wire.
//!
//! The rows come from the drawing's external-reference table (stable name
//! order), the status string is the database's own kebab-case `as_str`, and no
//! value is derived from geometry, a file extension or the reference name.

use std::collections::BTreeSet;

use super::super::super::*;
use super::super::test_support::*;

#[test]
fn references_are_projected_verbatim_in_stable_name_order() {
    // Inserted out of name order on purpose: the authoritative `BTreeMap` order
    // is the contract, never insertion order.
    let session = session_with_references(&[
        (
            "z-last",
            "..\\refs\\z.dwg",
            Some("/opt/refs/z.dwg"),
            cad_db::ExternalReferenceStatus::Loaded,
            None,
            true,
            2,
        ),
        (
            "a-first",
            "C:\\refs\\missing.dwg",
            None,
            cad_db::ExternalReferenceStatus::Missing,
            Some("xref.missing"),
            false,
            1,
        ),
    ]);
    let status = session.snapshot(None, None, 0);

    assert_eq!(status.references.len(), 2, "one row per authoritative xref");

    // Stable name order: `a-first` precedes `z-last` even though `z-last` was
    // inserted first.
    let first = &status.references[0];
    assert_eq!(first.name, "a-first");
    assert_eq!(
        first.path, "C:\\refs\\missing.dwg",
        "the raw source path is carried verbatim, before any host resolution"
    );
    assert_eq!(first.resolved_path, None);
    assert_eq!(
        first.status, "missing",
        "the status is the database's kebab-case spelling, never derived"
    );
    assert_eq!(first.reason.as_deref(), Some("xref.missing"));
    assert!(!first.overlay);
    assert_eq!(first.depth, 1);

    let second = &status.references[1];
    assert_eq!(second.name, "z-last");
    assert_eq!(second.path, "..\\refs\\z.dwg");
    assert_eq!(
        second.resolved_path.as_deref(),
        Some("/opt/refs/z.dwg"),
        "the resolved path is only what the host recorded"
    );
    assert_eq!(second.status, "loaded");
    assert_eq!(second.reason, None);
    assert!(second.overlay);
    assert_eq!(second.depth, 2);

    // Exact wire shape: camelCase keys, kebab-case statuses, stable order.
    let value = serde_json::to_value(&status).expect("the status serializes");
    let references = value["references"].as_array().expect("a JSON array");
    assert_eq!(references.len(), 2);

    let keys: BTreeSet<&str> = references[0]
        .as_object()
        .expect("a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "name",
            "path",
            "resolvedPath",
            "status",
            "reason",
            "overlay",
            "depth",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>(),
        "the row carries exactly the frozen camelCase keys"
    );

    assert_eq!(references[0]["name"], "a-first");
    assert_eq!(references[0]["path"], "C:\\refs\\missing.dwg");
    assert_eq!(references[0]["resolvedPath"], serde_json::Value::Null);
    assert_eq!(references[0]["status"], "missing");
    assert_eq!(references[0]["reason"], "xref.missing");
    assert_eq!(references[0]["overlay"], serde_json::json!(false));
    assert_eq!(references[0]["depth"], serde_json::json!(1));

    assert_eq!(references[1]["name"], "z-last");
    assert_eq!(references[1]["resolvedPath"], "/opt/refs/z.dwg");
    assert_eq!(references[1]["status"], "loaded");
    assert_eq!(references[1]["reason"], serde_json::Value::Null);
    assert_eq!(references[1]["overlay"], serde_json::json!(true));
    assert_eq!(references[1]["depth"], serde_json::json!(2));
}

#[test]
fn no_document_reports_an_empty_reference_list() {
    let status = unsupported_status(None);
    assert!(
        status.references.is_empty(),
        "no document declares no external references"
    );

    let value = serde_json::to_value(&status).expect("the status serializes");
    assert_eq!(
        value["references"],
        serde_json::json!([]),
        "the key is present and empty, never omitted"
    );
}

#[test]
fn empty_references_field_is_serialized_not_skipped() {
    let value = serde_json::to_value(unsupported_status(None)).expect("the status serializes");
    let object = value.as_object().expect("a JSON object");
    assert!(
        object.contains_key("references"),
        "`references` is additive but always sent, even when empty"
    );
    assert_eq!(object["references"], serde_json::json!([]));
}
