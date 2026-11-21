//! Strictness of the `tables/relations.json` codec (ADR-0043 P3).

use cad_domain::relation::{InContextState, Propagation, RelationReference, REL_STATE_TRANSITION};
use cad_domain::ymodel::PersistentIdOf;
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry,
    SpaceId, TransactionId, UnitContext,
};
use serde_json::{json, Value};

use crate::drawing::DrawingDatabase;
use crate::entity::{DbEntity, DbObject};
use crate::relation::{InContextRelation, InContextRelations};
use crate::scope::TableKind;
use crate::snapshot::{relations_in_context, SnapshotError, TableSnapshot};
use crate::DrawingDatabaseBuilder;

fn line(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: Point3::default(),
            end: Point3 {
                x: 1.0,
                ..Point3::default()
            },
        },
        draw_order: 0,
    }
}

fn builder() -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    for id in 1..=3u128 {
        b.insert_entity(line(id)).expect("entity");
    }
    b
}

fn sample_relation() -> InContextRelation {
    InContextRelation {
        source: EntityId(1),
        target: EntityId(2),
        reference: RelationReference::Face {
            key: "f1".to_string(),
        },
        state: InContextState::InContext,
        label: Some("Wall/Door".to_string()),
    }
}

fn sample_db() -> DrawingDatabase {
    let mut b = builder();
    let mut table = InContextRelations::new();
    table.insert(sample_relation()).expect("relation");
    table
        .insert(InContextRelation {
            source: EntityId(2),
            target: EntityId(3),
            reference: RelationReference::Vertex {
                key: "v1".to_string(),
            },
            state: InContextState::OutOfContext,
            label: None,
        })
        .expect("relation");
    b.set_relations(table).expect("relations");
    b.finish().expect("finish")
}

fn table_with(rows: Vec<Value>) -> TableSnapshot {
    let mut tables = TableSnapshot::default();
    tables.set(TableKind::Relations, rows);
    tables
}

fn decode_error(rows: Vec<Value>) -> SnapshotError {
    relations_in_context::decode_relations_in_context(&table_with(rows))
        .expect_err("rows must be refused")
}

fn row(source: u128, target: u128, kind: &str, key: &str, state: &str, label: Value) -> Value {
    json!({
        "source": {"entityId": EntityId(source).persistent().raw()},
        "target": {"entityId": EntityId(target).persistent().raw()},
        "reference": {"kind": kind, "key": key},
        "state": state,
        "label": label,
    })
}

#[test]
fn relation_rows_round_trip_with_states_and_null_labels() {
    let db = sample_db();
    let rows = relations_in_context::encode_relations_in_context(&db).expect("encode");
    assert_eq!(rows.len(), 2);
    let decoded =
        relations_in_context::decode_relations_in_context(&table_with(rows)).expect("decode");
    assert_eq!(&decoded, db.relations());
}

#[test]
fn duplicate_triples_are_refused() {
    let row = row(1, 2, "face", "f1", "in_context", Value::Null);
    let error = decode_error(vec![row.clone(), row]);
    assert!(error.detail.contains("rel.duplicate"), "{}", error.detail);
}

#[test]
fn unknown_reference_kind_or_state_is_refused() {
    let bad_kind = decode_error(vec![row(1, 2, "solid", "x", "in_context", Value::Null)]);
    assert!(
        bad_kind.detail.contains("unknown reference kind"),
        "{}",
        bad_kind.detail
    );
    let bad_state = decode_error(vec![row(1, 2, "face", "f1", "elsewhere", Value::Null)]);
    assert!(
        bad_state.detail.contains("unknown in-context state"),
        "{}",
        bad_state.detail
    );
}

#[test]
fn a_reference_key_must_be_non_empty_and_bounded() {
    let empty = decode_error(vec![row(1, 2, "face", "  ", "in_context", Value::Null)]);
    assert!(
        empty.detail.contains("must not be empty"),
        "{}",
        empty.detail
    );

    let long = "x".repeat(crate::REL_MAX_REFERENCE_KEY_BYTES + 1);
    let error = decode_error(vec![row(1, 2, "face", &long, "in_context", Value::Null)]);
    assert!(
        error.detail.contains("rel.budget_exceeded"),
        "{}",
        error.detail
    );
}

#[test]
fn a_row_must_carry_every_key_and_no_extra_key() {
    for missing in ["source", "target", "reference", "state", "label"] {
        let mut bad = row(1, 2, "face", "f1", "in_context", Value::Null);
        bad.as_object_mut().expect("object").remove(missing);
        let error = decode_error(vec![bad]);
        assert!(
            error.detail.contains("missing required field"),
            "{missing}: {}",
            error.detail
        );
    }
    let mut extra = row(1, 2, "face", "f1", "in_context", Value::Null);
    extra["owner"] = Value::from("x");
    let error = decode_error(vec![extra]);
    assert!(error.detail.contains("owner"), "{}", error.detail);
}

#[test]
fn the_row_budget_is_enforced() {
    let rows: Vec<Value> = (0..=crate::REL_MAX_RELATIONS)
        .map(|index| {
            row(
                1,
                2,
                "face",
                &format!("k{index}"),
                "in_context",
                Value::Null,
            )
        })
        .collect();
    let error = decode_error(rows);
    assert!(
        error.detail.contains("rel.budget_exceeded"),
        "{}",
        error.detail
    );

    // The encode side cannot hold an over-budget table: `insert` refuses it.
    let mut table = InContextRelations::new();
    for index in 0..crate::REL_MAX_RELATIONS {
        table
            .insert(InContextRelation {
                source: EntityId(1),
                target: EntityId(2),
                reference: RelationReference::Face {
                    key: format!("k{index}"),
                },
                state: InContextState::InContext,
                label: None,
            })
            .expect("row");
    }
    let error = table
        .insert(InContextRelation {
            source: EntityId(1),
            target: EntityId(2),
            reference: RelationReference::Face {
                key: "overflow".to_string(),
            },
            state: InContextState::InContext,
            label: None,
        })
        .expect_err("over-budget table");
    assert!(
        format!("{error}").contains("rel.budget_exceeded"),
        "{error}"
    );
}

#[test]
fn the_encoded_byte_budget_is_enforced_on_decode() {
    let key = "x".repeat(crate::REL_MAX_REFERENCE_KEY_BYTES);
    let rows: Vec<Value> = (0..9000)
        .map(|index| {
            row(
                1,
                2,
                "face",
                &format!("{key}{index}"),
                "in_context",
                Value::Null,
            )
        })
        .collect();
    let error = decode_error(rows);
    assert!(
        error.detail.contains("rel.budget_exceeded"),
        "{}",
        error.detail
    );
}

#[test]
fn mutation_state_transitions_and_revision_scope() {
    let mut db = sample_db();
    let reference = RelationReference::Face {
        key: "f1".to_string(),
    };
    let before = db.revision();
    let changes = db
        .update_relation_state(
            EntityId(1),
            EntityId(2),
            &reference,
            InContextState::Locked,
            "lock",
            TransactionId(1),
        )
        .expect("legal transition");
    assert_eq!(db.revision(), Revision(before.0 + 1), "one revision");
    assert_eq!(changes.after, db.revision());
    assert!(
        changes.changes.is_empty(),
        "the table row is not an entity change"
    );
    assert!(
        changes.scope.tables.contains(&TableKind::Relations),
        "the relation table is dirty"
    );
    assert!(!changes.scope.header);
    assert_eq!(
        db.relations()
            .get(EntityId(1), EntityId(2), &reference)
            .expect("row")
            .state,
        InContextState::Locked
    );

    // Locked cannot become Broken.
    let error = db
        .update_relation_state(
            EntityId(1),
            EntityId(2),
            &reference,
            InContextState::Broken,
            "break",
            TransactionId(2),
        )
        .expect_err("illegal transition");
    assert!(format!("{error}").contains(REL_STATE_TRANSITION), "{error}");
    assert_eq!(
        db.revision(),
        Revision(before.0 + 1),
        "refusal is not a revision"
    );

    // Dangling is terminal, and Locked must first return to a movable state.
    db.update_relation_state(
        EntityId(1),
        EntityId(2),
        &reference,
        InContextState::InContext,
        "unlock",
        TransactionId(3),
    )
    .expect("locked -> in_context");
    db.update_relation_state(
        EntityId(1),
        EntityId(2),
        &reference,
        InContextState::Dangling,
        "dangle",
        TransactionId(4),
    )
    .expect("in_context -> dangling");
    let error = db
        .update_relation_state(
            EntityId(1),
            EntityId(2),
            &reference,
            InContextState::InContext,
            "revive",
            TransactionId(5),
        )
        .expect_err("dangling is terminal");
    assert!(format!("{error}").contains(REL_STATE_TRANSITION), "{error}");

    // An identical state is a no-op.
    let current = db.revision();
    let noop = db
        .update_relation_state(
            EntityId(1),
            EntityId(2),
            &reference,
            InContextState::Dangling,
            "noop",
            TransactionId(6),
        )
        .expect("same state");
    assert_eq!(db.revision(), current);
    assert!(noop.scope.is_empty());
}

#[test]
fn the_builder_refuses_missing_endpoints() {
    let mut b = builder();
    let mut table = InContextRelations::new();
    table
        .insert(InContextRelation {
            source: EntityId(1),
            target: EntityId(99),
            reference: RelationReference::Edge {
                key: "e1".to_string(),
            },
            state: InContextState::InContext,
            label: None,
        })
        .expect("row");
    b.set_relations(table).expect("relations");
    let error = b.finish().expect_err("missing endpoint");
    assert!(
        format!("{error}").contains("rel.endpoint_missing"),
        "{error}"
    );
}

#[test]
fn a_self_edge_is_refused() {
    let mut table = InContextRelations::new();
    let error = table
        .insert(InContextRelation {
            source: EntityId(1),
            target: EntityId(1),
            reference: RelationReference::Face {
                key: "f1".to_string(),
            },
            state: InContextState::InContext,
            label: None,
        })
        .expect_err("self edge");
    assert!(format!("{error}").contains("rel.self_edge"), "{error}");
}

#[test]
fn relation_propagation_exposes_the_domain_policy() {
    assert_eq!(
        DrawingDatabase::relation_propagation("voids", "IfcWall", "IfcDoor"),
        Propagation::NoRule
    );
    assert_eq!(
        DrawingDatabase::relation_propagation("fills", "IfcOpeningElement", "IfcDoor"),
        Propagation::NoRule
    );
    assert_eq!(
        DrawingDatabase::relation_propagation("contains", "IfcBuildingStorey", "IfcWall"),
        Propagation::Never
    );
}

#[test]
fn relations_round_trip_through_a_document_snapshot() {
    let db = sample_db();
    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(1),
        )
        .expect("snapshot");
    assert!(snapshot.has_relations());
    let loaded = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    assert_eq!(loaded.relations(), db.relations());
}

#[test]
fn a_dangling_relation_row_is_refused_on_encode() {
    let mut db = sample_db();
    db.relations
        .insert(InContextRelation {
            source: EntityId(1),
            target: EntityId(99),
            reference: RelationReference::Edge {
                key: "e9".to_string(),
            },
            state: InContextState::InContext,
            label: None,
        })
        .expect("row");
    let error = relations_in_context::encode_relations_in_context(&db)
        .expect_err("dangling relation endpoint");
    assert!(
        error.detail.contains("rel.endpoint_missing"),
        "{}",
        error.detail
    );
}
