use super::*;
use cad_domain::{DatabaseId, DocumentId};
use cad_import_core::FidelityLevel;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn fixture(relative: &str) -> Vec<u8> {
    std::fs::read(fixtures_root().join(relative)).expect("fixture is committed")
}

fn request(bytes: Vec<u8>) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: cad_import_core::ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

fn one_line_db() -> cad_db::DrawingDatabase {
    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId};
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
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
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 5.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}

/// A one-triangle model-space mesh on layer `0`, the smallest database every
/// mesh/IFC/SKP writer accepts (same minimal pattern as
/// `cad-import-core::mesh_import`).
fn triangle_db() -> cad_db::DrawingDatabase {
    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{
        EntityId, LayerId, Mesh, ObjectId, Point3, Revision, SemanticGeometry, SpaceId,
    };
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    let normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "TestMesh".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Mesh(Mesh {
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
                normals: vec![normal; 3],
                face_sources: Vec::new(),
                colors: Vec::new(),
            }),
            draw_order: 0,
        })
        .unwrap();
    builder.finish().unwrap()
}

/// Every fixture must be claimed by the same lane the old `detect_format`
/// decision named, so the registry is the dispatch authority and the enum is
/// only a helper.
#[test]
fn committed_fixtures_route_to_the_expected_connector() {
    let registry = ConnectorRegistry::builtin();
    // (fixture, expected descriptor id, expected detect_format answer)
    let cases: &[(&str, &str, DrawingFormat)] = &[
        ("dwg/synthetic-four-lines.dwg", "dwg", DrawingFormat::Dwg),
        ("dxf/synthetic-acis-cube.dxf", "dxf", DrawingFormat::Dxf),
        ("ifc/synthetic-wall.ifc", "ifc", DrawingFormat::Ifc),
        ("stl/synthetic-quad.stl", "stl", DrawingFormat::Stl),
        ("stl/synthetic-quad-binary.stl", "stl", DrawingFormat::Stl),
        ("ply/synthetic-attrs-ascii.ply", "ply", DrawingFormat::Ply),
        ("off/synthetic-quad-triangle.off", "off", DrawingFormat::Off),
        ("dgn/smalltest.dgn", "dgn", DrawingFormat::Dgn),
        // DGN V8 is an OLE container: the DGN adapter's sniff must win over
        // the RVT lane.
        ("dgn/test_dgnv8.dgn", "dgn", DrawingFormat::Ole),
        (
            "rvt/upstream-metadata-scaffold-2024-no-geometry.rvt",
            "rvt",
            DrawingFormat::Ole,
        ),
    ];
    for (relative, expected_id, expected_format) in cases {
        let bytes = fixture(relative);
        assert_eq!(
            detect_format(&bytes),
            *expected_format,
            "{relative}: the shared detector changed"
        );
        let descriptor = registry
            .route(&bytes)
            .unwrap_or_else(|| panic!("{relative}: no connector claimed the bytes"));
        assert_eq!(descriptor.id, *expected_id, "{relative}");
    }

    // SKP: the repository has no committed `.skp` model (only a SOURCE.md
    // explaining the licensing boundary), so the probe is exercised on the
    // synthetic header the dispatcher tests use. NOT a real-model claim.
    let mut header = vec![
        0xFF,
        0xFE,
        0xFF,
        "SketchUp Model".encode_utf16().count() as u8,
    ];
    for unit in "SketchUp Model".encode_utf16() {
        header.extend_from_slice(&unit.to_le_bytes());
    }
    header.extend_from_slice(&[0xFF, 0xFE, 0xFF, 9]);
    for unit in "{17.3.116}".encode_utf16() {
        header.extend_from_slice(&unit.to_le_bytes());
    }
    header.extend_from_slice(&[0u8; 16]);
    assert_eq!(registry.route(&header).map(|d| d.id), Some("skp"));
}

#[test]
fn unknown_bytes_are_the_single_unsupported_input_refusal() {
    let registry = ConnectorRegistry::builtin();
    assert!(registry.route(b"glTF\x02\x00\x00\x00").is_none());
    let error = registry
        .import(
            &request(b"not a drawing".to_vec()),
            &|| false,
            &cad_import_core::NoopProgress,
        )
        .err()
        .expect("unknown bytes are refused");
    match error {
        CadError::CorruptData(message) => {
            assert_eq!(message, crate::import::UNSUPPORTED_INPUT)
        }
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn gltf_bytes_route_like_the_shared_detector() {
    // The registry owns dispatch and `detect_format` is only a helper; a glTF
    // document must not be claimed by one and called unknown by the other.
    let registry = ConnectorRegistry::builtin();
    let json = br#"{"asset":{"version":"2.0"}}"#;
    assert_eq!(detect_format(json), DrawingFormat::Gltf);
    assert_eq!(registry.route(json).map(|d| d.id), Some("gltf"));

    let mut glb = b"glTF".to_vec();
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&12u32.to_le_bytes());
    assert_eq!(detect_format(&glb), DrawingFormat::Gltf);
    assert_eq!(registry.route(&glb).map(|d| d.id), Some("gltf"));
}

#[test]
fn ole_sniff_happens_after_the_file_size_limit() {
    let registry = ConnectorRegistry::builtin();
    let mut request = request(fixture("dgn/test_dgnv8.dgn"));
    request.limits.max_file_bytes = 8;
    let error = registry
        .import(&request, &|| false, &cad_import_core::NoopProgress)
        .err()
        .expect("the OLE container is over the limit");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
}

#[test]
fn unimplemented_export_keys_refuse_by_name() {
    let registry = ConnectorRegistry::builtin();
    let database = one_line_db();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: None,
        space: cad_import_core::ExportSpace::Model,
        limits: cad_import_core::ExportLimits::default(),
    };
    let settings = ExportSettings::default();
    for (id, code) in [
        ("step", "export.step.not_implemented"),
        ("dgn", "export.dgn.not_implemented"),
        ("rvt", "export.rvt.not_implemented"),
    ] {
        assert!(
            registry
                .descriptor(id)
                .is_some_and(|descriptor| descriptor.export.is_none()),
            "{id} must not declare an export capability this build cannot honour"
        );
        match registry.export(id, &job, &settings) {
            Ok(_) => panic!("{id} must not silently succeed"),
            Err(CadError::Unsupported(message)) => {
                assert!(
                    message.contains(code),
                    "{id} refusal must name {code}: {message}"
                );
                assert!(
                    message.contains("writer is not wired in this build"),
                    "{id} refusal must say why: {message}"
                );
            }
            Err(other) => panic!("{id}: expected Unsupported, got {other:?}"),
        }
    }
    match registry.export("not-a-format", &job, &settings) {
        Err(CadError::Unsupported(message)) => assert!(
            message.contains("export.unknown_format: not-a-format"),
            "{message}"
        ),
        other => panic!("expected an unknown-format refusal, got {other:?}"),
    }
}

#[test]
fn dxf_export_goes_through_the_acadrust_adapter() {
    let registry = ConnectorRegistry::builtin();
    let database = one_line_db();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: None,
        space: cad_import_core::ExportSpace::Model,
        limits: cad_import_core::ExportLimits::default(),
    };
    let output = registry
        .export("dxf", &job, &ExportSettings::default())
        .expect("the DXF writer is wired");
    assert!(!output.bytes.is_empty());
    assert_eq!(output.report.connector, "dxf");
    assert_eq!(
        output.report.direction,
        cad_import_core::ReportDirection::Export
    );
}

/// Every read-only descriptor must be a **named** refusal.
///
/// A descriptor with `export: None` has no writer; if its id were missing
/// from `REFUSALS`, `export(id)` would fall through to `export.unknown_format`
/// and imply the format merely does not exist. The invariant is the inverse
/// of the existing "every export lane's descriptor declares a capability":
/// every capability that is absent is still an answerable format.
#[test]
fn every_descriptor_without_a_writer_is_a_named_refusal() {
    let registry = ConnectorRegistry::builtin();
    assert_eq!(registry.descriptors().count(), 12);
    let mut read_only = Vec::new();
    for descriptor in registry.descriptors() {
        if descriptor.export.is_none() {
            assert!(
                registry.refusals.iter().any(|(id, _)| *id == descriptor.id),
                "{} has no writer and is not in REFUSALS, so its id would be \
                     export.unknown_format",
                descriptor.id
            );
            read_only.push(descriptor.id);
        }
    }
    assert_eq!(
        read_only.len(),
        registry.refusals.len(),
        "REFUSALS must contain exactly the descriptors without a writer: {read_only:?}"
    );

    let database = one_line_db();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: None,
        space: cad_import_core::ExportSpace::Model,
        limits: cad_import_core::ExportLimits::default(),
    };
    for id in read_only {
        match registry.export(id, &job, &ExportSettings::default()) {
            Err(CadError::Unsupported(message)) => {
                assert!(
                    message.contains(&format!("export.{id}.not_implemented")),
                    "{id} must refuse by its own name: {message}"
                );
                assert!(
                    !message.contains("export.unknown_format"),
                    "{id} must not look like an unknown format: {message}"
                );
            }
            other => panic!("{id}: expected a named refusal, got {other:?}"),
        }
    }
}

#[test]
fn every_descriptor_is_listed_once() {
    let registry = ConnectorRegistry::builtin();
    let mut ids: Vec<&'static str> = registry.descriptors().map(|d| d.id).collect();
    assert_eq!(ids.len(), 12, "descriptors: {ids:?}");
    ids.sort_unstable();
    let mut unique = ids.clone();
    unique.dedup();
    assert_eq!(unique.len(), 12, "duplicate ids in {ids:?}");
    for id in [
        "dwg", "dxf", "step", "ifc", "dgn", "rvt", "skp", "obj", "stl", "ply", "off", "gltf",
    ] {
        assert!(unique.contains(&id), "missing descriptor {id}: {unique:?}");
        assert!(
            registry.descriptor(id).is_some(),
            "descriptor({id}) is None"
        );
    }
    // Every export key names a registered descriptor, and every refusal's
    // descriptor declares no writer.
    for (id, descriptor) in registry.refusals {
        assert!(
            registry.descriptor(id).is_some(),
            "refusal key {id} has no descriptor"
        );
        assert!(
            descriptor.export.is_none(),
            "{id} refuses but its descriptor declares an export capability"
        );
    }
    for connector in &registry.exports {
        assert!(
            registry.descriptor(connector.format_id).is_some(),
            "{} has no descriptor",
            connector.format_id
        );
    }
}

/// The registry may only advertise a writer it can actually run.
///
/// A key is in `exports` **iff** its descriptor declares an export
/// capability and the call produces real bytes; a refusal key's descriptor
/// declares none, and no key is both.
#[test]
fn every_export_key_matches_its_descriptor_capability() {
    let registry = ConnectorRegistry::builtin();
    let database = triangle_db();
    // IFC and SKP require a known source length unit; the other writers
    // accept `None`, so the shared job carries one metre.
    let units = cad_domain::UnitContext::from_meter();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: Some(&units),
        space: cad_import_core::ExportSpace::Model,
        limits: cad_import_core::ExportLimits::default(),
    };
    let settings = ExportSettings::default();
    assert!(
        !registry.exports.is_empty(),
        "the production registry must wire at least one writer"
    );
    for connector in &registry.exports {
        let descriptor = registry
            .descriptor(connector.format_id)
            .expect("a registered lane has a descriptor");
        assert!(
            descriptor.export.is_some(),
            "{} is a lane but its descriptor has no export capability",
            connector.format_id
        );
        let output = registry
            .export(connector.format_id, &job, &settings)
            .unwrap_or_else(|error| panic!("{} lane failed: {error:?}", connector.format_id));
        assert!(
            !output.bytes.is_empty(),
            "{} advertised a writer but wrote no bytes",
            connector.format_id
        );
        // The report names the *descriptor* id, so the ASCII STL variant
        // (`stl-ascii`) reports `stl` like the binary key does.
        assert_eq!(output.report.connector, descriptor.id);
    }
    for (id, _) in registry.refusals {
        assert!(
            !registry.exports.iter().any(|c| c.format_id == *id),
            "{id} is both a writer lane and a refusal"
        );
    }
    // Conversely, a declared capability must have a lane; a descriptor may
    // not advertise a writer nothing can dispatch to.
    for descriptor in registry.descriptors() {
        if descriptor.export.is_some() {
            assert!(
                registry
                    .exports
                    .iter()
                    .any(|connector| connector.descriptor.id == descriptor.id),
                "{} declares an export capability but has no export lane",
                descriptor.id
            );
        }
    }
}

/// Every writer lane must produce real bytes and a report naming its
/// descriptor. The byte prefixes are the container signatures this build's
/// readers recognise, so the smoke test would catch a lane wired to the wrong
/// exporter (for example `stl-ascii` writing binary STL).
#[test]
fn every_writer_lane_writes_real_bytes_and_names_its_connector() {
    let registry = ConnectorRegistry::builtin();
    let database = triangle_db();
    let units = cad_domain::UnitContext::from_meter();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: Some(&units),
        space: cad_import_core::ExportSpace::Model,
        limits: cad_import_core::ExportLimits::default(),
    };
    let settings = ExportSettings::default();
    // (registry key, expected byte prefix)
    let cases: &[(&str, Option<&[u8]>)] = &[
        ("stl", None),
        ("stl-ascii", Some(b"solid ")),
        ("obj", None),
        ("ply", Some(b"ply")),
        ("off", Some(b"OFF")),
        ("ifc", Some(b"ISO-10303-21;")),
        ("skp", Some(&[0xFF, 0xFE, 0xFF])),
        // The two glTF containers are one connector: `glb` is the binary
        // `glTF` container, `gltf` the JSON object.
        ("glb", Some(b"glTF")),
        ("gltf", Some(b"{")),
    ];
    for (id, prefix) in cases {
        let descriptor = registry
            .descriptor(id)
            .unwrap_or_else(|| panic!("{id} has no descriptor"));
        let output = registry
            .export(id, &job, &settings)
            .unwrap_or_else(|error| panic!("{id} lane failed: {error:?}"));
        assert!(!output.bytes.is_empty(), "{id} wrote no bytes");
        assert_eq!(
            output.report.connector, descriptor.id,
            "{id} report must name the descriptor id"
        );
        assert_eq!(
            output.report.direction,
            cad_import_core::ReportDirection::Export,
            "{id} must be an export report"
        );
        if let Some(prefix) = prefix {
            assert!(
                output.bytes.starts_with(prefix),
                "{id}: bytes must start with {prefix:?}"
            );
        }
    }
    // `stl-ascii` is a second key of the STL descriptor, not a descriptor.
    assert_eq!(registry.descriptor("stl-ascii").map(|d| d.id), Some("stl"));
}

#[test]
fn a_glb_export_round_trips_through_the_registry_import() {
    // The writer and the reader are two lanes of one connector: the GLB it
    // writes must be claimed by the glTF import probe and come back as the
    // same single-triangle mesh (the frame rotation is applied once in each
    // direction).
    let registry = ConnectorRegistry::builtin();
    let database = triangle_db();
    let units = cad_domain::UnitContext::from_meter();
    let job = ExportJob {
        database: &database,
        document_id: DocumentId(1),
        units: Some(&units),
        space: cad_import_core::ExportSpace::Model,
        limits: cad_import_core::ExportLimits::default(),
    };
    let output = registry
        .export("glb", &job, &ExportSettings::default())
        .expect("the GLB writer is wired");
    assert_eq!(output.report.connector, "gltf");
    assert_eq!(registry.route(&output.bytes).map(|d| d.id), Some("gltf"));

    let drawing = registry
        .import(
            &request(output.bytes),
            &|| false,
            &cad_import_core::NoopProgress,
        )
        .expect("the glTF lane imports the bytes it wrote");
    assert_eq!(
        drawing.database.entity_count(),
        1,
        "one triangle in, one mesh entity out"
    );
    let triangles: Vec<usize> = drawing
        .database
        .entities()
        .map(|entity| match &entity.geometry {
            cad_domain::SemanticGeometry::Mesh(mesh) => mesh.triangles.len(),
            other => panic!("expected a mesh, got {other:?}"),
        })
        .collect();
    assert_eq!(triangles, vec![1]);
}

#[test]
fn registry_import_matches_the_probe_decision() {
    // The registry delegates to `Importer::import_with_progress`, so a
    // registry import and a direct adapter import agree on the fixture.
    let registry = ConnectorRegistry::builtin();
    let bytes = fixture("stl/synthetic-quad.stl");
    assert_eq!(registry.route(&bytes).map(|d| d.id), Some("stl"));
    let drawing = registry
        .import(&request(bytes), &|| false, &cad_import_core::NoopProgress)
        .expect("the STL lane imports");
    assert_eq!(drawing.database.entity_count(), 1);
}

#[test]
fn a_connector_descriptor_is_available_through_the_trait() {
    let exporter = AcadrustExporter::new(
        ExportFormat::DxfText,
        AcisVersion::default(),
        DxfTarget::default(),
    );
    // `descriptor` exists on both `Connector` and `Exporter`; the test names
    // the trait it is exercising.
    assert_eq!(Exporter::descriptor(&exporter).id, "dxf");
    assert_eq!(cad_import_core::Connector::descriptor(&exporter).id, "dxf");
    assert_eq!(exporter.format_id(), "dxf");
    let descriptor = Exporter::descriptor(&exporter);
    assert!(descriptor.extensions.contains(&"dxf"));
    let export = descriptor.export.expect("DXF declares an export direction");
    assert_eq!(export.level, FidelityLevel::L1);
    assert!(export.lossy, "a Save As is lossy");
    assert_eq!(
        descriptor
            .import
            .expect("DXF declares an import direction")
            .container,
        "dxf"
    );
    assert_eq!(descriptor.name_key, "connector.dxf.name");
}
