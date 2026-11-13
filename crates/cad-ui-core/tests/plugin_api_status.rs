//! Contract for the shared `cad_api_status` answer.
//!
//! The descriptor is pure data: one checked-in JSON document parsed by
//! `cad-ui-core`, served by both transports through one registry row. This test
//! pins the registry membership, the camelCase wire shape and the two structural
//! rules the contract promises — a `pending` row carries its stable marker, a
//! `supported` row omits `reason` — and guards that the command is **not** a CAD
//! selector.

use cad_ui_core::session::{
    wire, CadApiOperationStatus, CadApiStatus, SHARED_WIRE_COMMAND_NAMES,
    TAURI_ONLY_WIRE_COMMAND_NAMES, WIRE_COMMAND_NAMES,
};

const COMMAND: &str = "cad_api_status";

#[test]
fn the_command_is_shared_and_not_desktop_only() {
    assert!(
        WIRE_COMMAND_NAMES.contains(&COMMAND),
        "the API status command must be registered"
    );
    assert!(
        SHARED_WIRE_COMMAND_NAMES.contains(&COMMAND),
        "both transports serve the API status command"
    );
    assert!(
        !TAURI_ONLY_WIRE_COMMAND_NAMES.contains(&COMMAND),
        "the API status command is not desktop-only"
    );
}

#[test]
fn the_wire_answer_parses_and_carries_version_one() {
    let status: CadApiStatus = wire::api_status().expect("the descriptor answers");
    assert_eq!(status.api_version, cad_ui_core::session::PLUGIN_API_VERSION);
    assert_eq!(status.api_version, 1);

    let value = serde_json::to_value(&status).expect("the descriptor serializes");
    let round_tripped: CadApiStatus =
        serde_json::from_value(value).expect("the wire shape round-trips");
    assert_eq!(round_tripped.api_version, 1);
}

#[test]
fn the_wire_shape_is_camel_case() {
    let value = serde_json::to_value(wire::api_status().expect("the descriptor answers"))
        .expect("the descriptor serializes");
    let object = value.as_object().expect("the answer is a JSON object");
    assert!(object.contains_key("apiVersion"), "camelCase apiVersion");
    assert!(
        !object.contains_key("api_version"),
        "the snake_case field name never reaches the wire"
    );
    assert!(object.contains_key("namespaces"));
    assert!(object.contains_key("operations"));
}

#[test]
fn pending_rows_carry_a_reason_and_supported_rows_omit_it() {
    let value = serde_json::to_value(wire::api_status().expect("the descriptor answers"))
        .expect("the descriptor serializes");
    let rows = value["operations"]
        .as_array()
        .expect("operations is an array");
    assert!(!rows.is_empty());

    let mut pending = 0;
    let mut supported: Vec<String> = Vec::new();
    for row in rows {
        match row["status"].as_str().expect("a status string") {
            "pending" => {
                pending += 1;
                let reason = row["reason"].as_str().expect("pending carries a reason");
                assert!(!reason.is_empty(), "the pending marker is not empty");
            }
            "supported" => {
                let namespace = row["namespace"].as_str().expect("a namespace string");
                let operation = row["operation"].as_str().expect("an operation string");
                assert!(
                    row.get("reason").is_none(),
                    "a supported row omits reason entirely: {row}"
                );
                supported.push(format!("{namespace}.{operation}"));
            }
            other => panic!("unexpected status value {other:?}"),
        }
    }
    assert!(pending > 0, "document/editor operations are pending here");
    // Three operations are served in this build (M2.2/M2.3: the two reflected
    // document reads and the database revision). A row flipping to `supported`
    // without a host implementation and an adapter `case` is a bug, so the set is
    // pinned by name, in descriptor order, rather than by count.
    assert_eq!(
        supported,
        vec![
            "ApplicationServices.documentManager.mdiActiveDocument".to_string(),
            "ApplicationServices.documentManager.documents".to_string(),
            "DatabaseServices.database.revision".to_string(),
        ],
        "this build serves exactly the reflected document reads and the database revision"
    );
}

#[test]
fn every_pending_row_names_its_namespace_and_operation() {
    let status = wire::api_status().expect("the descriptor answers");
    for row in &status.operations {
        if row.status != CadApiOperationStatus::Pending {
            continue;
        }
        let expected = format!("api.pending: {}.{}", row.namespace.as_str(), row.operation);
        assert_eq!(
            row.reason.as_deref(),
            Some(expected.as_str()),
            "pending row keeps the stable api.pending marker"
        );
    }
}

#[test]
fn the_api_status_command_is_not_a_cad_selector() {
    // `cad_commands` is the CAD selector vocabulary. The API descriptor is a
    // capability advertisement, not a command a user types, so it must stay out.
    assert!(
        !wire::commands().contains(&COMMAND),
        "cad_api_status must not be admitted as a CAD command"
    );
    assert!(
        !cad_ui_core::session::admissible_names().contains(&COMMAND),
        "cad_api_status is not an admissible selector"
    );
}
