//! Contract tests: frame/units, container shape, determinism and losses.
//!
//! The shared fixtures and the glTF/aspect readers live here; the cases are
//! split by subject (`geometry`, `attributes`, `materials`, `textures`) so no
//! single file has to carry every contract. Each case file pulls the prelude in
//! with `use super::*;`.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use cad_db::{
    DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, EntityColor,
    EntityRenderAttributes, EntityTransparency, Layer,
};
use cad_domain::{
    DatabaseId, DocumentId, EntityId, LayerId, Mesh, ObjectId, Point3, Revision, SemanticGeometry,
    SpaceId, UnitContext,
};
use cad_import_core::{
    ExportJob, ExportLimits, ExportOutput, ExportSpace, Exporter, ImportLimits, ImportRequest,
    ImportedDrawing, Importer,
};
use cad_import_gltf::{GltfImporter, DESCRIPTOR_GLTF};
use gltf::buffer::Source;

use super::{GltfContainer, GltfExporter};

const DATA_URI_PREFIX: &str = "data:application/octet-stream;base64,";

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn entity(object: u128, type_key: &str, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(object),
            type_key: type_key.to_string(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(object),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn database(entities: Vec<DbEntity>) -> DrawingDatabase {
    database_with_render(entities, Vec::new())
}

/// The same database, plus the importer-resolved display attributes.
fn database_with_render(
    entities: Vec<DbEntity>,
    render: Vec<(EntityId, EntityRenderAttributes)>,
) -> DrawingDatabase {
    database_with_textures(entities, render, Vec::new())
}

/// The same database, plus document-owned texture bytes.
fn database_with_textures(
    entities: Vec<DbEntity>,
    render: Vec<(EntityId, EntityRenderAttributes)>,
    textures: Vec<(&str, Vec<u8>)>,
) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    for (id, attributes) in render {
        builder
            .set_entity_render_attributes(id, attributes)
            .unwrap();
    }
    for (key, bytes) in textures {
        builder
            .insert_texture(key, Arc::from(bytes.into_boxed_slice()))
            .unwrap();
    }
    builder.finish().unwrap()
}

/// Render attributes with an explicit sRGB colour and optional explicit opacity.
///
/// `None` for `alpha` keeps the default `ByLayer` opacity, which the writer
/// must treat as fully opaque.
fn explicit(color: [u8; 3], alpha: Option<f32>) -> EntityRenderAttributes {
    EntityRenderAttributes {
        color: EntityColor::Explicit(color),
        transparency: alpha.map_or(EntityTransparency::ByLayer, EntityTransparency::Explicit),
        ..Default::default()
    }
}

/// One triangle whose first vertex is the CAD point `(1, 2, 3)`.
fn triangle_mesh() -> Mesh {
    Mesh {
        vertices: vec![
            point(1.0, 2.0, 3.0),
            point(2.0, 2.0, 3.0),
            point(1.0, 3.0, 3.0),
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

fn mesh_database(mesh: Mesh) -> DrawingDatabase {
    database(vec![entity(1, "AcDbMesh", SemanticGeometry::Mesh(mesh))])
}

fn export_with(
    container: GltfContainer,
    database: &DrawingDatabase,
    units: Option<&UnitContext>,
) -> ExportOutput {
    let job = ExportJob {
        database,
        document_id: DocumentId(1),
        units,
        space: ExportSpace::Model,
        limits: ExportLimits::default(),
    };
    GltfExporter::new(container, &DESCRIPTOR_GLTF)
        .export(container.format_id(), &job)
        .expect("export should succeed")
}

fn export(container: GltfContainer, database: &DrawingDatabase) -> ExportOutput {
    let units = UnitContext::from_meter();
    export_with(container, database, Some(&units))
}

/// Run the written bytes back through `cad_import_gltf`, the only reader that
/// defines what these files mean, so a round trip is an actual fixed point.
fn import_bytes(bytes: Vec<u8>) -> ImportedDrawing {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    GltfImporter::new()
        .import(&request, &|| false)
        .expect("the exported glTF imports")
}

/// The single mesh the importer produced.
fn imported_mesh(drawing: &ImportedDrawing) -> &Mesh {
    let mut meshes = drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        });
    let mesh = meshes.next().expect("one imported mesh");
    assert!(meshes.next().is_none(), "exactly one imported mesh");
    mesh
}

/// Resolve every buffer's bytes without the `import` feature: a GLB buffer is
/// the `BIN` chunk, a `.gltf` buffer is the embedded base64 data URI.
fn buffer_bytes(gltf: &gltf::Gltf) -> Vec<Vec<u8>> {
    gltf.buffers()
        .map(|buffer| match buffer.source() {
            Source::Bin => gltf.blob.clone().expect("GLB has a BIN chunk"),
            Source::Uri(uri) => {
                let payload = uri
                    .strip_prefix(DATA_URI_PREFIX)
                    .unwrap_or_else(|| panic!("unexpected buffer URI: {uri}"));
                STANDARD.decode(payload).expect("valid base64 payload")
            }
        })
        .collect()
}

fn accessor<'a>(gltf: &'a gltf::Gltf, index: usize) -> gltf::Accessor<'a> {
    gltf.accessors().nth(index).expect("accessor exists")
}

fn component_slice<'a>(
    accessor: &gltf::Accessor<'_>,
    buffers: &'a [Vec<u8>],
    i: usize,
) -> &'a [u8] {
    let view = accessor.view().expect("accessor has a bufferView");
    let data = &buffers[view.buffer().index()];
    let stride = view.stride().unwrap_or(accessor.size());
    let start = view.offset() + accessor.offset() + i * stride;
    &data[start..start + accessor.size()]
}

fn read_vec2_f32(gltf: &gltf::Gltf, buffers: &[Vec<u8>], index: usize) -> Vec<[f32; 2]> {
    let accessor = accessor(gltf, index);
    assert_eq!(accessor.dimensions(), gltf::accessor::Dimensions::Vec2);
    assert_eq!(accessor.data_type(), gltf::accessor::DataType::F32);
    assert!(!accessor.normalized(), "UV0 is not normalized");
    (0..accessor.count())
        .map(|i| {
            let bytes = component_slice(&accessor, buffers, i);
            [
                f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            ]
        })
        .collect()
}

fn read_vec3_f32(gltf: &gltf::Gltf, buffers: &[Vec<u8>], index: usize) -> Vec<[f32; 3]> {
    let accessor = accessor(gltf, index);
    assert_eq!(accessor.dimensions(), gltf::accessor::Dimensions::Vec3);
    assert_eq!(accessor.data_type(), gltf::accessor::DataType::F32);
    (0..accessor.count())
        .map(|i| {
            let bytes = component_slice(&accessor, buffers, i);
            [
                f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            ]
        })
        .collect()
}

fn read_vec4_f32(gltf: &gltf::Gltf, buffers: &[Vec<u8>], index: usize) -> Vec<[f32; 4]> {
    let accessor = accessor(gltf, index);
    assert_eq!(accessor.dimensions(), gltf::accessor::Dimensions::Vec4);
    assert_eq!(accessor.data_type(), gltf::accessor::DataType::F32);
    assert!(!accessor.normalized());
    (0..accessor.count())
        .map(|i| {
            let bytes = component_slice(&accessor, buffers, i);
            [
                f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                f32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            ]
        })
        .collect()
}

fn read_scalar_u32(gltf: &gltf::Gltf, buffers: &[Vec<u8>], index: usize) -> Vec<u32> {
    let accessor = accessor(gltf, index);
    assert_eq!(accessor.dimensions(), gltf::accessor::Dimensions::Scalar);
    assert_eq!(accessor.data_type(), gltf::accessor::DataType::U32);
    (0..accessor.count())
        .map(|i| {
            let bytes = component_slice(&accessor, buffers, i);
            u32::from_le_bytes(bytes[0..4].try_into().unwrap())
        })
        .collect()
}

// Cases, split by subject. Each file starts from `use super::*;`.
mod attributes;
mod geometry;
mod materials;
mod pbr;
mod textures;
