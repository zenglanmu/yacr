//! The `cad_api_call` contract tests.

use super::*;

fn document() -> CadApiDocument {
    CadApiDocument {
        id: "1".to_string(),
        name: "plan.dwg".to_string(),
        is_read_only: false,
        database_id: "7".to_string(),
        database_version: "12".to_string(),
    }
}

fn tables() -> CadApiSymbolTables {
    CadApiSymbolTables {
        blocks: vec![
            CadApiBlockRecord {
                name: "CHAIR".to_string(),
                origin: [1.0, 2.5, 0.0],
            },
            CadApiBlockRecord {
                name: "DOOR".to_string(),
                origin: [-3.0, 0.0, 1.5],
            },
        ],
        layers: vec![
            CadApiLayerRecord {
                name: "0".to_string(),
                aci: Some(7),
                rgb: "#ffffff".to_string(),
                visible: true,
                locked: false,
                frozen: false,
                linetype: "Continuous".to_string(),
            },
            CadApiLayerRecord {
                name: "WALLS".to_string(),
                // A true-colour layer: the ACI identity genuinely does not
                // exist, so the wire carries `null` (never a guessed index).
                aci: None,
                rgb: "#ff0000".to_string(),
                visible: false,
                locked: true,
                frozen: true,
                linetype: "DASHED".to_string(),
            },
        ],
        clayer: "0".to_string(),
    }
}

fn snapshot() -> CadApiSnapshot {
    CadApiSnapshot {
        document: Some(document()),
        symbol_tables: Some(Ok(tables())),
        selection: vec!["12".to_string(), "13".to_string()],
    }
}

fn no_document_snapshot() -> CadApiSnapshot {
    CadApiSnapshot {
        document: None,
        symbol_tables: None,
        selection: Vec::new(),
    }
}

/// One call without parameters (the active-document read's shape).
fn call(snapshot: Option<&CadApiSnapshot>, namespace: &str, operation: &str) -> CadApiOutcome {
    dispatch_api_call(snapshot, namespace, operation, None)
}

fn value_of(outcome: CadApiOutcome) -> Value {
    assert!(outcome.ok, "expected a value, got {outcome:?}");
    outcome.value.expect("a value")
}

fn error_of(outcome: CadApiOutcome) -> String {
    assert!(!outcome.ok, "expected a refusal, got {outcome:?}");
    assert_eq!(outcome.value, None, "a refusal carries no value");
    outcome.error.expect("a refusal marker")
}

#[test]
fn the_active_document_reports_the_nested_wire_shape() {
    let outcome = call(
        Some(&snapshot()),
        API_CALL_NAMESPACE,
        API_CALL_MDI_ACTIVE_DOCUMENT,
    );
    assert_eq!(outcome.error, None);
    assert_eq!(
        value_of(outcome),
        json!({
            "document": {
                "id": "1",
                "name": "plan.dwg",
                "isReadOnly": false,
                "database": { "id": "7", "version": "12" },
            }
        })
    );
}

#[test]
fn the_documents_list_reports_the_same_descriptor() {
    let listed = value_of(call(
        Some(&snapshot()),
        API_CALL_NAMESPACE,
        API_CALL_DOCUMENTS,
    ));
    let active = value_of(call(
        Some(&snapshot()),
        API_CALL_NAMESPACE,
        API_CALL_MDI_ACTIVE_DOCUMENT,
    ));
    let documents = listed["documents"].as_array().expect("an array");
    assert_eq!(documents.len(), 1, "this host holds one document");
    assert_eq!(
        documents[0], active["document"],
        "one descriptor shape for both reads"
    );
}

#[test]
fn no_document_reports_an_empty_document_list() {
    let outcome = call(
        Some(&no_document_snapshot()),
        API_CALL_NAMESPACE,
        API_CALL_DOCUMENTS,
    );
    assert!(outcome.ok, "no document is a fact, not a wait");
    assert_eq!(value_of(outcome), json!({ "documents": [] }));
}

#[test]
fn the_revision_matches_the_snapshot_database_id() {
    let parameters = json!({ "database": "7" });
    let outcome = dispatch_api_call(
        Some(&snapshot()),
        API_CALL_DATABASE_NAMESPACE,
        API_CALL_REVISION,
        Some(&parameters),
    );
    assert_eq!(outcome.error, None);
    assert_eq!(value_of(outcome), json!({ "revision": 12 }));
}

#[test]
fn the_revision_refuses_a_mismatched_database() {
    for parameters in [
        json!({ "database": "999" }),
        json!({ "database": "" }),
        json!({ "database": "07" }),
    ] {
        let outcome = dispatch_api_call(
            Some(&snapshot()),
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_REVISION,
            Some(&parameters),
        );
        assert_eq!(
            error_of(outcome),
            API_CALL_INVALID_ARGUMENT_DATABASE,
            "parameters {parameters} must not resolve another database"
        );
    }
}

#[test]
fn the_revision_refuses_missing_or_non_string_arguments() {
    for parameters in [
        Value::Null,
        json!({}),
        json!({ "other": "7" }),
        json!({ "database": 7 }),
        json!({ "database": null }),
        json!(["7"]),
    ] {
        let outcome = dispatch_api_call(
            Some(&snapshot()),
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_REVISION,
            Some(&parameters),
        );
        assert_eq!(
            error_of(outcome),
            API_CALL_INVALID_ARGUMENT_DATABASE,
            "parameters {parameters} must be refused by name"
        );
    }
    // An absent `parameters` object is the same refusal.
    assert_eq!(
        error_of(call(
            Some(&snapshot()),
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_REVISION
        )),
        API_CALL_INVALID_ARGUMENT_DATABASE
    );
}

#[test]
fn the_revision_without_a_document_is_no_document() {
    // The no-document fact wins over argument validation: there is no
    // database to name, so every parameter set gets the same answer.
    for parameters in [None, Some(json!({ "database": "7" }))] {
        let outcome = dispatch_api_call(
            Some(&no_document_snapshot()),
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_REVISION,
            parameters.as_ref(),
        );
        assert_eq!(error_of(outcome), API_CALL_NO_DOCUMENT);
    }
}

#[test]
fn the_revision_refuses_an_unparseable_snapshot_version() {
    // This build always renders the revision decimal; a violated invariant is
    // a named refusal, never a fabricated number.
    let broken = CadApiSnapshot {
        document: Some(CadApiDocument {
            database_version: "v12".to_string(),
            ..document()
        }),
        ..snapshot()
    };
    let parameters = json!({ "database": "7" });
    let outcome = dispatch_api_call(
        Some(&broken),
        API_CALL_DATABASE_NAMESPACE,
        API_CALL_REVISION,
        Some(&parameters),
    );
    assert_eq!(error_of(outcome), API_CALL_INVALID_SNAPSHOT);
}

fn symbol_tables_call(parameters: Option<&Value>) -> CadApiOutcome {
    dispatch_api_call(
        Some(&snapshot()),
        API_CALL_DATABASE_NAMESPACE,
        API_CALL_SYMBOL_TABLES,
        parameters,
    )
}

#[test]
fn the_symbol_tables_report_the_frozen_shape() {
    let parameters = json!({ "database": "7" });
    let outcome = symbol_tables_call(Some(&parameters));
    assert_eq!(outcome.error, None);
    assert_eq!(
        value_of(outcome),
        json!({
            "blockTable": {
                "name": "blocks",
                "records": [
                    { "name": "CHAIR", "isLayout": false, "origin": [1.0, 2.5, 0.0] },
                    { "name": "DOOR", "isLayout": false, "origin": [-3.0, 0.0, 1.5] },
                ],
            },
            "layerTable": {
                "name": "layers",
                "records": [
                    {
                        "name": "0",
                        "color": { "aci": 7, "rgb": "#ffffff" },
                        "visible": true,
                        "locked": false,
                        "frozen": false,
                        "linetype": "Continuous",
                    },
                    {
                        "name": "WALLS",
                        // A true-colour layer: `null` is the only honest ACI.
                        "color": { "aci": null, "rgb": "#ff0000" },
                        "visible": false,
                        "locked": true,
                        "frozen": true,
                        "linetype": "DASHED",
                    },
                ],
            },
            "clayer": "0",
        })
    );
}

#[test]
fn the_symbol_tables_refuse_an_unusable_database_argument() {
    for parameters in [
        None,
        Some(Value::Null),
        Some(json!({})),
        Some(json!({ "database": 7 })),
        Some(json!({ "database": "999" })),
        Some(json!({ "other": "7" })),
    ] {
        assert_eq!(
            error_of(symbol_tables_call(parameters.as_ref())),
            API_CALL_INVALID_ARGUMENT_DATABASE,
            "parameters {parameters:?} must be refused by name"
        );
    }
}

#[test]
fn the_symbol_tables_without_a_document_is_no_document() {
    for parameters in [None, Some(json!({ "database": "7" }))] {
        let outcome = dispatch_api_call(
            Some(&no_document_snapshot()),
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_SYMBOL_TABLES,
            parameters.as_ref(),
        );
        assert_eq!(error_of(outcome), API_CALL_NO_DOCUMENT);
    }
}

#[test]
fn the_symbol_tables_relay_a_failed_layer_projection() {
    let refused = CadApiSnapshot {
        document: Some(document()),
        symbol_tables: Some(Err("layer.projection_failed: boom".to_string())),
        selection: Vec::new(),
    };
    let parameters = json!({ "database": "7" });
    let outcome = dispatch_api_call(
        Some(&refused),
        API_CALL_DATABASE_NAMESPACE,
        API_CALL_SYMBOL_TABLES,
        Some(&parameters),
    );
    assert_eq!(
        error_of(outcome),
        "layer.projection_failed: boom",
        "a projection refusal is relayed, never replaced by empty tables"
    );
}

#[test]
fn a_snapshot_without_a_projection_is_named() {
    let inconsistent = CadApiSnapshot {
        document: Some(document()),
        symbol_tables: None,
        selection: Vec::new(),
    };
    let parameters = json!({ "database": "7" });
    let outcome = dispatch_api_call(
        Some(&inconsistent),
        API_CALL_DATABASE_NAMESPACE,
        API_CALL_SYMBOL_TABLES,
        Some(&parameters),
    );
    assert_eq!(error_of(outcome), API_CALL_INCOMPLETE_SNAPSHOT);
}

#[test]
fn the_current_selection_reports_object_ids_in_selection_order() {
    let outcome = call(
        Some(&snapshot()),
        API_CALL_EDITOR_NAMESPACE,
        API_CALL_CURRENT_SELECTION,
    );
    assert_eq!(outcome.error, None);
    assert_eq!(value_of(outcome), json!({ "objectIds": ["12", "13"] }));
}

#[test]
fn an_empty_selection_is_a_fact_not_a_refusal() {
    let empty = CadApiSnapshot {
        selection: Vec::new(),
        ..snapshot()
    };
    let outcome = call(
        Some(&empty),
        API_CALL_EDITOR_NAMESPACE,
        API_CALL_CURRENT_SELECTION,
    );
    assert!(outcome.ok, "nothing selected is a fact, not an error");
    assert_eq!(outcome.error, None);
    assert_eq!(value_of(outcome), json!({ "objectIds": [] }));
}

#[test]
fn the_current_selection_dedupes_instanced_and_sub_element_refs_by_id() {
    // `SelectionSet` distinguishes refs by (entity, instance, sub-element), so the
    // same entity can be selected twice. The read cannot express that identity
    // yet, but it must not drop the ref silently: every selected entity is named
    // once, first occurrence first.
    let repeated = CadApiSnapshot {
        selection: vec![
            "12".to_string(),
            "13".to_string(),
            "12".to_string(),
            "13".to_string(),
            "14".to_string(),
        ],
        ..snapshot()
    };
    let outcome = call(
        Some(&repeated),
        API_CALL_EDITOR_NAMESPACE,
        API_CALL_CURRENT_SELECTION,
    );
    assert_eq!(
        value_of(outcome),
        json!({ "objectIds": ["12", "13", "14"] })
    );
}

#[test]
fn the_current_selection_without_a_document_is_no_document() {
    let outcome = call(
        Some(&no_document_snapshot()),
        API_CALL_EDITOR_NAMESPACE,
        API_CALL_CURRENT_SELECTION,
    );
    assert_eq!(error_of(outcome), API_CALL_NO_DOCUMENT);
}

#[test]
fn the_current_selection_ignores_parameters() {
    let without = value_of(call(
        Some(&snapshot()),
        API_CALL_EDITOR_NAMESPACE,
        API_CALL_CURRENT_SELECTION,
    ));
    for parameters in [json!({}), json!({ "database": "7", "unused": [1, 2] })] {
        let with = value_of(dispatch_api_call(
            Some(&snapshot()),
            API_CALL_EDITOR_NAMESPACE,
            API_CALL_CURRENT_SELECTION,
            Some(&parameters),
        ));
        assert_eq!(with, without, "this read takes no arguments");
    }
}

#[test]
fn no_snapshot_is_cad_waiting_for_every_served_operation() {
    for (namespace, operation) in [
        (API_CALL_NAMESPACE, API_CALL_MDI_ACTIVE_DOCUMENT),
        (API_CALL_NAMESPACE, API_CALL_DOCUMENTS),
        (API_CALL_DATABASE_NAMESPACE, API_CALL_REVISION),
        (API_CALL_DATABASE_NAMESPACE, API_CALL_SYMBOL_TABLES),
        (API_CALL_EDITOR_NAMESPACE, API_CALL_CURRENT_SELECTION),
    ] {
        let outcome = call(None, namespace, operation);
        assert_eq!(error_of(outcome), CAD_WAITING, "{namespace}.{operation}");
    }
}

#[test]
fn an_unknown_operation_is_the_stable_refusal() {
    for (namespace, operation) in [
        (API_CALL_NAMESPACE, "documentManager.other"),
        (API_CALL_DATABASE_NAMESPACE, API_CALL_MDI_ACTIVE_DOCUMENT),
        (API_CALL_NAMESPACE, API_CALL_REVISION),
        (API_CALL_DATABASE_NAMESPACE, API_CALL_DOCUMENTS),
        (API_CALL_NAMESPACE, API_CALL_SYMBOL_TABLES),
        (API_CALL_EDITOR_NAMESPACE, "editor.other"),
        (API_CALL_EDITOR_NAMESPACE, API_CALL_MDI_ACTIVE_DOCUMENT),
        (API_CALL_NAMESPACE, API_CALL_CURRENT_SELECTION),
        (API_CALL_DATABASE_NAMESPACE, API_CALL_CURRENT_SELECTION),
        ("", ""),
        ("", API_CALL_MDI_ACTIVE_DOCUMENT),
    ] {
        let outcome = call(Some(&snapshot()), namespace, operation);
        assert_eq!(
            error_of(outcome),
            api_call_unknown(namespace, operation),
            "{namespace}.{operation} is not served"
        );
        assert_eq!(
            error_of(call(Some(&snapshot()), namespace, operation)),
            format!("unsupported-runtime: cad_api.{namespace}.{operation}")
        );
    }
}

#[test]
fn an_unknown_operation_is_refused_before_snapshot_and_arguments() {
    let parameters = json!({ "database": "nope" });
    let outcome = dispatch_api_call(None, "EditorInput", "getPoint", Some(&parameters));
    assert_eq!(
        error_of(outcome),
        "unsupported-runtime: cad_api.EditorInput.getPoint",
        "cad_waiting/argument refusals would promise a later success"
    );
}

#[test]
fn the_active_document_ignores_parameters() {
    let without = value_of(call(
        Some(&snapshot()),
        API_CALL_NAMESPACE,
        API_CALL_MDI_ACTIVE_DOCUMENT,
    ));
    for parameters in [json!({}), json!({ "database": "7", "unused": [1, 2] })] {
        let with = value_of(dispatch_api_call(
            Some(&snapshot()),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
            Some(&parameters),
        ));
        assert_eq!(with, without, "this read takes no arguments");
    }
}

#[test]
fn no_document_answers_a_null_document() {
    // A snapshot that exists without a document is a fact, answered as such
    // (never `cad_waiting`, which would claim the host has nothing to say).
    let outcome = call(
        Some(&no_document_snapshot()),
        API_CALL_NAMESPACE,
        API_CALL_MDI_ACTIVE_DOCUMENT,
    );
    assert!(outcome.ok);
    assert_eq!(outcome.error, None);
    assert_eq!(value_of(outcome), json!({ "document": null }));
    assert_eq!(
        api_call_document(None).value.expect("a value"),
        json!({ "document": null })
    );
}

#[test]
fn the_snapshot_serializes_as_the_frozen_wire_shape() {
    let serialized = serde_json::to_value(snapshot()).expect("serializes");
    assert_eq!(
        serialized,
        json!({
            "document": {
                "id": "1",
                "name": "plan.dwg",
                "isReadOnly": false,
                "database": { "id": "7", "version": "12" },
            }
        })
    );
    assert!(
        serialized.get("selection").is_none(),
        "the selection projection is served by editor.currentSelection, not the document envelope"
    );
    assert_eq!(
        serde_json::to_value(no_document_snapshot()).expect("serializes"),
        json!({ "document": null })
    );
}

#[test]
fn the_read_only_flag_travels_unchanged() {
    let mut document = document();
    document.is_read_only = true;
    let outcome = call(
        Some(&CadApiSnapshot {
            document: Some(document),
            ..snapshot()
        }),
        API_CALL_NAMESPACE,
        API_CALL_MDI_ACTIVE_DOCUMENT,
    );
    let value = value_of(outcome);
    assert_eq!(value["document"]["isReadOnly"], json!(true));
}

#[test]
fn the_reply_omits_the_absent_half() {
    let ok = serde_json::to_value(api_call_document(Some(document()))).expect("serializes");
    assert_eq!(ok["ok"], json!(true));
    assert!(ok.get("error").is_none(), "the refusal half is omitted");
    let refused = serde_json::to_value(CadApiOutcome::refusal("x")).expect("serializes");
    assert_eq!(refused["ok"], json!(false));
    assert!(refused.get("value").is_none(), "the value half is omitted");
}
