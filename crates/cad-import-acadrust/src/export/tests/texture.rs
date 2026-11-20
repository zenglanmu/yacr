//! ADR-0038 D11: the DXF/DWG writers record a base-colour texture they cannot
//! store, once per affected mesh entity, without changing a single output byte.
//!
//! Evidence boundary: these are contract tests over the writer + our own reader
//! agreement. No AutoCAD/ODA or any other vendor consumer was run.

use cad_import_core::{
    ExportJob, ExportLimits as CoreExportLimits, ExportSpace as CoreExportSpace, Exporter,
};

use crate::export::AcadrustExporter;

use super::*;

fn plain_triangle() -> Mesh {
    Mesh {
        vertices: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        triangles: vec![[0, 1, 2]],
        ..Mesh::default()
    }
}

fn textured_triangle() -> Mesh {
    let mut mesh = plain_triangle();
    mesh.uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    mesh.texture = Some("abc.png".into());
    mesh
}

fn core_job(database: &DrawingDatabase) -> ExportJob<'_> {
    ExportJob {
        database,
        document_id: DocumentId(1),
        units: None,
        space: CoreExportSpace::Model,
        limits: CoreExportLimits::default(),
    }
}

/// A textured mesh is written exactly as its untextured copy, and the texture
/// loss is one `Partial` entry under the container's own code.
#[test]
fn a_textured_mesh_records_the_texture_loss_for_dxf_and_dwg() {
    for (format, format_id, code) in [
        (
            ExportFormat::DxfText,
            "dxf",
            "export.dxf.texture_not_written",
        ),
        (ExportFormat::Dwg, "dwg", "export.dwg.texture_not_written"),
    ] {
        let target = match format {
            ExportFormat::DxfText => DxfTarget::default(),
            ExportFormat::Dwg => DxfTarget::R2018,
        };
        let exporter = AcadrustExporter::new(format, AcisVersion::default(), target);
        let with_db = db_with(vec![SemanticGeometry::Mesh(textured_triangle())]);
        let without_db = db_with(vec![SemanticGeometry::Mesh(plain_triangle())]);
        let with = exporter
            .export(format_id, &core_job(&with_db))
            .expect("a triangle mesh exports");
        let without = exporter
            .export(format_id, &core_job(&without_db))
            .expect("a triangle mesh exports");

        assert_eq!(
            with.bytes, without.bytes,
            "{format_id}: the texture must not change a single output byte"
        );
        assert_eq!(
            with.report.counts.exact, without.report.counts.exact,
            "{format_id}: the mesh geometry is still exact"
        );

        let entries: Vec<&cad_import_core::CompatEntry> = with
            .report
            .entries
            .iter()
            .filter(|entry| entry.code == code)
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "{format_id}: {code} must be recorded once"
        );
        assert_eq!(entries[0].status, SupportStatus::Partial);
        assert_eq!(entries[0].object, Some(ObjectId(1)));
        assert!(
            entries[0].message.contains("#1") && entries[0].message.contains("abc.png"),
            "{format_id}: the message must name the entity and the texture key: {}",
            entries[0].message
        );
        assert!(matches!(with.report.completeness, Completeness::Partial(_)));

        assert!(
            !without
                .report
                .entries
                .iter()
                .any(|entry| entry.code == code),
            "{format_id}: an untextured mesh must not report a texture loss"
        );
    }
}

/// One entry per affected mesh entity, naming each entity and its own key.
#[test]
fn each_textured_mesh_entity_gets_its_own_entry() {
    let mut second = plain_triangle();
    for vertex in &mut second.vertices {
        vertex.x += 10.0;
    }
    second.uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    second.texture = Some("def.png".into());
    let database = db_with(vec![
        SemanticGeometry::Mesh(textured_triangle()),
        SemanticGeometry::Mesh(second),
    ]);
    let exporter = AcadrustExporter::new(
        ExportFormat::DxfText,
        AcisVersion::default(),
        DxfTarget::default(),
    );
    let output = exporter
        .export("dxf", &core_job(&database))
        .expect("two triangle meshes export");
    let messages: Vec<&str> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.dxf.texture_not_written")
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages[0].contains("#1") && messages[0].contains("abc.png"));
    assert!(messages[1].contains("#2") && messages[1].contains("def.png"));
    // A polyface mesh is a conversion in this writer, never exact; the texture
    // entry must not change that count.
    let mut plain_second = plain_triangle();
    for vertex in &mut plain_second.vertices {
        vertex.x += 10.0;
    }
    let plain = db_with(vec![
        SemanticGeometry::Mesh(plain_triangle()),
        SemanticGeometry::Mesh(plain_second),
    ]);
    let plain_output = exporter
        .export("dxf", &core_job(&plain))
        .expect("two triangle meshes export");
    assert_eq!(
        output.report.counts.exact, plain_output.report.counts.exact,
        "the texture entry must not change the exact count"
    );
    assert_eq!(
        output.report.counts.converted, 4,
        "two meshes + two textures"
    );
}
