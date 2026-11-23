//! Stored display-edge loss and wire-body drop contracts.
//!
//! Both are reported by the shared entry point, so every neutral-mesh writer
//! (STL binary/ASCII, OBJ, PLY, OFF) must agree on the shape and the counts.

use super::*;

/// A sheet body whose display cache also carries stored edge polylines.
fn body_with_edges(mesh: Mesh, edges: Vec<Vec<Point3>>) -> SemanticGeometry {
    SemanticGeometry::Body3d(Box::new(Body3d {
        kind: BodyKind::Sheet,
        exact: BodyExact::Unavailable {
            reason: "test body".into(),
        },
        display: BodyDisplay {
            mesh,
            edges,
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Complete,
        },
    }))
}

/// A wire body: stored edges only, no display triangles.
fn wire_body(edges: Vec<Vec<Point3>>) -> SemanticGeometry {
    SemanticGeometry::Body3d(Box::new(Body3d {
        kind: BodyKind::Wire,
        exact: BodyExact::Unavailable {
            reason: "test wire body".into(),
        },
        display: BodyDisplay {
            mesh: Mesh::default(),
            edges,
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Partial(vec!["edges only".into()]),
        },
    }))
}

/// Two stored edge polylines: 3 segments + 1 segment = 4 stored segments.
fn stored_edges() -> Vec<Vec<Point3>> {
    vec![
        vec![
            Point3::default(),
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
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.5,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.5,
            },
        ],
    ]
}

/// A display mesh whose every triangle has zero area, so STL writes no facet.
fn degenerate_surface() -> Mesh {
    Mesh {
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
        triangles: vec![[0, 1, 1], [1, 2, 2]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
        uvs: Vec::new(),
        texture: None,
        pbr: None,
    }
}

#[test]
fn stored_edges_are_not_reported_when_the_surface_wrote_no_facet() {
    for format in [MeshFormat::StlBinary, MeshFormat::StlAscii] {
        let code = "export.stl.stored_edges_not_written";
        let with = export_output(
            format,
            &database(vec![entity(
                1,
                "AcDbSurface",
                body_with_edges(degenerate_surface(), stored_edges()),
            )]),
        );
        // The edges never change the bytes: both exports write no facet at all.
        let without = export_output(
            format,
            &database(vec![entity(
                1,
                "AcDbSurface",
                body_with_edges(degenerate_surface(), Vec::new()),
            )]),
        );
        assert_eq!(
            with.bytes,
            without.bytes,
            "{}: the stored edges must not reach the file",
            format.format_id()
        );
        assert_eq!(
            with.report.counts.converted,
            0,
            "{}: a body that wrote no facet has no written surface to lose edges from",
            format.format_id()
        );
        assert_eq!(with.report.counts.exact, 0);
        assert_eq!(with.report.counts.dropped, 2);
        assert!(
            !with.report.entries.iter().any(|entry| entry.code == code),
            "{}: an all-degenerate body must not report {code}",
            format.format_id()
        );
        let entries: Vec<_> = with
            .report
            .entries
            .iter()
            .filter(|entry| entry.object == Some(ObjectId(1)))
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "{}: only the facet drop remains: {entries:?}",
            format.format_id()
        );
        assert_eq!(entries[0].code, "export.stl.degenerate_triangle");
        assert_eq!(entries[0].status, SupportStatus::Unsupported);
    }

    // OBJ writes the degenerate triangle as-is, so its surface *is* written and
    // the same body still reports the stored-edge loss.
    let obj = export_output(
        MeshFormat::Obj,
        &database(vec![entity(
            1,
            "AcDbSurface",
            body_with_edges(degenerate_surface(), stored_edges()),
        )]),
    );
    assert_eq!(obj.report.counts.converted, 1);
    assert_eq!(
        obj.report
            .entries
            .iter()
            .filter(|entry| entry.code == "export.obj.stored_edges_not_written")
            .count(),
        1
    );
}

#[test]
fn a_body_with_stored_edges_reports_the_edge_loss_and_writes_the_same_bytes() {
    for format in [
        MeshFormat::StlBinary,
        MeshFormat::StlAscii,
        MeshFormat::Obj,
        MeshFormat::Ply,
        MeshFormat::Off,
    ] {
        let prefix = match format {
            // The two STL containers share one report prefix on purpose.
            MeshFormat::StlBinary | MeshFormat::StlAscii => "stl",
            MeshFormat::Obj => "obj",
            MeshFormat::Ply => "ply",
            MeshFormat::Off => "off",
        };
        let code = format!("export.{prefix}.stored_edges_not_written");
        let with = export_output(
            format,
            &database(vec![entity(
                1,
                "AcDbSurface",
                body_with_edges(quad(None), stored_edges()),
            )]),
        );
        let without = export_output(
            format,
            &database(vec![entity(1, "AcDbSurface", body(quad(None)))]),
        );
        assert_eq!(
            with.bytes,
            without.bytes,
            "{}: stored edges must not change the written bytes",
            format.format_id()
        );
        assert_eq!(
            with.report.counts.exact,
            without.report.counts.exact,
            "{}: the triangles are still exact",
            format.format_id()
        );
        assert_eq!(
            with.report.counts.converted,
            1,
            "{}: one affected entity is one conversion",
            format.format_id()
        );
        assert!(matches!(with.report.completeness, Completeness::Partial(_)));
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
        assert_eq!(entries[0].object, Some(ObjectId(1)));
        assert_eq!(entries[0].type_key.as_deref(), Some("AcDbSurface"));
        assert!(
            entries[0].message.contains("#1")
                && entries[0].message.contains("AcDbSurface")
                && entries[0]
                    .message
                    .contains("4 stored display edge segment(s)")
                && entries[0].message.contains("writes surfaces only"),
            "the message must name the entity and the real segment count: {}",
            entries[0].message
        );
        assert!(
            !without
                .report
                .entries
                .iter()
                .any(|entry| entry.code == code),
            "{}: an edge-less body must not report an edge loss",
            format.format_id()
        );
    }
}

#[test]
fn each_body_with_stored_edges_counts_one_conversion() {
    let database = database(vec![
        entity(1, "First", body_with_edges(quad(None), stored_edges())),
        entity(2, "Second", body_with_edges(quad(None), stored_edges())),
    ]);
    let output = export_output(MeshFormat::Obj, &database);
    let entries: Vec<_> = output
        .report
        .entries
        .iter()
        .filter(|entry| entry.code == "export.obj.stored_edges_not_written")
        .collect();
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert_eq!(output.report.counts.converted, 2);
    assert_eq!(output.report.counts.exact, 2);
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn a_wire_body_is_dropped_by_name_and_not_counted_as_written() {
    let database = database(vec![
        entity(1, "ModelMesh", SemanticGeometry::Mesh(quad(None))),
        entity(2, "AcDbWire", wire_body(stored_edges())),
    ]);
    let output = export_output(MeshFormat::Obj, &database);
    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.dropped, 1);
    assert_eq!(
        output.report.counts.converted, 0,
        "a wire body is a drop, never written-geometry edge loss"
    );
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.object == Some(ObjectId(2)))
        .expect("the wire body is named, not silently skipped");
    assert_eq!(entry.code, "export.obj.wire_body_not_written");
    assert_eq!(entry.status, SupportStatus::Unsupported);
    assert_eq!(entry.type_key.as_deref(), Some("AcDbWire"));
    assert!(
        entry.message.contains("4 stored display edge segment(s)")
            && entry.message.contains("no edge channel"),
        "the message must name the segment count and the missing channel: {}",
        entry.message
    );
    assert!(
        !output
            .report
            .entries
            .iter()
            .any(|other| other.object == Some(ObjectId(2))
                && other.code == "export.obj.stored_edges_not_written"),
        "a dropped wire body must not also report a written edge loss"
    );
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
}
