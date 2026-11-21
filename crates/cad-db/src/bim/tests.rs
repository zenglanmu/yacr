//! BIM side-table contracts (ADR-0042 D2/D3/D5/D6).
//!
//! What must hold for the feature to be honest:
//!
//! 1. an element's `bim` block round-trips through a real element row and is
//!    **omitted** when absent (a minor-5 reader is strict);
//! 2. the shared definition table round-trips, is strict about its rows and
//!    refuses an element that references a definition it does not hold;
//! 3. every budget is a refusal, never a truncation.

use cad_domain::relation::{Propagation, RelationFamily};
use cad_domain::*;
use serde_json::Value;

use crate::snapshot::{bim as bim_codec, elements};
use crate::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource,
    BimValue, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, EntityRenderAttributes,
    BIM_BUDGET_EXCEEDED, BIM_DANGLING_REFERENCE, BIM_MAX_DEFINITIONS, BIM_MAX_PROPERTIES_PER_SET,
    BIM_MAX_PSETS_PER_ELEMENT, BIM_MAX_STRING_BYTES,
};

// ---------------------------------------------------------------- fixtures ---

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(1.0, 0.0, 0.0),
        },
        draw_order: 0,
    }
}

fn source() -> BimSource {
    BimSource {
        connector: "ifc".to_string(),
        document_id: Some("doc-1".to_string()),
        id: Some("2O2Fr$t4X7Zf8Q".to_string()),
        native_id: Some("22".to_string()),
    }
}

fn definition_rows() -> BimDefinitions {
    let mut definitions = BimDefinitions::new();
    definitions
        .insert(BimDefinition::Level {
            id: "level:ifc:doc-1:L1".to_string(),
            name: "Level 1".to_string(),
            elevation: Some(0.0),
            source: source(),
            evidence: BimEvidence::Declared,
        })
        .expect("level");
    definitions
        .insert(BimDefinition::Type {
            id: "type:ifc:doc-1:T1".to_string(),
            name: "Basic Wall:Generic 200mm".to_string(),
            class: Some("IfcWallType".to_string()),
            category: Some("IfcWall".to_string()),
            psets: vec![BimPropertySet {
                name: "Pset_WallCommon".to_string(),
                properties: vec![BimProperty {
                    name: "LoadBearing".to_string(),
                    value: BimValue::Bool(true),
                }],
            }],
            source: source(),
            evidence: BimEvidence::Declared,
        })
        .expect("type");
    definitions
        .insert(BimDefinition::Material {
            id: "material:ifc:doc-1:M1".to_string(),
            name: "Concrete".to_string(),
            source: source(),
            evidence: BimEvidence::Declared,
        })
        .expect("material");
    definitions
}

fn element() -> BimElement {
    BimElement {
        category: Some("IfcWall".to_string()),
        name: Some("Basic Wall:Generic 200mm".to_string()),
        source: source(),
        level: Some("level:ifc:doc-1:L1".to_string()),
        element_type: Some("type:ifc:doc-1:T1".to_string()),
        material: Some("material:ifc:doc-1:M1".to_string()),
        psets: vec![BimPropertySet {
            name: "Pset_WallCommon".to_string(),
            properties: vec![
                BimProperty {
                    name: "IsExternal".to_string(),
                    value: BimValue::Bool(true),
                },
                BimProperty {
                    name: "Reference".to_string(),
                    value: BimValue::Real {
                        value: 0.2,
                        ifc_type: Some("IfcPositiveLengthMeasure".to_string()),
                        unit: Some("m".to_string()),
                    },
                },
            ],
        }],
        evidence: BimEvidence::Declared,
    }
}

// ------------------------------------------------------------- element row ---

#[test]
fn element_bim_round_trips_and_is_omitted_when_absent() {
    let entity = line(1);
    let attributes = EntityRenderAttributes::default();

    // Absent: the key is not written at all (not `"bim": null`).
    let without = elements::encode(&entity, Some(&attributes), false, None, None).expect("encode");
    assert!(
        without.get("bim").is_none(),
        "an element without BIM data must omit the key: {without}"
    );

    let bim = element();
    let encoded =
        elements::encode(&entity, Some(&attributes), false, Some(&bim), None).expect("encode");
    assert!(encoded.get("bim").is_some(), "the block is written");
    let text = serde_json::to_string(&encoded).expect("serialize");
    let parsed: Value = serde_json::from_str(&text).expect("parse");
    let decoded = elements::decode(&parsed).expect("decode");
    assert_eq!(decoded.bim, Some(bim));

    // A minor-5 row is the same row minus the key.
    let decoded = elements::decode(&without).expect("decode without bim");
    assert_eq!(decoded.bim, None);
}

#[test]
fn absent_bim_on_a_minor_five_row_decodes_as_none() {
    // A literal minor-5 row shape: exactly the pre-0042 descriptor field set.
    let entity = line(2);
    let value = elements::encode(&entity, None, false, None, None).expect("encode");
    let decoded = elements::decode(&value).expect("decode");
    assert_eq!(decoded.bim, None);
    // And the row really has no `bim` key to begin with.
    assert!(value.get("bim").is_none());
}

#[test]
fn a_row_with_bim_null_is_rejected() {
    let entity = line(3);
    let mut value = elements::encode(&entity, None, false, None, None).expect("encode");
    value["bim"] = Value::Null;
    let error = elements::decode(&value).expect_err("null bim");
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(
        error.detail.contains("expected a JSON object"),
        "{}",
        error.detail
    );
}

#[test]
fn a_unknown_bim_field_is_rejected() {
    let entity = line(4);
    let mut value = elements::encode(&entity, None, false, Some(&element()), None).expect("encode");
    value["bim"]["future_field"] = Value::from(1);
    let error = elements::decode(&value).expect_err("unknown field");
    assert!(error.detail.contains("future_field"), "{}", error.detail);
}

// -------------------------------------------------------------- definitions ---

fn snapshot_with(bim: Option<BimElement>, definitions: Option<BimDefinitions>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(crate::Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder.insert_entity(line(1)).expect("entity");
    if let Some(definitions) = definitions {
        builder
            .set_bim_definitions(definitions)
            .expect("definitions");
    }
    if let Some(bim) = bim {
        builder.set_entity_bim(EntityId(1), bim).expect("bim");
    }
    builder.finish().expect("database")
}

fn snapshot_of(db: &DrawingDatabase) -> crate::DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        Revision(1),
    )
    .expect("snapshot")
}

#[test]
fn bim_definitions_table_decodes_strictly_and_rejects_dangling_refs() {
    let db = snapshot_with(Some(element()), Some(definition_rows()));
    let snapshot = snapshot_of(&db);
    let rows = snapshot
        .tables
        .get(crate::TableKind::BimDefinitions)
        .expect("the table is written");
    assert_eq!(rows.len(), 3);

    // Round trip through the definition codec.
    let decoded = bim_codec::decode_definitions(&snapshot.tables).expect("decode");
    assert_eq!(decoded, definition_rows());

    // A row with an unknown field is refused.
    let mut tampered = snapshot.clone();
    let rows = tampered
        .tables
        .rows
        .get_mut(&crate::TableKind::BimDefinitions)
        .expect("table");
    rows[0]["future_field"] = Value::from(1);
    let error = DrawingDatabase::from_snapshot(&tampered).expect_err("unknown field");
    assert!(error.detail.contains("future_field"), "{}", error.detail);

    // A dangling element reference is refused at hydrate time.
    let mut dangling = snapshot.clone();
    let rows = dangling
        .tables
        .rows
        .get_mut(&crate::TableKind::BimDefinitions)
        .expect("table");
    rows.retain(|row| row["kind"] != "level");
    let error = DrawingDatabase::from_snapshot(&dangling).expect_err("dangling level");
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(
        error.detail.contains(BIM_DANGLING_REFERENCE),
        "{}",
        error.detail
    );
}

#[test]
fn a_definition_row_must_carry_its_kind_prefix() {
    let mut definitions = BimDefinitions::new();
    definitions
        .insert(BimDefinition::Level {
            id: "wrong:ifc:doc-1:L1".to_string(),
            name: "Level 1".to_string(),
            elevation: None,
            source: source(),
            evidence: BimEvidence::Declared,
        })
        .expect("insert only checks uniqueness");
    // The id prefix is a shape rule, checked by `validate_definition`.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    let error = builder
        .set_bim_definitions(definitions)
        .expect_err("the id prefix is checked");
    assert!(error.to_string().contains("level:"), "{}", error);
}

// ----------------------------------------------------------------- budgets ---

#[test]
fn bim_budget_exceeded_is_an_explicit_error() {
    // Per-element property-set budget: one element cannot hold 65 sets.
    let mut too_many_psets = element();
    too_many_psets.psets = (0..=BIM_MAX_PSETS_PER_ELEMENT)
        .map(|index| BimPropertySet {
            name: format!("Pset_{index}"),
            properties: Vec::new(),
        })
        .collect();
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(crate::Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder.insert_entity(line(1)).expect("entity");
    let error = builder
        .set_entity_bim(EntityId(1), too_many_psets)
        .expect_err("set budget");
    assert!(error.to_string().contains(BIM_BUDGET_EXCEEDED), "{error}");

    // Per-property budget.
    let mut too_many_properties = element();
    too_many_properties.psets[0].properties = (0..=BIM_MAX_PROPERTIES_PER_SET)
        .map(|index| BimProperty {
            name: format!("P{index}"),
            value: BimValue::Null,
        })
        .collect();
    let error = builder
        .set_entity_bim(EntityId(1), too_many_properties)
        .expect_err("property budget");
    assert!(error.to_string().contains(BIM_BUDGET_EXCEEDED), "{error}");

    // Per-string budget.
    let mut long_string = element();
    long_string.name = Some("x".repeat(BIM_MAX_STRING_BYTES + 1));
    let error = builder
        .set_entity_bim(EntityId(1), long_string)
        .expect_err("string budget");
    assert!(error.to_string().contains(BIM_BUDGET_EXCEEDED), "{error}");

    // Definition-count budget: refused, never truncated.
    let mut definitions = BimDefinitions::new();
    for index in 0..=BIM_MAX_DEFINITIONS {
        definitions
            .insert(BimDefinition::Material {
                id: format!("material:ifc:doc-1:M{index}"),
                name: format!("M{index}"),
                source: source(),
                evidence: BimEvidence::Declared,
            })
            .expect("unique id");
    }
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(crate::Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder.insert_entity(line(1)).expect("entity");
    builder
        .set_bim_definitions(definitions)
        .expect("rows are shape-valid");
    let error = builder.finish().expect_err("count budget");
    assert!(error.to_string().contains(BIM_BUDGET_EXCEEDED), "{error}");
}

#[test]
fn an_oversized_bim_string_in_an_archive_is_refused() {
    // The read side enforces the same string budget as the builder: a hostile
    // archive cannot make the reader allocate an unbounded name.
    let db = snapshot_with(Some(element()), Some(definition_rows()));
    let mut snapshot = snapshot_of(&db);
    let shard = &mut snapshot.shards[0];
    shard.elements[0].payload["bim"]["name"] = Value::from("x".repeat(BIM_MAX_STRING_BYTES + 1));
    let error = DrawingDatabase::from_snapshot(&snapshot).expect_err("oversized name");
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(
        error.detail.contains(BIM_BUDGET_EXCEEDED),
        "{}",
        error.detail
    );
}

#[test]
fn voids_and_fills_are_reference_edges_without_a_rule() {
    for (token, kind) in [
        ("voids", crate::BimRelationKind::Voids),
        ("fills", crate::BimRelationKind::Fills),
    ] {
        assert_eq!(kind.as_token(), token);
        assert_eq!(crate::BimRelationKind::parse(token), Some(kind));
        assert_eq!(
            cad_domain::relation::relation_family(token),
            RelationFamily::Reference,
            "{token} is a D1 reference edge"
        );
        assert_eq!(
            cad_domain::relation::relation_propagation(token, "IfcWall", "IfcDoor"),
            Propagation::NoRule,
            "P2 registers no (host, hosted) rule"
        );
        // The IFC exporter only writes `contains`/`nests`; its wildcard arm
        // reports every other kind as `export.ifc.bim.relation_not_written`, so
        // these tokens can never masquerade as a container/nesting edge.
        assert_ne!(token, "contains");
        assert_ne!(token, "nests");
    }
}
