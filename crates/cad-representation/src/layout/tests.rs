use super::*;
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer, PaperViewport};
use cad_domain::*;

use crate::RepresentationContext;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn viewport(clip: Vec<Point3>, scale: f64) -> PaperViewport {
    PaperViewport {
        clip,
        model_to_paper: Transform3::scale(scale),
        completeness: Completeness::Complete,
    }
}

/// Paper→model transform `model = scale * paper + translation`, matching the
/// direction the importer stores (`view_height / paper_height`).
fn stored_view(scale: f64, translation: Point3) -> Transform3 {
    let mut m = Transform3::scale(scale).matrix;
    m[0][3] = translation.x;
    m[1][3] = translation.y;
    m[2][3] = translation.z;
    Transform3 { matrix: m }
}

/// The importer's shape: two opposite paper corners + the model view centre,
/// with a model-units-per-paper-unit scale.
fn simple_viewport() -> PaperViewport {
    // Paper rectangle (0,0)-(100,50), model centre at (10, 20), 1:100 => the
    // stored ratio is 100 model units per paper unit.
    viewport(vec![p(0.0, 0.0), p(100.0, 50.0), p(10.0, 20.0)], 100.0)
}

/// A real four-corner 1:100 viewport: paper (0,0)-(100,50), model view
/// centre (10,20). The stored paper→model transform is `scale(100)` shifted
/// so that `paper_center = (50,25)` maps to `(10,20)`.
fn four_corner_viewport() -> PaperViewport {
    let translation = Point3 {
        x: 10.0 - 100.0 * 50.0,
        y: 20.0 - 100.0 * 25.0,
        z: 0.0,
    };
    PaperViewport {
        clip: vec![p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)],
        model_to_paper: stored_view(100.0, translation),
        completeness: Completeness::Complete,
    }
}
/// Exactly the shape the importer now writes for a 1:100 top/plan viewport:
/// paper (0,0)-(100,50), model view target (10,20), model_per_paper = 100.
fn imported_one_to_one_hundred() -> PaperViewport {
    let center = Point3 {
        x: 50.0,
        y: 25.0,
        z: 0.0,
    };
    let target = Point3 {
        x: 10.0,
        y: 20.0,
        z: 0.0,
    };
    let scale = 100.0;
    let clip = vec![p(0.0, 0.0), p(100.0, 0.0), p(100.0, 50.0), p(0.0, 50.0)];
    let mut m = Transform3::identity().matrix;
    m[0][0] = scale;
    m[1][1] = scale;
    m[0][3] = target.x - scale * center.x;
    m[1][3] = target.y - scale * center.y;
    PaperViewport {
        clip,
        model_to_paper: Transform3 { matrix: m },
        completeness: Completeness::Complete,
    }
}

// ---- unsupported states ------------------------------------------------

fn unsupported_code(viewport: &PaperViewport) -> &'static str {
    match viewport_transform(viewport) {
        ViewportState::Unsupported(why) => why.code,
        ViewportState::Supported(_) => panic!("expected an unsupported viewport"),
    }
}

fn db_with_layouts() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: p(0.0, 0.0),
            end: p(1.0, 0.0),
        },
        draw_order: 0,
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(1),
        name: "Layout1".into(),
        viewports: vec![four_corner_viewport()],
    })
    .unwrap();
    b.insert_layout(cad_db::Layout {
        id: LayoutId(2),
        name: "Layout2".into(),
        viewports: vec![viewport(vec![p(0.0, 0.0), p(10.0, 10.0), p(0.0, 0.0)], 0.0)],
    })
    .unwrap();
    b.finish().unwrap()
}

fn context() -> RepresentationContext {
    RepresentationContext::new(
        DocumentId(1),
        TolerancePolicy::default(),
        TaskStamp::new(DocumentId(1), 0),
    )
}

fn triangle_mesh(corners: [Point3; 3]) -> Mesh {
    Mesh {
        vertices: corners.to_vec(),
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    }
}

fn mesh_area(mesh: &Mesh) -> f64 {
    mesh.triangles
        .iter()
        .map(|t| {
            let a = mesh.vertices[t[0] as usize];
            let b = mesh.vertices[t[1] as usize];
            let c = mesh.vertices[t[2] as usize];
            ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() * 0.5
        })
        .sum()
}

mod build;
mod image;
mod viewport;
