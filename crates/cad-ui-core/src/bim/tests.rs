//! Wire contract for the BIM query/summary commands (ADR-0042 D11).
//!
//! These pin the **transport**, not the backend: the field names the frontend
//! parses (`camelCase`), the two `object` states, the four `value.kind` states,
//! the evidence and op spellings, and the explicit refusals for anything this
//! layer cannot express. A backend field rename breaks the compiler here; a wire
//! key rename breaks these assertions.

use super::*;

use cad_app::cad_query::QueryRequest;
use cad_db::{BimProperty, BimPropertySet, BimValue};
use cad_domain::{DocumentId, EntityId, RequestId, Revision};

fn page_request(offset: usize, limit: usize) -> QueryRequest {
    QueryRequest {
        document: DocumentId(1),
        revision: Revision(42),
        request: RequestId(7),
        offset,
        limit,
    }
}

fn source() -> BimSource {
    BimSource {
        connector: "ifc".to_string(),
        document_id: Some("doc-1".to_string()),
        id: Some("2O2Fr$t4X7Zf8NOew3FLSJ".to_string()),
        native_id: Some("22".to_string()),
    }
}

fn wall_row() -> BimRow {
    BimRow {
        object: BimKey::Source {
            connector: "ifc".to_string(),
            document_id: "doc-1".to_string(),
            id: "2O2Fr$t4X7Zf8NOew3FLSJ".to_string(),
        },
        entity_ids: vec![EntityId(7), EntityId(9)],
        category: Some("IfcWall".to_string()),
        name: Some("Basic Wall:Generic 200mm".to_string()),
        level_name: Some("Level 1".to_string()),
        type_name: Some("Basic Wall".to_string()),
        material_name: Some("Concrete".to_string()),
        source: source(),
        evidence: BimEvidence::Declared,
        property_summary: vec![BimPropertyCount {
            name: "Pset_WallCommon".to_string(),
            count: 3,
        }],
    }
}

fn request_json() -> Value {
    serde_json::json!({
        "filter": {
            "categories": ["IfcWall"],
            "levels": [],
            "types": [],
            "materials": [],
            "property": {
                "pset": "Pset_WallCommon",
                "property": "IsExternal",
                "op": "eq",
                "value": { "kind": "bool", "value": true },
            },
            "evidence": "declared",
        },
        "offset": 0,
        "limit": 100,
    })
}

// ------------------------------------------------------------------- request ---

#[test]
fn the_request_deserializes_every_field() {
    let request: BimQueryRequest = serde_json::from_value(request_json()).expect("request");
    assert_eq!(request.filter.categories, vec!["IfcWall"]);
    assert!(request.filter.levels.is_empty());
    assert!(request.filter.types.is_empty());
    assert!(request.filter.materials.is_empty());
    assert_eq!(request.filter.evidence, Some(BimEvidenceWire::Declared));
    let property = request.filter.property.as_ref().expect("property");
    assert_eq!(property.pset, "Pset_WallCommon");
    assert_eq!(property.property, "IsExternal");
    assert_eq!(property.op, BimOpWire::Eq);
    assert_eq!(property.value, BimValueWire::Bool { value: true });
    assert_eq!(request.offset, 0);
    assert_eq!(request.limit, 100);
}

#[test]
fn the_request_serializes_with_camel_case_keys_only() {
    let request = BimQueryRequest::new(
        BimFilterWire {
            categories: vec!["IfcWall".into()],
            levels: vec!["Level 1".into()],
            types: vec!["Basic Wall".into()],
            materials: vec!["Concrete".into()],
            property: Some(BimPropertyFilterWire {
                pset: "Pset_WallCommon".into(),
                property: "IsExternal".into(),
                op: BimOpWire::Contains,
                value: BimValueWire::Text {
                    value: "Con".into(),
                },
            }),
            evidence: Some(BimEvidenceWire::Heuristic),
        },
        0,
        100,
    );
    let value = serde_json::to_value(&request).expect("serialize");
    // Pin every key the frontend reads. A `rename_all` regression fails here.
    for key in ["filter", "offset", "limit"] {
        assert!(
            value.get(key).is_some(),
            "missing request key {key}: {value}"
        );
    }
    let filter = value.get("filter").expect("filter");
    for key in [
        "categories",
        "levels",
        "types",
        "materials",
        "property",
        "evidence",
    ] {
        assert!(
            filter.get(key).is_some(),
            "missing filter key {key}: {value}"
        );
    }
    let property = filter.get("property").expect("property");
    for key in ["pset", "property", "op", "value"] {
        assert!(
            property.get(key).is_some(),
            "missing property key {key}: {value}"
        );
    }
    let scalar = property.get("value").expect("value");
    assert_eq!(scalar.get("kind").and_then(Value::as_str), Some("text"));
    assert_eq!(scalar.get("value").and_then(Value::as_str), Some("Con"));
    assert_camel_case_keys(&value, "");
}

/// Walk every object and assert each key is camelCase (a `_` would mean a
/// missing `rename_all`/`rename_all_fields`). Values are not inspected, so a
/// source spelling such as `Pset_WallCommon` is fine.
fn assert_camel_case_keys(value: &Value, path: &str) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                assert!(
                    !key.contains('_'),
                    "snake_case key {path}{key} reached the wire: {value}"
                );
                assert_camel_case_keys(nested, &format!("{path}{key}."));
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_camel_case_keys(item, path);
            }
        }
        _ => {}
    }
}

#[test]
fn a_request_without_paging_uses_the_documented_defaults() {
    let request: BimQueryRequest = serde_json::from_value(serde_json::json!({
        "filter": { "categories": [] }
    }))
    .expect("only the filter is mandatory");
    assert_eq!(request.offset, 0);
    assert_eq!(request.limit, BIM_DEFAULT_LIMIT);
}

#[test]
fn the_four_value_kinds_become_the_backend_scalars() {
    let cases = [
        (
            serde_json::json!({ "kind": "bool", "value": true }),
            BimScalar::Bool(true),
        ),
        (
            serde_json::json!({ "kind": "integer", "value": 3 }),
            BimScalar::Integer(3),
        ),
        (
            serde_json::json!({ "kind": "real", "value": 1.5 }),
            BimScalar::real(1.5),
        ),
        (
            serde_json::json!({ "kind": "text", "value": "x" }),
            BimScalar::text("x"),
        ),
    ];
    for (json, expected) in cases {
        let value: BimValueWire = serde_json::from_value(json.clone()).expect("value");
        assert_eq!(value.to_scalar().expect("scalar"), expected, "{json}");
    }
}

#[test]
fn a_real_is_carried_exactly_never_rounded() {
    let value: BimValueWire =
        serde_json::from_value(serde_json::json!({ "kind": "real", "value": 0.123456789012345 }))
            .expect("real");
    match value.to_scalar().expect("scalar") {
        BimScalar::Real(real) => assert_eq!(real.get(), 0.123456789012345),
        other => panic!("expected a real, got {other:?}"),
    }
}

#[test]
fn an_unknown_value_kind_is_refused_by_marker() {
    let value: BimValueWire =
        serde_json::from_value(serde_json::json!({ "kind": "date", "value": "2026-11-21" }))
            .expect("an unknown kind still decodes so it can be named");
    assert_eq!(
        value,
        BimValueWire::Unsupported {
            token: "date".into()
        }
    );
    let error = value.to_scalar().expect_err("an unknown kind is refused");
    assert!(
        error.starts_with(BIM_UNSUPPORTED_VALUE),
        "the stable marker leads: {error}"
    );
    assert!(
        error.contains("date"),
        "the caller's own kind is named: {error}"
    );
}

#[test]
fn an_unknown_kind_is_named_through_the_filter_conversion() {
    let request: BimQueryRequest = serde_json::from_value(serde_json::json!({
        "filter": {
            "property": {
                "pset": "Pset_WallCommon",
                "property": "IsExternal",
                "op": "eq",
                "value": { "kind": "complex", "value": [1, 2] },
            }
        },
        "offset": 0,
        "limit": 10,
    }))
    .expect("request");
    let error = request.filter.to_filter().expect_err("refused");
    assert!(error.starts_with(BIM_UNSUPPORTED_VALUE), "{error}");
}

#[test]
fn a_non_finite_real_is_refused_by_marker() {
    for value in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let error = BimValueWire::Real { value }
            .to_scalar()
            .expect_err("a non-finite real is not a BIM value");
        assert!(error.starts_with(BIM_REAL_NOT_FINITE), "{error}");
    }
}

#[test]
fn the_op_and_evidence_spellings_are_lowercase_tokens() {
    for (wire, op) in [
        ("eq", BimPropertyOp::Eq),
        ("ne", BimPropertyOp::Ne),
        ("contains", BimPropertyOp::Contains),
    ] {
        let decoded: BimOpWire = serde_json::from_value(Value::String(wire.into())).expect("op");
        assert_eq!(decoded.to_op(), op, "{wire}");
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            Value::String(wire.into())
        );
    }
    for (wire, evidence) in [
        ("declared", BimEvidence::Declared),
        ("heuristic", BimEvidence::Heuristic),
    ] {
        let decoded: BimEvidenceWire =
            serde_json::from_value(Value::String(wire.into())).expect("evidence");
        assert_eq!(decoded.to_evidence(), evidence, "{wire}");
        assert_eq!(
            serde_json::to_value(BimEvidenceWire::from_evidence(evidence)).unwrap(),
            Value::String(wire.into())
        );
    }
}

// ------------------------------------------------------------------ responses ---

#[test]
fn the_object_kind_has_exactly_two_states() {
    let source = BimObjectWire::from_key(&BimKey::Source {
        connector: "ifc".into(),
        document_id: "doc-1".into(),
        id: "wall-1".into(),
    });
    assert_eq!(
        serde_json::to_value(&source).unwrap(),
        serde_json::json!({
            "kind": "source",
            "connector": "ifc",
            "documentId": "doc-1",
            "id": "wall-1",
        })
    );

    let entity = BimObjectWire::from_key(&BimKey::Entity(EntityId(7)));
    assert_eq!(
        serde_json::to_value(&entity).unwrap(),
        serde_json::json!({ "kind": "entity", "entityId": 7 })
    );
}

#[test]
fn a_query_response_carries_the_frozen_shape() {
    let page = QueryPage {
        request: page_request(0, 100),
        rows: vec![wall_row()],
        total: 12,
    };
    let response = query_response(page, Some(DatabaseId(1)));
    let value = serde_json::to_value(&response).expect("serialize");
    assert_eq!(
        value,
        serde_json::json!({
            "databaseId": 1,
            "revision": 42,
            "total": 12,
            "offset": 0,
            "limit": 100,
            "stale": false,
            "rows": [{
                "object": {
                    "kind": "source",
                    "connector": "ifc",
                    "documentId": "doc-1",
                    "id": "2O2Fr$t4X7Zf8NOew3FLSJ",
                },
                "entityIds": [7, 9],
                "category": "IfcWall",
                "name": "Basic Wall:Generic 200mm",
                "levelName": "Level 1",
                "typeName": "Basic Wall",
                "materialName": "Concrete",
                "source": {
                    "connector": "ifc",
                    "documentId": "doc-1",
                    "id": "2O2Fr$t4X7Zf8NOew3FLSJ",
                    "nativeId": "22",
                },
                "evidence": "declared",
                "propertySummary": [{ "name": "Pset_WallCommon", "count": 3 }],
            }],
        })
    );
    // The response decodes back into the same value: the frontend's round trip.
    let decoded: BimQueryResponse = serde_json::from_value(value).expect("decode");
    assert_eq!(decoded, response);
}

#[test]
fn an_entity_keyed_row_uses_the_entity_object() {
    let mut row = wall_row();
    row.object = BimKey::Entity(EntityId(7));
    row.source = BimSource::default();
    row.entity_ids = vec![EntityId(7)];
    let page = QueryPage {
        request: page_request(0, 0),
        rows: vec![row],
        total: 1,
    };
    let value = serde_json::to_value(query_response(page, Some(DatabaseId(3)))).expect("json");
    assert_eq!(value["rows"][0]["object"]["kind"], "entity");
    assert_eq!(value["rows"][0]["object"]["entityId"], 7);
    assert_eq!(value["total"], 1);
    assert_eq!(value["limit"], 0, "limit=0 is the caller's window");
    assert_eq!(value["stale"], false);
}

#[test]
fn no_document_answers_zero_identity_and_no_rows() {
    let page = QueryPage {
        request: page_request(0, 100),
        rows: Vec::new(),
        total: 0,
    };
    let value = serde_json::to_value(query_response(page, None)).expect("json");
    assert_eq!(value["databaseId"], 0);
    assert_eq!(value["total"], 0);
    assert_eq!(value["rows"], serde_json::json!([]));
}

#[test]
fn a_summary_response_carries_the_counters() {
    let summary = BimSummary {
        total: 12,
        by_category: vec![BimFacetCount {
            key: "ifcwall".into(),
            name: "IfcWall".into(),
            count: 12,
        }],
        by_level: vec![BimFacetCount {
            key: "level:ifc:doc:L1".into(),
            name: "Level 1".into(),
            count: 12,
        }],
        by_type: Vec::new(),
        by_material: vec![BimFacetCount {
            key: "material:ifc:doc:M1".into(),
            name: "Concrete".into(),
            count: 7,
        }],
    };
    let page = QueryPage {
        request: page_request(0, 1),
        rows: vec![summary],
        total: 1,
    };
    let response = summary_response(page, Some(DatabaseId(1)));
    assert_eq!(response.total, 12, "the outer total counts components");
    let value = serde_json::to_value(&response).expect("json");
    assert_eq!(
        value,
        serde_json::json!({
            "databaseId": 1,
            "revision": 42,
            "total": 12,
            "summary": {
                "total": 12,
                "byCategory": [{ "key": "ifcwall", "name": "IfcWall", "count": 12 }],
                "byLevel": [{ "key": "level:ifc:doc:L1", "name": "Level 1", "count": 12 }],
                "byType": [],
                "byMaterial": [{ "key": "material:ifc:doc:M1", "name": "Concrete", "count": 7 }],
            },
        })
    );
}

#[test]
fn a_summary_page_without_its_row_is_an_honest_empty_summary() {
    let page = QueryPage {
        request: page_request(0, 0),
        rows: Vec::new(),
        total: 1,
    };
    let value = serde_json::to_value(summary_response(page, None)).expect("json");
    assert_eq!(value["total"], 0);
    assert_eq!(value["summary"]["total"], 0);
    assert_eq!(value["summary"]["byCategory"], serde_json::json!([]));
}

#[test]
fn the_backend_filter_is_built_without_guessing() {
    let request: BimQueryRequest = serde_json::from_value(serde_json::json!({
        "filter": {
            "categories": ["IfcWall"],
            "levels": ["Level 1"],
            "types": ["Basic Wall"],
            "materials": ["Concrete"],
            "property": {
                "pset": "Pset_WallCommon",
                "property": "IsExternal",
                "op": "eq",
                "value": { "kind": "integer", "value": 3 },
            },
            "evidence": "heuristic",
        },
        "offset": 4,
        "limit": 6,
    }))
    .expect("request");
    let filter = request.filter.to_filter().expect("filter");
    assert_eq!(filter.categories, vec!["IfcWall"]);
    assert_eq!(filter.levels, vec!["Level 1"]);
    assert_eq!(filter.types, vec!["Basic Wall"]);
    assert_eq!(filter.materials, vec!["Concrete"]);
    assert_eq!(filter.evidence, Some(BimEvidence::Heuristic));
    let property = filter.property.expect("property");
    assert_eq!(property.pset, "Pset_WallCommon");
    assert_eq!(property.property, "IsExternal");
    assert_eq!(property.op, BimPropertyOp::Eq);
    assert_eq!(property.value, BimScalar::Integer(3));
    assert_eq!(request.offset, 4);
    assert_eq!(request.limit, 6);
}

#[test]
fn a_property_value_round_trips_through_the_backend_value_types() {
    // The stored types are the source of the four wire kinds; this pins that a
    // value can be carried backend → wire → backend unchanged.
    let stored = [
        BimValue::Bool(true),
        BimValue::Integer(-7),
        BimValue::Real {
            value: 2.5,
            ifc_type: None,
            unit: None,
        },
        BimValue::Text("Concrete".into()),
    ];
    for value in stored {
        let scalar = BimScalar::from_value(&value).expect("a non-null value projects");
        let wire = match &scalar {
            BimScalar::Bool(value) => BimValueWire::Bool { value: *value },
            BimScalar::Integer(value) => BimValueWire::Integer { value: *value },
            BimScalar::Real(value) => BimValueWire::Real { value: value.get() },
            BimScalar::Text(value) => BimValueWire::Text {
                value: value.clone(),
            },
        };
        assert_eq!(wire.to_scalar().expect("scalar"), scalar);
    }
}

#[test]
fn the_property_summary_counts_the_declared_sets() {
    // `BimRow::property_summary` is one entry per set; the wire keeps order.
    let sets = [
        BimPropertySet {
            name: "Pset_WallCommon".into(),
            properties: vec![BimProperty {
                name: "IsExternal".into(),
                value: BimValue::Bool(true),
            }],
        },
        BimPropertySet {
            name: "Pset_Revit_Type".into(),
            properties: vec![],
        },
    ];
    let row = BimRow {
        property_summary: sets
            .iter()
            .map(|set| BimPropertyCount {
                name: set.name.clone(),
                count: set.properties.len(),
            })
            .collect(),
        ..wall_row()
    };
    let wire = BimRowWire::from_row(&row);
    assert_eq!(
        wire.property_summary,
        vec![
            BimPropertyCountWire {
                name: "Pset_WallCommon".into(),
                count: 1
            },
            BimPropertyCountWire {
                name: "Pset_Revit_Type".into(),
                count: 0
            },
        ]
    );
}
