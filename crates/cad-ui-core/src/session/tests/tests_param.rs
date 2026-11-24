//! The document-level parameter list session bridge (ADR-0043 P1).
//!
//! What must hold: the generic host-read request answers the same rows the
//! session entry does, statuses come from the active configuration, a suppressed
//! row keeps its base value, and no document is an empty answer.

use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::param::{
    Expr, ParamBindings, ParamConfig, ParamConfigTable, ParamKind, ParamRow, ParamSeq, ParamTable,
};
use cad_domain::{
    DatabaseId, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId,
    TransactionId,
};

use super::super::*;
use super::test_support::*;
use crate::host_read::{HostReadAnswer, HostReadRequest};
use crate::param::{ParamListResponse, PARAM_LIST_KIND};

fn circle(id: u128, radius: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbCircle".to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Circle {
            center: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
        draw_order: 0,
    }
}

/// Base rows `r=20`, `q=10`, `s=7`; a circle bound to `r`; `mix` overrides
/// `r=5` and suppresses `s`.
fn param_database() -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(11));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder.insert_entity(circle(1, 20.0)).expect("circle");
    let table = ParamTable::from_rows([
        ParamRow::new(ParamSeq(1), "r", ParamKind::Length, Expr::Num(20.0)).expect("r"),
        ParamRow::new(ParamSeq(2), "q", ParamKind::Length, Expr::Num(10.0)).expect("q"),
        ParamRow::new(ParamSeq(3), "s", ParamKind::Length, Expr::Num(7.0)).expect("s"),
    ])
    .expect("table");
    builder.set_parameters(table).expect("parameters");
    let mut configs = ParamConfigTable::new();
    let mut mix = ParamConfig::new("mix").expect("name");
    mix.set_override(ParamSeq(1), Expr::Num(5.0))
        .expect("override");
    mix.suppress(ParamSeq(3));
    configs.insert(mix).expect("config");
    builder.set_param_configs(configs).expect("configs");
    builder
        .set_param_bindings(
            EntityId(1),
            ParamBindings::from_entries([("radius".to_string(), Expr::Ref(ParamSeq(1)))])
                .expect("bindings"),
        )
        .expect("bindings");
    let mut database = builder.finish().expect("param database");
    database
        .set_active_configuration(Some("mix".to_string()), "test", TransactionId(1))
        .expect("active config");
    database
}

fn param_session() -> CadSession {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    {
        let mut controller = session.core.controller.borrow_mut();
        let document = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document)
            .expect("the session's document")
            .drawing = Arc::new(param_database());
    }
    session
}

fn empty_session() -> CadSession {
    let mut options = options();
    options.document_at_start = StartDocument::Empty;
    CadSession::new(
        options,
        platform(Rc::new(FakeFiles {
            events: RefCell::new(Vec::new()),
        })),
        RenderSessionHandle::default(),
    )
    .expect("empty session builds")
}

#[test]
fn the_session_lists_the_active_configuration_statuses() {
    let session = param_session();
    let list = session.param_list().expect("the session answers");
    assert_eq!(list.active.as_deref(), Some("mix"));
    assert_eq!(list.total, 3);
    let rows: Vec<(&str, &str, f64)> = list
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.status.as_str(), row.value))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("r", "overridden", 5.0),
            ("q", "inherited", 10.0),
            ("s", "suppressed", 7.0),
        ]
    );
}

#[test]
fn the_generic_host_read_answers_the_same_parameter_list() {
    let session = param_session();
    let answer = session
        .host_read(&HostReadRequest::ParamList)
        .expect("the seam answers");
    let HostReadAnswer::ParamList(list) = answer else {
        panic!("a parameter-list request must answer a parameter list");
    };
    assert_eq!(list, session.param_list().expect("the session entry"));
    let value = serde_json::to_value(&list).expect("serialize");
    assert_eq!(value["active"], "mix");
    assert!(value["rows"].is_array());
}

#[test]
fn a_session_with_no_document_answers_an_empty_list() {
    let session = empty_session();
    assert!(
        session.core.controller.borrow().drawing().is_none(),
        "the fixture really starts with no document"
    );
    let expected = ParamListResponse::default();
    assert_eq!(session.param_list().expect("no document"), expected);
    let answer = session
        .host_read(&HostReadRequest::ParamList)
        .expect("no document is not an error");
    assert_eq!(answer, HostReadAnswer::ParamList(expected));
}

#[test]
fn the_request_kind_is_the_documented_spelling() {
    let value = serde_json::to_value(HostReadRequest::ParamList).expect("serialize");
    assert_eq!(value["kind"], PARAM_LIST_KIND);
}

// ---------------------------------------------------------------------------
// `-PARAMETERS`: the command-line Parameter Manager (ADR-0043 D9/D14).
// ---------------------------------------------------------------------------

use cad_app::{CommandId, RefusalId};
use cad_domain::param::{BinaryOp, ParamConst};

use crate::session::{
    parse_dash_parameters_argument, parse_expression, DashParametersOps, ParamTarget,
    ARGUMENT_SELECTORS, DASH_PARAMETERS_SELECTOR,
};

/// A session whose document has a layer and one circle but **no** parameter rows.
fn plain_param_session() -> CadSession {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(11));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("layer");
    builder.insert_entity(circle(1, 20.0)).expect("circle");
    let drawing = builder.finish().expect("plain database");
    {
        let mut controller = session.core.controller.borrow_mut();
        let document = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document)
            .expect("the session's document")
            .drawing = Arc::new(drawing);
    }
    session
}

/// The three names graduated out of the refusal vocabulary.
#[test]
fn the_parameter_manager_names_are_real_commands_or_chrome_only() {
    assert_eq!(normalize_command("-PARAMETERS"), Some(CommandId::ParamList));
    assert!(ARGUMENT_SELECTORS.contains(&DASH_PARAMETERS_SELECTOR));
    // `PARAMETERS`/`PARAMETERSCLOSE` are chrome-local: the panel and its close
    // word never reach the router, exactly like `PROPERTIES`/`PROPERTIESCLOSE`.
    assert_eq!(normalize_command("PARAMETERS"), None);
    assert_eq!(normalize_command("PARAMETERSCLOSE"), None);
    for selector in ["PARAMETERS", "-PARAMETERS", "PARAMETERSCLOSE"] {
        assert!(
            !RefusalId::ALL
                .iter()
                .any(|name| name.selector() == selector),
            "{selector} must no longer be a named refusal"
        );
    }
}

fn names(name: &str) -> Option<u64> {
    match name {
        "r" => Some(1),
        "q" => Some(2),
        "a" => Some(3),
        _ => None,
    }
}

#[test]
fn the_dash_parameters_grammar_parses_every_option() {
    assert_eq!(
        parse_dash_parameters_argument("?", names).expect("?"),
        DashParametersOps::List
    );
    assert_eq!(
        parse_dash_parameters_argument("LIST", names).expect("LIST"),
        DashParametersOps::List
    );

    let create =
        parse_dash_parameters_argument("NEW \"My width\" length 2 * (q + 1)", names).expect("NEW");
    assert_eq!(
        create,
        DashParametersOps::Create {
            name: "My width".into(),
            kind: ParamKind::Length,
            expr: Expr::Op {
                op: BinaryOp::Mul,
                lhs: Box::new(Expr::Num(2.0)),
                rhs: Box::new(Expr::Op {
                    op: BinaryOp::Add,
                    lhs: Box::new(Expr::Ref(ParamSeq(2))),
                    rhs: Box::new(Expr::Num(1.0)),
                }),
            },
        }
    );

    assert_eq!(
        parse_dash_parameters_argument("SET r pi / 2", names).expect("SET"),
        DashParametersOps::SetValue {
            target: ParamTarget::Seq(1),
            expr: Expr::Op {
                op: BinaryOp::Div,
                lhs: Box::new(Expr::Const(ParamConst::Pi)),
                rhs: Box::new(Expr::Num(2.0)),
            },
        }
    );
    // An unknown numeric token is a sequence, not an invented name.
    assert_eq!(
        parse_dash_parameters_argument("set 7 3", names).expect("SET seq"),
        DashParametersOps::SetValue {
            target: ParamTarget::Seq(7),
            expr: Expr::Num(3.0),
        }
    );
    assert_eq!(
        parse_dash_parameters_argument("RENAME q \"Quoted name\"", names).expect("RENAME"),
        DashParametersOps::Rename {
            target: ParamTarget::Seq(2),
            name: "Quoted name".into(),
        }
    );
    assert_eq!(
        parse_dash_parameters_argument("delete a", names).expect("DELETE"),
        DashParametersOps::Delete {
            target: ParamTarget::Seq(3),
        }
    );
}

#[test]
fn the_dash_parameters_grammar_refuses_by_name() {
    for refused in [
        "NEW r length",
        "NEW r kelvin 1",
        "NEW r",
        "SET",
        "SET r",
        "RENAME r",
        "DELETE",
        "DELETE r extra",
        "LIST extra",
        "FROB r",
        "NEW r length sqrt(4)",
        "NEW r length unknown_name + 1",
        "NEW r length (1 + 2",
        "NEW r length 1 2",
        "NEW r length $",
    ] {
        assert!(
            parse_dash_parameters_argument(refused, names).is_err(),
            "{refused:?} must be refused"
        );
    }
    let error = parse_dash_parameters_argument("NEW r length missing + 1", names).unwrap_err();
    assert_eq!(error.code(), "param.unknown_reference");
    let error = parse_dash_parameters_argument("NEW r length 1..2", names).unwrap_err();
    assert_eq!(error.code(), "param.invalid_value");
}

#[test]
fn parse_expression_builds_the_domain_tree_with_precedence() {
    let expr = parse_expression("1 + 2 * 3", &names).expect("expression");
    assert_eq!(
        expr,
        Expr::Op {
            op: BinaryOp::Add,
            lhs: Box::new(Expr::Num(1.0)),
            rhs: Box::new(Expr::Op {
                op: BinaryOp::Mul,
                lhs: Box::new(Expr::Num(2.0)),
                rhs: Box::new(Expr::Num(3.0)),
            }),
        }
    );
    // Unary minus, power (right-associative) and parentheses.
    let expr = parse_expression("-2 ^ 3 + (4)", &names).expect("expression");
    assert_eq!(
        expr,
        Expr::Op {
            op: BinaryOp::Add,
            lhs: Box::new(Expr::Neg(Box::new(Expr::Pow {
                base: Box::new(Expr::Num(2.0)),
                exp: Box::new(Expr::Num(3.0)),
            }))),
            rhs: Box::new(Expr::Num(4.0)),
        }
    );
}

#[test]
fn typed_parameter_writes_round_trip_through_the_session() {
    let session = plain_param_session();

    session
        .command("-PARAMETERS NEW \"My width\" length 2 * 3")
        .expect("create");
    let list = session.param_list().expect("list");
    assert_eq!(list.total, 1);
    assert_eq!(list.rows[0].name, "My width");
    assert_eq!(list.rows[0].kind, "length");
    assert_eq!(list.rows[0].value, 6.0);
    assert_eq!(list.rows[0].seq, 1);

    session
        .command("-PARAMETERS SET \"My width\" 10")
        .expect("set value");
    assert_eq!(session.param_list().expect("list").rows[0].value, 10.0);

    session
        .command("-PARAMETERS NEW q length 4")
        .expect("create q");
    session
        .command("-PARAMETERS SET \"My width\" q * 5")
        .expect("set expression");
    assert_eq!(
        session.param_list().expect("list").rows[0].value,
        20.0,
        "the expression follows its referenced row"
    );

    session
        .command("-PARAMETERS RENAME \"My width\" w")
        .expect("rename");
    let list = session.param_list().expect("list");
    assert_eq!(list.rows[0].name, "w");
    assert_eq!(list.rows[0].value, 20.0, "rename keeps the values");

    // `w` still references `q`, so `q` is not deletable until that edge is gone.
    let error = session
        .command("-PARAMETERS DELETE q")
        .expect_err("a referenced row is refused");
    assert!(error.contains("param.referenced"), "got {error}");
    session
        .command("-PARAMETERS SET w 20")
        .expect("replace the expression with a literal");
    session.command("-PARAMETERS DELETE q").expect("delete q");
    session.command("-PARAMETERS DELETE w").expect("delete w");
    assert_eq!(session.param_list().expect("list").total, 0);
}

#[test]
fn typed_parameter_writes_refuse_by_name_and_change_nothing() {
    let session = plain_param_session();
    session
        .command("-PARAMETERS NEW r length 2")
        .expect("create");

    let error = session
        .command("-PARAMETERS NEW r length 3")
        .expect_err("duplicate create");
    assert!(error.contains("param.duplicate_name"), "got {error}");

    let error = session
        .command("-PARAMETERS SET missing 1")
        .expect_err("unknown name");
    assert!(error.contains("param.unknown_name"), "got {error}");

    let error = session
        .command("-PARAMETERS SET 9 1")
        .expect_err("unknown sequence");
    assert!(error.contains("param.unknown_reference"), "got {error}");

    let error = session
        .command("-PARAMETERS NEW y length nope + 1")
        .expect_err("unknown expression name");
    assert!(error.contains("param.unknown_reference"), "got {error}");

    let error = session
        .command("-PARAMETERS FROB r")
        .expect_err("bad grammar");
    assert!(error.contains("unsupported CAD command"), "got {error}");

    let list = session.param_list().expect("list");
    assert_eq!(list.total, 1, "every refusal wrote nothing");
    assert_eq!(list.rows[0].name, "r");
    assert_eq!(list.rows[0].value, 2.0);
}

#[test]
fn the_bare_dash_parameters_report_is_admitted_and_read_only() {
    let session = param_session();
    let revision = || {
        session
            .core
            .controller
            .borrow()
            .drawing()
            .map(|drawing| drawing.revision())
    };
    let before = revision();
    session
        .command("-PARAMETERS")
        .expect("the list report runs");
    assert_eq!(revision(), before, "the report never bumps the revision");
}
