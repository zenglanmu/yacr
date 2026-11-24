//! `PARAM SET` (the table-level value/expression write) and the
//! `-PARAMETERS`/`PARAMETERS` read-only report (ADR-0043 D9).
//!
//! Split out of `param.rs` (the code-size gate): both halves exercise the same
//! fixtures the structural tests own, reached through their `pub(super)` helpers,
//! so the two files pin one document model instead of two.
//!
//! What must hold: a `SetValue` is one transaction / one revision / one undo step,
//! the derived geometry follows the table, every refusal (unknown reference,
//! dimension mismatch, cycle, a configuration-masked row) writes nothing, and the
//! list report is read-only and names the solver boundary in words.

use super::param::{
    bind, circle, configured_app, create, param_app, radius, redo, seq_of, stored_drawing, undo,
    undo_depth,
};
use super::*;
use cad_domain::param::{Expr, ParamKind, ParamSeq};

fn set_value(
    app: &mut Application,
    session: &mut SessionState,
    seq: ParamSeq,
    expr: Expr,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::SetValue { seq, expr }),
    )
}

fn param_config(
    app: &mut Application,
    session: &mut SessionState,
    name: Option<&str>,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Param,
        CommandPayload::Param(crate::ParamOp::Config {
            name: name.map(str::to_string),
        }),
    )
}

#[test]
fn set_value_rewrites_the_bound_geometry_in_one_revision_and_one_undo_step() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let r = seq_of(&app, "r");
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(r))],
    )
    .expect("bind");
    assert_eq!(radius(&app, 1), 20.0);

    let before = revision(&app);
    let depth = undo_depth(&app);
    let seq = seq_of(&app, "r");
    set_value(&mut app, &mut session, seq, Expr::Num(35.0)).expect("set");

    assert_eq!(radius(&app, 1), 35.0, "the derived geometry follows");
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "one table row and its derived geometry are one revision"
    );
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");

    undo(&mut app, &mut session);
    assert_eq!(radius(&app, 1), 20.0, "undo restores the row and geometry");
    redo(&mut app, &mut session);
    assert_eq!(radius(&app, 1), 35.0, "redo reapplies the same edit");
}

#[test]
fn set_value_accepts_an_expression_over_other_parameters() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    session.selection.replace([ref_of(1)]);
    create(&mut app, &mut session, "q", ParamKind::Length, 2.0);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    let r = seq_of(&app, "r");
    let q = seq_of(&app, "q");
    bind(
        &mut app,
        &mut session,
        vec![("radius".into(), Expr::Ref(r))],
    )
    .expect("bind");

    set_value(
        &mut app,
        &mut session,
        r,
        Expr::Op {
            op: cad_domain::param::BinaryOp::Mul,
            lhs: Box::new(Expr::Ref(q)),
            rhs: Box::new(Expr::Num(7.5)),
        },
    )
    .expect("set expression");
    assert_eq!(radius(&app, 1), 15.0, "q * 7.5 = 15");

    // Changing the referenced parameter re-derives through the expression.
    set_value(&mut app, &mut session, q, Expr::Num(4.0)).expect("set q");
    assert_eq!(radius(&app, 1), 30.0, "4 * 7.5 = 30");
}

#[test]
fn set_value_refusals_write_nothing() {
    let (mut app, mut session) = param_app(vec![circle(1, 10.0)]);
    create(&mut app, &mut session, "r", ParamKind::Length, 20.0);
    create(&mut app, &mut session, "a", ParamKind::Angle, 0.5);
    let before = revision(&app);
    let depth = undo_depth(&app);

    // An unknown reference is refused by the merged write path.
    let seq = seq_of(&app, "r");
    let error = set_value(&mut app, &mut session, seq, Expr::Ref(ParamSeq(99))).unwrap_err();
    assert!(
        format!("{error:?}").contains("param.unknown_reference"),
        "got {error:?}"
    );

    // A dimension mismatch (`length` = `angle`) is refused, never converted.
    // A `number` reference is deliberately *not* a mismatch: it is dimensionless
    // and may scale a length (the domain's own rule).
    let angle = seq_of(&app, "a");
    let error = set_value(&mut app, &mut session, seq, Expr::Ref(angle)).unwrap_err();
    assert!(
        format!("{error:?}").contains("param.unit_mismatch"),
        "got {error:?}"
    );

    // A cycle is refused before anything is stored.
    let error = set_value(&mut app, &mut session, seq, Expr::Ref(seq)).unwrap_err();
    assert!(
        format!("{error:?}").contains("param.cycle"),
        "got {error:?}"
    );

    assert_eq!(revision(&app), before, "no refusal writes");
    assert_eq!(undo_depth(&app), depth, "no refusal records history");
    assert_eq!(
        stored_drawing(&app)
            .parameters()
            .get_by_name("r")
            .expect("r still exists")
            .expr,
        Expr::Num(20.0)
    );
}

#[test]
fn set_value_refuses_a_configuration_masked_row() {
    let (mut app, mut session) = configured_app();
    param_config(&mut app, &mut session, Some("small")).expect("activate small");
    let before = revision(&app);
    let depth = undo_depth(&app);

    let error = set_value(&mut app, &mut session, ParamSeq(1), Expr::Num(30.0)).unwrap_err();
    let text = format!("{error:?}");
    assert!(text.contains("param.config_override"), "got {text}");
    assert!(
        text.contains("overridden by configuration small"),
        "got {text}"
    );
    assert_eq!(revision(&app), before, "nothing is written");
    assert_eq!(undo_depth(&app), depth);

    // A suppressed row is refused with the same marker.
    param_config(&mut app, &mut session, Some("hide")).expect("activate hide");
    let error = set_value(&mut app, &mut session, ParamSeq(3), Expr::Num(9.0)).unwrap_err();
    let text = format!("{error:?}");
    assert!(text.contains("param.config_override"), "got {text}");
    assert!(
        text.contains("suppressed by configuration hide"),
        "got {text}"
    );
}

#[test]
fn the_parameter_list_report_reads_every_row_and_names_the_solver_boundary() {
    let (mut app, mut session) = configured_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ParamList,
        CommandPayload::None,
    )
    .expect("the report runs");
    assert!(outcome.changes.is_none(), "a report writes nothing");
    assert!(outcome.objects.is_empty());
    assert_eq!(revision(&app), before);
    let codes: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(codes.first(), Some(&"param.list"));
    assert_eq!(codes.iter().filter(|code| **code == "param.row").count(), 3);
    assert_eq!(codes.last(), Some(&"param.constraint_not_modeled"));
    let text = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("r"), "the row name is reported: {text}");
    assert!(text.contains("length mm"), "the kind and unit: {text}");
    assert!(
        text.contains("solver-driven"),
        "the constraint boundary is explicit: {text}"
    );
}

#[test]
fn the_parameter_list_report_refuses_without_a_document() {
    let mut app = Application::new();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::ParamList,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("document not open"),
        "got {error:?}"
    );
}
