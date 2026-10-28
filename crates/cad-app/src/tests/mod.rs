//! cad-app contract tests.

use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer, Layout, PaperViewport};

// Aliases kept local so the tests read without re-importing camera helpers.
fn cam_xy_work_plane(z: f64) -> WorkPlane {
    xy_work_plane(z)
}
fn cad_app_len(v: Point3) -> f64 {
    camera::length3(v)
}
fn cad_app_dot(a: Point3, b: Point3) -> f64 {
    camera::dot3(a, b)
}

fn application_with_document() -> (Application, SessionState) {
    let mut app = Application::new();
    let document_id = DocumentId(1);
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    let drawing = builder.finish().unwrap();
    app.workspace.documents.insert(
        document_id,
        Document {
            id: document_id,
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([0u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), document_id, [800.0, 600.0]),
    );
    let session = SessionState::new(document_id, AppMode::Work);
    (app, session)
}

fn command(id: CommandId, payload: CommandPayload) -> Command {
    Command {
        schema_version: 1,
        id,
        document: DocumentId(1),
        viewport: ViewportId(1),
        payload,
    }
}

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// An application whose single document's bounds are exactly `min..max`.
///
/// Used by the view contracts (`FIT`, `ZOOM WINDOW`, `ZOOM PREVIOUS`), which need
/// a real extent to frame rather than an empty drawing.
fn application_with_bounds(min: Point3, max: Point3) -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(1),
            type_key: "AcDbPolyline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Polyline {
            points: vec![
                min,
                Point3 {
                    x: max.x,
                    y: min.y,
                    z: min.z,
                },
                max,
                Point3 {
                    x: min.x,
                    y: max.y,
                    z: min.z,
                },
            ],
            bulges: Vec::new(),
            closed: true,
        },
        draw_order: 0,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

fn application_with_layouts() -> (Application, SessionState) {
    let (mut app, session) = application_with_document();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let corners = || {
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 100.0,
                y: 50.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 20.0,
                z: 0.0,
            },
        ]
    };
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: vec![PaperViewport {
            clip: corners(),
            model_to_paper: Transform3::scale(100.0),
            completeness: Completeness::Complete,
        }],
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(2),
        name: "Broken".into(),
        viewports: vec![PaperViewport {
            clip: corners(),
            model_to_paper: Transform3::scale(100.0),
            completeness: Completeness::Partial(vec!["importer dropped height".into()]),
        }],
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
    (app, session)
}

mod area_object;
mod cjk_baseline;
mod commands;
mod drawing;
mod drawing_space;
mod grips;
mod grips_curves;
mod grips_dimension;
mod grips_ellipse;
mod grips_insert;
mod grips_polyline;
mod grips_spline;
mod grips_text;
mod id_report;
mod layer_current_visibility;
mod layer_property;
mod layers;
mod layers_isolate;
mod layers_off;
mod layers_previous;
mod locked_layers;
mod point_display_style;
mod view_camera;
mod zoom_previous;
mod zoom_window;
