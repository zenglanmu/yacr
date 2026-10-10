//! Export contract tests.
//!
//! Writer + our reader agreement is **internal consistency**, not third-party
//! compatibility: nothing here claims AutoCAD, ODA or any other consumer opens
//! the output.

use std::io::Cursor;
use std::sync::Arc;

use acadrust::{CadDocument, DxfReader, DxfWriter, EntityType, Transparency, Vector3};
use cad_db::{
    BlockDefinition, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder,
    DynamicBlockState, DynamicBlockVisibility, EntityLineType, EntityRenderAttributes,
    EntityTransparency, Layer, LineType as DbLineType, LinetypePattern, Style,
};
use cad_domain::*;

use super::report::{ExportCounts, ExportFormat, ExportLimits, ExportReport, ExportSpace};
use super::{export, ExportRequest};
use crate::{AcadrustImporter, ImportLimits, ImportRequest, Importer};

/// Import bytes through the single acadrust boundary (test-local helper).
fn import_bytes(bytes: &[u8]) -> DrawingDatabase {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("fixture imports")
        .database
}

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn db_with(geometries: Vec<SemanticGeometry>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    for (index, geometry) in geometries.into_iter().enumerate() {
        let id = index as u128 + 1;
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "test".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(id),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry,
                draw_order: index as i64,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

fn export_db(db: &DrawingDatabase) -> (Vec<u8>, ExportReport) {
    export(&ExportRequest {
        database: db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: None,
    })
    .unwrap()
}

fn export_one(geometry: SemanticGeometry) -> (Vec<u8>, ExportReport) {
    export_db(&db_with(vec![geometry]))
}

fn read_entities(bytes: &[u8]) -> Vec<EntityType> {
    DxfReader::from_reader(Cursor::new(bytes.to_vec()))
        .unwrap()
        .read()
        .unwrap()
        .entities()
        .cloned()
        .collect()
}

fn assert_point(actual: Point3, expected: Point3) {
    assert!(
        (actual.x - expected.x).abs() < 1e-9
            && (actual.y - expected.y).abs() < 1e-9
            && (actual.z - expected.z).abs() < 1e-9,
        "point {actual:?} != {expected:?}"
    );
}

// ---------------------------------------------------------------------------
// 1. Per-kind mapping units (writer + our reader = internal consistency).
// ---------------------------------------------------------------------------

#[test]
fn line_maps_to_an_exact_dxf_line() {
    let (bytes, report) = export_one(SemanticGeometry::Line {
        start: p(0.0, 0.0, 0.0),
        end: p(10.0, 5.0, 0.0),
    });
    let entities = read_entities(&bytes);
    assert_eq!(entities.len(), 1);
    match &entities[0] {
        EntityType::Line(line) => {
            assert_eq!(line.start, Vector3::new(0.0, 0.0, 0.0));
            assert_eq!(line.end, Vector3::new(10.0, 5.0, 0.0));
        }
        other => panic!("expected LINE, got {other:?}"),
    }
    assert_eq!(report.counts.exact, 1);
    assert_eq!(report.completeness, Completeness::Complete);
}

#[test]
fn z_constant_polyline_keeps_bulges_as_an_exact_lwpolyline() {
    let (bytes, report) = export_one(SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0, 2.0), p(5.0, 0.0, 2.0), p(5.0, 5.0, 2.0)],
        bulges: vec![0.0, 0.5, 0.0],
        closed: false,
    });
    match &read_entities(&bytes)[0] {
        EntityType::LwPolyline(poly) => {
            assert!((poly.elevation - 2.0).abs() < 1e-9);
            assert_eq!(poly.vertices.len(), 3);
            assert!((poly.vertices[1].bulge - 0.5).abs() < 1e-12);
        }
        other => panic!("expected LWPOLYLINE, got {other:?}"),
    }
    assert_eq!(report.counts.exact, 1);
}

#[test]
fn non_planar_polyline_maps_to_polyline3d_and_drops_bulges_explicitly() {
    let (bytes, report) = export_one(SemanticGeometry::Polyline {
        points: vec![p(0.0, 0.0, 0.0), p(5.0, 0.0, 3.0), p(5.0, 5.0, 6.0)],
        bulges: vec![0.0, 0.5, 0.0],
        closed: false,
    });
    match &read_entities(&bytes)[0] {
        EntityType::Polyline3D(poly) => assert_eq!(poly.vertices.len(), 3),
        other => panic!("expected POLYLINE3D, got {other:?}"),
    }
    assert_eq!(report.counts.converted, 1);
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.polyline_bulge"));
}

#[test]
fn arc_and_ellipse_and_point_map_exactly() {
    let (bytes, report) = export_one(SemanticGeometry::Arc {
        center: p(1.0, 2.0, 0.0),
        normal: p(0.0, 0.0, 1.0),
        radius: 4.0,
        start: 0.25,
        sweep: 1.0,
    });
    match &read_entities(&bytes)[0] {
        EntityType::Arc(arc) => {
            assert!((arc.radius - 4.0).abs() < 1e-12);
            assert!((arc.start_angle - 0.25).abs() < 1e-12);
            assert!((arc.end_angle - 1.25).abs() < 1e-12);
        }
        other => panic!("expected ARC, got {other:?}"),
    }
    assert_eq!(report.counts.exact, 1);

    let (bytes, _) = export_one(SemanticGeometry::Ellipse {
        center: p(0.0, 0.0, 0.0),
        normal: p(0.0, 0.0, 1.0),
        major_axis: p(3.0, 0.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    });
    assert!(matches!(&read_entities(&bytes)[0], EntityType::Ellipse(_)));

    let (bytes, _) = export_one(SemanticGeometry::Point(p(7.0, 8.0, 9.0)));
    match &read_entities(&bytes)[0] {
        EntityType::Point(point) => assert_eq!(point.location, Vector3::new(7.0, 8.0, 9.0)),
        other => panic!("expected POINT, got {other:?}"),
    }
}

#[test]
fn mesh_maps_to_a_polyface_mesh_and_reports_the_drop() {
    let mesh = Mesh {
        vertices: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        triangles: vec![[0, 1, 2]],
        normals: vec![p(0.0, 0.0, 1.0); 3],
        face_sources: Vec::new(),
        colors: vec![[255, 0, 0]; 3],
    };
    let (bytes, report) = export_one(SemanticGeometry::Mesh(mesh));
    match &read_entities(&bytes)[0] {
        EntityType::PolyfaceMesh(poly) => {
            assert_eq!(poly.vertices.len(), 3);
            assert_eq!(poly.faces.len(), 1);
        }
        other => panic!("expected POLYFACEMESH, got {other:?}"),
    }
    assert_eq!(report.counts.converted, 1);
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.mesh"));
}

#[test]
fn single_line_text_is_exact_and_newline_text_becomes_mtext() {
    let (bytes, report) = export_one(SemanticGeometry::Text {
        text: "HELLO".into(),
        position: p(1.0, 2.0, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
    });
    assert!(matches!(&read_entities(&bytes)[0], EntityType::Text(_)));
    assert_eq!(report.counts.exact, 1);

    let (bytes, report) = export_one(SemanticGeometry::Text {
        text: "LINE1\nLINE2".into(),
        position: p(1.0, 2.0, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
    });
    assert!(matches!(&read_entities(&bytes)[0], EntityType::MText(_)));
    assert_eq!(report.counts.converted, 1);
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.text_mtext"));
}

#[test]
fn insert_explodes_block_members_with_the_placement_transform() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    // Block-local member line (0,0)-(1,0).
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(2),
                type_key: "test".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(2),
            layer: LayerId(0),
            space: SpaceId::Block(BlockId(1)),
            geometry: SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            draw_order: 0,
        })
        .unwrap();
    // Model INSERT at (10, 0) with the identity rotation/scale.
    let mut transform = Transform3::identity();
    transform.matrix[0][3] = 10.0;
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "test".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Insert {
                block: BlockId(1),
                transform,
            },
            draw_order: 0,
        })
        .unwrap();
    let db = builder.finish().unwrap();

    let (bytes, report) = export_db(&db);
    let entities = read_entities(&bytes);
    assert_eq!(entities.len(), 1);
    match &entities[0] {
        EntityType::Line(line) => {
            assert_eq!(line.start, Vector3::new(10.0, 0.0, 0.0));
            assert_eq!(line.end, Vector3::new(11.0, 0.0, 0.0));
        }
        other => panic!("expected exploded LINE, got {other:?}"),
    }
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.insert"));
    // The block is referenced, so it is not reported as unreferenced.
    assert!(!report
        .entries
        .iter()
        .any(|e| e.code == "export.dropped.unreferenced_block"));
}

#[test]
fn unsupported_kinds_are_dropped_with_stable_codes() {
    let cases: Vec<(&str, SemanticGeometry)> = vec![
        (
            "shape",
            SemanticGeometry::Shape {
                shape_name: "test".into(),
                code: 1,
                position: p(0.0, 0.0, 0.0),
                size: 1.0,
                rotation: 0.0,
                font: None,
            },
        ),
        (
            "image",
            SemanticGeometry::Image {
                origin: p(0.0, 0.0, 0.0),
                u: p(1.0, 0.0, 0.0),
                v: p(0.0, 1.0, 0.0),
                pixels: [1.0, 1.0],
                file: None,
                clip: None,
                visible: true,
            },
        ),
        (
            "mask",
            SemanticGeometry::Mask {
                boundary: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)],
                inverted: false,
            },
        ),
        (
            "opaque",
            SemanticGeometry::Opaque {
                type_key: "ACAD_PROXY_ENTITY".into(),
                version: 1,
                payload: Vec::new(),
            },
        ),
    ];
    for (kind, geometry) in cases {
        let (bytes, report) = export_one(geometry);
        assert!(
            read_entities(&bytes).is_empty(),
            "{kind} must not be emitted"
        );
        assert_eq!(report.counts.dropped, 1, "{kind}");
        assert!(
            report
                .entries
                .iter()
                .any(|e| e.code == format!("export.dropped.{kind}")),
            "{kind}: {:?}",
            report.entries
        );
        assert_ne!(report.completeness, Completeness::Complete, "{kind}");
    }
}

// ---------------------------------------------------------------------------
// 2. OCS round-trip: export(import(x)) == x.
// ---------------------------------------------------------------------------

/// Build a source acadrust document with a tilted circle and a bulge polyline,
/// import it, export it, import the export, and compare the geometry.
#[test]
fn tilted_circle_and_bulge_polyline_round_trip_through_ocs() {
    let mut source = CadDocument::new();
    let mut circle = acadrust::entities::Circle::from_coords(3.0, 4.0, 5.0, 2.0);
    let tilted = Vector3::new(1.0, 1.0, 1.0).normalize();
    circle.normal = tilted;
    source.add_entity(EntityType::Circle(circle)).unwrap();

    let mut poly = acadrust::entities::LwPolyline::new();
    poly.elevation = 1.5;
    poly.add_vertex(acadrust::entities::LwVertex {
        location: acadrust::Vector2::new(0.0, 0.0),
        bulge: 0.0,
        start_width: 0.0,
        end_width: 0.0,
        vertex_id: 0,
    });
    poly.add_vertex(acadrust::entities::LwVertex {
        location: acadrust::Vector2::new(4.0, 0.0),
        bulge: 0.75,
        start_width: 0.0,
        end_width: 0.0,
        vertex_id: 0,
    });
    poly.add_vertex(acadrust::entities::LwVertex {
        location: acadrust::Vector2::new(4.0, 3.0),
        bulge: 0.0,
        start_width: 0.0,
        end_width: 0.0,
        vertex_id: 0,
    });
    source.add_entity(EntityType::LwPolyline(poly)).unwrap();

    let source_bytes = DxfWriter::new(&source).write_to_vec().unwrap();
    let imported = import_bytes(&source_bytes);
    let (exported_bytes, report) = export_db(&imported);
    let reimported = import_bytes(&exported_bytes);

    // Compare the geometry the importer resolves for each pass.
    let first: Vec<&SemanticGeometry> = imported.entities().map(|e| &e.geometry).collect();
    let second: Vec<&SemanticGeometry> = reimported.entities().map(|e| &e.geometry).collect();
    assert_eq!(first.len(), second.len(), "entity count changed");
    for (a, b) in first.iter().zip(second.iter()) {
        match (a, b) {
            (
                SemanticGeometry::Circle {
                    center: ca,
                    normal: na,
                    radius: ra,
                },
                SemanticGeometry::Circle {
                    center: cb,
                    normal: nb,
                    radius: rb,
                },
            ) => {
                assert_point(*ca, *cb);
                assert_point(*na, *nb);
                assert!((ra - rb).abs() < 1e-9);
            }
            (
                SemanticGeometry::Polyline {
                    points: pa,
                    bulges: ba,
                    closed: closed_a,
                },
                SemanticGeometry::Polyline {
                    points: pb,
                    bulges: bb,
                    closed: closed_b,
                },
            ) => {
                assert_eq!(closed_a, closed_b);
                assert_eq!(pa.len(), pb.len());
                for (x, y) in pa.iter().zip(pb.iter()) {
                    assert_point(*x, *y);
                }
                assert_eq!(ba.len(), bb.len());
                for (x, y) in ba.iter().zip(bb.iter()) {
                    assert!((x - y).abs() < 1e-9);
                }
            }
            other => panic!("kind changed across the round trip: {other:?}"),
        }
    }
    // Both source kinds are exact.
    assert_eq!(report.counts.dropped, 0);
    assert_eq!(report.counts.converted, 0);
}

// ---------------------------------------------------------------------------
// 3. Fixture self round-trip with explicit loss assertions.
// ---------------------------------------------------------------------------

fn fixture_bytes(path: &str) -> Arc<[u8]> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join(path)).expect("fixture readable");
    Arc::from(bytes.into_boxed_slice())
}

/// The set of `export.*` codes a fixture's export produced, sorted.
fn code_set(report: &ExportReport) -> Vec<String> {
    let mut codes: Vec<String> = report
        .entries
        .iter()
        .map(|e| e.code.clone())
        .chain(report.notes.iter().map(|e| e.code.clone()))
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

#[test]
fn committed_fixtures_report_their_expected_losses() {
    // Hard-coded expected loss codes per manifest-governed fixture. A regression
    // that starts dropping (or no longer drops) a kind fails here.
    let expectations: &[(&str, &[&str])] = &[
        (
            "fixtures/plot/a4-layout.dwg",
            &[
                "export.dropped.paper_space",
                "export.note.bylayer_flattened",
                "export.note.units_missing",
            ],
        ),
        (
            "fixtures/dwg/synthetic-four-lines.dwg",
            &["export.note.bylayer_flattened", "export.note.units_missing"],
        ),
        (
            "fixtures/dxf/qcad-flange/flange.dxf",
            &[
                "export.converted.compound",
                "export.converted.mesh",
                "export.dropped.paper_space",
                "export.dropped.unreferenced_block",
                "export.note.bylayer_flattened",
                "export.note.units_missing",
            ],
        ),
    ];
    for (path, expected) in expectations {
        let imported = import_bytes(&fixture_bytes(path));
        let (bytes, report) = export_db(&imported);
        // The output is well-formed ASCII DXF (our reader agrees).
        let reimported = import_bytes(&bytes);

        // (a) When nothing was converted, every model geometry is exact and must
        // round-trip pointwise. A fixture with conversions legitimately changes
        // structure, so its exactness is asserted through the code set instead.
        if report.counts.converted == 0 {
            let model = |db: &DrawingDatabase| -> Vec<SemanticGeometry> {
                let mut geometries: Vec<(i64, SemanticGeometry)> = db
                    .entities()
                    .filter(|e| e.space == SpaceId::Model)
                    .map(|e| (e.draw_order, e.geometry.clone()))
                    .collect();
                geometries.sort_by_key(|(order, _)| *order);
                geometries.into_iter().map(|(_, g)| g).collect()
            };
            let before = model(&imported);
            let after = model(&reimported);
            assert_eq!(
                before.len(),
                after.len(),
                "{path}: model entity count changed"
            );
            for (a, b) in before.iter().zip(after.iter()) {
                match (a, b) {
                    (
                        SemanticGeometry::Line { start: sa, end: ea },
                        SemanticGeometry::Line { start: sb, end: eb },
                    ) => {
                        assert_point(*sa, *sb);
                        assert_point(*ea, *eb);
                    }
                    (x, y) => assert_eq!(x, y, "{path}: geometry changed"),
                }
            }
        }

        // (b) The loss code set matches the hard-coded expectation.
        assert_eq!(&code_set(&report), expected, "{path}");
        // (c) Any drop implies the export is not Complete.
        if report.counts.dropped > 0 {
            assert_ne!(report.completeness, Completeness::Complete, "{path}");
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Deterministic DXF text for a tiny synthetic document.
// ---------------------------------------------------------------------------

#[test]
fn tiny_document_writes_deterministic_ascii_dxf() {
    let db = db_with(vec![
        SemanticGeometry::Line {
            start: p(0.0, 0.0, 0.0),
            end: p(1.0, 0.0, 0.0),
        },
        SemanticGeometry::Circle {
            center: p(2.0, 2.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 1.0,
        },
    ]);
    let (first, report) = export_db(&db);
    let (second, _) = export_db(&db);
    // Deterministic: identical input bytes produce identical output bytes.
    assert_eq!(first, second);
    assert_eq!(report.counts.bytes, first.len());
    let text = std::str::from_utf8(&first).expect("ASCII DXF is valid UTF-8");
    for marker in ["SECTION", "ENTITIES", "LINE", "CIRCLE", "EOF"] {
        assert!(text.contains(marker), "missing DXF marker {marker}");
    }
    // A stable, diffable excerpt: the entity type records in output order.
    let types: Vec<&str> = text
        .lines()
        .filter(|line| matches!(line.trim(), "LINE" | "CIRCLE" | "LWPOLYLINE"))
        .collect();
    assert_eq!(types, vec!["LINE", "CIRCLE"]);
}

// ---------------------------------------------------------------------------
// 4. Negative contract: the leaf mapping is exhaustive (compile-enforced) and
//    every dropped kind yields an explicit diagnostic.
// ---------------------------------------------------------------------------

#[test]
fn dropping_anything_never_yields_a_complete_report() {
    let db = db_with(vec![SemanticGeometry::Opaque {
        type_key: "ACAD_PROXY_ENTITY".into(),
        version: 1,
        payload: Vec::new(),
    }]);
    let (_, report) = export_db(&db);
    assert_eq!(report.counts.dropped, 1);
    assert_eq!(report.counts.exact, 0);
    assert!(matches!(report.completeness, Completeness::Missing(_)));
    assert!(report.counts.bytes > 0);
}

#[test]
fn limits_are_enforced_explicitly() {
    let db = db_with(vec![
        SemanticGeometry::Point(p(0.0, 0.0, 0.0)),
        SemanticGeometry::Point(p(1.0, 0.0, 0.0)),
    ]);
    let result = export(&ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits {
            max_output_entities: 1,
            max_explode_depth: 32,
        },
        units: None,
    });
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
}

/// Guard against a silently empty report type: `ExportCounts` stays constructible.
#[test]
fn export_counts_default_is_zero() {
    assert_eq!(ExportCounts::default().bytes, 0);
}

// ---------------------------------------------------------------------------
// M1/M2/M3/M4/M5/M6/M7 regression coverage.
// ---------------------------------------------------------------------------

fn entity(id: u128, geometry: SemanticGeometry, space: SpaceId, draw_order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "test".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order,
    }
}

fn base_builder() -> DrawingDatabaseBuilder {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
}

#[test]
fn oversized_polyface_mesh_is_dropped_not_corrupted() {
    // An out-of-range face index must not wrap into a negative (invisible edge)
    // or 0 (unused face) index.
    let bad_index = Mesh {
        vertices: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        triangles: vec![[0, 1, 40000]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let (bytes, report) = export_one(SemanticGeometry::Mesh(bad_index));
    assert!(read_entities(&bytes).is_empty());
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.dropped.polyface_index_range"));

    // Too many vertices is dropped too.
    let too_many = Mesh {
        vertices: (0..32768).map(|i| p(i as f64, 0.0, 0.0)).collect(),
        triangles: Vec::new(),
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    };
    let (bytes, report) = export_one(SemanticGeometry::Mesh(too_many));
    assert!(read_entities(&bytes).is_empty());
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.dropped.polyface_index_range"));
}

#[test]
fn dangling_style_id_is_substituted_and_reported_as_a_conversion() {
    let (bytes, report) = export_one(SemanticGeometry::Text {
        text: "X".into(),
        position: p(0.0, 0.0, 0.0),
        style: StyleId(99),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
    });
    assert_eq!(report.counts.exact, 0, "a substituted style is not exact");
    assert_eq!(report.counts.converted, 1);
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.text" && e.message.contains("Standard")));
    match &read_entities(&bytes)[0] {
        EntityType::Text(text) => assert_eq!(text.style, "Standard"),
        other => panic!("expected TEXT, got {other:?}"),
    }
}

#[test]
fn transparency_and_linetype_scale_are_written() {
    let mut builder = base_builder();
    builder
        .insert_linetype(DbLineType {
            id: LinetypeId(1),
            name: "DASHED".into(),
            pattern: LinetypePattern::from_elements([0.5, -0.25]),
            complex: false,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(1),
            EntityRenderAttributes {
                transparency: EntityTransparency::Explicit(0.5),
                color: cad_db::EntityColor::ByLayer,
                lineweight: cad_db::EntityLineWeight::ByLayer,
                linetype: EntityLineType::Explicit {
                    name: "DASHED".into(),
                    pattern: LinetypePattern::from_elements([0.5, -0.25]),
                    scale: 2.5,
                },
                geometry_source: GeometrySource::Analytic,
                annotative: Default::default(),
            },
        )
        .unwrap();
    let db = builder.finish().unwrap();
    let (bytes, _) = export_db(&db);
    match &read_entities(&bytes)[0] {
        EntityType::Line(line) => {
            assert_eq!(line.common.transparency, Transparency::Explicit(128));
            assert_eq!(line.common.linetype, "DASHED");
            assert!((line.common.linetype_scale - 2.5).abs() < 1e-12);
        }
        other => panic!("expected LINE, got {other:?}"),
    }
}

#[test]
fn aligned_text_writes_the_alignment_point_and_round_trips() {
    let (bytes, report) = export_one(SemanticGeometry::Text {
        text: "X".into(),
        position: p(3.0, 4.0, 0.0),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Center,
        v_align: TextAlignV::Middle,
    });
    assert_eq!(report.counts.exact, 1);
    match &read_entities(&bytes)[0] {
        EntityType::Text(text) => {
            assert_eq!(
                text.alignment_point,
                Some(Vector3::new(3.0, 4.0, 0.0)),
                "non-default alignment must anchor at the alignment point"
            );
        }
        other => panic!("expected TEXT, got {other:?}"),
    }
    // Re-import picks the alignment point as the position (inverse of import).
    let reimported = import_bytes(&bytes);
    let position = reimported
        .entities()
        .find_map(|e| match &e.geometry {
            SemanticGeometry::Text { position, .. } => Some(*position),
            _ => None,
        })
        .expect("text reimports");
    assert_point(position, p(3.0, 4.0, 0.0));
}

#[test]
fn mtext_writes_the_attachment_point() {
    let (bytes, report) = export_one(SemanticGeometry::Text {
        text: "A\nB".into(),
        position: p(1.0, 2.0, 0.0),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Right,
        v_align: TextAlignV::Top,
    });
    match &read_entities(&bytes)[0] {
        EntityType::MText(mtext) => {
            assert_eq!(
                mtext.attachment_point,
                acadrust::entities::AttachmentPoint::TopRight
            );
        }
        other => panic!("expected MTEXT, got {other:?}"),
    }
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.text_mtext"));
}

#[test]
fn dynamic_block_visibility_exports_only_visible_members() {
    let mut builder = base_builder();
    for (id, y) in [(2u128, 0.0), (3u128, 1.0)] {
        builder
            .insert_entity(entity(
                id,
                SemanticGeometry::Line {
                    start: p(0.0, y, 0.0),
                    end: p(1.0, y, 0.0),
                },
                SpaceId::Block(BlockId(1)),
                0,
            ))
            .unwrap();
    }
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2), EntityId(3)],
            dynamic_visibility: Some(DynamicBlockVisibility {
                member_entities: vec![EntityId(2), EntityId(3)],
                states: vec![DynamicBlockState {
                    name: "A".into(),
                    entities: vec![EntityId(2)],
                }],
                active_state: Some("A".into()),
            }),
        })
        .unwrap();
    let mut transform = Transform3::identity();
    transform.matrix[0][3] = 10.0;
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform,
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let db = builder.finish().unwrap();
    let (bytes, report) = export_db(&db);
    let entities = read_entities(&bytes);
    assert_eq!(entities.len(), 1, "only the visible member is exported");
    match &entities[0] {
        EntityType::Line(line) => assert_eq!(line.start, Vector3::new(10.0, 0.0, 0.0)),
        other => panic!("expected LINE, got {other:?}"),
    }
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.converted.insert" && e.message.contains("visibility")));
}

#[test]
fn insert_cycle_is_dropped_with_an_explicit_reason() {
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: Transform3::identity(),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: Transform3::identity(),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let db = builder.finish().unwrap();
    let (bytes, report) = export_db(&db);
    assert!(read_entities(&bytes).is_empty());
    assert!(report
        .entries
        .iter()
        .any(|e| e.code == "export.dropped.insert_cycle"));
}

#[test]
fn insert_depth_beyond_the_limit_is_an_explicit_error() {
    let mut builder = base_builder();
    // A -> B -> C (each block's member is an INSERT of the next).
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(11)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(2),
            entities: vec![EntityId(12)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(3),
            entities: vec![EntityId(13)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            11,
            SemanticGeometry::Insert {
                block: BlockId(2),
                transform: Transform3::identity(),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            12,
            SemanticGeometry::Insert {
                block: BlockId(3),
                transform: Transform3::identity(),
            },
            SpaceId::Block(BlockId(2)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            13,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(3)),
            0,
        ))
        .unwrap();
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform: Transform3::identity(),
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let db = builder.finish().unwrap();
    let result = export(&ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits {
            max_output_entities: 1_000_000,
            max_explode_depth: 1,
        },
        units: None,
    });
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
}

#[test]
fn insert_applies_rotation_and_scale_in_the_right_order() {
    let mut builder = base_builder();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(entity(
            2,
            SemanticGeometry::Line {
                start: p(0.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            SpaceId::Block(BlockId(1)),
            0,
        ))
        .unwrap();
    // transform = translate(5,0) * rotate(90deg) * scale(2): (1,0) -> (5,2).
    let scale = Transform3 {
        matrix: [
            [2.0, 0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let rotate = Transform3 {
        matrix: [
            [0.0, -1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let transform = Transform3::translation(p(5.0, 0.0, 0.0))
        .matrix_mul(&rotate)
        .matrix_mul(&scale);
    builder
        .insert_entity(entity(
            1,
            SemanticGeometry::Insert {
                block: BlockId(1),
                transform,
            },
            SpaceId::Model,
            0,
        ))
        .unwrap();
    let db = builder.finish().unwrap();
    let (bytes, _) = export_db(&db);
    match &read_entities(&bytes)[0] {
        EntityType::Line(line) => {
            assert!((line.start - Vector3::new(5.0, 0.0, 0.0)).length() < 1e-9);
            assert!((line.end - Vector3::new(5.0, 2.0, 0.0)).length() < 1e-9);
        }
        other => panic!("expected LINE, got {other:?}"),
    }
}

#[test]
fn unit_context_maps_to_insunits() {
    let db = db_with(vec![SemanticGeometry::Point(p(0.0, 0.0, 0.0))]);
    let (bytes, report) = export(&ExportRequest {
        database: &db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format: ExportFormat::DxfText,
        limits: ExportLimits::default(),
        units: Some(UnitContext {
            source: Unit::Millimeter,
            display: Unit::Millimeter,
            display_per_source: Some(1.0),
            decimal_places: 3,
        }),
    })
    .unwrap();
    let doc = DxfReader::from_reader(Cursor::new(bytes))
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(doc.header.insertion_units, 4);
    assert!(!report
        .notes
        .iter()
        .any(|note| note.code.starts_with("export.note.units")));
}

#[test]
fn entities_are_emitted_in_draw_order() {
    let mut builder = base_builder();
    for (id, x, order) in [(1u128, 20.0, 2i64), (2u128, 0.0, 0i64), (3u128, 10.0, 1i64)] {
        builder
            .insert_entity(entity(
                id,
                SemanticGeometry::Line {
                    start: p(x, 0.0, 0.0),
                    end: p(x + 1.0, 0.0, 0.0),
                },
                SpaceId::Model,
                order,
            ))
            .unwrap();
    }
    let db = builder.finish().unwrap();
    let (bytes, _) = export_db(&db);
    let starts: Vec<f64> = read_entities(&bytes)
        .iter()
        .filter_map(|e| match e {
            EntityType::Line(line) => Some(line.start.x),
            _ => None,
        })
        .collect();
    assert_eq!(starts, vec![0.0, 10.0, 20.0]);
}
