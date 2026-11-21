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
