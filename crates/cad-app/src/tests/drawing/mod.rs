//! Drawing/edit command contract tests (spec F-EDIT, `docs/drawing-edit.md` §2/§3).

use super::*;

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}
use cad_db::{BlockDefinition, DbEntity, DbObject, DynamicBlockState, DynamicBlockVisibility};

fn ent(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: order,
    }
}

/// A drawing with layer 0, plus a block (one line child) and an INSERT.
fn drawing_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(0),
        entities: vec![EntityId(100)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(ent(
        100,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        1,
    ))
    .unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: 1.0,
        },
        2,
    ))
    .unwrap();
    b.insert_entity(ent(
        3,
        SemanticGeometry::Insert {
            block: cad_domain::BlockId(0),
            transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
        },
        3,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([1u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn ref_of(entity: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn single_line_app(attributes: Option<cad_db::EntityRenderAttributes>) -> Application {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    ))
    .unwrap();
    if let Some(attributes) = attributes {
        b.set_entity_render_attributes(EntityId(1), attributes)
            .unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([3u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    app
}

fn non_default_attributes() -> cad_db::EntityRenderAttributes {
    use cad_db::{
        EntityColor, EntityLineType, EntityLineWeight, EntityRenderAttributes, EntityTransparency,
        LinetypePattern,
    };
    EntityRenderAttributes {
        transparency: EntityTransparency::Explicit(0.5),
        color: EntityColor::Explicit([10, 20, 30]),
        lineweight: EntityLineWeight::Explicit(0.7),
        linetype: EntityLineType::Explicit {
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            scale: 2.0,
        },
        geometry_source: cad_domain::GeometrySource::Analytic,
        annotative: Default::default(),
    }
}

/// One target line (0,0)->(10,0) on layer 0 plus a boundary entity.
fn line_and_boundary_app(
    boundary: DbEntity,
    attributes: Option<cad_db::EntityRenderAttributes>,
) -> Application {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    ))
    .unwrap();
    if let Some(attributes) = attributes {
        b.set_entity_render_attributes(EntityId(1), attributes)
            .unwrap();
    }
    b.insert_entity(boundary).unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([4u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    app
}

/// A supported line (id 1) plus an unsupported Opaque entity (id 2).
fn line_and_opaque_app() -> Application {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Opaque {
            type_key: "ACAD_PROXY_ENTITY".into(),
            version: 1,
            payload: vec![1, 2, 3],
        },
        1,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([6u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    app
}

mod arc_extend;
mod clipboard;
mod create_trim;
mod offset;
mod reports;
mod selection_reconcile;
mod transform;
