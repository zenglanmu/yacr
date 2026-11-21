//! Strictness of the `tables/parameters.json` codec and the element `params`
//! block (ADR-0043 D4/D7).

use cad_domain::param::{
    Expr, ParamBindings, ParamKind, ParamRow, ParamSeq, ParamTable, ParamUnit,
};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, LayoutId, ObjectId, Point3, Revision,
    SemanticGeometry, SpaceId, UnitContext,
};
use serde_json::{json, Value};

use crate::drawing::DrawingDatabase;
use crate::entity::{DbEntity, DbObject};
use crate::scope::TableKind;
use crate::snapshot::{elements, parameters, SnapshotError, TableSnapshot};
use crate::DrawingDatabaseBuilder;

fn table_with(rows: Vec<Value>) -> TableSnapshot {
    let mut tables = TableSnapshot::default();
    tables.set(TableKind::Parameters, rows);
    tables
}

fn row(seq: u64, name: &str, kind: &str, unit: Option<&str>, expr: Value) -> Value {
    let mut row = json!({"seq": seq, "name": name, "kind": kind, "expr": expr});
    if let Some(unit) = unit {
        row["unit"] = Value::from(unit);
    }
    row
}

fn decode_error(rows: Vec<Value>) -> SnapshotError {
    parameters::decode_parameters(&table_with(rows)).expect_err("rows must be refused")
}

fn domain_row(seq: u64, name: &str, kind: ParamKind, expr: Expr) -> ParamRow {
    ParamRow::new(ParamSeq(seq), name, kind, expr).expect("row")
}

#[test]
fn p1_expressions_round_trip_and_reject_unknown_shapes() {
    let expr = Expr::if_then(
        Expr::binary(
            cad_domain::param::BinaryOp::Sub,
            Expr::num(2.0),
            Expr::num(1.0),
        ),
        Expr::unary(
            cad_domain::param::UnaryOp::Sqrt,
            Expr::pow(Expr::num(9.0), Expr::num(2.0)),
        ),
        Expr::negated(Expr::num(4.0)),
    );
    let encoded = parameters::encode_expr(&expr);
    let decoded = parameters::decode_expr(&encoded, "test".to_string()).expect("round trip");
    assert_eq!(decoded, expr);
    assert!(expr.is_p1());

    for bad in [
        json!({"fn": "nope", "arg": {"num": 1.0}}),
        json!({"num": 1.0, "exp": {"num": 2.0}}),
        json!({"pow": {"num": 1.0}}),
        json!({"fn": "sqrt"}),
        json!({"arg": {"num": 1.0}}),
        json!({"exp": {"num": 1.0}}),
        json!({"if": {"cond": {"num": 1.0}, "then": {"num": 2.0}}}),
        json!({"neg": {"num": 1.0}, "num": 2.0}),
    ] {
        assert!(
            parameters::decode_expr(&bad, "test".to_string()).is_err(),
            "must refuse {bad}"
        );
    }
}

// ------------------------------------------------------------ table codec ---

#[test]
fn parameter_rows_round_trip_with_and_without_a_unit() {
    let mut expected = ParamTable::new();
    expected
        .insert(
            domain_row(7, "板厚", ParamKind::Length, Expr::num(12.5))
                .with_unit(ParamUnit::Millimetre),
        )
        .expect("row");
    // A number row carries no unit; decoding must come back as `None`, not as
    // the canonical `mm` the length row has.
    expected
        .insert(domain_row(
            8,
            "孔径",
            ParamKind::Number,
            Expr::num(std::f64::consts::PI),
        ))
        .expect("row");
    expected
        .insert(domain_row(
            9,
            "倾角",
            ParamKind::Length,
            Expr::binary(
                cad_domain::param::BinaryOp::Div,
                Expr::reference(ParamSeq(7)),
                Expr::num(2.0),
            ),
        ))
        .expect("row");

    let rows =
        parameters::encode_parameters(&db_with(expected.clone())).expect("encode parameters");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0]["unit"],
        json!("mm"),
        "the length row keeps its unit"
    );
    assert!(
        rows[1].get("unit").is_none(),
        "a unitless row omits the key instead of writing null"
    );

    let decoded = parameters::decode_parameters(&table_with(rows)).expect("decode");
    assert_eq!(decoded, expected);
    assert_eq!(
        decoded.get(ParamSeq(8)).expect("row 8").unit,
        None,
        "absent unit decodes as None"
    );
    assert_eq!(
        decoded.get(ParamSeq(9)).expect("row 9").unit,
        None,
        "an explicit canonical unit is optional even for a dimensioned kind"
    );
}

#[test]
fn a_null_unit_is_refused() {
    let mut bad = row(7, "板厚", "length", None, json!({"num": 1.0}));
    bad["unit"] = Value::Null;
    let error = decode_error(vec![bad]);
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(error.detail.contains("unit"), "{}", error.detail);
}

#[test]
fn a_row_must_carry_every_required_key_and_no_extra_key() {
    for missing in ["seq", "name", "kind", "expr"] {
        let mut bad = row(7, "板厚", "length", None, json!({"num": 1.0}));
        bad.as_object_mut().expect("object").remove(missing);
        let error = decode_error(vec![bad]);
        assert!(
            error.detail.contains("missing required field"),
            "{missing}: {}",
            error.detail
        );
    }
    let mut extra = row(7, "板厚", "length", None, json!({"num": 1.0}));
    extra["colour"] = Value::from(1);
    let error = decode_error(vec![extra]);
    assert!(error.detail.contains("colour"), "{}", error.detail);
}

#[test]
fn an_expression_must_carry_exactly_one_tag() {
    let mut two = row(7, "板厚", "length", None, json!({"num": 1.0}));
    two["expr"] = json!({"num": 1.0, "ref": 8});
    assert!(decode_error(vec![two]).detail.contains("exactly one"));

    let none = row(7, "板厚", "length", None, json!({"note": "nothing"}));
    assert!(decode_error(vec![none]).detail.contains("exactly one"));

    let unknown_op = row(
        7,
        "板厚",
        "length",
        None,
        json!({"op": "pow", "lhs": {"num": 1.0}, "rhs": {"num": 2.0}}),
    );
    assert!(decode_error(vec![unknown_op])
        .detail
        .contains("unknown operator"));

    let unknown_const = row(7, "板厚", "length", None, json!({"const": "e"}));
    assert!(decode_error(vec![unknown_const])
        .detail
        .contains("unknown constant"));
}

#[test]
fn a_dangling_reference_is_refused() {
    let dangling = row(7, "板厚", "length", None, json!({"ref": 99}));
    let error = decode_error(vec![dangling]);
    assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_UNKNOWN_REFERENCE),
        "{}",
        error.detail
    );
}

#[test]
fn a_reference_cycle_is_refused() {
    let rows = vec![
        row(1, "a", "length", None, json!({"ref": 2})),
        row(2, "b", "length", None, json!({"ref": 1})),
    ];
    let error = decode_error(rows);
    assert!(
        error.detail.contains(cad_domain::param::PARAM_CYCLE),
        "{}",
        error.detail
    );
}

#[test]
fn duplicate_sequence_or_name_is_refused() {
    let rows = vec![
        row(7, "板厚", "length", None, json!({"num": 1.0})),
        row(7, "另一块", "length", None, json!({"num": 2.0})),
    ];
    let error = decode_error(rows);
    assert!(
        error.detail.contains("duplicate parameter sequence"),
        "{}",
        error.detail
    );

    let rows = vec![
        row(7, "板厚", "length", None, json!({"num": 1.0})),
        row(8, "板厚", "length", None, json!({"num": 2.0})),
    ];
    let error = decode_error(rows);
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_DUPLICATE_NAME),
        "{}",
        error.detail
    );
}

#[test]
fn a_non_finite_literal_is_refused() {
    let rows = vec![row(
        7,
        "板厚",
        "length",
        None,
        json!({"num": {"$f64": "nan"}}),
    )];
    let error = decode_error(rows);
    assert!(
        error
            .detail
            .contains(cad_domain::param::PARAM_INVALID_VALUE),
        "{}",
        error.detail
    );
}

// ----------------------------------------------------- element `params` ---

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

fn db_with(parameters: ParamTable) -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    b.insert_layout(crate::tables::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .expect("layout");
    b.insert_entity(circle(1)).expect("entity");
    b.set_parameters(parameters).expect("parameters");
    b.finish().expect("finish")
}

fn bound_db() -> DrawingDatabase {
    let mut parameters = ParamTable::new();
    parameters
        .insert(domain_row(7, "radius", ParamKind::Length, Expr::num(12.5)))
        .expect("row");
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    b.insert_layout(crate::tables::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .expect("layout");
    b.insert_entity(circle(1)).expect("entity");
    b.set_parameters(parameters).expect("parameters");
    b.set_param_bindings(
        EntityId(1),
        ParamBindings::from_entries([("radius".to_string(), Expr::reference(ParamSeq(7)))])
            .expect("bindings"),
    )
    .expect("bindings");
    b.finish().expect("finish")
}

fn snapshot_of(db: &DrawingDatabase) -> crate::snapshot::DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        Revision(3),
    )
    .expect("snapshot")
}

#[test]
fn an_element_without_bindings_omits_the_params_key() {
    let snapshot = snapshot_of(&db_with(ParamTable::new()));
    let payload = &snapshot.shards[0].elements[0].payload;
    assert!(
        payload.get("params").is_none(),
        "an absent block omits the key"
    );
    assert!(
        !snapshot.has_params(),
        "no table, no block, no parametric data"
    );
}

#[test]
fn a_bound_element_round_trips_through_the_snapshot() {
    let db = bound_db();
    let snapshot = snapshot_of(&db);
    assert!(snapshot.has_params());
    let payload = &snapshot.shards[0].elements[0].payload;
    assert_eq!(
        payload["params"],
        json!({"radius": {"ref": 7}}),
        "the block is a flat slot -> expression object"
    );

    let loaded = DrawingDatabase::from_snapshot(&snapshot).expect("hydrate");
    assert_eq!(loaded.parameters(), db.parameters());
    assert_eq!(
        loaded.param_bindings(EntityId(1)),
        db.param_bindings(EntityId(1))
    );
}

#[test]
fn a_null_params_block_is_refused() {
    let snapshot = snapshot_of(&bound_db());
    let mut payload = snapshot.shards[0].elements[0].payload.clone();
    payload["params"] = Value::Null;
    let error = elements::decode(&payload).expect_err("null is not an absent block");
    assert!(error.detail.contains("params"), "{}", error.detail);
    assert!(error.detail.contains("null"), "{}", error.detail);
}

#[test]
fn an_unknown_extra_key_inside_the_block_is_refused() {
    // The block itself accepts dynamic slot names, but each *expression* is
    // strict: an extra key inside it is a malformed row.
    let snapshot = snapshot_of(&bound_db());
    let mut payload = snapshot.shards[0].elements[0].payload.clone();
    payload["params"] = json!({"radius": {"num": 1.0, "unit": "mm"}});
    let error = elements::decode(&payload).expect_err("extra expression key");
    assert!(error.detail.contains("unit") || error.detail.contains("exactly one"));
}

#[test]
fn a_binding_with_an_unknown_slot_refuses_the_build() {
    // The codec accepts any slot name (shape layer); the builder owns the slot
    // registry and must refuse it (`param.unknown_slot`).
    let mut parameters = ParamTable::new();
    parameters
        .insert(domain_row(7, "radius", ParamKind::Length, Expr::num(2.0)))
        .expect("row");
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    b.insert_entity(circle(1)).expect("entity");
    b.set_parameters(parameters).expect("parameters");
    b.set_param_bindings(
        EntityId(1),
        ParamBindings::from_entries([("diameter".to_string(), Expr::reference(ParamSeq(7)))])
            .expect("bindings"),
    )
    .expect("bindings");
    let error = b.finish().expect_err("unknown slot must refuse the build");
    assert!(
        format!("{error}").contains(cad_domain::param::PARAM_UNKNOWN_SLOT),
        "{error}"
    );
}

#[test]
fn a_binding_on_a_geometry_type_without_an_evaluator_is_refused() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(crate::tables::Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    let mut entity = circle(1);
    entity.geometry = SemanticGeometry::Line {
        start: Point3::default(),
        end: Point3 {
            x: 10.0,
            ..Point3::default()
        },
    };
    b.insert_entity(entity).expect("entity");
    b.set_param_bindings(
        EntityId(1),
        ParamBindings::from_entries([("radius".to_string(), Expr::num(1.0))]).expect("bindings"),
    )
    .expect("bindings");
    let error = b.finish().expect_err("no evaluator must refuse the build");
    assert!(
        format!("{error}").contains(cad_domain::param::PARAM_NO_EVALUATOR),
        "{error}"
    );
}
