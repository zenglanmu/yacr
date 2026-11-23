//! Contract tests for the SketchUp export writer.
//!
//! The reader side (this same crate's root importer) is used only as a
//! **self-consistency** oracle: it proves the bytes are the container the pinned
//! `skpwright` 0.6.0 models, nothing more. No SketchUp/Trimble product opened
//! these files, so a passing round trip is **not** vendor acceptance, and no
//! pixel, GPU or measurement claim follows from it.

use cad_connector_core::{
    ExportJob, ExportLimits, ExportOutput, ExportSpace, Exporter, ImportLimits, ImportRequest,
    Importer,
};
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::{
    Body3d, BodyDisplay, BodyExact, BodyKind, CadError, CadResult, Completeness, DatabaseId,
    DocumentId, EntityId, LayerId, Mesh, ObjectId, Point3, Precision, Revision, SemanticGeometry,
    SpaceId, UnitContext,
};

const SKP_MAGIC: [u8; 3] = [0xFF, 0xFE, 0xFF];

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// One counter-clockwise triangle in the XY plane.
fn triangle() -> Mesh {
    Mesh {
        vertices: vec![
            point(0.0, 0.0, 0.0),
            point(1.0, 0.0, 0.0),
            point(0.0, 1.0, 0.0),
        ],
        triangles: vec![[0, 1, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

/// A database with layer `0` (plus layer `1` for the flattening test).
fn database(entities: Vec<(SemanticGeometry, LayerId)>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(1), "A-WALL")] {
        builder
            .insert_layer(Layer {
                id,
                name: name.into(),
                visible: true,
            })
            .unwrap();
    }
    for (index, (geometry, layer)) in entities.into_iter().enumerate() {
        let id = ObjectId(index as u128 + 1);
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id,
                    type_key: type_key(&geometry).into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(id.0),
                layer,
                space: SpaceId::Model,
                geometry,
                draw_order: index as i64,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn type_key(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Mesh(_) => "AcDbSubDMesh",
        SemanticGeometry::Line { .. } => "AcDbLine",
        SemanticGeometry::Body3d(_) => "AcDb3dSolid",
        _ => "AcDbEntity",
    }
}

fn meter() -> UnitContext {
    UnitContext::from_meter()
}

fn job<'a>(database: &'a DrawingDatabase, units: Option<&'a UnitContext>) -> ExportJob<'a> {
    ExportJob {
        database,
        document_id: DocumentId(1),
        units,
        space: ExportSpace::Model,
        limits: ExportLimits::default(),
    }
}

fn export(database: &DrawingDatabase, units: Option<&UnitContext>) -> CadResult<ExportOutput> {
    let job = job(database, units);
    crate::SkpExporter::new(&crate::DESCRIPTOR_SKP).export("skp", &job)
}

fn unsupported(error: CadError) -> String {
    let CadError::Unsupported(message) = error else {
        panic!("expected CadError::Unsupported, got {error:?}");
    };
    message
}

fn codes(output: &ExportOutput) -> Vec<&str> {
    output
        .report
        .entries
        .iter()
        .map(|e| e.code.as_str())
        .collect()
}

/// The mesh an entity displays, whichever carrier the importer chose.
///
/// The writer always emits the face ring's `MeshEdge`s, so a read-back triangle
/// is a `Body3d` display body (ADR-0049), not a bare `Mesh`; the export oracle
/// must look at the display geometry in both cases.
fn display_mesh(entity: &DbEntity) -> Option<&Mesh> {
    match &entity.geometry {
        SemanticGeometry::Mesh(mesh) => Some(mesh),
        SemanticGeometry::Body3d(body) => Some(&body.display.mesh),
        _ => None,
    }
}

fn mesh_triangles(database: &DrawingDatabase) -> usize {
    database
        .entities()
        .filter_map(display_mesh)
        .map(|mesh| mesh.triangles.len())
        .sum()
}

fn import(bytes: Vec<u8>) -> cad_domain::CadResult<cad_connector_core::ImportedDrawing> {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: std::sync::Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    crate::SkpImporter.import(&request, &|| false)
}

// ---- Shared fixtures ------------------------------------------------------

fn one_edge() -> Vec<Vec<Point3>> {
    vec![vec![point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)]]
}

fn display_body(kind: BodyKind, mesh: Mesh, edges: Vec<Vec<Point3>>) -> Body3d {
    Body3d {
        kind,
        exact: BodyExact::Unavailable {
            reason: "synthetic".into(),
        },
        display: BodyDisplay {
            mesh,
            edges,
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Partial(vec!["synthetic".into()]),
        },
    }
}

fn parsed_model(output: &ExportOutput) -> skpwright::Model {
    skpwright::Model::parse(&output.bytes).expect("the written container parses")
}

/// The canonical undirected vertex pairs of a parsed run's edges.
fn canonical_edges(model: &skpwright::Model) -> Vec<(u32, u32)> {
    model.geometry[0]
        .mesh
        .edges
        .iter()
        .map(|edge| (edge.v0.min(edge.v1), edge.v0.max(edge.v1)))
        .collect()
}

mod basic;
mod geometry;
mod wire;
