//! BIM query contracts (ADR-0042 D11).
//!
//! What must hold for the feature to be honest:
//!
//! 1. one source element split into several geometry rows is one component;
//! 2. facets AND across, terms OR inside one facet;
//! 3. property matching handles real/integer, bool and text without guessing;
//! 4. statistics count components, not geometry rows;
//! 5. an empty document is an empty page, and a revision change rebuilds;
//! 6. an over-budget filter is an explicit error, and row order is stable.

use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource,
    BimValue, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer,
};
use cad_domain::*;

use super::*;
use crate::QueryRequest;

const LEVEL_1: &str = "level:ifc:doc:L1";
const LEVEL_2: &str = "level:ifc:doc:L2";
const TYPE_1: &str = "type:ifc:doc:T1";
const MATERIAL_1: &str = "material:ifc:doc:M1";

fn request() -> QueryRequest {
    QueryRequest {
        document: DocumentId(1),
        revision: Revision(0),
        request: RequestId(1),
        offset: 0,
        limit: 100,
    }
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
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
        draw_order: 0,
    }
}

fn source(id: &str) -> BimSource {
    BimSource {
        connector: "ifc".to_string(),
        document_id: Some("doc".to_string()),
        id: Some(id.to_string()),
        native_id: None,
    }
}

fn key(id: &str) -> BimKey {
    BimKey::Source {
        connector: "ifc".to_string(),
        document_id: "doc".to_string(),
        id: id.to_string(),
    }
}

fn property_set(name: &str, properties: Vec<(&str, BimValue)>) -> BimPropertySet {
    BimPropertySet {
        name: name.to_string(),
        properties: properties
            .into_iter()
            .map(|(name, value)| BimProperty {
                name: name.to_string(),
                value,
            })
            .collect(),
    }
}

fn real(value: f64) -> BimValue {
    BimValue::Real {
        value,
        ifc_type: None,
        unit: None,
    }
}

fn element(
    source_id: &str,
    category: Option<&str>,
    name: Option<&str>,
    level: Option<&str>,
    element_type: Option<&str>,
    material: Option<&str>,
    psets: Vec<BimPropertySet>,
) -> BimElement {
    BimElement {
        category: category.map(str::to_string),
        name: name.map(str::to_string),
        source: source(source_id),
        level: level.map(str::to_string),
        element_type: element_type.map(str::to_string),
        material: material.map(str::to_string),
        psets,
        evidence: BimEvidence::Declared,
    }
}

fn wall(source_id: &str, name: &str, level: &str) -> BimElement {
    element(
        source_id,
        Some("IfcWall"),
        Some(name),
        Some(level),
        Some(TYPE_1),
        Some(MATERIAL_1),
        Vec::new(),
    )
}

fn definitions() -> BimDefinitions {
    let mut definitions = BimDefinitions::new();
    for (id, name, elevation) in [(LEVEL_1, "Level 1", 0.0), (LEVEL_2, "Level 2", 3.0)] {
        definitions
            .insert(BimDefinition::Level {
                id: id.to_string(),
                name: name.to_string(),
                elevation: Some(elevation),
                source: source("storey"),
                evidence: BimEvidence::Declared,
            })
            .expect("level");
    }
    for (id, name, class) in [
        (TYPE_1, "Basic Wall:Generic 200mm", "IfcWallType"),
        ("type:ifc:doc:T2", "Floor:Slab", "IfcSlabType"),
    ] {
        definitions
            .insert(BimDefinition::Type {
                id: id.to_string(),
                name: name.to_string(),
                class: Some(class.to_string()),
                category: None,
                psets: Vec::new(),
                source: source("type"),
                evidence: BimEvidence::Declared,
            })
            .expect("type");
    }
    for (id, name) in [(MATERIAL_1, "Concrete"), ("material:ifc:doc:M2", "Steel")] {
        definitions
            .insert(BimDefinition::Material {
                id: id.to_string(),
                name: name.to_string(),
                source: source("material"),
                evidence: BimEvidence::Declared,
            })
            .expect("material");
    }
    definitions
}

fn database_with(revision: u64, rows: Vec<(u128, BimElement)>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder.set_revision(Revision(revision));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    for (id, bim) in rows {
        builder.insert_entity(line(id)).expect("entity");
        builder
            .set_entity_bim(EntityId(id), bim)
            .expect("entity bim");
    }
    builder
        .set_bim_definitions(definitions())
        .expect("definitions");
    builder.finish().expect("database")
}

fn row_keys(rows: &[BimRow]) -> BTreeSet<String> {
    rows.iter()
        .map(|row| match &row.object {
            BimKey::Source { id, .. } => id.clone(),
            BimKey::Entity(entity) => entity.0.to_string(),
        })
        .collect()
}

fn keys_of(rows: &[&str]) -> BTreeSet<String> {
    rows.iter().map(|id| id.to_string()).collect()
}

// ---------------------------------------------------------------- the index ---

#[test]
fn index_groups_one_source_key_across_several_entities() {
    let db = database_with(
        0,
        vec![
            (1, wall("wall-1", "Wall", LEVEL_1)),
            (2, wall("wall-1", "Wall", LEVEL_1)),
            (3, wall("wall-1", "Wall", LEVEL_1)),
        ],
    );
    let index = BimIndex::build(&db);

    assert_eq!(index.len(), 1);
    assert_eq!(
        index.entities_of(&key("wall-1")),
        &BTreeSet::from([EntityId(1), EntityId(2), EntityId(3)])
    );
    assert_eq!(
        index.by_category().get("ifcwall"),
        Some(&BTreeSet::from([key("wall-1")]))
    );
}

// ----------------------------------------------------------------- filtering ---

#[test]
fn filter_ands_across_facets_and_ors_inside_one() {
    let db = database_with(
        0,
        vec![
            (1, wall("wall-l1", "Wall L1", LEVEL_1)),
            (2, wall("wall-l2", "Wall L2", LEVEL_2)),
            (
                3,
                element(
                    "slab-l1",
                    Some("IfcSlab"),
                    Some("Slab L1"),
                    Some(LEVEL_1),
                    None,
                    None,
                    Vec::new(),
                ),
            ),
            (
                4,
                element(
                    "slab-l2",
                    Some("IfcSlab"),
                    Some("Slab L2"),
                    Some(LEVEL_2),
                    None,
                    None,
                    Vec::new(),
                ),
            ),
        ],
    );
    let index = BimIndex::build(&db);

    let filter = BimFilter {
        categories: vec!["IfcWall".to_string(), "IfcSlab".to_string()],
        levels: vec!["Level 1".to_string()],
        ..BimFilter::default()
    };
    let rows = index.rows(&filter).expect("rows");
    assert_eq!(row_keys(&rows), keys_of(&["wall-l1", "slab-l1"]));

    // Case folding is ASCII-only and does not rewrite the stored token.
    let lower = BimFilter {
        categories: vec!["ifcwall".to_string()],
        ..BimFilter::default()
    };
    let rows = index.rows(&lower).expect("rows");
    assert_eq!(row_keys(&rows), keys_of(&["wall-l1", "wall-l2"]));
    assert!(rows
        .iter()
        .all(|row| row.category.as_deref() == Some("IfcWall")));
}

#[test]
fn property_filter_matches_numeric_bool_and_text_values() {
    let db = database_with(
        0,
        vec![
            (
                1,
                element(
                    "external",
                    Some("IfcWall"),
                    None,
                    None,
                    None,
                    None,
                    vec![property_set(
                        "Pset_WallCommon",
                        vec![("IsExternal", BimValue::Bool(true))],
                    )],
                ),
            ),
            (
                2,
                element(
                    "internal",
                    Some("IfcWall"),
                    None,
                    None,
                    None,
                    None,
                    vec![property_set(
                        "Pset_WallCommon",
                        vec![("IsExternal", BimValue::Bool(false))],
                    )],
                ),
            ),
            (
                3,
                element(
                    "rated",
                    Some("IfcWall"),
                    None,
                    None,
                    None,
                    None,
                    vec![property_set(
                        "Pset_WallCommon",
                        vec![("FireRating", BimValue::Text("Fire 2h".to_string()))],
                    )],
                ),
            ),
            (
                4,
                element(
                    "unrated",
                    Some("IfcWall"),
                    None,
                    None,
                    None,
                    None,
                    vec![property_set(
                        "Pset_WallCommon",
                        vec![("FireRating", BimValue::Text("None".to_string()))],
                    )],
                ),
            ),
        ],
    );
    let index = BimIndex::build(&db);

    let external = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_WallCommon",
            "IsExternal",
            BimPropertyOp::Eq,
            BimScalar::Bool(true),
        )),
        ..BimFilter::default()
    };
    assert_eq!(
        row_keys(&index.rows(&external).expect("rows")),
        keys_of(&["external"])
    );

    let internal = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_WallCommon",
            "IsExternal",
            BimPropertyOp::Ne,
            BimScalar::Bool(true),
        )),
        ..BimFilter::default()
    };
    assert_eq!(
        row_keys(&index.rows(&internal).expect("rows")),
        keys_of(&["internal"])
    );

    let rated = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_WallCommon",
            "FireRating",
            BimPropertyOp::Eq,
            BimScalar::text("Fire 2h"),
        )),
        ..BimFilter::default()
    };
    assert_eq!(
        row_keys(&index.rows(&rated).expect("rows")),
        keys_of(&["rated"])
    );

    let contains = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_WallCommon",
            "FireRating",
            BimPropertyOp::Contains,
            BimScalar::text("Fire"),
        )),
        ..BimFilter::default()
    };
    assert_eq!(
        row_keys(&index.rows(&contains).expect("rows")),
        keys_of(&["rated"])
    );

    // A missing property never satisfies `Ne`.
    let missing = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_WallCommon",
            "Reference",
            BimPropertyOp::Ne,
            BimScalar::text("x"),
        )),
        ..BimFilter::default()
    };
    assert!(index.rows(&missing).expect("rows").is_empty());
}

#[test]
fn a_real_is_compared_with_an_integer() {
    let db = database_with(
        0,
        vec![
            (
                1,
                element(
                    "real-two",
                    None,
                    None,
                    None,
                    None,
                    None,
                    vec![property_set("Pset_Q", vec![("Count", real(2.0))])],
                ),
            ),
            (
                2,
                element(
                    "int-two",
                    None,
                    None,
                    None,
                    None,
                    None,
                    vec![property_set(
                        "Pset_Q",
                        vec![("Count", BimValue::Integer(2))],
                    )],
                ),
            ),
            (
                3,
                element(
                    "real-fraction",
                    None,
                    None,
                    None,
                    None,
                    None,
                    vec![property_set("Pset_Q", vec![("Count", real(2.5))])],
                ),
            ),
        ],
    );
    let index = BimIndex::build(&db);

    let integer_two = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_Q",
            "Count",
            BimPropertyOp::Eq,
            BimScalar::Integer(2),
        )),
        ..BimFilter::default()
    };
    assert_eq!(
        row_keys(&index.rows(&integer_two).expect("rows")),
        keys_of(&["real-two", "int-two"])
    );

    let real_two = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_Q",
            "Count",
            BimPropertyOp::Eq,
            BimScalar::real(2.0),
        )),
        ..BimFilter::default()
    };
    assert_eq!(
        row_keys(&index.rows(&real_two).expect("rows")),
        keys_of(&["real-two", "int-two"])
    );

    let integer_three = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_Q",
            "Count",
            BimPropertyOp::Eq,
            BimScalar::Integer(3),
        )),
        ..BimFilter::default()
    };
    assert!(index.rows(&integer_three).expect("rows").is_empty());

    let not_two = BimFilter {
        property: Some(BimPropertyFilter::new(
            "Pset_Q",
            "Count",
            BimPropertyOp::Ne,
            BimScalar::Integer(2),
        )),
        ..BimFilter::default()
    };
    assert_eq!(
        row_keys(&index.rows(&not_two).expect("rows")),
        keys_of(&["real-fraction"])
    );
}

#[test]
fn level_filter_accepts_definition_id_or_display_name() {
    let db = database_with(
        0,
        vec![
            (1, wall("wall-l1", "Wall L1", LEVEL_1)),
            (2, wall("wall-l2", "Wall L2", LEVEL_2)),
        ],
    );
    let index = BimIndex::build(&db);

    let by_id = BimFilter {
        levels: vec![LEVEL_1.to_string()],
        ..BimFilter::default()
    };
    let by_name = BimFilter {
        levels: vec!["Level 1".to_string()],
        ..BimFilter::default()
    };
    let by_folded_name = BimFilter {
        levels: vec!["level 1".to_string()],
        ..BimFilter::default()
    };

    let expected = keys_of(&["wall-l1"]);
    assert_eq!(row_keys(&index.rows(&by_id).expect("rows")), expected);
    assert_eq!(row_keys(&index.rows(&by_name).expect("rows")), expected);
    assert_eq!(
        row_keys(&index.rows(&by_folded_name).expect("rows")),
        expected
    );
}

// -------------------------------------------------------------------- stats ---

#[test]
fn summary_counts_each_source_element_once() {
    let db = database_with(
        0,
        vec![
            (1, wall("wall-1", "Wall", LEVEL_1)),
            (2, wall("wall-1", "Wall", LEVEL_1)),
            (3, wall("wall-1", "Wall", LEVEL_1)),
            (
                4,
                element(
                    "slab-1",
                    Some("IfcSlab"),
                    Some("Slab"),
                    Some(LEVEL_2),
                    None,
                    None,
                    Vec::new(),
                ),
            ),
        ],
    );
    let index = BimIndex::build(&db);
    let summary = index.summary(&BimFilter::default()).expect("summary");

    assert_eq!(summary.total, 2);
    assert_eq!(
        summary.by_category,
        vec![
            BimFacetCount {
                key: "ifcslab".to_string(),
                name: "IfcSlab".to_string(),
                count: 1,
            },
            BimFacetCount {
                key: "ifcwall".to_string(),
                name: "IfcWall".to_string(),
                count: 1,
            },
        ]
    );
    assert_eq!(
        summary.by_level,
        vec![
            BimFacetCount {
                key: LEVEL_1.to_string(),
                name: "Level 1".to_string(),
                count: 1,
            },
            BimFacetCount {
                key: LEVEL_2.to_string(),
                name: "Level 2".to_string(),
                count: 1,
            },
        ]
    );

    let rows = index.rows(&BimFilter::default()).expect("rows");
    let wall = rows
        .iter()
        .find(|row| row.object == key("wall-1"))
        .expect("wall row");
    assert_eq!(wall.entity_ids, vec![EntityId(1), EntityId(2), EntityId(3)]);
}

// ---------------------------------------------------------------- lifecycle ---

#[test]
fn an_empty_database_returns_an_empty_page() {
    let db = DrawingDatabaseBuilder::new(DatabaseId(1))
        .finish()
        .expect("database");
    let service = QueryService::new();

    let page = service
        .bim_query(&db, &BimFilter::default(), request())
        .expect("page");
    assert_eq!(page.total, 0);
    assert!(page.rows.is_empty());

    let summary = service
        .bim_summary(&db, &BimFilter::default(), request())
        .expect("summary page");
    assert_eq!(summary.total, 1);
    assert_eq!(summary.rows.len(), 1);
    assert_eq!(summary.rows[0].total, 0);
    assert!(summary.rows[0].by_category.is_empty());
}

#[test]
fn the_index_is_rebuilt_when_the_revision_changes() {
    let first = database_with(0, vec![(1, wall("wall-1", "Wall", LEVEL_1))]);
    let service = QueryService::new();
    let page = service
        .bim_query(&first, &BimFilter::default(), request())
        .expect("first page");
    assert_eq!(page.total, 1);
    assert_eq!(page.request.revision, Revision(0));

    let second = database_with(
        7,
        vec![
            (1, wall("wall-1", "Wall", LEVEL_1)),
            (2, wall("wall-2", "Wall 2", LEVEL_2)),
        ],
    );
    assert_eq!(
        BimIndex::build(&second).built_at(),
        (DatabaseId(1), Revision(7))
    );

    // Same DatabaseId, new revision: the cached revision-0 index must not be
    // reused.
    let page = service
        .bim_query(&second, &BimFilter::default(), request())
        .expect("second page");
    assert_eq!(page.total, 2);
    assert_eq!(page.request.revision, Revision(7));
    assert_eq!(row_keys(&page.rows), keys_of(&["wall-1", "wall-2"]));
}

// -------------------------------------------------------------- guard rails ---

#[test]
fn a_filter_over_64_terms_is_an_explicit_error() {
    let db = database_with(0, Vec::new());
    let service = QueryService::new();

    let mut filter = BimFilter {
        categories: (0..=BIM_MAX_FILTER_TERMS)
            .map(|index| format!("category-{index}"))
            .collect(),
        ..BimFilter::default()
    };
    assert_eq!(filter.term_count(), BIM_MAX_FILTER_TERMS + 1);

    let error = service
        .bim_query(&db, &filter, request())
        .expect_err("over budget");
    match error {
        CadError::InvalidInput(detail) => {
            assert!(
                detail.starts_with(BIM_FILTER_TOO_MANY_TERMS),
                "detail must carry the stable marker: {detail}"
            );
        }
        other => panic!("expected an explicit InvalidInput, got {other:?}"),
    }

    // Exactly the budget is accepted (nothing is truncated).
    filter.categories.truncate(BIM_MAX_FILTER_TERMS);
    assert_eq!(filter.term_count(), BIM_MAX_FILTER_TERMS);
    let page = service
        .bim_query(&db, &filter, request())
        .expect("at budget");
    assert_eq!(page.total, 0);
}

#[test]
fn rows_are_deterministically_ordered() {
    let rows = vec![
        (9, wall("z9", "Beta", LEVEL_1)),
        (3, wall("a1", "Beta", LEVEL_1)),
        (
            7,
            element(
                "s3",
                Some("IfcSlab"),
                Some("A slab"),
                Some(LEVEL_1),
                None,
                None,
                Vec::new(),
            ),
        ),
        (1, wall("a2", "Beta", LEVEL_1)),
        (5, wall("k5", "Alpha", LEVEL_1)),
    ];

    let mut reversed = rows.clone();
    reversed.reverse();
    let forward = BimIndex::build(&database_with(0, rows));
    let backward = BimIndex::build(&database_with(0, reversed));

    let forward_rows = forward.rows(&BimFilter::default()).expect("rows");
    let backward_rows = backward.rows(&BimFilter::default()).expect("rows");
    assert_eq!(forward_rows, backward_rows);

    let order: Vec<(Option<&str>, Option<&str>, String)> = forward_rows
        .iter()
        .map(|row| {
            let source_id = match &row.object {
                BimKey::Source { id, .. } => id.clone(),
                BimKey::Entity(entity) => entity.0.to_string(),
            };
            (row.category.as_deref(), row.name.as_deref(), source_id)
        })
        .collect();
    assert_eq!(
        order,
        vec![
            (Some("IfcSlab"), Some("A slab"), "s3".to_string()),
            (Some("IfcWall"), Some("Alpha"), "k5".to_string()),
            (Some("IfcWall"), Some("Beta"), "a1".to_string()),
            (Some("IfcWall"), Some("Beta"), "a2".to_string()),
            (Some("IfcWall"), Some("Beta"), "z9".to_string()),
        ]
    );
}
