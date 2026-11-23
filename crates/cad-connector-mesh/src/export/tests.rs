//! Contract tests: determinism, importer round-trips, losses, limits.

use std::sync::Arc;

use crate::{
    ObjImporter, OffImporter, PlyImporter, StlImporter, DESCRIPTOR_OBJ, DESCRIPTOR_OFF,
    DESCRIPTOR_PLY, DESCRIPTOR_STL,
};
use cad_connector_core::{
    ConnectorDescriptor, ExportJob, ExportLimits, ExportOutput, ExportSpace, Exporter,
    ImportLimits, ImportRequest, ImportedDrawing, Importer,
};
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer, Layout};
use cad_domain::{
    BlockId, Body3d, BodyDisplay, BodyExact, BodyKind, CadError, Completeness, DatabaseId,
    DocumentId, EntityId, LayerId, LayoutId, Mesh, ObjectId, Point3, Precision, Revision,
    SemanticGeometry, SpaceId, SupportStatus, UnitContext,
};

use super::{MeshExporter, MeshFormat};

const QUAD_COLORS: [[u8; 3]; 4] = [[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];

fn descriptor(format: MeshFormat) -> &'static ConnectorDescriptor {
    match format {
        MeshFormat::StlBinary | MeshFormat::StlAscii => &DESCRIPTOR_STL,
        MeshFormat::Obj => &DESCRIPTOR_OBJ,
        MeshFormat::Ply => &DESCRIPTOR_PLY,
        MeshFormat::Off => &DESCRIPTOR_OFF,
    }
}

fn quad(colors: Option<[[u8; 3]; 4]>) -> Mesh {
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    Mesh {
        vertices: vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        normals: vec![normal; 4],
        face_sources: Vec::new(),
        colors: colors.map(|colors| colors.to_vec()).unwrap_or_default(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

/// The same quad with a UV0 set and a document texture key.
fn textured(mut mesh: Mesh, key: &str) -> Mesh {
    mesh.uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    mesh.texture = Some(key.to_string());
    mesh
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

fn in_space(mut entity: DbEntity, space: SpaceId) -> DbEntity {
    entity.space = space;
    entity
}

fn translated(mut mesh: Mesh, offset: Point3) -> Mesh {
    for vertex in &mut mesh.vertices {
        vertex.x += offset.x;
        vertex.y += offset.y;
        vertex.z += offset.z;
    }
    mesh
}

fn database(entities: Vec<DbEntity>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    builder.finish().unwrap()
}

fn mesh_database(mesh: Mesh) -> DrawingDatabase {
    database(vec![entity(1, "TestMesh", SemanticGeometry::Mesh(mesh))])
}

fn job(database: &DrawingDatabase) -> ExportJob<'_> {
    ExportJob {
        database,
        document_id: DocumentId(1),
        units: None,
        space: ExportSpace::Model,
        limits: ExportLimits::default(),
    }
}

fn export_output(format: MeshFormat, database: &DrawingDatabase) -> ExportOutput {
    let task = job(database);
    MeshExporter::new(format, descriptor(format))
        .export(format.format_id(), &task)
        .unwrap()
}

fn import(bytes: &[u8], importer: &dyn Importer) -> ImportedDrawing {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    importer.import(&request, &|| false).unwrap()
}

fn one_mesh(drawing: &ImportedDrawing) -> &Mesh {
    let meshes: Vec<&Mesh> = drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .collect();
    assert_eq!(meshes.len(), 1, "the reader must yield exactly one mesh");
    meshes[0]
}

#[test]
fn every_format_is_byte_identical_across_runs() {
    let database = mesh_database(quad(Some(QUAD_COLORS)));
    for format in [
        MeshFormat::StlBinary,
        MeshFormat::StlAscii,
        MeshFormat::Obj,
        MeshFormat::Ply,
        MeshFormat::Off,
    ] {
        let first = export_output(format, &database);
        let second = export_output(format, &database);
        assert_eq!(
            first.bytes,
            second.bytes,
            "{} must be deterministic",
            format.format_id()
        );
        assert_eq!(first.report.counts.bytes, first.bytes.len());
    }
}

#[test]
fn stl_binary_round_trips() {
    let database = mesh_database(quad(None));
    let output = export_output(MeshFormat::StlBinary, &database);
    let drawing = import(&output.bytes, &StlImporter::new());
    assert_eq!(one_mesh(&drawing).triangles.len(), 2);
    assert_eq!(output.report.completeness, Completeness::Complete);
}

#[test]
fn stl_ascii_round_trips() {
    let database = mesh_database(quad(None));
    let output = export_output(MeshFormat::StlAscii, &database);
    let drawing = import(&output.bytes, &StlImporter::new());
    assert_eq!(one_mesh(&drawing).triangles.len(), 2);
}

#[test]
fn obj_round_trips() {
    let database = mesh_database(quad(None));
    let output = export_output(MeshFormat::Obj, &database);
    let drawing = import(&output.bytes, &ObjImporter::new());
    assert_eq!(one_mesh(&drawing).triangles.len(), 2);
}

#[test]
fn ply_round_trips_and_keeps_colours() {
    let database = mesh_database(quad(Some(QUAD_COLORS)));
    let output = export_output(MeshFormat::Ply, &database);
    let drawing = import(&output.bytes, &PlyImporter::new());
    let mesh = one_mesh(&drawing);
    assert_eq!(mesh.triangles.len(), 2);
    assert_eq!(mesh.colors, QUAD_COLORS.to_vec());
    assert_eq!(output.report.completeness, Completeness::Complete);
}

#[test]
fn ply_round_trip_keeps_the_second_mesh_vertex_indices() {
    let offset = Point3 {
        x: 1000.0,
        y: 0.0,
        z: 0.0,
    };
    let database = database(vec![
        entity(1, "First", SemanticGeometry::Mesh(quad(None))),
        entity(
            2,
            "Second",
            SemanticGeometry::Mesh(translated(quad(None), offset)),
        ),
    ]);
    let output = export_output(MeshFormat::Ply, &database);
    assert_eq!(output.report.completeness, Completeness::Complete);

    let drawing = import(&output.bytes, &PlyImporter::new());
    let mesh = one_mesh(&drawing);
    assert_eq!(
        mesh.vertices.len(),
        8,
        "both quads must keep their vertices"
    );
    assert_eq!(mesh.triangles.len(), 4, "both quads must keep their faces");

    // The second mesh was translated far away; its faces must reference its own
    // (translated) vertices, not indices rebased onto the first mesh.
    for vertex in &mesh.vertices[4..] {
        assert!(
            (1000.0..=1001.0).contains(&vertex.x),
            "second mesh vertex landed at {vertex:?}"
        );
        assert!((0.0..=1.0).contains(&vertex.y) && vertex.z.abs() < 1e-6);
    }
    let far_faces = mesh
        .triangles
        .iter()
        .filter(|triangle| {
            triangle
                .iter()
                .all(|index| mesh.vertices[*index as usize].x >= 999.0)
        })
        .count();
    assert_eq!(far_faces, 2, "the translated quad must own two faces");
    let near_faces = mesh
        .triangles
        .iter()
        .filter(|triangle| {
            triangle
                .iter()
                .all(|index| mesh.vertices[*index as usize].x <= 1.0)
        })
        .count();
    assert_eq!(near_faces, 2, "the first quad must own two faces");

    let far_x_min = mesh.vertices[4..]
        .iter()
        .map(|vertex| vertex.x)
        .fold(f64::INFINITY, f64::min);
    let far_x_max = mesh.vertices[4..]
        .iter()
        .map(|vertex| vertex.x)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!((far_x_min - 1000.0).abs() < 1e-6);
    assert!((far_x_max - 1001.0).abs() < 1e-6);
}

#[test]
fn off_round_trips_and_keeps_colours() {
    let database = mesh_database(quad(Some(QUAD_COLORS)));
    let output = export_output(MeshFormat::Off, &database);
    let drawing = import(&output.bytes, &OffImporter::new());
    let mesh = one_mesh(&drawing);
    assert_eq!(mesh.triangles.len(), 2);
    assert_eq!(mesh.colors, QUAD_COLORS.to_vec());
    assert_eq!(output.report.completeness, Completeness::Complete);
}

#[test]
fn stl_and_obj_report_dropped_colours() {
    let database = mesh_database(quad(Some(QUAD_COLORS)));
    for format in [MeshFormat::StlBinary, MeshFormat::StlAscii, MeshFormat::Obj] {
        let output = export_output(format, &database);
        let expected = if format == MeshFormat::Obj {
            "export.obj.colors_dropped"
        } else {
            "export.stl.colors_dropped"
        };
        let entry = output
            .report
            .entries
            .iter()
            .find(|entry| entry.code == expected)
            .unwrap_or_else(|| panic!("{} must report {expected}", format.format_id()));
        assert_eq!(entry.status, SupportStatus::Partial);
        assert!(matches!(
            output.report.completeness,
            Completeness::Partial(_)
        ));
        assert_eq!(output.report.counts.converted, 1);
    }
}

#[test]
fn a_textured_mesh_reports_the_texture_not_written_and_writes_the_same_bytes() {
    for format in [
        MeshFormat::StlBinary,
        MeshFormat::StlAscii,
        MeshFormat::Obj,
        MeshFormat::Ply,
        MeshFormat::Off,
    ] {
        let code = match format {
            // The two STL containers share one report prefix on purpose.
            MeshFormat::StlBinary | MeshFormat::StlAscii => "export.stl.texture_not_written",
            MeshFormat::Obj => "export.obj.texture_not_written",
            MeshFormat::Ply => "export.ply.texture_not_written",
            MeshFormat::Off => "export.off.texture_not_written",
        };
        let with = export_output(format, &mesh_database(textured(quad(None), "abc.png")));
        let without = export_output(format, &mesh_database(quad(None)));
        assert_eq!(
            with.bytes,
            without.bytes,
            "{} must write the same bytes for a textured and an untextured mesh",
            format.format_id()
        );
        assert_eq!(
            with.report.counts.exact,
            without.report.counts.exact,
            "{}: the mesh geometry is still exact",
            format.format_id()
        );

        let entries: Vec<_> = with
            .report
            .entries
            .iter()
            .filter(|entry| entry.code == code)
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "{} must report {code}",
            format.format_id()
        );
        assert_eq!(entries[0].status, SupportStatus::Partial);
        assert!(
            entries[0].message.contains("#1") && entries[0].message.contains("abc.png"),
            "the message must name the entity and the texture key: {}",
            entries[0].message
        );
        assert!(matches!(with.report.completeness, Completeness::Partial(_)));

        assert!(
            !without
                .report
                .entries
                .iter()
                .any(|entry| entry.code == code),
            "{}: an untextured mesh must not report a texture loss",
            format.format_id()
        );
        assert_eq!(
            without.report.completeness,
            Completeness::Complete,
            "{}: an untextured mesh keeps the previous verdict",
            format.format_id()
        );
    }
}

#[test]
fn each_textured_mesh_entity_gets_its_own_texture_entry() {
    let database = database(vec![
        entity(
            1,
            "First",
            SemanticGeometry::Mesh(textured(quad(None), "abc.png")),
        ),
        entity(
            2,
            "Second",
            SemanticGeometry::Mesh(textured(quad(None), "def.png")),
        ),
    ]);
    let output = export_output(MeshFormat::Obj, &database);
    let messages: Vec<&str> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.obj.texture_not_written")
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages[0].contains("#1") && messages[0].contains("abc.png"));
    assert!(messages[1].contains("#2") && messages[1].contains("def.png"));
    assert_eq!(output.report.counts.exact, 2);
    assert_eq!(output.report.counts.converted, 2);
}

#[test]
fn non_mesh_entity_is_a_named_drop() {
    let line = entity(
        2,
        "AcDbLine",
        SemanticGeometry::Line {
            start: Point3::default(),
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
    );
    let database = database(vec![
        entity(1, "TestMesh", SemanticGeometry::Mesh(quad(None))),
        line,
    ]);
    let output = export_output(MeshFormat::Off, &database);
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.mesh.unsupported_geometry")
        .expect("the Line must be reported as unsupported geometry");
    assert_eq!(entry.status, SupportStatus::Unsupported);
    assert_eq!(entry.object, Some(ObjectId(2)));
    assert_eq!(entry.type_key.as_deref(), Some("AcDbLine"));
    assert!(entry.message.contains("Line"));
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
    assert_eq!(output.report.counts.dropped, 1);
}

fn assert_space_is_not_exported(space: SpaceId) {
    let offset = Point3 {
        x: 500.0,
        y: 0.0,
        z: 0.0,
    };
    let database = database(vec![
        entity(1, "ModelMesh", SemanticGeometry::Mesh(quad(None))),
        in_space(
            entity(
                2,
                "OffModelMesh",
                SemanticGeometry::Mesh(translated(quad(None), offset)),
            ),
            space,
        ),
    ]);
    let output = export_output(MeshFormat::Ply, &database);
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.ply.space_not_exported")
        .expect("geometry outside model space must be a named drop");
    assert_eq!(entry.status, SupportStatus::Unsupported);
    assert_eq!(entry.object, Some(ObjectId(2)));
    assert_eq!(entry.type_key.as_deref(), Some("OffModelMesh"));
    assert!(entry.message.contains("outside model space"));
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.dropped, 1);
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));

    // The dropped geometry must not leak into the written vertices.
    let drawing = import(&output.bytes, &PlyImporter::new());
    let mesh = one_mesh(&drawing);
    assert_eq!(mesh.vertices.len(), 4);
    assert!(mesh.vertices.iter().all(|vertex| vertex.x <= 1.0));
}

#[test]
fn block_space_mesh_is_a_named_drop() {
    assert_space_is_not_exported(SpaceId::Block(BlockId(1)));
}

#[test]
fn paper_space_mesh_is_a_named_drop() {
    assert_space_is_not_exported(SpaceId::Paper(LayoutId(1)));
}

fn body(mesh: Mesh) -> SemanticGeometry {
    SemanticGeometry::Body3d(Box::new(Body3d {
        kind: BodyKind::Sheet,
        exact: BodyExact::Unavailable {
            reason: "test body".into(),
        },
        display: BodyDisplay {
            mesh,
            edges: Vec::new(),
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Complete,
        },
    }))
}

#[test]
fn body_display_mesh_is_exported() {
    let database = database(vec![entity(1, "AcDbSurface", body(quad(None)))]);
    let output = export_output(MeshFormat::StlBinary, &database);
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.completeness, Completeness::Complete);
    assert!(!output
        .report
        .entries
        .iter()
        .any(|entry| entry.code == "export.mesh.empty_mesh"));
}

#[test]
fn empty_mesh_and_empty_body_share_the_empty_mesh_code() {
    let database = database(vec![
        entity(1, "AcDbSurface", body(Mesh::default())),
        entity(2, "EmptyMesh", SemanticGeometry::Mesh(Mesh::default())),
        entity(3, "ModelMesh", SemanticGeometry::Mesh(quad(None))),
    ]);
    let output = export_output(MeshFormat::StlBinary, &database);
    let empty_entries: Vec<(String, Option<ObjectId>)> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code.starts_with("export.mesh."))
        .map(|entry| (entry.code.clone(), entry.object))
        .collect();
    assert_eq!(
        empty_entries,
        vec![
            ("export.mesh.empty_mesh".to_string(), Some(ObjectId(1))),
            ("export.mesh.empty_mesh".to_string(), Some(ObjectId(2))),
        ],
        "empty Mesh and empty Body3d must share one code"
    );
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.dropped, 2);
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
}

#[test]
fn triangle_limit_refuses_by_name() {
    let database = mesh_database(quad(None));
    let task = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: None,
        space: ExportSpace::Model,
        limits: ExportLimits {
            max_output_triangles: 1,
            ..ExportLimits::default()
        },
    };
    let error = MeshExporter::new(MeshFormat::Ply, &DESCRIPTOR_PLY)
        .export("ply", &task)
        .unwrap_err();
    assert!(
        error.to_string().contains("export.ply.limits:"),
        "unexpected error: {error}"
    );
}

#[test]
fn entity_limit_refuses_by_name() {
    let database = mesh_database(quad(None));
    let task = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: None,
        space: ExportSpace::Model,
        limits: ExportLimits {
            max_output_entities: 0,
            ..ExportLimits::default()
        },
    };
    let error = MeshExporter::new(MeshFormat::StlBinary, &DESCRIPTOR_STL)
        .export("stl", &task)
        .unwrap_err();
    assert!(
        error.to_string().contains("export.stl.limits:"),
        "unexpected error: {error}"
    );
}

#[test]
fn triangle_limit_is_checked_before_copying_each_mesh() {
    let database = database(vec![
        entity(1, "FirstMesh", SemanticGeometry::Mesh(quad(None))),
        entity(2, "SecondMesh", SemanticGeometry::Mesh(quad(None))),
    ]);
    let task = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: None,
        space: ExportSpace::Model,
        limits: ExportLimits {
            max_output_triangles: 3,
            ..ExportLimits::default()
        },
    };
    let error = MeshExporter::new(MeshFormat::Off, &DESCRIPTOR_OFF)
        .export("off", &task)
        .unwrap_err();
    assert!(
        error.to_string().contains("export.off.limits:"),
        "unexpected error: {error}"
    );
    assert!(
        matches!(error, CadError::InvalidInput(_)),
        "over-budget documents must be refused"
    );
}

#[test]
fn empty_document_is_refused_for_every_format() {
    let database = database(Vec::new());
    for format in [
        MeshFormat::StlBinary,
        MeshFormat::StlAscii,
        MeshFormat::Obj,
        MeshFormat::Ply,
        MeshFormat::Off,
    ] {
        let error = MeshExporter::new(format, descriptor(format))
            .export(format.format_id(), &job(&database))
            .unwrap_err();
        let expected = format!("export.{}.no_geometry:", format.format_id());
        assert!(
            error.to_string().contains(&expected),
            "{} must refuse an empty document, got: {error}",
            format.format_id()
        );
        assert!(
            matches!(error, CadError::Unsupported(_)),
            "{} must report unsupported, got: {error}",
            format.format_id()
        );
    }
}

#[test]
fn obj_reports_normals_dropped_when_they_are_incomplete() {
    let mut without_normals = quad(None);
    without_normals.normals.clear();
    let database = database(vec![
        entity(1, "WithNormals", SemanticGeometry::Mesh(quad(None))),
        entity(2, "WithoutNormals", SemanticGeometry::Mesh(without_normals)),
    ]);
    let output = export_output(MeshFormat::Obj, &database);
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == "export.obj.normals_dropped")
        .expect("incomplete normals must be reported");
    assert_eq!(entry.status, SupportStatus::Partial);
    assert!(entry.message.contains("normal"));
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
    let text = String::from_utf8(output.bytes).expect("OBJ is UTF-8");
    assert!(
        !text.lines().any(|line| line.starts_with("vn ")),
        "no vn line may be written when the normal set is incomplete"
    );
}

#[test]
fn stl_exact_count_excludes_meshes_with_dropped_facets() {
    let mesh = Mesh {
        vertices: vec![
            Point3::default(),
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        // The second facet repeats an index and is a zero-area triangle.
        triangles: vec![[0, 1, 2], [0, 1, 1]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    };
    let database = mesh_database(mesh);
    let output = export_output(MeshFormat::StlBinary, &database);
    assert_eq!(output.report.counts.exact, 0);
    assert_eq!(output.report.counts.dropped, 1);
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
    let facets = u32::from_le_bytes(output.bytes[80..84].try_into().expect("4-byte facet count"));
    assert_eq!(facets, 1, "the degenerate facet must not be written");
}

#[test]
fn mismatched_format_id_is_refused() {
    let database = mesh_database(quad(None));
    let error = MeshExporter::new(MeshFormat::StlBinary, &DESCRIPTOR_STL)
        .export("obj", &job(&database))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("export.obj.mismatched_descriptor"),
        "unexpected error: {error}"
    );
}

#[test]
fn units_are_reported_as_notes() {
    let database = mesh_database(quad(None));
    let known_units = UnitContext::from_meter();
    let task = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: Some(&known_units),
        space: ExportSpace::Model,
        limits: ExportLimits::default(),
    };
    let with_units = MeshExporter::new(MeshFormat::Obj, &DESCRIPTOR_OBJ)
        .export("obj", &task)
        .unwrap();
    assert!(with_units
        .report
        .entries
        .iter()
        .any(|entry| entry.code == "export.obj.units_not_stored"
            && entry.status == SupportStatus::Unverified));
    assert_eq!(with_units.report.completeness, Completeness::Complete);

    let unknown = export_output(MeshFormat::Obj, &database);
    assert!(unknown
        .report
        .entries
        .iter()
        .any(|entry| entry.code == "export.obj.units_unknown"
            && entry.status == SupportStatus::Unverified));
}
