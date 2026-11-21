//! Document-level parameter list host reads (ADR-0043 P1).
//!
//! The host projects the base table plus the active configuration's status; these
//! contracts pin the row order, each status word, the suppressed-row base value,
//! the active name, and the no-document empty answer.

use super::*;

use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::param::{
    Expr, ParamBindings, ParamConfig, ParamConfigTable, ParamKind, ParamRow, ParamSeq, ParamTable,
};
use cad_domain::{
    DatabaseId, EntityId, LayerId, ObjectId, Revision, SemanticGeometry, SpaceId, TransactionId,
};

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
            center: p(0.0, 0.0),
            normal: cad_domain::Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
        draw_order: 0,
    }
}

/// Base rows `r=20`, `q=10`, `s=7`; a circle bound to `r`; optional configs:
/// `mix` overrides `r=5` and suppresses `s`.
fn param_database(activate_mix: bool) -> DrawingDatabase {
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
    if activate_mix {
        database
            .set_active_configuration(Some("mix".to_string()), "test", TransactionId(1))
            .expect("active config");
    }
    database
}

fn param_core(activate_mix: bool) -> HostCore {
    let core = core();
    {
        let mut controller = core.controller.borrow_mut();
        let document = controller.document_id;
        controller
            .application
            .workspace
            .documents
            .get_mut(&document)
            .expect("the host always has a document")
            .drawing = Arc::new(param_database(activate_mix));
    }
    core
}

/// A production-shaped core with no document at startup.
fn no_document_core() -> HostCore {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let mut options = options();
    options.document_at_start = StartDocument::Empty;
    HostCore::new(options, platform, RenderSessionHandle::default()).expect("empty host core")
}

#[test]
fn the_base_table_lists_every_row_as_inherited() {
    let list = param_core(false).param_list().expect("the host answers");
    assert_eq!(list.active, None);
    assert_eq!(list.total, 3, "total counts the base rows");
    let rows: Vec<(&str, &str, f64)> = list
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.status.as_str(), row.value))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("r", "inherited", 20.0),
            ("q", "inherited", 10.0),
            ("s", "inherited", 7.0)
        ],
        "ascending sequence order, base values"
    );
    assert!(list.rows.iter().all(|row| row.kind == "length"));
    assert!(list
        .rows
        .iter()
        .all(|row| row.unit.as_deref() == Some("mm")));
}

#[test]
fn the_active_configuration_marks_overridden_inherited_and_suppressed_rows() {
    let list = param_core(true).param_list().expect("the host answers");
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
            // The override is the effective value.
            ("r", "overridden", 5.0),
            // `q` is referenced by no binding and no config: inherited.
            ("q", "inherited", 10.0),
            // `s` is removed from the effective table but still listed at its
            // base value.
            ("s", "suppressed", 7.0),
        ]
    );
    assert_eq!(
        list.rows[0].seq, 1,
        "the wire row carries the table sequence"
    );
}

#[test]
fn no_document_answers_an_empty_list() {
    let core = no_document_core();
    assert!(core.controller.borrow().drawing().is_none());
    let list = core.param_list().expect("no document is not an error");
    assert_eq!(list.active, None);
    assert!(list.rows.is_empty());
    assert_eq!(list.total, 0);
}
