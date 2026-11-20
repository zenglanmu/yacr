//! Container shape, frame/units conversion, determinism and geometry losses.

use super::*;

use cad_domain::{Completeness, SupportStatus};
use gltf::Semantic;

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
        uvs: Vec::new(),
        texture: None,
        pbr: None,
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
