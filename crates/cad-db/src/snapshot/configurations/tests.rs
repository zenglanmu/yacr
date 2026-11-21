//! Strictness of the `tables/configurations.json` codec (ADR-0043 P1).

use cad_domain::param::{
    BinaryOp, Expr, ParamConfig, ParamConfigTable, ParamKind, ParamRow, ParamSeq, ParamTable,
};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry,
    SpaceId, UnitContext,
};
use serde_json::{json, Value};

use crate::drawing::DrawingDatabase;
use crate::entity::{DbEntity, DbObject};
use crate::scope::TableKind;
use crate::snapshot::{configurations, parameters, SnapshotError, TableSnapshot};
use crate::DrawingDatabaseBuilder;

fn base_table() -> ParamTable {
    let mut table = ParamTable::new();
    for seq in 1..=3u64 {
        table
            .insert(
                ParamRow::new(
                    ParamSeq(seq),
                    format!("p{seq}"),
                    ParamKind::Length,
                    Expr::num(seq as f64),
                )
                .expect("row"),
            )
            .expect("insert");
    }
    table
}

fn table_with(rows: Vec<Value>) -> TableSnapshot {
    let mut tables = TableSnapshot::default();
    tables.set(TableKind::Configurations, rows);
    tables
}

fn decode_error(rows: Vec<Value>) -> SnapshotError {
    configurations::decode_configurations(&table_with(rows), &base_table())
        .expect_err("rows must be refused")
}

fn config_row(name: &str, parent: Value, overrides: Value, suppressed: Value) -> Value {
    json!({"name": name, "parent": parent, "overrides": overrides, "suppressed": suppressed})
}

fn sample_rows() -> Vec<Value> {
    vec![
        config_row("base", Value::Null, json!({"1": {"num": 4.0}}), json!([2])),
        config_row(
            "thin",
            Value::from("base"),
            json!({"3": {"op": "add", "lhs": {"ref": 1}, "rhs": {"num": 2.0}}}),
            json!([]),
        ),
    ]
}

// ------------------------------------------------------------ round trip ---

#[test]
fn configuration_rows_round_trip_with_parent_overrides_and_suppression() {
    let base = base_table();
    let rows = sample_rows();
    let decoded =
        configurations::decode_configurations(&table_with(rows.clone()), &base).expect("decode");

    let mut expected = ParamConfigTable::new();
    let mut base_config = ParamConfig::new("base").expect("config");
    base_config
        .set_override(ParamSeq(1), Expr::num(4.0))
        .expect("override");
    base_config.suppress(ParamSeq(2));
    expected.insert(base_config).expect("row");
    let mut thin = ParamConfig::new("thin")
        .expect("config")
        .with_parent("base")
        .expect("parent");
    thin.set_override(
        ParamSeq(3),
        Expr::binary(BinaryOp::Add, Expr::reference(ParamSeq(1)), Expr::num(2.0)),
    )
    .expect("override");
    expected.insert(thin).expect("row");
    assert_eq!(decoded, expected);

    // The encoder reproduces the same rows (name order is deterministic).
    let encoded =
        configurations::encode_configurations(&db_with(base, expected.clone())).expect("encode");
    assert_eq!(encoded, rows, "round trip is a fixed point");
    assert_eq!(
        encoded[0]["parent"],
        Value::Null,
        "an absent parent is written as null, never omitted"
    );
}

fn db_with(base: ParamTable, configs: ParamConfigTable) -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
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
    })
    .expect("entity");
    b.set_parameters(base).expect("parameters");
    b.set_param_configs(configs).expect("configs");
    b.finish().expect("finish")
}

#[test]
fn configurations_round_trip_through_a_document_snapshot() {
    let base = base_table();
    let rows = sample_rows();
    let configs = configurations::decode_configurations(&table_with(rows), &base).expect("decode");
    let db = db_with(base, configs.clone());
    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(1),
        )
        .expect("snapshot");
    assert!(snapshot.has_configurations());

    let loaded = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    assert_eq!(loaded.param_configs(), &configs);
    assert_eq!(
        loaded.active_configuration(),
        None,
        "runtime state is not persisted"
    );
}

// -------------------------------------------------------------- refusals ---

#[test]
fn a_duplicate_configuration_name_is_refused() {
    let error = decode_error(vec![
        config_row("thin", Value::Null, json!({}), json!([])),
        config_row("thin", Value::Null, json!({}), json!([])),
    ]);
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_DUPLICATE_NAME),
        "{}",
        error.detail
    );
}

#[test]
fn an_unknown_parent_is_refused() {
    let error = decode_error(vec![config_row(
        "thin",
        Value::from("nope"),
        json!({}),
        json!([]),
    )]);
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_UNKNOWN_CONFIG),
        "{}",
        error.detail
    );
}

#[test]
fn a_parent_cycle_is_refused() {
    let error = decode_error(vec![
        config_row("a", Value::from("b"), json!({}), json!([])),
        config_row("b", Value::from("a"), json!({}), json!([])),
    ]);
    assert!(
        error.detail.contains(cad_domain::param::PARAM_CONFIG_CYCLE),
        "{}",
        error.detail
    );
}

#[test]
fn an_override_or_suppression_of_a_missing_sequence_is_refused() {
    let override_error = decode_error(vec![config_row(
        "thin",
        Value::Null,
        json!({"99": {"num": 1.0}}),
        json!([]),
    )]);
    assert!(
        override_error
            .detail
            .contains(cad_domain::param::PARAM_UNKNOWN_REFERENCE),
        "{}",
        override_error.detail
    );

    let suppress_error = decode_error(vec![config_row(
        "thin",
        Value::Null,
        json!({}),
        json!([99]),
    )]);
    assert!(
        suppress_error
            .detail
            .contains(cad_domain::param::PARAM_UNKNOWN_REFERENCE),
        "{}",
        suppress_error.detail
    );
}

#[test]
fn a_parent_key_is_required_but_null_is_accepted() {
    let mut missing = config_row("thin", Value::Null, json!({}), json!([]));
    missing.as_object_mut().expect("object").remove("parent");
    let error = decode_error(vec![missing]);
    assert!(
        error.detail.contains("missing required field"),
        "{}",
        error.detail
    );

    let accepted = configurations::decode_configurations(
        &table_with(vec![config_row("thin", Value::Null, json!({}), json!([]))]),
        &base_table(),
    )
    .expect("null parent is valid");
    assert!(accepted.get("thin").expect("row").parent.is_none());
}

#[test]
fn an_extra_key_is_refused() {
    let mut row = config_row("thin", Value::Null, json!({}), json!([]));
    row["owner"] = Value::from("x");
    let error = decode_error(vec![row]);
    assert!(error.detail.contains("owner"), "{}", error.detail);
}

#[test]
fn a_non_canonical_override_key_is_refused() {
    for key in ["0", "01", "+1", "1.0", "x"] {
        let mut overrides = serde_json::Map::new();
        overrides.insert(key.to_string(), json!({"num": 1.0}));
        let error = decode_error(vec![config_row(
            "thin",
            Value::Null,
            Value::Object(overrides),
            json!([]),
        )]);
        assert!(
            error
                .detail
                .contains(cad_domain::param::PARAM_INVALID_VALUE),
            "{key}: {}",
            error.detail
        );
    }
}

#[test]
fn a_duplicate_or_zero_suppression_is_refused() {
    let duplicate = decode_error(vec![config_row(
        "thin",
        Value::Null,
        json!({}),
        json!([2, 2]),
    )]);
    assert!(
        duplicate
            .detail
            .contains(cad_domain::param::PARAM_INVALID_VALUE),
        "{}",
        duplicate.detail
    );
    let zero = decode_error(vec![config_row("thin", Value::Null, json!({}), json!([0]))]);
    assert!(
        zero.detail.contains(cad_domain::param::PARAM_INVALID_VALUE),
        "{}",
        zero.detail
    );
}

// ---------------------------------------------------------------- budgets ---

#[test]
fn the_row_budget_is_enforced_on_encode_and_decode() {
    let rows: Vec<Value> = (0..=crate::PARAM_MAX_CONFIGS)
        .map(|index| config_row(&format!("c{index}"), Value::Null, json!({}), json!([])))
        .collect();
    let error = decode_error(rows);
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_BUDGET_EXCEEDED),
        "{}",
        error.detail
    );

    let mut configs = ParamConfigTable::new();
    for index in 0..=crate::PARAM_MAX_CONFIGS {
        configs
            .insert(ParamConfig::new(format!("c{index}")).expect("config"))
            .expect("row");
    }
    // `finish` refuses the table before an encode can even be attempted.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.set_param_configs(configs).expect("configs");
    let error = b.finish().expect_err("row budget refuses the build");
    assert!(
        format!("{error}").contains(cad_domain::param::PARAM_BUDGET_EXCEEDED),
        "{error}"
    );
}

#[test]
fn the_encoded_byte_budget_is_enforced_on_encode_and_decode() {
    fn tree(nodes: u32) -> Expr {
        if nodes <= 1 {
            Expr::num(1.0)
        } else {
            Expr::binary(BinaryOp::Add, tree(nodes / 2), tree(nodes - 1 - nodes / 2))
        }
    }
    let expr = tree(255);
    let mut base = ParamTable::new();
    for seq in 1..=2000u64 {
        base.insert(
            ParamRow::new(
                ParamSeq(seq),
                format!("p{seq}"),
                ParamKind::Length,
                Expr::num(1.0),
            )
            .expect("row"),
        )
        .expect("insert");
    }
    let mut overrides = serde_json::Map::new();
    for seq in 1..=2000u64 {
        overrides.insert(seq.to_string(), parameters::encode_expr(&expr));
    }
    let rows = vec![config_row(
        "huge",
        Value::Null,
        Value::Object(overrides),
        json!([]),
    )];
    let error = configurations::decode_configurations(&table_with(rows.clone()), &base)
        .expect_err("over-budget table is refused on decode");
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_BUDGET_EXCEEDED),
        "{}",
        error.detail
    );

    let mut config = ParamConfig::new("huge").expect("config");
    for seq in 1..=2000u64 {
        config
            .set_override(ParamSeq(seq), expr.clone())
            .expect("override");
    }
    let mut configs = ParamConfigTable::new();
    configs.insert(config).expect("row");
    let db = db_with(base, configs);
    let error = configurations::encode_configurations(&db)
        .expect_err("over-budget table is refused on encode");
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_BUDGET_EXCEEDED),
        "{}",
        error.detail
    );
}

#[test]
fn a_configuration_naming_a_missing_base_row_is_refused_on_encode() {
    let mut db = db_with(base_table(), ParamConfigTable::new());
    // Simulate a hand-built database (or a stale row) whose overlay names a
    // parameter the base table no longer has.
    let mut config = ParamConfig::new("stale").expect("config");
    config
        .set_override(ParamSeq(99), Expr::num(1.0))
        .expect("override");
    db.param_configs.insert(config).expect("config");
    let error = configurations::encode_configurations(&db)
        .expect_err("the writer must refuse an archive the reader rejects");
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_UNKNOWN_REFERENCE),
        "{}",
        error.detail
    );
    let error = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(1),
        )
        .expect_err("the snapshot writer refuses too");
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_UNKNOWN_REFERENCE),
        "{}",
        error.detail
    );
}
