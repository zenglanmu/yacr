//! Headless parametric-evaluation acceptance (ADR-0043 P0).
//!
//! The CLI crate is the windowless host: this test builds a database, commits a
//! parameter + binding through the one merged write path, evaluates the table
//! and re-derives the element geometry with **no GPU, no window and no carrier
//! adapter**. It is the evidence that the evaluator is a pure function of the
//! database (`docs/adr/0043-parametric-dependency.md` D5/D7); it is not a
//! rendering, pixel or performance claim.

use cad_db::{
    DbEntity, DbObject, DrawingDatabaseBuilder, Layer, Layout, ParamBindingPatch, ParamPatch,
};
use cad_domain::param::{self, BinaryOp, Expr, ParamBindings, ParamRow, ParamSeq, ParamTable};
use cad_domain::{
    DatabaseId, EntityId, LayerId, LayoutId, Point3, Revision, SemanticGeometry, SpaceId,
    TransactionId,
};

fn circle(radius: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: cad_domain::ObjectId(1),
            type_key: "AcDbCircle".to_string(),
            revision: Revision(0),
            source_handle: Some("1".to_string()),
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Circle {
            center: Point3::default(),
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

#[test]
fn a_parameter_binding_re_derives_geometry_without_a_gpu_or_window() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(0),
            name: "Model".to_string(),
            viewports: vec![],
        })
        .unwrap();
    builder.insert_entity(circle(10.0)).unwrap();
    let mut drawing = builder.finish().unwrap();

    let table = ParamTable::from_rows([
        ParamRow::new(
            ParamSeq(1),
            "plate",
            param::ParamKind::Length,
            Expr::num(12.5),
        )
        .unwrap(),
        ParamRow::new(
            ParamSeq(2),
            "hole",
            param::ParamKind::Length,
            Expr::binary(BinaryOp::Div, Expr::reference(ParamSeq(1)), Expr::num(2.0)),
        )
        .unwrap(),
    ])
    .unwrap();

    let outcome = drawing
        .apply_parametric_update(
            "headless bind",
            TransactionId(3),
            &table
                .rows()
                .map(|row| ParamPatch {
                    seq: row.seq,
                    before: None,
                    after: Some(row.clone()),
                })
                .collect::<Vec<_>>(),
            &[ParamBindingPatch {
                id: EntityId(1),
                before: None,
                after: Some(
                    ParamBindings::from_entries([(
                        "radius".to_string(),
                        Expr::reference(ParamSeq(2)),
                    )])
                    .unwrap(),
                ),
                before_geometry: None,
                after_geometry: None,
            }],
        )
        .unwrap();

    // The table evaluated deterministically and the circle took 12.5 / 2.
    assert_eq!(outcome.values.get(ParamSeq(1)).unwrap().value, 12.5);
    assert_eq!(outcome.values.get(ParamSeq(2)).unwrap().value, 6.25);
    match drawing.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Circle { radius, .. } => assert_eq!(radius, 6.25),
        ref other => panic!("unexpected geometry {other:?}"),
    }
    assert_eq!(outcome.updated, vec![EntityId(1)]);
    assert!(outcome.diagnostics.is_empty());
}
