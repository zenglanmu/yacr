//! Contract tests: frame/units, container shape, determinism and losses.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use cad_db::{
    DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, EntityColor,
    EntityRenderAttributes, EntityTransparency, Layer,
};
use cad_domain::{
    Completeness, DatabaseId, DocumentId, EntityId, LayerId, Mesh, ObjectId, Point3, Revision,
    SemanticGeometry, SpaceId, SupportStatus, UnitContext,
};
use cad_import_core::{
    ExportJob, ExportLimits, ExportOutput, ExportSpace, Exporter, ImportLimits, ImportRequest,
    ImportedDrawing, Importer,
};
use cad_import_gltf::{GltfImporter, DESCRIPTOR_GLTF};
use gltf::buffer::Source;
use gltf::Semantic;

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

#[test]
fn glb_round_trips_positions_in_the_y_up_frame() {
    let database = mesh_database(triangle_mesh());
    let output = export(GltfContainer::Glb, &database);

    assert!(output.bytes.starts_with(b"glTF"));
    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.meshes().count(), 1);
    let mesh = gltf.meshes().next().unwrap();
    assert_eq!(mesh.primitives().count(), 1);
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.mode(), gltf::mesh::Mode::Triangles);

    let position = primitive.get(&Semantic::Positions).expect("POSITION");
    assert_eq!(position.count(), 3);
    assert!(position.min().is_some(), "POSITION must carry min");
    assert!(position.max().is_some(), "POSITION must carry max");

    let buffers = buffer_bytes(&gltf);
    let positions = read_vec3_f32(&gltf, &buffers, position.index());
    // CAD (1, 2, 3) -> glTF (1, 3, -2) under (x, y, z) -> (x, z, -y).
    assert_eq!(positions[0], [1.0, 3.0, -2.0]);
    let indices = primitive.indices().expect("indices");
    assert_eq!(read_scalar_u32(&gltf, &buffers, indices.index()), [0, 1, 2]);

    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.dropped, 0);
    assert!(matches!(output.report.completeness, Completeness::Complete));
    let codes: Vec<&str> = output
        .report
        .entries
        .iter()
        .map(|entry| entry.code.as_str())
        .collect();
    assert!(codes.contains(&"export.gltf.units_to_metres"));
    assert!(codes.contains(&"export.gltf.frame_to_y_up"));
}

#[test]
fn gltf_embeds_the_buffer_as_a_data_uri() {
    let database = mesh_database(triangle_mesh());
    let output = export(GltfContainer::Gltf, &database);

    assert!(!output.bytes.starts_with(b"glTF"));
    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid .gltf");
    let buffer = gltf.buffers().next().expect("one buffer");
    let Source::Uri(uri) = buffer.source() else {
        panic!("a .gltf buffer must be an external (data) URI");
    };
    assert!(uri.starts_with(DATA_URI_PREFIX), "unexpected URI: {uri}");

    let buffers = buffer_bytes(&gltf);
    assert_eq!(buffers[0].len(), buffer.length());
    assert_eq!(buffer.length() % 4, 0, "the buffer is 4-byte padded");

    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let position = primitive.get(&Semantic::Positions).expect("POSITION");
    let positions = read_vec3_f32(&gltf, &buffers, position.index());
    assert_eq!(positions[0], [1.0, 3.0, -2.0]);
}

#[test]
fn exporting_twice_yields_identical_bytes() {
    let database = mesh_database(triangle_mesh());
    for container in [GltfContainer::Glb, GltfContainer::Gltf] {
        let first = export(container, &database);
        let second = export(container, &database);
        assert_eq!(
            first.bytes, second.bytes,
            "{container:?} is not deterministic"
        );
    }
}

#[test]
fn unknown_units_are_refused() {
    let database = mesh_database(triangle_mesh());
    let unknown = UnitContext::drawing_units();
    for units in [None, Some(&unknown)] {
        let job = ExportJob {
            database: &database,
            document_id: DocumentId(1),
            units,
            space: ExportSpace::Model,
            limits: ExportLimits::default(),
        };
        let error = GltfExporter::new(GltfContainer::Glb, &DESCRIPTOR_GLTF)
            .export("glb", &job)
            .expect_err("unknown units must be refused");
        assert!(
            error.to_string().contains("export.gltf.units_unknown"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn source_units_are_scaled_to_metres() {
    let database = mesh_database(triangle_mesh());
    let millimetres = UnitContext {
        source: cad_domain::Unit::Millimeter,
        display: cad_domain::Unit::Meter,
        display_per_source: Some(0.001),
        decimal_places: 3,
    };
    let output = export_with(GltfContainer::Glb, &database, Some(&millimetres));
    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let position = gltf
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .get(&Semantic::Positions)
        .expect("POSITION");
    let positions = read_vec3_f32(&gltf, &buffers, position.index());
    assert_eq!(positions[0], [0.001, 0.003, -0.002]);
}

#[test]
fn non_mesh_entity_is_dropped_and_makes_the_report_missing() {
    let line = SemanticGeometry::Line {
        start: point(0.0, 0.0, 0.0),
        end: point(1.0, 0.0, 0.0),
    };
    let database = database(vec![
        entity(1, "AcDbMesh", SemanticGeometry::Mesh(triangle_mesh())),
        entity(2, "AcDbLine", line),
    ]);
    let output = export(GltfContainer::Glb, &database);

    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.dropped, 1);
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.gltf.unsupported_geometry")
        .expect("Line drop entry");
    assert_eq!(entry.status, SupportStatus::Unsupported);
    assert_eq!(entry.object, Some(ObjectId(2)));

    // The Line did not become a second node.
    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.meshes().count(), 1);
}

#[test]
fn a_document_without_writable_mesh_is_refused() {
    let line = SemanticGeometry::Line {
        start: point(0.0, 0.0, 0.0),
        end: point(1.0, 0.0, 0.0),
    };
    let database = database(vec![entity(1, "AcDbLine", line)]);
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: None,
        space: ExportSpace::Model,
        limits: ExportLimits::default(),
    };
    let error = GltfExporter::new(GltfContainer::Glb, &DESCRIPTOR_GLTF)
        .export("glb", &job)
        .expect_err("a document with no writable mesh must be refused");
    // Unknown units are refused first here, so supply a unit and retry to reach
    // the geometry refusal itself.
    assert!(error.to_string().contains("export.gltf.units_unknown"));

    let units = UnitContext::from_meter();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: Some(&units),
        space: ExportSpace::Model,
        limits: ExportLimits::default(),
    };
    let error = GltfExporter::new(GltfContainer::Glb, &DESCRIPTOR_GLTF)
        .export("glb", &job)
        .expect_err("a document with no writable mesh must be refused");
    assert!(
        error.to_string().contains("export.gltf.no_geometry"),
        "unexpected error: {error}"
    );
}

#[test]
fn degenerate_triangle_is_dropped_and_not_counted_exact() {
    let mesh = Mesh {
        vertices: vec![
            point(0.0, 0.0, 0.0),
            point(1.0, 0.0, 0.0),
            point(0.0, 1.0, 0.0),
        ],
        // The second facet repeats an index and is a zero-area triangle.
        triangles: vec![[0, 1, 2], [0, 1, 1]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);

    assert_eq!(output.report.counts.exact, 0);
    assert_eq!(output.report.counts.dropped, 1);
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.gltf.degenerate_triangle")
        .expect("degenerate entry");
    assert_eq!(entry.status, SupportStatus::Unsupported);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let indices = primitive.indices().expect("indices");
    assert_eq!(
        read_scalar_u32(&gltf, &buffers, indices.index()),
        [0, 1, 2],
        "the degenerate triangle must not be written"
    );
}

#[test]
fn complete_normals_and_colors_are_written() {
    let normal = point(0.0, 0.0, 1.0);
    let colors = vec![[128, 128, 128], [255, 255, 255], [0, 0, 0]];
    let mesh = Mesh {
        normals: vec![normal; 3],
        colors: colors.clone(),
        ..triangle_mesh()
    };
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let normal_accessor = primitive.get(&Semantic::Normals).expect("NORMAL");
    let normals = read_vec3_f32(&gltf, &buffers, normal_accessor.index());
    // The CAD +Z normal rotates to glTF +Y.
    assert_eq!(normals[0], [0.0, 1.0, 0.0]);
    let color_accessor = primitive.get(&Semantic::Colors(0)).expect("COLOR_0");
    let written = read_vec4_f32(&gltf, &buffers, color_accessor.index());
    // glTF COLOR_0 is linear: stored sRGB 128 is 0.21586..., not 128/255.
    assert!(
        (written[0][0] - 0.215_86).abs() < 1e-4,
        "sRGB 128 must decode to linear ~0.21586, got {}",
        written[0][0]
    );
    assert_eq!(written[0][1], written[0][0]);
    assert_eq!(written[0][2], written[0][0]);
    assert_eq!(written[0][3], 1.0, "vertex alpha is always opaque");
    assert_eq!(written[1], [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(written[2], [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn vertex_colors_survive_an_export_import_round_trip() {
    let colors = vec![[10u8, 128, 255]; 3];
    let mesh = Mesh {
        colors: colors.clone(),
        ..triangle_mesh()
    };
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);

    let drawing = import_bytes(output.bytes);
    let imported = imported_mesh(&drawing);
    assert_eq!(imported.colors.len(), 3);
    for (written, expected) in imported.colors.iter().zip(colors.iter()) {
        for channel in 0..3 {
            let delta = i32::from(written[channel]) - i32::from(expected[channel]);
            assert!(
                delta.abs() <= 1,
                "sRGB channel drifted past rounding: wrote {written:?} for {expected:?}"
            );
        }
    }
}

#[test]
fn vertex_colors_with_explicit_opacity_are_reported_partial() {
    // `COLOR_0` alpha is always written opaque, so an explicit entity opacity
    // below 1.0 cannot be stored and must not be a silent `Complete`.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(7, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(
            EntityId(7),
            EntityRenderAttributes {
                color: EntityColor::ByLayer,
                transparency: EntityTransparency::Explicit(0.5),
                ..Default::default()
            },
        )],
    );
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.gltf.vertex_color_opacity_not_written")
        .expect("the unwritten vertex opacity is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(
        entry.message.contains("#7"),
        "the entry must name the entity: {}",
        entry.message
    );
    assert_eq!(
        output.report.counts.converted, 1,
        "one entity, one conversion entry"
    );

    // The vertex colours are still written, at the documented opaque alpha.
    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let colors = primitive.get(&Semantic::Colors(0)).expect("COLOR_0");
    for written in read_vec4_f32(&gltf, &buffers, colors.index()) {
        assert_eq!(written[3], 1.0, "vertex alpha is written opaque");
    }
    assert!(primitive.material().index().is_none());
}

#[test]
fn vertex_colors_with_non_finite_opacity_are_reported_partial() {
    // A non-finite opacity cannot be stored either; it is reported through the
    // same conversion instead of being silently treated as authored opacity.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::ByLayer,
                transparency: EntityTransparency::Explicit(f32::NAN),
                ..Default::default()
            },
        )],
    );
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.gltf.vertex_color_opacity_not_written")
        .expect("a non-finite opacity is reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(entry.message.contains("#1"));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let buffers = buffer_bytes(&gltf);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    let colors = primitive.get(&Semantic::Colors(0)).expect("COLOR_0");
    for written in read_vec4_f32(&gltf, &buffers, colors.index()) {
        assert_eq!(written[3], 1.0, "the written alpha stays opaque");
    }
}

#[test]
fn vertex_colors_with_layer_transparency_keep_the_layer_note() {
    // A layer-driven opacity has no glTF representation; it stays the one
    // evidence note and never becomes a `Partial` conversion.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::ByLayer,
                transparency: EntityTransparency::ByLayer,
                ..Default::default()
            },
        )],
    );
    let output = export(GltfContainer::Glb, &database);

    let notes: Vec<_> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.gltf.layer_appearance_not_written")
        .collect();
    assert_eq!(notes.len(), 1, "the layer gap is reported once");
    assert_eq!(notes[0].status, SupportStatus::Unverified);
    assert!(
        !output
            .report
            .entries
            .iter()
            .any(|entry| entry.code == "export.gltf.vertex_color_opacity_not_written"),
        "a layer opacity is a note, not an explicit unwritten opacity"
    );
    assert_eq!(output.report.counts.converted, 0);
    assert!(matches!(output.report.completeness, Completeness::Complete));
}

#[test]
fn vertex_colors_with_full_explicit_opacity_stay_complete() {
    // An explicit opacity of exactly 1.0 is what `COLOR_0` alpha already is, so
    // nothing was converted.
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::ByLayer,
                transparency: EntityTransparency::Explicit(1.0),
                ..Default::default()
            },
        )],
    );
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(output.report.completeness, Completeness::Complete));
    assert_eq!(output.report.counts.converted, 0);
    assert!(!output
        .report
        .entries
        .iter()
        .any(|entry| entry.code == "export.gltf.vertex_color_opacity_not_written"));
}

#[test]
fn partially_present_normals_and_colors_are_dropped_and_reported() {
    let mesh = Mesh {
        // Only half of the vertices carry a normal, and only one has a colour:
        // the writer must not invent the missing values.
        vertices: vec![
            point(1.0, 2.0, 3.0),
            point(2.0, 2.0, 3.0),
            point(1.0, 3.0, 3.0),
            point(2.0, 3.0, 3.0),
        ],
        triangles: vec![[0, 1, 2], [1, 3, 2]],
        normals: vec![point(0.0, 0.0, 1.0), point(0.0, 0.0, 1.0)],
        face_sources: Vec::new(),
        colors: vec![[1, 2, 3]],
    };
    let database = mesh_database(mesh);
    let output = export(GltfContainer::Glb, &database);

    let codes: Vec<&str> = output
        .report
        .entries
        .iter()
        .map(|entry| entry.code.as_str())
        .collect();
    assert!(codes.contains(&"export.gltf.normals_dropped"));
    assert!(codes.contains(&"export.gltf.colors_dropped"));
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    assert_eq!(output.report.counts.converted, 2);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    assert!(primitive.get(&Semantic::Normals).is_none());
    assert!(primitive.get(&Semantic::Colors(0)).is_none());
}

#[test]
fn explicit_entity_color_becomes_one_opaque_material() {
    let database = database_with_render(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(triangle_mesh()),
        )],
        vec![(EntityId(1), explicit([255, 0, 0], None))],
    );
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(
        gltf.materials().count(),
        1,
        "one explicit colour, one material"
    );
    let material = gltf.materials().next().unwrap();
    assert_eq!(material.alpha_mode(), gltf::material::AlphaMode::Opaque);
    let factor = material.pbr_metallic_roughness().base_color_factor();
    assert_eq!(factor, [1.0, 0.0, 0.0, 1.0]);

    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.material().index(), Some(0));
    // No vertex colours, so the material is the only colour channel.
    assert!(primitive.get(&Semantic::Colors(0)).is_none());
}

#[test]
fn explicit_entity_color_with_half_opacity_becomes_a_blend_material() {
    let database = database_with_render(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(triangle_mesh()),
        )],
        vec![(EntityId(1), explicit([255, 0, 0], Some(0.5)))],
    );
    let output = export(GltfContainer::Glb, &database);
    assert!(matches!(output.report.completeness, Completeness::Complete));

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    let material = gltf.materials().next().expect("one material");
    assert_eq!(material.alpha_mode(), gltf::material::AlphaMode::Blend);
    let factor = material.pbr_metallic_roughness().base_color_factor();
    assert_eq!(factor[0], 1.0);
    assert_eq!(factor[3], 0.5);
}

#[test]
fn identical_entity_colors_share_one_material() {
    let database = database_with_render(
        vec![
            entity(1, "GltfMesh", SemanticGeometry::Mesh(triangle_mesh())),
            entity(
                2,
                "GltfMesh",
                SemanticGeometry::Mesh(Mesh {
                    vertices: vec![
                        point(5.0, 0.0, 0.0),
                        point(6.0, 0.0, 0.0),
                        point(5.0, 1.0, 0.0),
                    ],
                    ..triangle_mesh()
                }),
            ),
        ],
        vec![
            (EntityId(1), explicit([255, 0, 0], None)),
            (EntityId(2), explicit([255, 0, 0], None)),
        ],
    );
    let output = export(GltfContainer::Glb, &database);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.meshes().count(), 2);
    assert_eq!(gltf.materials().count(), 1, "the colour is deduplicated");
    for mesh in gltf.meshes() {
        let primitive = mesh.primitives().next().unwrap();
        assert_eq!(primitive.material().index(), Some(0));
    }
}

#[test]
fn by_layer_entity_color_writes_no_material_and_one_note() {
    // A database with no stored render attributes resolves to `ByLayer`.
    let database = mesh_database(triangle_mesh());
    let output = export(GltfContainer::Glb, &database);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.materials().count(), 0);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    assert!(primitive.material().index().is_none());

    let notes: Vec<_> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.gltf.layer_appearance_not_written")
        .collect();
    assert_eq!(
        notes.len(),
        1,
        "the layer gap is reported once, not per entity"
    );
    assert_eq!(notes[0].status, SupportStatus::Unverified);
    // A note is evidence, not a loss: the verdict stays Complete.
    assert!(matches!(output.report.completeness, Completeness::Complete));
}

#[test]
fn vertex_colors_win_over_an_explicit_entity_color() {
    let mesh = Mesh {
        colors: vec![[10, 20, 30]; 3],
        ..triangle_mesh()
    };
    let database = database_with_render(
        vec![entity(1, "GltfMesh", SemanticGeometry::Mesh(mesh))],
        vec![(EntityId(1), explicit([255, 0, 0], None))],
    );
    let output = export(GltfContainer::Glb, &database);

    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.gltf.entity_color_overridden")
        .expect("the overridden entity colour is reported");
    assert_eq!(entry.status, SupportStatus::Partial);

    let gltf = gltf::Gltf::from_slice(&output.bytes).expect("valid GLB");
    assert_eq!(gltf.materials().count(), 0);
    let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
    assert!(primitive.get(&Semantic::Colors(0)).is_some());
    assert!(primitive.material().index().is_none());
}

#[test]
fn explicit_entity_color_survives_a_gltf_round_trip() {
    let database = database_with_render(
        vec![entity(
            1,
            "GltfMesh",
            SemanticGeometry::Mesh(triangle_mesh()),
        )],
        vec![(EntityId(1), explicit([10, 128, 255], Some(0.5)))],
    );
    let output = export(GltfContainer::Glb, &database);

    let drawing = import_bytes(output.bytes);
    let entity = drawing.database.entities().next().expect("one entity");
    let attributes = drawing.database.entity_render_attributes(entity.id);
    let EntityColor::Explicit(rgb) = attributes.color else {
        panic!(
            "the explicit colour was not restored: {:?}",
            attributes.color
        );
    };
    for (written, expected) in rgb.iter().zip([10u8, 128, 255].iter()) {
        let delta = i32::from(*written) - i32::from(*expected);
        assert!(
            delta.abs() <= 1,
            "sRGB channel drifted past rounding: wrote {rgb:?}"
        );
    }
    let EntityTransparency::Explicit(alpha) = attributes.transparency else {
        panic!(
            "the explicit opacity was not restored: {:?}",
            attributes.transparency
        );
    };
    assert!((alpha - 0.5).abs() < 1e-6, "opacity became {alpha}");
}

#[test]
fn mismatched_format_id_is_refused() {
    let database = mesh_database(triangle_mesh());
    let units = UnitContext::from_meter();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: Some(&units),
        space: ExportSpace::Model,
        limits: ExportLimits::default(),
    };
    let error = GltfExporter::new(GltfContainer::Glb, &DESCRIPTOR_GLTF)
        .export("gltf", &job)
        .expect_err("a mismatched registry key must be refused");
    assert!(
        error
            .to_string()
            .contains("export.gltf.mismatched_descriptor"),
        "unexpected error: {error}"
    );
}
