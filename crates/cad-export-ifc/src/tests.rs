//! Contract tests for the IFC4 writer.
//!
//! **Evidence boundary.** The "roundtrip" test below proves *self-consistency*
//! only: the bytes this writer produces are read back by the pinned
//! `cad-import-ifc` reader with the same triangle count. It is not a golden
//! file, not an external IFC checker, not a vendor application opening the file
//! and not a certification.

use std::sync::Arc;

use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer, Layout};
use cad_domain::{
    CadError, Completeness, DatabaseId, DocumentId, EntityId, LayerId, LayoutId, Mesh, ObjectId,
    Point3, Revision, SemanticGeometry, SpaceId, SupportStatus, Unit, UnitContext,
};
use cad_import_core::{
    ExportJob, ExportLimits, ExportSpace, Exporter, ImportLimits, ImportRequest, Importer,
};

use super::*;

fn exporter() -> IfcExporter {
    IfcExporter::new(&cad_import_ifc::DESCRIPTOR_IFC)
}

fn triangle_mesh() -> Mesh {
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
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ],
        triangles: vec![[0, 1, 2]],
        ..Mesh::default()
    }
}

fn entity(id: u128, type_key: &str, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn database(entities: Vec<DbEntity>) -> DrawingDatabase {
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
    for row in entities {
        builder.insert_entity(row).unwrap();
    }
    builder
        .insert_layout(Layout {
            id: LayoutId(1),
            name: "Layout1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    builder.finish().unwrap()
}

fn triangle_database() -> DrawingDatabase {
    database(vec![entity(
        1,
        "TestMesh",
        SemanticGeometry::Mesh(triangle_mesh()),
    )])
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

fn export(database: &DrawingDatabase, units: &UnitContext) -> ExportOutput {
    exporter()
        .export("ifc", &job(database, Some(units)))
        .expect("a triangle model exports")
}

/// The written file is IFC4 Part 21 with the spatial root and a tessellated
/// face set, and the report says one mesh was written exactly.
#[test]
fn a_triangle_exports_as_an_ifc4_spatial_structure_with_a_face_set() {
    let units = UnitContext::from_meter();
    let database = triangle_database();
    let output = export(&database, &units);
    let text = std::str::from_utf8(&output.bytes).expect("IFC is ASCII");
    assert!(
        text.starts_with("ISO-10303-21;"),
        "{}",
        &text[..40.min(text.len())]
    );
    assert!(text.contains("FILE_SCHEMA(('IFC4'))"), "the schema is IFC4");
    assert!(text.contains("IFCPROJECT"));
    assert!(text.contains("IFCBUILDING"));
    assert!(text.contains("IFCBUILDINGELEMENTPROXY"));
    assert!(text.contains("IFCTRIANGULATEDFACESET"));
    assert!(text.contains("'TestMesh'"), "the type key names the object");

    let report = &output.report;
    assert_eq!(report.connector, "ifc");
    assert_eq!(report.direction, ReportDirection::Export);
    assert_eq!(report.level, FidelityLevel::L1);
    assert_eq!(report.counts.exact, 1);
    assert_eq!(report.counts.converted, 0);
    assert_eq!(report.counts.dropped, 0);
    assert_eq!(report.counts.bytes, output.bytes.len());
    assert!(report.entries.is_empty());
    assert!(report.capabilities.is_empty());
    assert_eq!(report.completeness, Completeness::Complete);
}

/// Self-consistency only: the pinned reader reads the bytes back with the same
/// triangle count. Not vendor acceptance, not a golden file.
#[test]
fn the_written_bytes_round_trip_through_the_pinned_reader() {
    let units = UnitContext::from_meter();
    let database = triangle_database();
    let output = export(&database, &units);
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(2),
        bytes: Arc::from(output.bytes.clone().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    };
    let drawing = cad_import_ifc::IfcImporter::new()
        .import(&request, &|| false)
        .expect("the written IFC4 bytes are readable by the pinned reader");
    let meshes: Vec<&Mesh> = drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .collect();
    assert_eq!(
        meshes.len(),
        1,
        "one IfcBuildingElementProxy carries the one mesh"
    );
    assert_eq!(meshes[0].triangles.len(), 1, "the triangle survives");
    assert_eq!(meshes[0].vertices.len(), 3);
}

/// An unknown source unit is a named refusal, never an assumed millimetre.
#[test]
fn unknown_units_are_refused_by_name() {
    let database = triangle_database();
    let drawing_units = UnitContext::drawing_units();
    let error = exporter()
        .export("ifc", &job(&database, Some(&drawing_units)))
        .expect_err("drawing units are not a length");
    match error {
        CadError::Unsupported(message) => assert!(
            message.starts_with("export.ifc.units_unknown"),
            "expected the stable refusal code, got: {message}"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }

    let error = exporter()
        .export("ifc", &job(&database, None))
        .expect_err("a missing context is not a length");
    match error {
        CadError::Unsupported(message) => assert!(
            message.starts_with("export.ifc.units_unknown"),
            "expected the stable refusal code, got: {message}"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

/// Non-mesh geometry is dropped with its own stable code and makes the report
/// `Missing`; the file itself is still a valid empty IFC4 project.
#[test]
fn a_line_is_reported_dropped_and_the_report_is_missing() {
    let units = UnitContext::from_meter();
    let database = database(vec![entity(
        1,
        "AcDbLine",
        SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
    )]);
    let output = export(&database, &units);
    let report = &output.report;
    assert_eq!(report.counts.exact, 0);
    assert_eq!(report.counts.converted, 0);
    assert_eq!(report.counts.dropped, 1);
    assert_eq!(report.entries.len(), 1);
    let entry = &report.entries[0];
    assert_eq!(entry.code, UNSUPPORTED_GEOMETRY);
    assert_eq!(entry.status, SupportStatus::Unsupported);
    assert_eq!(entry.object, Some(ObjectId(1)));
    assert_eq!(entry.type_key.as_deref(), Some("AcDbLine"));
    assert_eq!(entry.direction, ReportDirection::Export);
    assert!(matches!(&report.completeness, Completeness::Missing(reasons) if reasons.len() == 1));
    let text = std::str::from_utf8(&output.bytes).expect("IFC is ASCII");
    assert!(text.starts_with("ISO-10303-21;"));
    assert!(text.contains("IFCPROJECT"));
}

/// Source units are converted to SI metres once, at the export boundary.
#[test]
fn millimetre_coordinates_are_written_in_metres() {
    let mut mesh = triangle_mesh();
    for vertex in &mut mesh.vertices {
        vertex.x *= 1000.0;
        vertex.y *= 1000.0;
    }
    let database = database(vec![entity(1, "TestMesh", SemanticGeometry::Mesh(mesh))]);
    let units = UnitContext {
        source: Unit::Millimeter,
        ..UnitContext::from_meter()
    };
    let output = export(&database, &units);
    let text = std::str::from_utf8(&output.bytes).expect("IFC is ASCII");
    assert!(
        text.contains("(1.,0.,0.)"),
        "1000 mm is written as 1 m; got:\n{text}"
    );
}

/// The same database always produces byte-identical output: no clock, no uuid.
#[test]
fn exporting_twice_is_byte_identical() {
    let units = UnitContext::from_meter();
    let database = triangle_database();
    let first = export(&database, &units);
    let second = export(&database, &units);
    assert_eq!(first.bytes, second.bytes);
    assert_eq!(first.report.counts.exact, second.report.counts.exact);
}

/// A registry key this writer does not produce is refused by name.
#[test]
fn a_mismatched_format_id_is_refused_by_name() {
    let units = UnitContext::from_meter();
    let database = triangle_database();
    let error = exporter()
        .export("stl", &job(&database, Some(&units)))
        .expect_err("an IFC writer must not accept another key");
    match error {
        CadError::Unsupported(message) => assert_eq!(
            message,
            "export.stl.mismatched_descriptor: exporter writes ifc"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

/// A mesh with no triangle is a named drop, not an empty success.
#[test]
fn a_triangleless_mesh_is_reported_dropped() {
    let units = UnitContext::from_meter();
    let database = database(vec![entity(
        1,
        "TestMesh",
        SemanticGeometry::Mesh(Mesh::default()),
    )]);
    let output = export(&database, &units);
    assert_eq!(output.report.counts.dropped, 1);
    assert_eq!(output.report.entries[0].code, EMPTY_MESH);
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
}

/// Per-vertex colours and normals cannot ride on an `IfcTriangulatedFaceSet`:
/// the mesh is still written, and each attribute is reported once by kind so the
/// verdict is `Partial` instead of a clean `Complete`.
#[test]
fn a_mesh_with_colours_and_normals_writes_but_reports_both_losses() {
    let units = UnitContext::from_meter();
    let mut mesh = triangle_mesh();
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    mesh.normals = vec![normal; 3];
    mesh.colors = vec![[200, 10, 10]; 3];
    let database = database(vec![entity(1, "TestMesh", SemanticGeometry::Mesh(mesh))]);
    let output = export(&database, &units);
    let report = &output.report;
    assert_eq!(report.counts.exact, 1, "the geometry itself is written");
    assert_eq!(report.counts.dropped, 0, "nothing is missing from the file");
    assert_eq!(report.entries.len(), 2, "{:?}", report.entries);
    let codes: Vec<&str> = report.entries.iter().map(|e| e.code.as_str()).collect();
    assert!(codes.contains(&COLORS_DROPPED), "{codes:?}");
    assert!(codes.contains(&NORMALS_DROPPED), "{codes:?}");
    for entry in &report.entries {
        assert_eq!(entry.status, SupportStatus::Partial);
        assert_eq!(entry.object, None, "one entry per kind, not per entity");
    }
    assert!(matches!(
        &report.completeness,
        Completeness::Partial(reasons) if reasons.len() == 2
    ));
    let text = std::str::from_utf8(&output.bytes).expect("IFC is ASCII");
    assert!(text.contains("IFCTRIANGULATEDFACESET"));
}

/// A written mesh that references a base-colour texture keeps its geometry in
/// the file, and the texture loss is one `Partial` conversion naming the entity
/// and the key. The bytes are identical to the untextured mesh.
#[test]
fn a_textured_mesh_reports_the_texture_not_written_and_writes_the_same_bytes() {
    let units = UnitContext::from_meter();
    let mut textured = triangle_mesh();
    textured.uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    textured.texture = Some("abc.png".into());
    let with = export(
        &database(vec![entity(
            1,
            "TestMesh",
            SemanticGeometry::Mesh(textured),
        )]),
        &units,
    );
    let without = export(&triangle_database(), &units);
    assert_eq!(
        with.bytes, without.bytes,
        "the IFC writer must write the same bytes for a textured and an untextured mesh"
    );
    assert_eq!(
        with.report.counts.exact, without.report.counts.exact,
        "the mesh geometry is still exact"
    );

    let entries: Vec<&CompatEntry> = with
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == TEXTURE_NOT_WRITTEN)
        .collect();
    assert_eq!(entries.len(), 1, "{:?}", with.report.entries);
    assert_eq!(entries[0].status, SupportStatus::Partial);
    assert_eq!(entries[0].object, Some(ObjectId(1)));
    assert!(
        entries[0].message.contains("#1") && entries[0].message.contains("abc.png"),
        "the message must name the entity and the texture key: {}",
        entries[0].message
    );
    assert_eq!(with.report.counts.converted, 1);
    assert!(matches!(with.report.completeness, Completeness::Partial(_)));
    assert!(
        !without
            .report
            .entries
            .iter()
            .any(|entry| entry.code == TEXTURE_NOT_WRITTEN),
        "an untextured mesh must not report a texture loss"
    );
    assert_eq!(without.report.completeness, Completeness::Complete);
}

/// An entity that is not in the file at all is reported once: the texture on it
/// is part of that drop, never a second entry.
#[test]
fn a_dropped_entity_reports_its_texture_loss_only_once() {
    let units = UnitContext::from_meter();
    let mut textured = triangle_mesh();
    textured.uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    textured.texture = Some("abc.png".into());
    let mut row = entity(1, "PaperMesh", SemanticGeometry::Mesh(textured));
    row.space = SpaceId::Paper(LayoutId(1));
    let database = database(vec![row]);
    let output = export(&database, &units);
    assert_eq!(output.report.counts.dropped, 1);
    assert_eq!(output.report.counts.converted, 0);
    assert_eq!(output.report.entries.len(), 1);
    assert_eq!(output.report.entries[0].code, SPACE_NOT_EXPORTED);
    assert!(!output
        .report
        .entries
        .iter()
        .any(|entry| entry.code == TEXTURE_NOT_WRITTEN));
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
}

/// An entity on a named layer (or carrying a display attribute) is written, but
/// the layer itself is not: the report names `export.ifc.layers_dropped`.
#[test]
fn a_layered_mesh_reports_the_layer_loss() {
    let units = UnitContext::from_meter();
    let mut row = entity(1, "TestMesh", SemanticGeometry::Mesh(triangle_mesh()));
    row.layer = LayerId(1);
    let database = database(vec![row]);
    let output = export(&database, &units);
    let report = &output.report;
    assert_eq!(report.counts.exact, 1);
    assert_eq!(report.counts.dropped, 0);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].code, LAYERS_DROPPED);
    assert_eq!(report.entries[0].status, SupportStatus::Partial);
    assert!(matches!(report.completeness, Completeness::Partial(_)));
}

/// A database with no entity still writes a valid skeleton-only IFC4 project and
/// reports `Complete`: there was no geometry to lose.
#[test]
fn an_empty_database_writes_a_valid_skeleton_and_reports_complete() {
    let units = UnitContext::from_meter();
    let database = database(Vec::new());
    let output = export(&database, &units);
    let text = std::str::from_utf8(&output.bytes).expect("IFC is ASCII");
    assert!(text.starts_with("ISO-10303-21;"));
    assert!(text.contains("FILE_SCHEMA(('IFC4'))"));
    assert!(text.contains("IFCPROJECT"));
    assert!(text.contains("IFCBUILDING"));
    assert!(
        !text.contains("IFCTRIANGULATEDFACESET"),
        "there is no mesh to write"
    );
    assert_eq!(output.report.counts.exact, 0);
    assert_eq!(output.report.counts.dropped, 0);
    assert!(output.report.entries.is_empty());
    assert_eq!(output.report.completeness, Completeness::Complete);
}
