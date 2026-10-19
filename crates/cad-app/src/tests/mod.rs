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

mod cjk_baseline;
mod commands;
mod drawing;
mod drawing_space;
mod view_camera;
