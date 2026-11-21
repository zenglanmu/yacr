//! Strictness of the `tables/features.json` codec (ADR-0043 P2).

use cad_domain::param::{Expr, ParamBindings, ParamKind, ParamRow, ParamSeq, ParamTable};
use cad_domain::ymodel::PersistentIdOf;
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry,
    SpaceId, TransactionId, UnitContext,
};
use serde_json::{json, Value};

use crate::drawing::DrawingDatabase;
use crate::entity::{DbEntity, DbObject};
use crate::feature::{FeatureDefinition, FeatureDefinitions, FeatureOperation};
use crate::scope::TableKind;
use crate::snapshot::{features, SnapshotError, TableSnapshot};
use crate::DrawingDatabaseBuilder;

fn circle(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbCircle".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Circle {
            center: Point3::default(),
            normal: Point3::default(),
            radius: 10.0,
        },
        draw_order: 0,
    }
}

fn line(id: u128) -> DbEntity {
    let mut entity = circle(id);
    entity.geometry = SemanticGeometry::Line {
        start: Point3::default(),
        end: Point3 {
            x: 1.0,
            ..Point3::default()
        },
    };
    entity
}

fn builder() -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    b
}

fn base_parameters() -> ParamTable {
    let mut table = ParamTable::new();
    table
        .insert(ParamRow::new(ParamSeq(7), "r", ParamKind::Length, Expr::num(5.0)).expect("row"))
        .expect("insert");
    table
}

fn depth_bindings() -> ParamBindings {
    ParamBindings::from_entries([("radius".to_string(), Expr::reference(ParamSeq(7)))])
        .expect("bindings")
}

fn sample_feature() -> FeatureDefinition {
    FeatureDefinition {
        id: EntityId(1),
        order: 0,
        operation: FeatureOperation::Extrude,
        inputs: vec![EntityId(2)],
        parameters: depth_bindings(),
    }
}

fn sample_db() -> DrawingDatabase {
    let mut b = builder();
    b.insert_entity(circle(1)).expect("feature entity");
    b.insert_entity(line(2)).expect("input entity");
    b.set_parameters(base_parameters()).expect("parameters");
    let mut table = FeatureDefinitions::new();
    table.insert(sample_feature()).expect("feature");
    b.set_features(table).expect("features");
    b.finish().expect("finish")
}

fn table_with(rows: Vec<Value>) -> TableSnapshot {
    let mut tables = TableSnapshot::default();
    tables.set(TableKind::Features, rows);
    tables
}

fn decode_error(rows: Vec<Value>) -> SnapshotError {
    features::decode_features(&table_with(rows)).expect_err("rows must be refused")
}

fn row(id: u128, order: u32, operation: &str) -> Value {
    json!({
        "id": {"kind": "entity", "entityId": EntityId(id).persistent().raw()},
        "order": order,
        "operation": operation,
        "inputs": [],
        "parameters": {},
    })
}

#[test]
fn feature_rows_round_trip_in_evaluation_order() {
    let db = sample_db();
    let rows = features::encode_features(&db).expect("encode");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["operation"], json!("extrude"));
    let decoded = features::decode_features(&table_with(rows)).expect("decode");
    assert_eq!(decoded, *db.features());
}

#[test]
fn every_operation_token_round_trips() {
    for operation in [
        FeatureOperation::Extrude,
        FeatureOperation::Revolve,
        FeatureOperation::Loft,
        FeatureOperation::Sweep,
        FeatureOperation::BooleanUnion,
        FeatureOperation::BooleanSubtract,
        FeatureOperation::BooleanIntersect,
        FeatureOperation::Fillet,
        FeatureOperation::Shell,
        FeatureOperation::Hole,
        FeatureOperation::Pattern,
    ] {
        assert_eq!(
            FeatureOperation::parse(operation.as_token()),
            Some(operation)
        );
        let decoded = features::decode_features(&table_with(vec![row(1, 0, operation.as_token())]))
            .expect("decode");
        assert_eq!(decoded.get(EntityId(1)).expect("row").operation, operation);
    }
}

#[test]
fn an_unknown_operation_is_refused() {
    let error = decode_error(vec![row(1, 0, "warp")]);
    assert!(
        error.detail.contains("unknown feature operation"),
        "{}",
        error.detail
    );
}

#[test]
fn duplicate_ids_and_orders_are_refused() {
    let duplicate_id = decode_error(vec![row(1, 0, "extrude"), row(1, 1, "revolve")]);
    assert!(
        duplicate_id.detail.contains("duplicate"),
        "{}",
        duplicate_id.detail
    );
    let duplicate_order = decode_error(vec![row(1, 0, "extrude"), row(2, 0, "revolve")]);
    assert!(
        duplicate_order.detail.contains("duplicate"),
        "{}",
        duplicate_order.detail
    );
}

#[test]
fn a_row_must_carry_every_key_and_no_extra_key() {
    for missing in ["id", "order", "operation", "inputs", "parameters"] {
        let mut bad = row(1, 0, "extrude");
        bad.as_object_mut().expect("object").remove(missing);
        let error = decode_error(vec![bad]);
        assert!(
            error.detail.contains("missing required field"),
            "{missing}: {}",
            error.detail
        );
    }
    let mut extra = row(1, 0, "extrude");
    extra["colour"] = Value::from(1);
    let error = decode_error(vec![extra]);
    assert!(error.detail.contains("colour"), "{}", error.detail);
}

#[test]
fn a_non_entity_id_is_refused() {
    let mut bad = row(1, 0, "extrude");
    bad["id"]["entityId"] = Value::from(1);
    let error = decode_error(vec![bad]);
    assert!(
        error.detail.contains("not a persistent id") || error.detail.contains("entity-family"),
        "{}",
        error.detail
    );
}

#[test]
fn the_row_budget_is_enforced() {
    let rows: Vec<Value> = (0..=crate::FEATURE_MAX_DEFINITIONS)
        .map(|index| row(index as u128 + 1, index as u32, "extrude"))
        .collect();
    let error = decode_error(rows);
    assert!(
        error.detail.contains("feature.budget_exceeded"),
        "{}",
        error.detail
    );

    // The encode side cannot hold an over-budget table: `insert` refuses it.
    let mut table = FeatureDefinitions::new();
    for index in 0..crate::FEATURE_MAX_DEFINITIONS {
        table
            .insert(FeatureDefinition {
                id: EntityId(index as u128 + 1),
                order: index as u32,
                operation: FeatureOperation::Extrude,
                inputs: Vec::new(),
                parameters: ParamBindings::new(),
            })
            .expect("row");
    }
    let error = table
        .insert(FeatureDefinition {
            id: EntityId(10_000),
            order: 10_000,
            operation: FeatureOperation::Extrude,
            inputs: Vec::new(),
            parameters: ParamBindings::new(),
        })
        .expect_err("over-budget table");
    assert!(
        format!("{error}").contains("feature.budget_exceeded"),
        "{error}"
    );
}

#[test]
fn the_encoded_byte_budget_is_enforced() {
    fn tree(nodes: u32) -> Expr {
        if nodes <= 1 {
            Expr::num(1.0)
        } else {
            Expr::binary(
                cad_domain::param::BinaryOp::Add,
                tree(nodes / 2),
                tree(nodes - 1 - nodes / 2),
            )
        }
    }
    let expr = tree(255);
    let mut b = builder();
    b.set_parameters(base_parameters()).expect("parameters");
    let mut table = FeatureDefinitions::new();
    for index in 0..2000u64 {
        b.insert_entity(circle(index as u128 + 1)).expect("entity");
        table
            .insert(FeatureDefinition {
                id: EntityId(index as u128 + 1),
                order: index as u32,
                operation: FeatureOperation::Extrude,
                inputs: Vec::new(),
                parameters: ParamBindings::from_entries([("radius".to_string(), expr.clone())])
                    .expect("bindings"),
            })
            .expect("row");
    }
    b.set_features(table).expect("features");
    let db = b.finish().expect("finish");
    let error = features::encode_features(&db).expect_err("over-budget table");
    assert!(
        error.detail.contains("feature.budget_exceeded"),
        "{}",
        error.detail
    );
}

#[test]
fn evaluate_feature_refuses_every_operation_without_a_revision_bump() {
    let mut b = builder();
    b.insert_entity(circle(1)).expect("entity");
    b.insert_entity(circle(2)).expect("entity");
    b.set_parameters(base_parameters()).expect("parameters");
    let mut table = FeatureDefinitions::new();
    table
        .insert(FeatureDefinition {
            id: EntityId(1),
            order: 0,
            operation: FeatureOperation::Extrude,
            inputs: Vec::new(),
            parameters: ParamBindings::new(),
        })
        .expect("row");
    table
        .insert(FeatureDefinition {
            id: EntityId(2),
            order: 1,
            operation: FeatureOperation::Fillet,
            inputs: Vec::new(),
            parameters: ParamBindings::new(),
        })
        .expect("row");
    b.set_features(table).expect("features");
    let mut db = b.finish().expect("finish");
    let before = db.revision();
    for id in [EntityId(1), EntityId(2)] {
        let error = db
            .evaluate_feature(id, TransactionId(1))
            .expect_err("P2 has no evaluator");
        assert!(
            format!("{error}").contains(cad_domain::relation::REL_NOT_IMPLEMENTED),
            "{error}"
        );
        assert!(format!("{error}").contains("has no evaluator"), "{error}");
    }
    assert_eq!(db.revision(), before, "a refusal is not a revision");
}

#[test]
fn the_builder_refuses_missing_elements_inputs_and_unknown_slots() {
    // Missing feature element.
    let mut b = builder();
    b.insert_entity(circle(1)).expect("entity");
    b.set_parameters(base_parameters()).expect("parameters");
    let mut table = FeatureDefinitions::new();
    table
        .insert(FeatureDefinition {
            id: EntityId(99),
            order: 0,
            operation: FeatureOperation::Extrude,
            inputs: Vec::new(),
            parameters: ParamBindings::new(),
        })
        .expect("row");
    b.set_features(table).expect("features");
    let error = b.finish().expect_err("missing feature element");
    assert!(
        format!("{error}").contains("feature.target_missing"),
        "{error}"
    );

    // Missing input.
    let mut b = builder();
    b.insert_entity(circle(1)).expect("entity");
    let mut table = FeatureDefinitions::new();
    table
        .insert(FeatureDefinition {
            id: EntityId(1),
            order: 0,
            operation: FeatureOperation::Extrude,
            inputs: vec![EntityId(99)],
            parameters: ParamBindings::new(),
        })
        .expect("row");
    b.set_features(table).expect("features");
    let error = b.finish().expect_err("missing input");
    assert!(
        format!("{error}").contains("feature.input_missing"),
        "{error}"
    );

    // Self input is refused by the table constructor.
    let mut table = FeatureDefinitions::new();
    let error = table
        .insert(FeatureDefinition {
            id: EntityId(1),
            order: 0,
            operation: FeatureOperation::Extrude,
            inputs: vec![EntityId(1)],
            parameters: ParamBindings::new(),
        })
        .expect_err("self input");
    assert!(format!("{error}").contains("feature.self_input"), "{error}");

    // Unknown slot on the feature's own element.
    let mut b = builder();
    b.insert_entity(circle(1)).expect("entity");
    b.set_parameters(base_parameters()).expect("parameters");
    let mut table = FeatureDefinitions::new();
    table
        .insert(FeatureDefinition {
            id: EntityId(1),
            order: 0,
            operation: FeatureOperation::Extrude,
            inputs: Vec::new(),
            parameters: ParamBindings::from_entries([(
                "diameter".to_string(),
                Expr::reference(ParamSeq(7)),
            )])
            .expect("bindings"),
        })
        .expect("row");
    b.set_features(table).expect("features");
    let error = b.finish().expect_err("unknown slot");
    assert!(
        format!("{error}").contains(cad_domain::param::PARAM_UNKNOWN_SLOT),
        "{error}"
    );
}

#[test]
fn features_round_trip_through_a_document_snapshot() {
    let db = sample_db();
    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(1),
        )
        .expect("snapshot");
    assert!(snapshot.has_features());
    let loaded = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    assert_eq!(loaded.features(), db.features());
}

#[test]
fn a_dangling_feature_row_is_refused_on_encode() {
    let mut db = sample_db();
    // A hand-built row the builder gate would refuse: the save path must too.
    db.features
        .insert(FeatureDefinition {
            id: EntityId(99),
            order: 5,
            operation: FeatureOperation::Extrude,
            inputs: Vec::new(),
            parameters: ParamBindings::new(),
        })
        .expect("row");
    let error = features::encode_features(&db).expect_err("dangling feature entity");
    assert!(
        error.detail.contains("feature.target_missing"),
        "{}",
        error.detail
    );
}
