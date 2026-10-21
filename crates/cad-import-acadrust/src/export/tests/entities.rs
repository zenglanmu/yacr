//! Per-kind entity mapping and OCS round-trip (writer + our reader = internal consistency).

use super::*;

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
fn negative_sweep_arc_round_trips_to_the_same_point_set() {
    let start = 1.0_f64;
    let sweep = -0.5_f64;
    let (bytes, report) = export_one(SemanticGeometry::Arc {
        center: p(2.0, 3.0, 0.0),
        normal: p(0.0, 0.0, 1.0),
        radius: 4.0,
        start,
        sweep,
    });
    let tau = std::f64::consts::TAU;
    let entities = read_entities(&bytes);
    let arc = match &entities[0] {
        EntityType::Arc(arc) => arc,
        other => panic!("expected ARC, got {other:?}"),
    };
    // Still an exact mapping, not a silently-mirrored complement.
    assert_eq!(report.counts.exact, 1);
    // A DXF ARC is a CCW point set, so a negative sweep exports with the angles
    // swapped: from `start + sweep` CCW to `start`.
    assert!((arc.start_angle - (start + sweep).rem_euclid(tau)).abs() < 1e-9);
    assert!((arc.end_angle - start.rem_euclid(tau)).abs() < 1e-9);
    let span = (arc.end_angle - arc.start_angle).rem_euclid(tau);
    assert!((span - sweep.abs()).abs() < 1e-9, "span {span}");

    // The exported arc's endpoints are the original arc's endpoints, swapped.
    let point_at = |angle: f64| Vector3::new(2.0 + 4.0 * angle.cos(), 3.0 + 4.0 * angle.sin(), 0.0);
    let approx = |a: Vector3, b: Vector3| (a - b).length() < 1e-9;
    assert!(approx(point_at(arc.start_angle), point_at(start + sweep)));
    assert!(approx(point_at(arc.end_angle), point_at(start)));
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
        wrap_width: None,
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
        wrap_width: None,
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
// 2. ACIS solids are tessellated at import: export must name the conversion.
// ---------------------------------------------------------------------------

#[test]
fn a_tessellated_acis_solid_exports_as_a_polyface_mesh_not_a_3dsolid() {
    use acadrust::io::dwg::dwg_writer::DwgWriter;

    let sat = include_str!("../../../../../fixtures/acis/cube.sat");
    let mut doc = CadDocument::new();
    doc.add_entity(EntityType::Solid3D(acadrust::entities::Solid3D::from_sat(
        sat,
    )))
    .unwrap();
    let dwg = DwgWriter::write_to_vec(&doc).unwrap();

    let db = import_bytes(&dwg);
    assert!(
        db.entities()
            .any(|entity| matches!(entity.geometry, SemanticGeometry::Mesh(_))),
        "the importer must have tessellated the 3DSOLID"
    );

    let (bytes, report) = export_db(&db);
    let entities = read_entities(&bytes);
    assert!(
        entities
            .iter()
            .any(|entity| matches!(entity, EntityType::PolyfaceMesh(_))),
        "a tessellated solid must export as a polyface mesh"
    );
    assert!(
        !entities
            .iter()
            .any(|entity| matches!(entity, EntityType::Solid3D(_))),
        "no ACIS source is retained, so a 3DSOLID cannot be round-tripped"
    );

    let conversion = report
        .entries
        .iter()
        .find(|entry| {
            entry.code == "export.converted.mesh" && entry.message.contains("not as a 3DSOLID")
        })
        .expect("the mesh conversion must be recorded by name");
    assert!(
        conversion.message.contains("original SAT/SAB source"),
        "the conversion must state the lost 3DSOLID round trip: {}",
        conversion.message
    );
}
