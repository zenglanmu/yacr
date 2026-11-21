//! BIM property-row contracts (ADR-0042 D4).
//!
//! The panel may only emit the eight fixed `bim_*` keys, each at most once, and
//! only for an element that really carries a `bim` block. These tests read the
//! real `cad-app` projection over a real `DrawingDatabase`, so a duplicate or
//! dynamic key would fail here before it could silently drop a row under the
//! multi-selection `find` lookup.
use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource,
    BimValue, DbEntity, DbObject, DrawingDatabaseBuilder, Layer,
};
use cad_domain::{DatabaseId, DocumentId, EntityId, LayerId, ObjectId, Revision, SemanticGeometry};

use crate::selection::{model_ref, PropertyRow, SelectionProperties, SelectionSet};

const ENTITY: EntityId = EntityId(1);
const OTHER: EntityId = EntityId(2);

/// The eight static keys Phase 1 may emit, in panel order.
const BIM_KEYS: [&str; 8] = [
    "bim_category",
    "bim_name",
    "bim_level",
    "bim_type",
    "bim_material",
    "bim_source",
    "bim_properties",
    "bim_evidence",
];

fn line(id: EntityId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id.0),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id,
        layer: LayerId(0),
        space: cad_domain::SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: cad_domain::Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: cad_domain::Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
        draw_order: id.0 as i64,
    }
}

fn property_set(name: &str, properties: usize) -> BimPropertySet {
    BimPropertySet {
        name: name.into(),
        properties: (0..properties)
            .map(|index| BimProperty {
                name: format!("P{index}"),
                value: BimValue::Integer(index as i64),
            })
            .collect(),
    }
}

fn definitions() -> BimDefinitions {
    let source = BimSource {
        connector: "ifc".into(),
        document_id: Some("2O2Fr$t4XyZ1234567890".into()),
        id: Some("2O2Fr$t4XyZ1234567890".into()),
        native_id: Some("22".into()),
    };
    let mut definitions = BimDefinitions::new();
    for row in [
        BimDefinition::Level {
            id: "level:ifc:doc:1".into(),
            name: "Level 1".into(),
            elevation: Some(0.0),
            source: source.clone(),
            evidence: BimEvidence::Declared,
        },
        BimDefinition::Type {
            id: "type:ifc:doc:9".into(),
            name: "Basic Wall:Generic 200mm".into(),
            class: Some("IfcWallType".into()),
            category: Some("IfcWall".into()),
            psets: Vec::new(),
            source: source.clone(),
            evidence: BimEvidence::Declared,
        },
        BimDefinition::Material {
            id: "material:ifc:doc:3".into(),
            name: "Concrete".into(),
            source,
            evidence: BimEvidence::Declared,
        },
    ] {
        definitions.insert(row).unwrap();
    }
    definitions
}

fn wall(evidence: BimEvidence, category: &str) -> BimElement {
    BimElement {
        category: Some(category.into()),
        name: Some("Basic Wall:Generic 200mm".into()),
        source: BimSource {
            connector: "ifc".into(),
            document_id: Some("2O2Fr$t4XyZ1234567890".into()),
            id: Some("2O2Fr$t4XyZ1234567890".into()),
            native_id: Some("22".into()),
        },
        level: Some("level:ifc:doc:1".into()),
        element_type: Some("type:ifc:doc:9".into()),
        material: Some("material:ifc:doc:3".into()),
        psets: vec![
            property_set("Pset_WallCommon", 3),
            property_set("Pset_Revit_Type", 5),
        ],
        evidence,
    }
}

fn bim_database(entities: &[(EntityId, Option<BimElement>)]) -> cad_db::DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.set_bim_definitions(definitions()).unwrap();
    for (id, bim) in entities {
        builder.insert_entity(line(*id)).unwrap();
        if let Some(bim) = bim {
            builder.set_entity_bim(*id, bim.clone()).unwrap();
        }
    }
    builder.finish().unwrap()
}

fn extract(database: &cad_db::DrawingDatabase, entities: &[EntityId]) -> SelectionProperties {
    SelectionProperties::extract(
        database,
        &SelectionSet::from_refs(entities.iter().map(|id| model_ref(DocumentId(1), *id))),
    )
}

fn row<'a>(properties: &'a SelectionProperties, key: &str) -> &'a PropertyRow {
    properties
        .rows
        .iter()
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("missing row {key}"))
}

#[test]
fn bim_panel_rows_use_static_keys_and_a_pset_summary() {
    let database = bim_database(&[(ENTITY, Some(wall(BimEvidence::Declared, "IfcWall")))]);
    let properties = extract(&database, &[ENTITY]);

    // Exactly the eight fixed keys, in panel order, with their stable group.
    let bim_rows: Vec<&PropertyRow> = properties
        .rows
        .iter()
        .filter(|row| row.key.starts_with("bim_"))
        .collect();
    assert_eq!(
        bim_rows.iter().map(|row| row.key).collect::<Vec<_>>(),
        BIM_KEYS
    );
    for row in &bim_rows {
        assert_eq!(
            row.group.key(),
            "bim",
            "{} must be in the BIM group",
            row.key
        );
        assert!(!row.editable, "{} is read-only in Phase 1", row.key);
        assert!(!row.mixed);
    }

    assert_eq!(
        row(&properties, "bim_category").value.as_deref(),
        Some("IfcWall")
    );
    assert_eq!(
        row(&properties, "bim_name").value.as_deref(),
        Some("Basic Wall:Generic 200mm")
    );
    // Definition references resolve to the shared row's display name.
    assert_eq!(
        row(&properties, "bim_level").value.as_deref(),
        Some("Level 1")
    );
    assert_eq!(
        row(&properties, "bim_type").value.as_deref(),
        Some("Basic Wall:Generic 200mm")
    );
    assert_eq!(
        row(&properties, "bim_material").value.as_deref(),
        Some("Concrete")
    );
    // One readable provenance line; both identifiers are abbreviated to 8 chars.
    assert_eq!(
        row(&properties, "bim_source").value.as_deref(),
        Some("ifc / doc 2O2Fr$t4... / id 2O2Fr$t4...")
    );
    // A single-line PSet summary with the real stored property counts.
    assert_eq!(
        row(&properties, "bim_properties").value.as_deref(),
        Some("Pset_WallCommon (3), Pset_Revit_Type (5)")
    );
    assert_eq!(
        row(&properties, "bim_evidence").value.as_deref(),
        Some("declared")
    );

    // No key may repeat: the multi-select `find` lookup keeps only one of them.
    assert_eq!(
        unique_keys(&properties.rows).len(),
        properties.rows.len(),
        "every row key appears at most once"
    );
}

#[test]
fn a_non_bim_element_has_no_bim_rows() {
    let database = bim_database(&[(ENTITY, None)]);
    let properties = extract(&database, &[ENTITY]);
    assert!(
        !properties.rows.is_empty(),
        "the entity rows themselves still exist"
    );
    assert!(
        properties
            .rows
            .iter()
            .all(|row| !row.key.starts_with("bim_")),
        "an element with no bim block adds no BIM row and no placeholder"
    );
}

#[test]
fn bim_rows_do_not_duplicate_keys_under_multi_select() {
    let database = bim_database(&[
        (ENTITY, Some(wall(BimEvidence::Declared, "IfcWall"))),
        (OTHER, Some(wall(BimEvidence::Heuristic, "IfcSlab"))),
    ]);
    let properties = extract(&database, &[ENTITY, OTHER]);

    // The intersection keeps one row per key, and the differing values are
    // marked mixed instead of producing a second row.
    let bim_rows: Vec<&PropertyRow> = properties
        .rows
        .iter()
        .filter(|row| row.key.starts_with("bim_"))
        .collect();
    assert_eq!(
        bim_rows.iter().map(|row| row.key).collect::<Vec<_>>(),
        BIM_KEYS
    );
    assert_eq!(unique_keys(&properties.rows).len(), properties.rows.len());
    assert!(row(&properties, "bim_category").mixed);
    assert!(properties.mixed_keys.contains(&"bim_category"));
    assert!(row(&properties, "bim_evidence").mixed);
    // The shared definitions still agree.
    assert_eq!(
        row(&properties, "bim_level").value.as_deref(),
        Some("Level 1")
    );

    // One BIM element plus one plain element: the intersection excludes every
    // BIM key, because the plain element does not report it.
    let mixed_database = bim_database(&[
        (ENTITY, Some(wall(BimEvidence::Declared, "IfcWall"))),
        (OTHER, None),
    ]);
    let mixed = extract(&mixed_database, &[ENTITY, OTHER]);
    assert!(mixed.rows.iter().all(|row| !row.key.starts_with("bim_")));
}

#[test]
fn bim_evidence_is_localized_via_catalog() {
    // The row carries the persisted machine token, never a sentence. The human
    // copy lives in the one catalog (both languages) under
    // `properties.value.bim_evidence.<token>`.
    for evidence in [BimEvidence::Declared, BimEvidence::Heuristic] {
        let database = bim_database(&[(ENTITY, Some(wall(evidence, "IfcWall")))]);
        let properties = extract(&database, &[ENTITY]);
        let value = row(&properties, "bim_evidence")
            .value
            .clone()
            .expect("a BIM element always reports its evidence");
        assert_eq!(value, evidence.as_str());
        assert!(
            value.is_ascii(),
            "the row value is a machine token, not localized copy: {value:?}"
        );
        assert!(
            !value.contains(' '),
            "the row value is a token, not a sentence: {value:?}"
        );
        let key = format!("properties.value.bim_evidence.{value}");
        for catalog in [EN_CATALOG, ZH_CATALOG] {
            let copy = catalog_entry(catalog, &key);
            assert!(!copy.is_empty(), "missing {key}");
            assert_ne!(copy, key, "{key} must carry real copy");
        }
    }
}

/// The two shipped catalogs, read at compile time so the Rust side proves the
/// evidence tokens have copy in **both** languages without a runtime file read.
const EN_CATALOG: &str = include_str!("../../../cad-ui-core/i18n/en.json");
const ZH_CATALOG: &str = include_str!("../../../cad-ui-core/i18n/zh-CN.json");

/// The flat-JSON string value at `key`, or `""` when absent. Parsing the whole
/// catalog also proves both files stay flat JSON objects.
fn catalog_entry(catalog: &str, key: &str) -> String {
    let parsed: serde_json::Value =
        serde_json::from_str(catalog).expect("the catalog is a flat JSON object");
    parsed
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn unique_keys(rows: &[PropertyRow]) -> std::collections::BTreeSet<&'static str> {
    rows.iter().map(|row| row.key).collect()
}
