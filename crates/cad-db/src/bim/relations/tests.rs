//! Relation side-table contracts (ADR-0042 D3 tail / D12).
//!
//! What must hold:
//!
//! 1. an edge round-trips through the codec and the snapshot, endpoints included;
//! 2. an empty table is not written at all;
//! 3. a duplicate edge is a refusal, not a silent merge;
//! 4. every budget is a refusal, never a truncation;
//! 5. unknown kind tokens are a malformed document, not a new kind.

use std::collections::BTreeMap;

use cad_domain::*;

use crate::{
    BimKey, BimRelation, BimRelationKind, BimRelations, DrawingDatabase, DrawingDatabaseBuilder,
    TableKind, BIM_BUDGET_EXCEEDED, BIM_INVALID, BIM_MAX_RELATIONS,
};

fn source_key(connector: &str, document: &str, id: &str) -> BimKey {
    BimKey::Source {
        connector: connector.to_string(),
        document_id: document.to_string(),
        id: id.to_string(),
    }
}

fn relation(kind: BimRelationKind, parent: &str, child: &str) -> BimRelation {
    BimRelation {
        kind,
        parent: source_key("ifc", "doc-1", parent),
        child: source_key("ifc", "doc-1", child),
        source: None,
        label: None,
    }
}

fn relations() -> BimRelations {
    let mut relations = BimRelations::new();
    relations
        .insert(BimRelation {
            kind: BimRelationKind::Aggregates,
            parent: source_key("ifc", "doc-1", "P1"),
            child: source_key("ifc", "doc-1", "C1"),
            source: Some(source_key("ifc", "doc-1", "R1")),
            label: Some("Level 1 contains Wall 1".to_string()),
        })
        .expect("aggregates");
    relations
        .insert(BimRelation {
            kind: BimRelationKind::Other("rvt-assembly".to_string()),
            parent: source_key("rvt", "doc-2", "A"),
            child: BimKey::Entity(EntityId(7)),
            source: None,
            label: None,
        })
        .expect("other");
    relations
}

fn database_with(relations: BimRelations) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(crate::Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder.set_bim_relations(relations).expect("relations");
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
fn duplicate_relation_is_rejected() {
    let mut set = BimRelations::new();
    set.insert(relation(BimRelationKind::Contains, "P", "C"))
        .expect("first edge");
    // Same kind and endpoints, even with a different label: one edge, two
    // meanings is exactly the ambiguity the map key refuses.
    let mut duplicate = relation(BimRelationKind::Contains, "P", "C");
    duplicate.label = Some("a second label".to_string());
    let error = set.insert(duplicate).expect_err("duplicate edge");
    assert_eq!(error.marker, BIM_INVALID);
    assert!(error.detail.contains("duplicate relation"), "{error}");
    assert_eq!(set.len(), 1, "the refused row did not land");
}

#[test]
fn relations_budget_excess_is_an_explicit_error() {
    // Build the over-budget set directly: `insert` would refuse nothing here,
    // but 100k inserts make the test slow for no extra coverage.
    let mut rows = BTreeMap::new();
    for index in 0..=BIM_MAX_RELATIONS {
        let row = relation(BimRelationKind::Contains, "P", &format!("C{index}"));
        rows.insert(
            (row.kind.clone(), row.parent.clone(), row.child.clone()),
            row,
        );
    }
    let relations = BimRelations::from_rows(rows);
    let error = crate::bim::validate_relations(&relations).expect_err("over budget");
    assert_eq!(error.marker, BIM_BUDGET_EXCEEDED);
    assert!(
        error.detail.contains(&BIM_MAX_RELATIONS.to_string()),
        "{}",
        error.detail
    );
}

#[test]
fn unknown_relation_kind_token_is_not_parsed() {
    for token in ["mystery", "Other", "other:", "other:   ", ""] {
        assert_eq!(
            BimRelationKind::parse(token),
            None,
            "{token:?} must not parse"
        );
    }
    assert_eq!(
        BimRelationKind::parse("other:rvt-assembly"),
        Some(BimRelationKind::Other("rvt-assembly".to_string()))
    );
    assert_eq!(
        BimRelationKind::parse("aggregates"),
        Some(BimRelationKind::Aggregates)
    );
}

#[test]
fn relation_shape_rejects_empty_endpoint_strings() {
    let mut set = BimRelations::new();
    let error = set
        .insert(BimRelation {
            kind: BimRelationKind::Contains,
            parent: source_key("", "doc-1", "P"),
            child: source_key("ifc", "doc-1", "C"),
            source: None,
            label: None,
        })
        .expect_err("empty connector");
    assert_eq!(error.marker, BIM_INVALID);
}

#[test]
fn an_empty_relations_table_is_not_written() {
    let plain = database_with(BimRelations::new());
    let snapshot = snapshot_of(&plain);
    assert!(
        snapshot.tables.get(TableKind::BimRelations).is_none(),
        "an empty relation table must not create an archive entry"
    );

    let with_rows = database_with(relations());
    let snapshot = snapshot_of(&with_rows);
    assert_eq!(
        snapshot
            .tables
            .get(TableKind::BimRelations)
            .expect("the table is written")
            .len(),
        2
    );
}

#[test]
fn relation_rows_round_trip_through_the_codec() {
    use crate::snapshot::{relations as codec, TableSnapshot};

    let db = database_with(relations());
    let mut tables = TableSnapshot::default();
    tables.set(
        TableKind::BimRelations,
        codec::encode_relations(&db).expect("encode"),
    );
    let decoded = codec::decode_relations(&tables).expect("decode");
    assert_eq!(decoded, relations());

    // The snapshot -> database -> snapshot path is a fixed point for edges too.
    let first = snapshot_of(&db);
    let loaded = DrawingDatabase::from_snapshot(&first).expect("hydrate");
    assert_eq!(loaded.bim_relations(), &relations());
    let second = snapshot_of(&loaded);
    assert_eq!(first, second);
}
