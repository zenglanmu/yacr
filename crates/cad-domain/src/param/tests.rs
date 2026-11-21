use super::*;
use crate::{Point3, SemanticGeometry, StyleId, TextAlignH, TextAlignV, TextLineSpacing};

fn seq(value: u64) -> ParamSeq {
    ParamSeq(value)
}

fn row(seq_value: u64, name: &str, kind: ParamKind, expr: Expr) -> ParamRow {
    ParamRow::new(seq(seq_value), name, kind, expr).expect("valid row")
}

fn table(rows: Vec<ParamRow>) -> ParamTable {
    ParamTable::from_rows(rows).expect("valid table")
}

fn circle(radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center: Point3::default(),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        radius,
    }
}

fn polyline(width: f64) -> SemanticGeometry {
    SemanticGeometry::Polyline {
        points: vec![Point3::default()],
        bulges: Vec::new(),
        closed: false,
        width,
    }
}

fn text(height: f64) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "hello".into(),
        position: Point3::default(),
        style: StyleId(0),
        height,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: TextLineSpacing::default(),
    }
}

#[test]
fn a_bare_number_takes_the_row_kind() {
    let values = evaluate_table(&table(vec![row(
        1,
        "thickness",
        ParamKind::Length,
        Expr::num(12.5),
    )]))
    .unwrap();
    let value = values.get(seq(1)).unwrap();
    assert_eq!(value.value, 12.5);
    assert_eq!(value.kind, ParamKind::Length);
}

#[test]
fn references_and_arithmetic_evaluate_in_order() {
    let values = evaluate_table(&table(vec![
        row(1, "thickness", ParamKind::Length, Expr::num(12.5)),
        row(
            2,
            "hole",
            ParamKind::Length,
            Expr::binary(BinaryOp::Div, Expr::reference(seq(1)), Expr::num(2.0)),
        ),
        row(
            3,
            "offset",
            ParamKind::Length,
            Expr::binary(BinaryOp::Add, Expr::reference(seq(2)), Expr::num(1.0)),
        ),
    ]))
    .unwrap();
    assert_eq!(values.get(seq(2)).unwrap().value, 6.25);
    assert_eq!(values.get(seq(3)).unwrap().value, 7.25);
}

#[test]
fn pi_is_dimensionless_and_multiplies_a_length() {
    let values = evaluate_table(&table(vec![row(
        1,
        "circumference",
        ParamKind::Length,
        Expr::binary(BinaryOp::Mul, Expr::Const(ParamConst::Pi), Expr::num(2.0)),
    )]))
    .unwrap();
    assert!((values.get(seq(1)).unwrap().value - std::f64::consts::TAU).abs() < 1e-12);
}

#[test]
fn a_self_reference_is_a_cycle() {
    let error = evaluate_table(&table(vec![row(
        1,
        "a",
        ParamKind::Number,
        Expr::reference(seq(1)),
    )]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_CYCLE);
}

#[test]
fn an_indirect_cycle_is_detected() {
    let error = evaluate_table(&table(vec![
        row(1, "a", ParamKind::Number, Expr::reference(seq(2))),
        row(2, "b", ParamKind::Number, Expr::reference(seq(1))),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_CYCLE);
}

#[test]
fn a_dangling_reference_is_refused() {
    let error = evaluate_table(&table(vec![row(
        1,
        "a",
        ParamKind::Number,
        Expr::reference(seq(9)),
    )]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNKNOWN_REFERENCE);
}

#[test]
fn a_free_number_adopts_the_fixed_kind() {
    let values = evaluate_table(&table(vec![
        row(1, "length", ParamKind::Length, Expr::num(1.0)),
        row(
            2,
            "mixed",
            ParamKind::Length,
            Expr::binary(BinaryOp::Add, Expr::reference(seq(1)), Expr::num(1.0)),
        ),
    ]))
    .unwrap();
    assert_eq!(values.get(seq(2)).unwrap().value, 2.0);
}

#[test]
fn a_number_parameter_can_stand_in_for_a_length() {
    // A `number` parameter is dimensionless, so it adopts the row's kind like a
    // literal does; this is not a conversion.
    let values = evaluate_table(&table(vec![
        row(1, "count", ParamKind::Number, Expr::num(3.0)),
        row(2, "length", ParamKind::Length, Expr::reference(seq(1))),
    ]))
    .unwrap();
    assert_eq!(values.get(seq(2)).unwrap().value, 3.0);
    assert_eq!(values.get(seq(2)).unwrap().kind, ParamKind::Length);
}

#[test]
fn multiplying_two_lengths_is_a_unit_mismatch() {
    let error = evaluate_table(&table(vec![
        row(1, "a", ParamKind::Length, Expr::num(2.0)),
        row(2, "b", ParamKind::Length, Expr::num(3.0)),
        row(
            3,
            "area?",
            ParamKind::Length,
            Expr::binary(
                BinaryOp::Mul,
                Expr::reference(seq(1)),
                Expr::reference(seq(2)),
            ),
        ),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNIT_MISMATCH);
}

#[test]
fn dividing_two_lengths_is_dimensionless() {
    let values = evaluate_table(&table(vec![
        row(1, "a", ParamKind::Length, Expr::num(6.0)),
        row(2, "b", ParamKind::Length, Expr::num(3.0)),
        row(
            3,
            "ratio",
            ParamKind::Number,
            Expr::binary(
                BinaryOp::Div,
                Expr::reference(seq(1)),
                Expr::reference(seq(2)),
            ),
        ),
    ]))
    .unwrap();
    assert_eq!(values.get(seq(3)).unwrap().value, 2.0);
}

#[test]
fn a_reference_to_another_dimension_is_a_unit_mismatch() {
    let error = evaluate_table(&table(vec![
        row(1, "angle", ParamKind::Angle, Expr::num(0.5)),
        row(2, "length", ParamKind::Length, Expr::reference(seq(1))),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNIT_MISMATCH);
}

#[test]
fn a_number_parameter_scales_a_length() {
    let values = evaluate_table(&table(vec![
        row(1, "count", ParamKind::Number, Expr::num(3.0)),
        row(2, "length", ParamKind::Length, Expr::num(2.0)),
        row(
            3,
            "scaled",
            ParamKind::Length,
            Expr::binary(
                BinaryOp::Mul,
                Expr::reference(seq(1)),
                Expr::reference(seq(2)),
            ),
        ),
    ]))
    .unwrap();
    assert_eq!(values.get(seq(3)).unwrap().value, 6.0);
}

#[test]
fn two_number_parameters_multiply() {
    let values = evaluate_table(&table(vec![
        row(1, "a", ParamKind::Number, Expr::num(2.0)),
        row(2, "b", ParamKind::Number, Expr::num(4.0)),
        row(
            3,
            "product",
            ParamKind::Number,
            Expr::binary(
                BinaryOp::Mul,
                Expr::reference(seq(1)),
                Expr::reference(seq(2)),
            ),
        ),
    ]))
    .unwrap();
    assert_eq!(values.get(seq(3)).unwrap().value, 8.0);
}

#[test]
fn dividing_a_length_by_an_angle_is_refused() {
    let error = evaluate_table(&table(vec![
        row(1, "length", ParamKind::Length, Expr::num(6.0)),
        row(2, "angle", ParamKind::Angle, Expr::num(2.0)),
        row(
            3,
            "ratio",
            ParamKind::Number,
            Expr::binary(
                BinaryOp::Div,
                Expr::reference(seq(1)),
                Expr::reference(seq(2)),
            ),
        ),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNIT_MISMATCH);
}

#[test]
fn dividing_a_number_by_a_length_is_refused() {
    let error = evaluate_table(&table(vec![
        row(1, "count", ParamKind::Number, Expr::num(6.0)),
        row(2, "length", ParamKind::Length, Expr::num(2.0)),
        row(
            3,
            "ratio",
            ParamKind::Number,
            Expr::binary(
                BinaryOp::Div,
                Expr::reference(seq(1)),
                Expr::reference(seq(2)),
            ),
        ),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNIT_MISMATCH);
}

#[test]
fn division_by_zero_is_an_invalid_value() {
    let error = evaluate_table(&table(vec![row(
        1,
        "boom",
        ParamKind::Number,
        Expr::binary(BinaryOp::Div, Expr::num(1.0), Expr::num(0.0)),
    )]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_INVALID_VALUE);
}

#[test]
fn a_non_finite_literal_is_refused_at_row_validation() {
    let error =
        ParamRow::new(seq(1), "a", ParamKind::Number, Expr::num(f64::INFINITY)).unwrap_err();
    assert_eq!(error.code(), PARAM_INVALID_VALUE);
}

#[test]
fn duplicate_names_are_refused() {
    let mut table = table(vec![row(1, "a", ParamKind::Number, Expr::num(1.0))]);
    let error = table
        .insert(row(2, "a", ParamKind::Number, Expr::num(2.0)))
        .unwrap_err();
    assert_eq!(error.code(), PARAM_DUPLICATE_NAME);
}

#[test]
fn renaming_keeps_the_sequence_and_frees_the_old_name() {
    let mut table = table(vec![row(1, "a", ParamKind::Number, Expr::num(1.0))]);
    table
        .insert(row(1, "b", ParamKind::Number, Expr::num(2.0)))
        .unwrap();
    assert_eq!(table.get_by_name("a"), None);
    assert_eq!(table.get_by_name("b").unwrap().seq, seq(1));
    table
        .insert(row(2, "a", ParamKind::Number, Expr::num(3.0)))
        .unwrap();
}

#[test]
fn a_zero_sequence_or_empty_name_is_invalid() {
    assert_eq!(
        ParamRow::new(ParamSeq(0), "a", ParamKind::Number, Expr::num(1.0))
            .unwrap_err()
            .code(),
        PARAM_INVALID_VALUE
    );
    assert_eq!(
        ParamRow::new(seq(1), "  ", ParamKind::Number, Expr::num(1.0))
            .unwrap_err()
            .code(),
        PARAM_INVALID_VALUE
    );
}

#[test]
fn an_expression_over_the_depth_budget_is_refused() {
    let mut expr = Expr::num(1.0);
    for _ in 0..PARAM_MAX_EXPR_DEPTH {
        expr = Expr::binary(BinaryOp::Add, expr, Expr::num(1.0));
    }
    let error = ParamRow::new(seq(1), "a", ParamKind::Number, expr).unwrap_err();
    assert_eq!(error.code(), PARAM_BUDGET_EXCEEDED);
}

#[test]
fn an_expression_over_the_node_budget_is_refused() {
    let mut expr = Expr::num(1.0);
    for _ in 0..PARAM_MAX_EXPR_NODES {
        expr = Expr::binary(BinaryOp::Add, expr, Expr::num(1.0));
    }
    let error = ParamRow::new(seq(1), "a", ParamKind::Number, expr).unwrap_err();
    assert_eq!(error.code(), PARAM_BUDGET_EXCEEDED);
}

#[test]
fn too_many_rows_is_a_budget_refusal() {
    let mut table = ParamTable::new();
    for index in 1..=PARAM_MAX_ROWS as u64 {
        table
            .insert(row(
                index,
                &format!("p{index}"),
                ParamKind::Number,
                Expr::num(1.0),
            ))
            .unwrap();
    }
    let error = table
        .insert(row(
            PARAM_MAX_ROWS as u64 + 1,
            "one_too_many",
            ParamKind::Number,
            Expr::num(1.0),
        ))
        .unwrap_err();
    assert_eq!(error.code(), PARAM_BUDGET_EXCEEDED);
}

#[test]
fn bindings_resolve_against_the_type_slot_registry() {
    let values =
        evaluate_table(&table(vec![row(1, "r", ParamKind::Length, Expr::num(5.0))])).unwrap();
    let bindings =
        ParamBindings::from_entries([("radius".to_string(), Expr::reference(seq(1)))]).unwrap();
    let resolved = bindings.resolve("cad:Circle", &values).unwrap();
    assert_eq!(resolved.get("radius"), Some(&5.0));

    let unknown = ParamBindings::from_entries([("height".to_string(), Expr::num(2.0))]).unwrap();
    assert_eq!(
        unknown.resolve("cad:Circle", &values).unwrap_err().code(),
        PARAM_UNKNOWN_SLOT
    );
}

#[test]
fn a_binding_of_the_wrong_kind_is_refused() {
    let values = evaluate_table(&table(vec![row(
        1,
        "angle",
        ParamKind::Angle,
        Expr::num(0.5),
    )]))
    .unwrap();
    let bindings =
        ParamBindings::from_entries([("radius".to_string(), Expr::reference(seq(1)))]).unwrap();
    assert_eq!(
        bindings.resolve("cad:Circle", &values).unwrap_err().code(),
        PARAM_UNIT_MISMATCH
    );
}

#[test]
fn a_number_parameter_binds_to_a_length_slot() {
    let values = evaluate_table(&table(vec![row(
        1,
        "count",
        ParamKind::Number,
        Expr::num(3.0),
    )]))
    .unwrap();
    let bindings =
        ParamBindings::from_entries([("radius".to_string(), Expr::reference(seq(1)))]).unwrap();
    assert_eq!(
        bindings
            .resolve("cad:Circle", &values)
            .unwrap()
            .get("radius"),
        Some(&3.0)
    );
}

#[test]
fn apply_slot_rewrites_only_the_named_field() {
    match apply_slot(&circle(2.0), "radius", 7.0).unwrap() {
        SemanticGeometry::Circle { radius, .. } => assert_eq!(radius, 7.0),
        other => panic!("unexpected geometry {other:?}"),
    }
    match apply_slot(&polyline(0.0), "width", 1.5).unwrap() {
        SemanticGeometry::Polyline { width, .. } => assert_eq!(width, 1.5),
        other => panic!("unexpected geometry {other:?}"),
    }
    match apply_slot(&text(2.0), "height", 4.0).unwrap() {
        SemanticGeometry::Text { height, .. } => assert_eq!(height, 4.0),
        other => panic!("unexpected geometry {other:?}"),
    }
}

#[test]
fn a_degenerate_slot_value_is_a_geometry_failure_not_a_unit_error() {
    assert_eq!(
        apply_slot(&circle(2.0), "radius", 0.0).unwrap_err().code(),
        PARAM_GEOMETRY_FAILED
    );
    assert_eq!(
        apply_slot(&polyline(1.0), "width", -1.0)
            .unwrap_err()
            .code(),
        PARAM_GEOMETRY_FAILED
    );
    assert_eq!(
        apply_slot(&text(1.0), "height", 0.0).unwrap_err().code(),
        PARAM_GEOMETRY_FAILED
    );
}

#[test]
fn an_unknown_slot_or_type_has_a_distinct_marker() {
    assert_eq!(
        apply_slot(&circle(1.0), "diameter", 2.0)
            .unwrap_err()
            .code(),
        PARAM_UNKNOWN_SLOT
    );
    assert_eq!(
        apply_slot(
            &SemanticGeometry::Line {
                start: Point3::default(),
                end: Point3::default(),
            },
            "radius",
            1.0
        )
        .unwrap_err()
        .code(),
        PARAM_NO_EVALUATOR
    );
}

#[test]
fn geometry_type_keys_are_the_descriptor_spellings() {
    assert_eq!(geometry_type_key(&circle(1.0)), Some("cad:Circle"));
    assert_eq!(geometry_type_key(&polyline(1.0)), Some("cad:Polyline"));
    assert_eq!(geometry_type_key(&text(1.0)), Some("cad:Text"));
    assert_eq!(
        geometry_type_key(&SemanticGeometry::Point(Point3::default())),
        None
    );
}

#[test]
fn affected_params_follow_references_transitively() {
    let table = table(vec![
        row(1, "a", ParamKind::Number, Expr::num(1.0)),
        row(
            2,
            "b",
            ParamKind::Number,
            Expr::binary(BinaryOp::Add, Expr::reference(seq(1)), Expr::num(1.0)),
        ),
        row(
            3,
            "c",
            ParamKind::Number,
            Expr::binary(BinaryOp::Add, Expr::reference(seq(2)), Expr::num(1.0)),
        ),
        row(4, "d", ParamKind::Number, Expr::num(9.0)),
    ]);
    let affected = affected_params(&table, &BTreeSet::from([seq(1)]));
    assert_eq!(affected, BTreeSet::from([seq(1), seq(2), seq(3)]));
}

#[test]
fn affected_entities_include_direct_edits_and_referencing_bindings() {
    let mut bindings = BTreeMap::new();
    bindings.insert(
        EntityId(1),
        ParamBindings::from_entries([("radius".to_string(), Expr::reference(seq(1)))]).unwrap(),
    );
    bindings.insert(
        EntityId(2),
        ParamBindings::from_entries([("radius".to_string(), Expr::num(3.0))]).unwrap(),
    );
    let affected = affected_entities(
        &bindings,
        &BTreeSet::from([seq(1)]),
        &BTreeSet::from([EntityId(7)]),
    )
    .unwrap();
    assert_eq!(affected, BTreeSet::from([EntityId(1), EntityId(7)]));
}

#[test]
fn p1_unary_pow_neg_and_if_evaluate() {
    let values = evaluate_table(&table(vec![
        row(1, "base", ParamKind::Number, Expr::num(9.0)),
        row(
            2,
            "root",
            ParamKind::Number,
            Expr::unary(UnaryOp::Sqrt, Expr::reference(seq(1))),
        ),
        row(
            3,
            "square",
            ParamKind::Number,
            Expr::pow(Expr::reference(seq(2)), Expr::num(2.0)),
        ),
        row(
            4,
            "negated",
            ParamKind::Number,
            Expr::negated(Expr::reference(seq(3))),
        ),
        row(
            5,
            "chosen",
            ParamKind::Number,
            Expr::if_then(
                Expr::binary(BinaryOp::Sub, Expr::reference(seq(1)), Expr::num(9.0)),
                Expr::num(111.0),
                Expr::reference(seq(4)),
            ),
        ),
        row(
            6,
            "rounded",
            ParamKind::Number,
            Expr::unary(UnaryOp::Round, Expr::num(2.6)),
        ),
    ]))
    .unwrap();
    assert_eq!(values.get(seq(2)).unwrap().value, 3.0);
    assert_eq!(values.get(seq(3)).unwrap().value, 9.0);
    assert_eq!(values.get(seq(4)).unwrap().value, -9.0);
    assert_eq!(values.get(seq(5)).unwrap().value, -9.0);
    assert_eq!(values.get(seq(6)).unwrap().value, 3.0);
}

#[test]
fn trigonometry_accepts_an_angle_and_rejects_a_length() {
    let values = evaluate_table(&table(vec![
        row(1, "theta", ParamKind::Angle, Expr::num(0.0)),
        row(
            2,
            "sine",
            ParamKind::Number,
            Expr::unary(UnaryOp::Sin, Expr::reference(seq(1))),
        ),
    ]))
    .unwrap();
    assert_eq!(values.get(seq(2)).unwrap().value, 0.0);

    let error = evaluate_table(&table(vec![
        row(1, "length", ParamKind::Length, Expr::num(1.0)),
        row(
            2,
            "sine",
            ParamKind::Number,
            Expr::unary(UnaryOp::Sin, Expr::reference(seq(1))),
        ),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNIT_MISMATCH);
}

#[test]
fn a_power_of_a_dimensioned_value_is_refused() {
    let error = evaluate_table(&table(vec![
        row(1, "length", ParamKind::Length, Expr::num(2.0)),
        row(
            2,
            "area?",
            ParamKind::Number,
            Expr::pow(Expr::reference(seq(1)), Expr::num(2.0)),
        ),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNIT_MISMATCH);
}

#[test]
fn an_if_branch_can_adopt_a_free_literal() {
    let values = evaluate_table(&table(vec![
        row(1, "length", ParamKind::Length, Expr::num(2.0)),
        row(
            2,
            "chosen",
            ParamKind::Length,
            Expr::if_then(Expr::num(1.0), Expr::reference(seq(1)), Expr::num(9.0)),
        ),
    ]))
    .unwrap();
    assert_eq!(values_of(&values, 2), 2.0);
}

#[test]
fn an_if_with_two_fixed_branch_kinds_is_refused() {
    let error = evaluate_table(&table(vec![
        row(1, "length", ParamKind::Length, Expr::num(2.0)),
        row(2, "angle", ParamKind::Angle, Expr::num(1.0)),
        row(
            3,
            "mixed",
            ParamKind::Length,
            Expr::if_then(
                Expr::num(1.0),
                Expr::reference(seq(1)),
                Expr::reference(seq(2)),
            ),
        ),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNIT_MISMATCH);
}

fn values_of(values: &ParamValues, seq_value: u64) -> f64 {
    values.get(seq(seq_value)).unwrap().value
}

#[test]
fn sqrt_of_a_length_is_refused() {
    let error = evaluate_table(&table(vec![
        row(1, "length", ParamKind::Length, Expr::num(9.0)),
        row(
            2,
            "root",
            ParamKind::Number,
            Expr::unary(UnaryOp::Sqrt, Expr::reference(seq(1))),
        ),
    ]))
    .unwrap_err();
    assert_eq!(error.code(), PARAM_UNIT_MISMATCH);
}

#[test]
fn an_if_does_not_evaluate_the_untaken_branch() {
    let values = evaluate_table(&table(vec![row(
        1,
        "chosen",
        ParamKind::Number,
        Expr::if_then(
            Expr::num(1.0),
            Expr::num(7.0),
            Expr::binary(BinaryOp::Div, Expr::num(1.0), Expr::num(0.0)),
        ),
    )]))
    .unwrap();
    assert_eq!(values_of(&values, 1), 7.0);
}

#[test]
fn p1_nodes_are_flagged_for_the_minor_tier() {
    assert!(!Expr::num(1.0).is_p1());
    assert!(!Expr::binary(BinaryOp::Add, Expr::num(1.0), Expr::reference(seq(1))).is_p1());
    assert!(Expr::negated(Expr::num(1.0)).is_p1());
    assert!(Expr::pow(Expr::num(1.0), Expr::num(2.0)).is_p1());
    assert!(Expr::unary(UnaryOp::Abs, Expr::num(1.0)).is_p1());
    assert!(Expr::if_then(Expr::num(1.0), Expr::num(2.0), Expr::num(3.0)).is_p1());
}

#[test]
fn evaluation_is_deterministic() {
    let table = table(vec![
        row(1, "a", ParamKind::Length, Expr::num(2.0)),
        row(
            2,
            "b",
            ParamKind::Length,
            Expr::binary(BinaryOp::Mul, Expr::reference(seq(1)), Expr::num(3.0)),
        ),
    ]);
    let first = evaluate_table(&table).unwrap();
    let second = evaluate_table(&table).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.iter().count(), 2);
}
