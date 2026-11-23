//! Stored edges, wire bodies on mesh bodies, and the single merged root run
//! (ADR-0049 §2).

use crate::export::{
    DEGENERATE_TRIANGLE, ENTITY_GROUPING_FLATTENED, LIMITS_EXCEEDED, STORED_EDGES_NOT_WRITTEN,
};
use cad_domain::ObjectId;

use super::*;

// ---- Stored edges and wire bodies (ADR-0049 §2) ---------------------------

fn one_edge() -> Vec<Vec<Point3>> {
    vec![vec![point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)]]
}

fn display_body(kind: BodyKind, mesh: Mesh, edges: Vec<Vec<Point3>>) -> Body3d {
    Body3d {
        kind,
        exact: BodyExact::Unavailable {
            reason: "synthetic".into(),
        },
        display: BodyDisplay {
            mesh,
            edges,
            tolerance: None,
            precision: Precision::Unknown,
            completeness: Completeness::Partial(vec!["synthetic".into()]),
        },
    }
}

#[test]
fn a_body_with_stored_edges_writes_its_triangles_and_reports_the_edges() {
    let body = display_body(BodyKind::Unknown, triangle(), one_edge());
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    assert_eq!(output.report.counts.exact, 1, "the triangles are written");
    assert_eq!(output.report.counts.dropped, 0);
    assert_eq!(
        output.report.counts.converted, 1,
        "one affected body counts one conversion"
    );
    assert!(
        matches!(output.report.completeness, Completeness::Partial(_)),
        "triangles written, edges not: Partial, never Complete"
    );
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == STORED_EDGES_NOT_WRITTEN)
        .expect("the dropped edges are named");
    assert_eq!(entry.status, cad_domain::SupportStatus::Partial);
    assert_eq!(entry.object, Some(ObjectId(1)));
    assert!(
        entry.message.contains("does not copy the stored edge set")
            && entry.message.contains("regenerates"),
        "the message must say the set is not copied and triangle boundaries are regenerated: {}",
        entry.message
    );
    // The written container really has the triangle.
    assert!(mesh_triangles(&import(output.bytes).unwrap().database) >= 1);
}

/// A stored edge that coincides with a triangle boundary is not copied as such,
/// but the writer regenerates it from the written triangle — so the round trip
/// really carries that boundary edge.
#[test]
fn a_stored_triangle_boundary_edge_is_regenerated_in_the_written_container() {
    let body = display_body(BodyKind::Unknown, triangle(), one_edge());
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    let parsed = skpwright::Model::parse(&output.bytes).expect("the written container parses");
    assert_eq!(parsed.geometry.len(), 1);
    let written: Vec<(u32, u32)> = parsed.geometry[0]
        .mesh
        .edges
        .iter()
        .map(|edge| (edge.v0.min(edge.v1), edge.v0.max(edge.v1)))
        .collect();
    for boundary in [(0, 1), (1, 2), (0, 2)] {
        assert!(
            written.contains(&boundary),
            "the written triangle boundary {boundary:?} is missing from {written:?}"
        );
    }
    // The stored edge set itself is not copied: the report still says so.
    assert_eq!(codes(&output), [STORED_EDGES_NOT_WRITTEN]);
}

/// `converted` counts affected entities: two bodies with stored edges are two
/// conversions, plus the merged-run grouping note (one more).
#[test]
fn each_affected_body_counts_one_conversion() {
    let first = display_body(BodyKind::Unknown, triangle(), one_edge());
    let second = display_body(BodyKind::Unknown, triangle(), one_edge());
    let database = database(vec![
        (SemanticGeometry::Body3d(Box::new(first)), LayerId(0)),
        (SemanticGeometry::Body3d(Box::new(second)), LayerId(0)),
    ]);
    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(output.report.counts.exact, 2);
    assert_eq!(
        output.report.counts.converted, 3,
        "two stored-edge losses plus the one merged-run grouping loss"
    );
    assert_eq!(
        output
            .report
            .entries
            .iter()
            .filter(|entry| entry.code == STORED_EDGES_NOT_WRITTEN)
            .count(),
        2
    );
    let grouping = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == ENTITY_GROUPING_FLATTENED)
        .expect("two sources in one run is a named grouping loss");
    assert_eq!(grouping.status, cad_domain::SupportStatus::Partial);
    assert!(
        grouping.message.contains('2'),
        "the note names how many sources were merged: {}",
        grouping.message
    );
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
}

/// A mesh body whose triangles are all degenerate writes no surface geometry,
/// so the drop is the only verdict: claiming `STORED_EDGES_NOT_WRITTEN` would
/// describe a loss for an entity that never reached the file.
#[test]
fn a_dropped_all_degenerate_mesh_body_claims_no_stored_edge_loss() {
    let mut mesh = triangle();
    mesh.vertices[2] = point(2.0, 0.0, 0.0);
    let body = display_body(BodyKind::Unknown, mesh, one_edge());
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    assert_eq!(codes(&output), [DEGENERATE_TRIANGLE]);
    assert_eq!(output.report.counts.dropped, 1);
    assert_eq!(output.report.counts.exact, 0);
    assert_eq!(
        output.report.counts.converted, 0,
        "a body that wrote nothing converts nothing"
    );
    assert!(
        !output
            .report
            .entries
            .iter()
            .any(|entry| entry.code == STORED_EDGES_NOT_WRITTEN),
        "no stored-edge loss for a body whose triangles were dropped: {:?}",
        codes(&output)
    );
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
}

// ---- One merged root run (the pinned writer authors only the first) --------

fn translated_triangle(dx: f64) -> Mesh {
    let mut mesh = triangle();
    for vertex in &mut mesh.vertices {
        vertex.x += dx;
    }
    mesh
}

/// Regression pin: the pinned writer (`build26.rs`) authors only the FIRST root
/// `GeometryRun`, so one run per source silently dropped every source but the
/// first while reporting them exact. Every source must land in one run.
#[test]
fn three_source_meshes_write_one_root_run_with_every_triangle() {
    let database = database(vec![
        (SemanticGeometry::Mesh(translated_triangle(0.0)), LayerId(0)),
        (
            SemanticGeometry::Mesh(translated_triangle(10.0)),
            LayerId(0),
        ),
        (
            SemanticGeometry::Mesh(translated_triangle(20.0)),
            LayerId(0),
        ),
    ]);
    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(
        output.report.counts.exact, 3,
        "every source is reported exact"
    );
    assert_eq!(output.report.counts.dropped, 0);
    assert_eq!(codes(&output), [ENTITY_GROUPING_FLATTENED]);
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));

    let parsed = parsed_model(&output);
    assert_eq!(parsed.geometry.len(), 1, "exactly one root run is authored");
    let run = &parsed.geometry[0];
    assert_eq!(run.def_index, None);
    assert_eq!(run.mesh.vertices.len(), 9, "3 vertices per source");
    assert_eq!(run.mesh.faces.len(), 3, "3 triangles, one per source");
    let mut face_min_x: Vec<f64> = run
        .mesh
        .faces
        .iter()
        .map(|face| {
            face.outer
                .iter()
                .map(|&index| run.mesh.vertices[index as usize][0])
                .fold(f64::INFINITY, f64::min)
        })
        .collect();
    face_min_x.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        face_min_x,
        [0.0, 10.0, 20.0],
        "every source's triangle is present after merging"
    );
    skpwright::verify(&output.bytes).expect("the merged container passes skpwright::verify");
}

/// The merged run offsets each later source's triangle indices by the running
/// vertex base instead of reusing indices 0.. of the previous source.
#[test]
fn a_merged_run_offsets_later_sources_by_the_vertex_base() {
    let database = database(vec![
        (SemanticGeometry::Mesh(translated_triangle(0.0)), LayerId(0)),
        (
            SemanticGeometry::Mesh(translated_triangle(10.0)),
            LayerId(0),
        ),
    ]);
    let output = export(&database, Some(&meter())).unwrap();
    let parsed = parsed_model(&output);
    assert_eq!(parsed.geometry.len(), 1);
    let run = &parsed.geometry[0];
    assert_eq!(run.mesh.vertices.len(), 6);
    assert_eq!(run.mesh.faces.len(), 2, "no source triangle is lost");
    assert_eq!(run.mesh.faces[0].outer, vec![0, 1, 2]);
    assert_eq!(
        run.mesh.faces[1].outer,
        vec![3, 4, 5],
        "the second source is offset by the first source's 3 vertices"
    );
    assert!((run.mesh.vertices[3][0] - 10.0).abs() < 1e-12);
}

/// `max_output_entities` budgets sources, not the merged run (which is always
/// one): counting runs would let two sources pass a one-entity budget.
#[test]
fn the_entity_budget_counts_sources_not_the_single_run() {
    let database = database(vec![
        (SemanticGeometry::Mesh(translated_triangle(0.0)), LayerId(0)),
        (
            SemanticGeometry::Mesh(translated_triangle(10.0)),
            LayerId(0),
        ),
    ]);
    let units = meter();
    let mut job = job(&database, Some(&units));
    job.limits.max_output_entities = 1;
    let message = unsupported(
        crate::SkpExporter::new(&crate::DESCRIPTOR_SKP)
            .export("skp", &job)
            .unwrap_err(),
    );
    assert!(message.starts_with(LIMITS_EXCEEDED), "{message}");
    assert!(message.contains("source entities"), "{message}");
}

/// Budget counting cannot rely on sources that actually wrote geometry: an
/// all-degenerate mesh writes nothing, and if only written sources were counted
/// a document of such bodies would slip past `max_output_entities`.
#[test]
fn the_entity_budget_counts_sources_that_write_no_geometry() {
    let mut degenerate = triangle();
    degenerate.vertices[2] = point(2.0, 0.0, 0.0);
    let database = database(vec![
        (SemanticGeometry::Mesh(triangle()), LayerId(0)),
        (SemanticGeometry::Mesh(degenerate), LayerId(0)),
    ]);
    let units = meter();
    let mut job = job(&database, Some(&units));
    job.limits.max_output_entities = 1;
    let message = unsupported(
        crate::SkpExporter::new(&crate::DESCRIPTOR_SKP)
            .export("skp", &job)
            .unwrap_err(),
    );
    assert!(message.starts_with(LIMITS_EXCEEDED), "{message}");
    assert!(message.contains('2'), "{message}");
}
