//! Strictness of the `tables/bim-relations.json` codec (ADR-0042 D3 tail).

use cad_domain::ymodel::PersistentIdOf;
use serde_json::{json, Value};

use crate::bim::{BimKey, BimRelation, BimRelationKind, BimRelations};
use crate::scope::TableKind;
use crate::snapshot::{relations, TableSnapshot};

fn table_with(rows: Vec<Value>) -> TableSnapshot {
    let mut tables = TableSnapshot::default();
    tables.set(TableKind::BimRelations, rows);
    tables
}

fn row() -> Value {
    json!({
        "kind": "contains",
        "parent": {"kind": "source", "connector": "ifc", "document_id": "doc-1", "id": "P"},
        "child": {"kind": "source", "connector": "ifc", "document_id": "doc-1", "id": "C"},
        "source": null,
        "label": null,
    })
}

fn decode_error(row: Value) -> crate::SnapshotError {
    relations::decode_relations(&table_with(vec![row])).expect_err("row must be refused")
}

#[test]
fn relation_row_round_trips_with_entity_endpoints_and_nulls() {
    let rows = vec![
        row(),
        json!({
            "kind": "other:rvt-assembly",
            "parent": {"kind": "source", "connector": "rvt", "document_id": "doc-2", "id": "A"},
            "child": {"kind": "entity", "entityId": cad_domain::EntityId(7).persistent().raw()},
            "source": {"kind": "source", "connector": "rvt", "document_id": "doc-2", "id": "R"},
            "label": "Assembly",
        }),
    ];
    let decoded = relations::decode_relations(&table_with(rows)).expect("decode");

    let mut expected = BimRelations::new();
    expected
        .insert(BimRelation {
            kind: BimRelationKind::Contains,
            parent: BimKey::Source {
                connector: "ifc".to_string(),
                document_id: "doc-1".to_string(),
                id: "P".to_string(),
            },
            child: BimKey::Source {
                connector: "ifc".to_string(),
                document_id: "doc-1".to_string(),
                id: "C".to_string(),
            },
            source: None,
            label: None,
        })
        .expect("edge");
    expected
        .insert(BimRelation {
            kind: BimRelationKind::Other("rvt-assembly".to_string()),
            parent: BimKey::Source {
                connector: "rvt".to_string(),
                document_id: "doc-2".to_string(),
                id: "A".to_string(),
            },
            child: BimKey::Entity(cad_domain::EntityId(7)),
            source: Some(BimKey::Source {
                connector: "rvt".to_string(),
                document_id: "doc-2".to_string(),
                id: "R".to_string(),
            }),
            label: Some("Assembly".to_string()),
        })
        .expect("edge");
    assert_eq!(decoded, expected);
}

#[test]
fn relation_row_with_unknown_kind_token_is_rejected() {
    let mut bad = row();
    bad["kind"] = Value::from("mystery");
    let error = decode_error(bad);
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(
        error.detail.contains("unknown relation kind"),
        "{}",
        error.detail
    );
}

#[test]
fn relation_row_with_unknown_endpoint_kind_is_rejected() {
    let mut bad = row();
    bad["parent"] = json!({"kind": "layer", "id": 1});
    let error = decode_error(bad);
    assert!(
        error.detail.contains("unknown relation endpoint kind"),
        "{}",
        error.detail
    );
}

#[test]
fn a_relation_row_must_carry_every_key() {
    // `source` and `label` are required keys whose "no value" spelling is
    // `null`: omitting one is a malformed row, not an implicit null.
    for missing in ["kind", "parent", "child", "source", "label"] {
        let mut bad = row();
        bad.as_object_mut().expect("object").remove(missing);
        let error = decode_error(bad);
        assert!(
            error.detail.contains("missing required field"),
            "{missing}: {}",
            error.detail
        );
    }
}

#[test]
fn a_relation_row_refuses_an_extra_key() {
    let mut bad = row();
    bad["evidence"] = Value::from("declared");
    let error = decode_error(bad);
    assert!(error.detail.contains("unknown field"), "{}", error.detail);
}

#[test]
fn a_source_endpoint_refuses_element_provenance_keys() {
    // `native_id` belongs to the element `bim.source` block; an endpoint is a
    // key, not provenance, so carrying it here is an unknown field.
    let mut bad = row();
    bad["parent"]["native_id"] = Value::Null;
    let error = decode_error(bad);
    assert!(error.detail.contains("native_id"), "{}", error.detail);
}

#[test]
fn an_entity_endpoint_must_carry_an_entity_persistent_id() {
    let mut bare = row();
    bare["child"] = json!({"kind": "entity", "entityId": 7});
    let error = decode_error(bare);
    assert!(
        error.detail.contains("not a persistent identifier"),
        "{}",
        error.detail
    );

    let mut wrong_family = row();
    wrong_family["child"] = json!({
        "kind": "entity",
        "entityId": cad_domain::LayerId(1).persistent().raw(),
    });
    let error = decode_error(wrong_family);
    assert!(
        error.detail.contains("not an entity-family"),
        "{}",
        error.detail
    );
}

#[test]
fn duplicate_relation_rows_are_rejected_on_read() {
    let error = relations::decode_relations(&table_with(vec![row(), row()]))
        .expect_err("two identical rows");
    assert!(
        error.detail.contains("duplicate relation row"),
        "{}",
        error.detail
    );
}
