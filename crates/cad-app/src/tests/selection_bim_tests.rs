//! BIM property-row contracts (ADR-0042 D4).
//!
//! The panel emits seven fixed `bim_*` keys, plus one dynamic row per stored,
//! non-null PSet property (`bim.pset.{set}.{property}`), capped per element.
//! These tests read the real `cad-app` projection over a real
//! `DrawingDatabase`, so a duplicate or unstable dynamic key fails here before
//! it could silently drop a row under the multi-selection `find` lookup.
//!
//! The dynamic key grammar has one implementation ([`BimPropertyKey`]): these
//! tests pin its canonical form, the strict parse and the `editable`/`kind`
//! rules the panel and the wire admission both rely on.
use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource,
    BimValue, DbEntity, DbObject, DrawingDatabaseBuilder, Layer,
};
use cad_domain::{
    DatabaseId, DocumentId, EntityId, InstancePath, LayerId, ObjectId, Revision, SelectionRef,
    SemanticGeometry, SubElementId,
};

use crate::selection::{
    model_ref, PropertyGroup, PropertyKind, PropertyRow, SelectionProperties, SelectionSet,
};
use crate::selection_bim::{is_bim_property_key, BimPropertyKey, MAX_DYNAMIC_BIM_ROWS};

const ENTITY: EntityId = EntityId(1);
const OTHER: EntityId = EntityId(2);

/// The seven static keys the panel may emit, in panel order.
const BIM_KEYS: [&str; 7] = [
    "bim_category",
    "bim_name",
    "bim_level",
    "bim_type",
    "bim_material",
    "bim_source",
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

fn property(name: &str, value: BimValue) -> BimProperty {
    BimProperty {
        name: name.into(),
        value,
    }
}

fn set(name: &str, properties: Vec<BimProperty>) -> BimPropertySet {
    BimPropertySet {
        name: name.into(),
        properties,
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

fn wall_with(evidence: BimEvidence, category: &str, psets: Vec<BimPropertySet>) -> BimElement {
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
        psets,
        evidence,
    }
}

fn wall(evidence: BimEvidence, category: &str) -> BimElement {
    wall_with(
        evidence,
        category,
        vec![
            property_set("Pset_WallCommon", 3),
            property_set("Pset_Revit_Type", 5),
        ],
    )
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

fn dynamic_rows(properties: &SelectionProperties) -> Vec<&PropertyRow> {
    properties
        .rows
        .iter()
        .filter(|row| row.key.starts_with("bim.pset."))
        .collect()
}

#[test]
fn bim_panel_rows_keep_the_seven_static_keys_and_drop_the_pset_summary() {
    let database = bim_database(&[(ENTITY, Some(wall(BimEvidence::Declared, "IfcWall")))]);
    let properties = extract(&database, &[ENTITY]);

    // Exactly the seven fixed keys, in panel order, with their stable group.
    let bim_rows: Vec<&PropertyRow> = properties
        .rows
        .iter()
        .filter(|row| row.key.starts_with("bim_"))
        .collect();
    assert_eq!(
        bim_rows
            .iter()
            .map(|row| row.key.as_ref())
            .collect::<Vec<_>>(),
        BIM_KEYS
    );
    assert!(
        properties
            .rows
            .iter()
            .all(|row| row.key != "bim_properties"),
        "the one-line PSet summary is replaced by per-property rows"
    );
    for row in &bim_rows {
        assert_eq!(
            row.group.key(),
            "bim",
            "{} must be in the BIM group",
            row.key
        );
        assert!(row.label.is_none(), "a static row has no source label");
        assert!(!row.editable, "{} is read-only", row.key);
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
fn bim_pset_properties_become_one_row_each() {
    let database = bim_database(&[(ENTITY, Some(wall(BimEvidence::Declared, "IfcWall")))]);
    let properties = extract(&database, &[ENTITY]);

    // 3 properties in the first set plus 5 in the second: one row each, in the
    // stored (declared) order.
    let rows = dynamic_rows(&properties);
    assert_eq!(rows.len(), 3 + 5);
    assert_eq!(
        rows.iter().map(|row| row.key.as_ref()).collect::<Vec<_>>(),
        vec![
            "bim.pset.0.0",
            "bim.pset.0.1",
            "bim.pset.0.2",
            "bim.pset.1.0",
            "bim.pset.1.1",
            "bim.pset.1.2",
            "bim.pset.1.3",
            "bim.pset.1.4",
        ]
    );
    for row in &rows {
        assert_eq!(row.group, PropertyGroup::Bim);
        assert_eq!(row.kind, PropertyKind::Number);
        assert!(
            row.editable,
            "a scalar property of a directly addressed element is writable"
        );
        assert!(!row.mixed);
    }
    assert_eq!(row(&properties, "bim.pset.0.0").value.as_deref(), Some("0"));
    assert_eq!(row(&properties, "bim.pset.1.4").value.as_deref(), Some("4"));
}

#[test]
fn dynamic_rows_have_unique_keys_and_human_labels() {
    let database = bim_database(&[(ENTITY, Some(wall(BimEvidence::Declared, "IfcWall")))]);
    let properties = extract(&database, &[ENTITY]);

    let rows = dynamic_rows(&properties);
    let keys: Vec<String> = rows.iter().map(|row| row.key.to_string()).collect();
    assert_eq!(
        unique_keys(&properties.rows).len(),
        properties.rows.len(),
        "dynamic keys must stay unique"
    );
    assert!(keys.iter().all(|key| key.starts_with("bim.pset.")));

    // The source's own names travel in `label`, not in the machine key.
    assert_eq!(
        row(&properties, "bim.pset.0.0").label.as_deref(),
        Some("Pset_WallCommon.P0")
    );
    assert_eq!(
        row(&properties, "bim.pset.1.4").label.as_deref(),
        Some("Pset_Revit_Type.P4")
    );
    for row in &rows {
        let label = row.label.as_deref().expect("a dynamic row has a label");
        assert!(
            label.is_ascii(),
            "the label is source data, not localized copy: {label:?}"
        );
        // The label is not a machine key: it carries the source's separators.
        assert!(label.contains('.'));
    }
}

#[test]
fn a_null_property_has_no_row() {
    let element = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![set(
            "Pset_WallCommon",
            vec![
                property("Missing", BimValue::Null),
                property("IsExternal", BimValue::Bool(true)),
            ],
        )],
    );
    let database = bim_database(&[(ENTITY, Some(element))]);
    let properties = extract(&database, &[ENTITY]);

    assert!(
        properties.rows.iter().all(|row| row.key != "bim.pset.0.0"),
        "a null property is not a value, so it gets no row"
    );
    // The stored index is preserved: the real value is still at position 1.
    assert_eq!(
        row(&properties, "bim.pset.0.1").value.as_deref(),
        Some("true")
    );
    assert_eq!(row(&properties, "bim.pset.0.1").kind, PropertyKind::Bool);
}

#[test]
fn a_real_property_with_a_unit_is_a_text_row() {
    let element = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![set(
            "Pset_WallCommon",
            vec![property(
                "Width",
                BimValue::Real {
                    value: 3.5,
                    ifc_type: Some("IfcPositiveLengthMeasure".into()),
                    unit: Some("m".into()),
                },
            )],
        )],
    );
    let database = bim_database(&[(ENTITY, Some(element))]);
    let properties = extract(&database, &[ENTITY]);

    let width = row(&properties, "bim.pset.0.0");
    assert_eq!(
        width.value.as_deref(),
        Some("3.5 m"),
        "the shortest round-trip form plus the source's unit symbol"
    );
    assert_eq!(
        width.kind,
        PropertyKind::Text,
        "a number editor cannot hold \"3.5 m\"; the unit makes it a text row"
    );
    assert!(width.editable, "the row itself is still writable");
}

#[test]
fn a_real_property_without_a_unit_is_a_number_row() {
    let element = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![set(
            "Pset_WallCommon",
            vec![property(
                "Volume",
                BimValue::Real {
                    value: 12.25,
                    ifc_type: None,
                    unit: None,
                },
            )],
        )],
    );
    let database = bim_database(&[(ENTITY, Some(element))]);
    let properties = extract(&database, &[ENTITY]);

    let volume = row(&properties, "bim.pset.0.0");
    assert_eq!(
        volume.value.as_deref(),
        Some("12.25"),
        "no unit means no fabricated unit suffix"
    );
    assert_eq!(
        volume.kind,
        PropertyKind::Number,
        "a unit-less real is exactly what a number editor round-trips"
    );
}

#[test]
fn at_most_64_dynamic_rows_per_element() {
    let element = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![property_set("Pset_Big", MAX_DYNAMIC_BIM_ROWS + 6)],
    );
    let database = bim_database(&[(ENTITY, Some(element))]);
    let properties = extract(&database, &[ENTITY]);

    let rows = dynamic_rows(&properties);
    assert_eq!(rows.len(), MAX_DYNAMIC_BIM_ROWS);
    assert_eq!(rows.first().unwrap().key.as_ref(), "bim.pset.0.0");
    assert_eq!(
        rows.last().unwrap().key.as_ref(),
        "bim.pset.0.63",
        "the cap keeps a stable prefix, not a silent reshuffle"
    );
    assert!(
        properties.rows.iter().all(|row| row.key != "bim.pset.0.64"),
        "rows beyond the cap are not emitted"
    );
}

#[test]
fn mismatched_labels_across_a_multi_selection_show_mixed() {
    // Same stored position (set 0, property 0) and even the same value, but the
    // source names differ: the slot no longer describes one property.
    let first = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![set(
            "Pset_WallCommon",
            vec![property("Width", BimValue::Integer(1))],
        )],
    );
    let second = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![set(
            "Pset_Revit_Type",
            vec![property("Height", BimValue::Integer(1))],
        )],
    );
    let database = bim_database(&[(ENTITY, Some(first)), (OTHER, Some(second))]);
    let properties = extract(&database, &[ENTITY, OTHER]);

    let merged = row(&properties, "bim.pset.0.0");
    assert!(
        merged.mixed,
        "different source names make the row a mixed slot"
    );
    assert_eq!(merged.value, None);
    assert_eq!(
        merged.label, None,
        "the label cannot name two different source properties"
    );
    assert!(properties
        .mixed_keys
        .iter()
        .any(|key| key == "bim.pset.0.0"));
}

#[test]
fn dynamic_rows_never_offer_picker_options() {
    let database = bim_database(&[(ENTITY, Some(wall(BimEvidence::Declared, "IfcWall")))]);
    let properties = extract(&database, &[ENTITY]);
    for row in dynamic_rows(&properties) {
        assert!(
            row.options.is_empty(),
            "a dynamic row has no picker: {}",
            row.key
        );
        assert!(!row.mixed);
    }
}

#[test]
fn a_bim_key_round_trips_through_key_and_parse() {
    for (set, property) in [(0usize, 0usize), (1, 4), (12, 63), (usize::MAX, 0)] {
        let key = BimPropertyKey { set, property };
        let text = key.key();
        assert_eq!(text, format!("bim.pset.{set}.{property}"));
        assert!(
            text.bytes().all(|byte| !byte.is_ascii_whitespace()),
            "a key carries no whitespace: {text:?}"
        );
        assert_eq!(BimPropertyKey::parse(&text), Some(key), "{text}");
        assert!(is_bim_property_key(&text), "{text}");
    }
    // Every key the panel emits parses back to its own stored position.
    let database = bim_database(&[(ENTITY, Some(wall(BimEvidence::Declared, "IfcWall")))]);
    let properties = extract(&database, &[ENTITY]);
    for row in dynamic_rows(&properties) {
        let parsed =
            BimPropertyKey::parse(&row.key).unwrap_or_else(|| panic!("{} does not parse", row.key));
        assert_eq!(parsed.key(), row.key.as_ref());
    }
}

#[test]
fn a_non_canonical_bim_key_is_not_a_bim_key() {
    for key in [
        "bim.pset.00.0",
        "bim.pset.0.00",
        "bim.pset.0",
        "bim.pset.0.0.1",
        "bim.pset.x",
        "bim.pset.",
        "bim.pset.0.",
        "bim.pset..0",
        "bim.pset.+1.0",
        "bim.pset.-1.0",
        "bim.pset. 0.0",
        "bim.pset.0. 0",
        " bim.pset.0.0",
        "bim.pset.0.0 ",
        "BIM.pset.0.0",
        "bim.pset.184467440737095516160.0",
        "",
    ] {
        assert_eq!(BimPropertyKey::parse(key), None, "{key:?}");
        assert!(!is_bim_property_key(key), "{key:?}");
    }
}

#[test]
fn dynamic_rows_are_editable_only_for_addressable_scalar_values() {
    let element = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![set(
            "Pset_WallCommon",
            vec![
                property(
                    "Width",
                    BimValue::Real {
                        value: 3.5,
                        ifc_type: None,
                        unit: Some("m".into()),
                    },
                ),
                property("Count", BimValue::Integer(2)),
                property("IsExternal", BimValue::Bool(true)),
                property("Name", BimValue::Text("wall".into())),
            ],
        )],
    );
    let database = bim_database(&[(ENTITY, Some(element))]);

    // Directly addressed top-level element: every emitted scalar row is writable.
    let direct = extract(&database, &[ENTITY]);
    let rows = dynamic_rows(&direct);
    assert_eq!(rows.len(), 4);
    for row in &rows {
        assert!(row.editable, "{} is a scalar at a direct address", row.key);
    }

    // A block-instance member and a sub-element reach the same entity through an
    // indirection no write path accepts, so the same rows are read-only.
    for reference in [
        SelectionRef {
            document: DocumentId(1),
            entity: ENTITY,
            instance: InstancePath(vec![EntityId(9)]),
            sub_element: None,
        },
        SelectionRef {
            document: DocumentId(1),
            entity: ENTITY,
            instance: InstancePath::default(),
            sub_element: Some(SubElementId {
                source_key: "face:0".into(),
                topology_revision: Revision(1),
            }),
        },
    ] {
        let properties =
            SelectionProperties::extract(&database, &SelectionSet::from_refs([reference]));
        let rows = dynamic_rows(&properties);
        assert_eq!(rows.len(), 4);
        for row in rows {
            assert!(
                !row.editable,
                "{} is not addressable through an indirect ref",
                row.key
            );
        }
    }
}

#[test]
fn a_mismatched_label_makes_the_row_not_editable() {
    // Same stored slot and even the same value, but different source names: the
    // slot no longer describes one property, so no single write addresses it.
    let first = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![set(
            "Pset_WallCommon",
            vec![property("Width", BimValue::Integer(1))],
        )],
    );
    let second = wall_with(
        BimEvidence::Declared,
        "IfcWall",
        vec![set(
            "Pset_Revit_Type",
            vec![property("Height", BimValue::Integer(1))],
        )],
    );
    let database = bim_database(&[(ENTITY, Some(first)), (OTHER, Some(second))]);
    let properties = extract(&database, &[ENTITY, OTHER]);

    let merged = row(&properties, "bim.pset.0.0");
    assert!(merged.mixed);
    assert!(
        !merged.editable,
        "the same slot of two different source properties cannot be written as one"
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
            .all(|row| !row.key.starts_with("bim")),
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
        bim_rows
            .iter()
            .map(|row| row.key.as_ref())
            .collect::<Vec<_>>(),
        BIM_KEYS
    );
    assert_eq!(unique_keys(&properties.rows).len(), properties.rows.len());
    assert!(row(&properties, "bim_category").mixed);
    assert!(properties
        .mixed_keys
        .iter()
        .any(|key| key == "bim_category"));
    assert!(row(&properties, "bim_evidence").mixed);
    // The shared definitions still agree.
    assert_eq!(
        row(&properties, "bim_level").value.as_deref(),
        Some("Level 1")
    );
    // The two elements declare identical PSet names and values, so the dynamic
    // slots agree and keep their labels — and stay writable, because both refs
    // address their element directly and agree on which property this is.
    let shared = row(&properties, "bim.pset.0.0");
    assert!(!shared.mixed);
    assert_eq!(shared.label.as_deref(), Some("Pset_WallCommon.P0"));
    assert!(shared.editable);

    // One BIM element plus one plain element: the intersection excludes every
    // BIM key, because the plain element does not report it.
    let mixed_database = bim_database(&[
        (ENTITY, Some(wall(BimEvidence::Declared, "IfcWall"))),
        (OTHER, None),
    ]);
    let mixed = extract(&mixed_database, &[ENTITY, OTHER]);
    assert!(mixed.rows.iter().all(|row| !row.key.starts_with("bim")));
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

fn unique_keys(rows: &[PropertyRow]) -> std::collections::BTreeSet<String> {
    rows.iter().map(|row| row.key.to_string()).collect()
}
