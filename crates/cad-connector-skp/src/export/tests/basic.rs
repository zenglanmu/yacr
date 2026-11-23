//! Container/report basics: container shape, the pinned reader's round trip, unit
//! scale, per-attribute loss notes, budgets and descriptor checks.

use crate::export::{
    metres_per_source_unit, COLORS_DROPPED, DEGENERATE_TRIANGLE, LAYER_MATERIAL_FLATTENED,
    LIMITS_EXCEEDED, MISMATCHED_DESCRIPTOR, NORMALS_DROPPED, TEXTURE_NOT_WRITTEN, UNITS_UNKNOWN,
    UNSUPPORTED_GEOMETRY,
};
use cad_db::{EntityColor, EntityRenderAttributes};
use cad_domain::{ObjectId, Unit};

use super::*;

#[test]
fn an_exported_triangle_is_a_sketchup_container_with_a_complete_report() {
    let database = database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    assert!(output.bytes.len() > SKP_MAGIC.len());
    assert_eq!(
        output.bytes[..3],
        SKP_MAGIC,
        "the 2026 container starts with the FF FE FF string-record marker"
    );
    let report = &output.report;
    assert_eq!(report.connector, "skp");
    assert_eq!(
        report.direction,
        cad_connector_core::ReportDirection::Export
    );
    assert_eq!(report.level, cad_connector_core::FidelityLevel::L1);
    assert_eq!(report.counts.exact, 1);
    assert_eq!(report.counts.dropped, 0);
    assert_eq!(report.counts.bytes, output.bytes.len());
    assert_eq!(report.completeness, Completeness::Complete);
    assert!(report.entries.is_empty());
    assert!(report.capabilities.is_empty());
    // Stronger than "parses": the pinned crate's own whole-file grammar check
    // (open, re-encode, compare) accepts the bytes.
    skpwright::verify(&output.bytes).expect("the written container passes skpwright::verify");
}

/// Self-consistency only: the pinned `skpwright` reader reads the writer's bytes
/// back. This says nothing about SketchUp opening the file.
#[test]
fn the_pinned_reader_reads_the_written_triangle_back() {
    let database = database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    let imported = import(output.bytes).unwrap();
    let entities = imported
        .database
        .entities()
        .filter(|entity| display_mesh(entity).is_some_and(|mesh| !mesh.triangles.is_empty()))
        .count();
    assert!(entities >= 1, "the written triangles must read back");
    assert_eq!(mesh_triangles(&imported.database), 1);
    // The reader reports its own preview ceiling; the writer makes no claim on it.
    assert!(!imported.report.capabilities.is_empty());
}

#[test]
fn the_source_unit_scale_survives_the_round_trip() {
    let mut units = meter();
    units.source = Unit::Millimeter;
    let database = database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(0))]);
    let output = export(&database, Some(&units)).unwrap();
    assert_eq!(metres_per_source_unit(Some(&units)).unwrap(), 0.001);

    let imported = import(output.bytes).unwrap();
    let mesh = imported
        .database
        .entities()
        .find_map(display_mesh)
        .expect("one mesh entity");
    let mut xs: Vec<f64> = mesh.vertices.iter().map(|p| p.x).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // 1 mm source unit becomes 0.001 m, not 1 m and not 1 mm-of-a-metre-file.
    assert!((xs[xs.len() - 1] - 0.001).abs() < 1e-12, "{xs:?}");
}

#[test]
fn a_body_display_mesh_is_written() {
    let body = Body3d {
        kind: BodyKind::Sheet,
        exact: BodyExact::Unavailable {
            reason: "synthetic".into(),
        },
        display: BodyDisplay {
            mesh: triangle(),
            edges: Vec::new(),
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Complete,
        },
    };
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.completeness, Completeness::Complete);
}

#[test]
fn a_missing_or_drawing_unit_is_refused_by_name() {
    let database = database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(0))]);
    for units in [None, Some(UnitContext::drawing_units())] {
        let message = unsupported(export(&database, units.as_ref()).unwrap_err());
        assert!(message.starts_with(UNITS_UNKNOWN), "{message}");
    }
}

#[test]
fn a_line_entity_is_dropped_and_the_report_is_missing() {
    let database = database(vec![(
        SemanticGeometry::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(1.0, 0.0, 0.0),
        },
        LayerId(0),
    )]);
    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(output.report.counts.dropped, 1);
    assert_eq!(output.report.counts.exact, 0);
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
    assert_eq!(codes(&output), [UNSUPPORTED_GEOMETRY]);
    assert_eq!(
        output.report.entries[0].type_key.as_deref(),
        Some("AcDbLine")
    );
    assert!(output.report.entries[0].object.is_some());
}

#[test]
fn a_collinear_triangle_is_dropped_by_name() {
    let mut mesh = triangle();
    mesh.vertices[2] = point(2.0, 0.0, 0.0);
    let database = database(vec![(SemanticGeometry::Mesh(mesh), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(codes(&output), [DEGENERATE_TRIANGLE]);
    assert_eq!(output.report.counts.dropped, 1);
    assert_eq!(output.report.counts.exact, 0);
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
}

#[test]
fn layers_are_flattened_with_a_note_instead_of_a_silent_loss() {
    let database = database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(1))]);
    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(codes(&output), [LAYER_MATERIAL_FLATTENED]);
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.dropped, 0);
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
}

/// Per-vertex colours and normals have no SketchUp face carrier: the mesh is
/// still written, and each attribute is reported once so the verdict is
/// `Partial`, never a silent loss.
#[test]
fn per_vertex_colours_and_normals_are_reported_dropped() {
    let mut mesh = triangle();
    let normal = point(0.0, 0.0, 1.0);
    mesh.normals = vec![normal; 3];
    mesh.colors = vec![[10, 200, 10]; 3];
    let database = database(vec![(SemanticGeometry::Mesh(mesh), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.dropped, 0);
    let dropped_codes = codes(&output);
    assert!(dropped_codes.contains(&COLORS_DROPPED), "{dropped_codes:?}");
    assert!(
        dropped_codes.contains(&NORMALS_DROPPED),
        "{dropped_codes:?}"
    );
    for entry in &output.report.entries {
        assert_eq!(entry.status, cad_domain::SupportStatus::Partial);
        assert!(entry.object.is_none(), "one entry per kind, not per entity");
    }
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
}

/// A written mesh that references a base-colour texture keeps its faces in the
/// file, and the texture loss is one `Partial` conversion naming the entity and
/// the key. The bytes are identical to the untextured mesh.
#[test]
fn a_textured_mesh_reports_the_texture_not_written() {
    let mut textured = triangle();
    textured.uvs = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    textured.texture = Some("abc.png".into());
    let with = export(
        &database(vec![(SemanticGeometry::Mesh(textured), LayerId(0))]),
        Some(&meter()),
    )
    .unwrap();
    let without = export(
        &database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(0))]),
        Some(&meter()),
    )
    .unwrap();
    assert_eq!(
        with.bytes, without.bytes,
        "the SketchUp writer must write the same bytes for a textured and an untextured mesh"
    );
    assert_eq!(
        with.report.counts.exact, without.report.counts.exact,
        "the faces are still exact"
    );
    assert_eq!(codes(&with), [TEXTURE_NOT_WRITTEN]);
    let entry = &with.report.entries[0];
    assert_eq!(entry.status, cad_domain::SupportStatus::Partial);
    assert_eq!(entry.object, Some(ObjectId(1)));
    assert!(
        entry.message.contains("#1") && entry.message.contains("abc.png"),
        "the message must name the entity and the texture key: {}",
        entry.message
    );
    assert_eq!(with.report.counts.converted, 1);
    assert!(matches!(with.report.completeness, Completeness::Partial(_)));
    assert!(
        !codes(&without).contains(&TEXTURE_NOT_WRITTEN),
        "an untextured mesh must not report a texture loss"
    );
    assert_eq!(without.report.completeness, Completeness::Complete);
}

#[test]
fn an_explicit_colour_also_triggers_the_flattening_note() {
    // The render attributes are part of the database, so this fixture sets them
    // before finishing the builder.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(2));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbSubDMesh".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(triangle()),
            draw_order: 0,
        })
        .unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::Explicit([200, 10, 10]),
                ..Default::default()
            },
        )
        .unwrap();
    let database = builder.finish().unwrap();

    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(codes(&output), [LAYER_MATERIAL_FLATTENED]);
}

#[test]
fn the_same_document_writes_the_same_bytes() {
    let database = database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(0))]);
    let first = export(&database, Some(&meter())).unwrap();
    let second = export(&database, Some(&meter())).unwrap();
    assert_eq!(first.bytes, second.bytes);
    assert_eq!(first.bytes.len(), second.report.counts.bytes);
}

#[test]
fn a_triangle_budget_is_refused_by_name() {
    let database = database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(0))]);
    let units = meter();
    let mut job = job(&database, Some(&units));
    job.limits.max_output_triangles = 0;
    let message = unsupported(
        crate::SkpExporter::new(&crate::DESCRIPTOR_SKP)
            .export("skp", &job)
            .unwrap_err(),
    );
    assert!(message.starts_with(LIMITS_EXCEEDED), "{message}");
}

#[test]
fn an_empty_database_is_refused_instead_of_writing_an_empty_success() {
    let database = database(Vec::new());
    let message = unsupported(export(&database, Some(&meter())).unwrap_err());
    assert!(message.starts_with(crate::export::NO_GEOMETRY), "{message}");
}

#[test]
fn a_mismatched_format_id_is_refused_by_name() {
    let database = database(vec![(SemanticGeometry::Mesh(triangle()), LayerId(0))]);
    let units = meter();
    let job = job(&database, Some(&units));
    let exporter = crate::SkpExporter::new(&crate::DESCRIPTOR_SKP);
    assert_eq!(exporter.format_id(), "skp");
    let message = unsupported(exporter.export("stl", &job).unwrap_err());
    assert!(message.starts_with(MISMATCHED_DESCRIPTOR), "{message}");
    assert!(message.contains("\"stl\""), "{message}");
}
