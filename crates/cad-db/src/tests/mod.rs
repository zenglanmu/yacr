//! Unit tests.

use super::*;
use cad_domain::param::{Expr, ParamBindings, ParamKind, ParamRow, ParamSeq, ParamTable};
use cad_domain::*;

mod attribute;
mod bim_edit;
mod block_edit;
mod bounds;
mod builder;
mod deletion_guards;
mod display;
mod domain;
mod drawing;
mod drawing_clip;
mod drawing_display;
mod guides;
mod header_geometry;
mod history_replay;
mod identity;
mod layer_attributes;
mod layer_inherit;
mod layer_patch;
mod layers;
mod layout_edit;
mod layout_patch;
mod locked_layers;
mod materials;
mod named_delete;
mod param;
mod param_config;
mod pbr;
mod plot;
mod point_style;
mod resources;
mod scale;
mod scale_edit;
mod scenes;
mod scope;
mod spline_transform;
mod spline_validation;
mod style_tables;
mod table_validation;
mod text_transform;
mod textures;
mod validate_geometry;
mod views;
mod visibility;
mod xref;

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn raw_entity(id: u128, space: SpaceId, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: id as i64,
    }
}

fn line_entity(id: u128, start: Point3, end: Point3) -> DbEntity {
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
        geometry: SemanticGeometry::Line { start, end },
        draw_order: id as i64,
    }
}

/// A database with layer 0, layout 1 and block 0 (no members yet).
fn writable_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(crate::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.finish().unwrap()
}

// ------------------------------------------------- shared param fixtures ---

fn row(seq: u64, name: &str, expr: Expr) -> ParamRow {
    ParamRow::new(ParamSeq(seq), name, ParamKind::Length, expr).expect("row")
}

fn circle(radius: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbCircle".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Circle {
            center: Point3::default(),
            normal: Point3::default(),
            radius,
        },
        draw_order: 0,
    }
}

/// Layer 0, layout 1 and one circle with the given radius; no parameters yet.
fn base(radius: f64) -> DrawingDatabaseBuilder {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .expect("layer");
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .expect("layout");
    b.insert_entity(circle(radius)).expect("entity");
    b
}

fn bindings(slot: &str, expr: Expr) -> ParamBindings {
    ParamBindings::from_entries([(slot.to_string(), expr)]).expect("bindings")
}

fn radius_of(db: &DrawingDatabase) -> f64 {
    match db.entities[&EntityId(1)].geometry {
        SemanticGeometry::Circle { radius, .. } => radius,
        ref other => panic!("expected a circle, got {other:?}"),
    }
}

/// One parameter (`r = 10`) bound to the circle's radius.
fn bound_db() -> DrawingDatabase {
    let mut parameters = ParamTable::new();
    parameters
        .insert(row(7, "r", Expr::num(10.0)))
        .expect("row");
    let mut b = base(10.0);
    b.set_parameters(parameters).expect("parameters");
    b.set_param_bindings(
        EntityId(1),
        bindings("radius", Expr::reference(ParamSeq(7))),
    )
    .expect("bindings");
    b.finish().expect("finish")
}
