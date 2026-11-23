//! Wire-body free edges: chaining, zero-length skipping, de-duplication and the
//! shared merged root run.

use crate::export::{
    EDGE_SEMANTICS_NOT_WRITTEN, ENTITY_GROUPING_FLATTENED, UNSUPPORTED_GEOMETRY,
    WIRE_BODY_NO_WRITABLE_EDGES, ZERO_LENGTH_EDGE,
};
use cad_domain::ObjectId;

use super::*;

// ---- Wire-body free edges ------------------------------------------------

/// A wire-only body is written as free edges, not dropped: one root run with
/// its vertices and edges, which the pinned reader gives back as a wire body.
#[test]
fn a_wire_only_body_is_written_as_free_edges() {
    let body = display_body(BodyKind::Wire, Mesh::default(), one_edge());
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    assert_eq!(output.report.counts.exact, 1);
    assert_eq!(output.report.counts.dropped, 0);
    assert_eq!(codes(&output), [EDGE_SEMANTICS_NOT_WRITTEN]);
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));

    let parsed = parsed_model(&output);
    assert_eq!(parsed.geometry.len(), 1);
    let run = &parsed.geometry[0];
    assert_eq!(run.def_index, None);
    assert!(run.mesh.faces.is_empty());
    assert_eq!(run.mesh.vertices.len(), 2);
    assert_eq!(canonical_edges(&parsed), [(0, 1)]);
    assert!(
        run.mesh
            .edges
            .iter()
            .all(|edge| edge.curve.is_none() && edge.layer == 0 && edge.material.is_none()),
        "free edges carry no curve, material or layer"
    );

    // Self-consistency: the same crate reads the free edge back as a wire body.
    let imported = import(output.bytes).unwrap();
    let entity = imported
        .database
        .entities()
        .find(|entity| entity.object.type_key == crate::SKP_EDGE_TYPE_KEY)
        .expect("the free edge reads back as an SkpPreviewEdges body");
    let SemanticGeometry::Body3d(body) = &entity.geometry else {
        panic!("a free edge reads back as a display body");
    };
    assert_eq!(body.kind, BodyKind::Wire);
    assert!(body.display.mesh.triangles.is_empty());
    assert_eq!(body.display.edges.len(), 1);
    assert!((body.display.edges[0][1].x - 1.0).abs() < 1e-12);
}

/// N>2 points chain into N-1 segments and share the interior vertex index.
#[test]
fn a_polyline_with_n_points_writes_n_minus_one_chained_edges() {
    let edges = vec![vec![
        point(0.0, 0.0, 0.0),
        point(1.0, 0.0, 0.0),
        point(1.0, 1.0, 0.0),
    ]];
    let body = display_body(BodyKind::Wire, Mesh::default(), edges);
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    let parsed = parsed_model(&output);
    let run = &parsed.geometry[0];
    assert_eq!(run.mesh.vertices.len(), 3, "each point is appended");
    assert_eq!(canonical_edges(&parsed), [(0, 1), (1, 2)]);
    assert_eq!(
        run.mesh
            .edges
            .iter()
            .filter(|edge| edge.v0 == 1 || edge.v1 == 1)
            .count(),
        2,
        "the interior index is shared by both segments"
    );
}

/// A display segment whose endpoints coincide cannot be a SketchUp edge: it is
/// skipped and named, and the rest of the wire still writes. The source is not
/// `exact` — a segment of the source geometry did not reach the file.
#[test]
fn zero_length_wire_segments_are_skipped_by_name() {
    let edges = vec![vec![
        point(0.0, 0.0, 0.0),
        point(0.0, 0.0, 0.0),
        point(1.0, 0.0, 0.0),
    ]];
    let body = display_body(BodyKind::Wire, Mesh::default(), edges);
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    assert_eq!(
        output.report.counts.exact, 0,
        "a skipped zero-length segment means the source is not written in full"
    );
    assert_eq!(output.report.counts.dropped, 0);
    assert!(
        output.report.counts.converted >= 1,
        "the zero-length loss is a named conversion: {:?}",
        output.report.counts
    );
    let parsed = parsed_model(&output);
    assert_eq!(canonical_edges(&parsed), [(1, 2)]);
    let note = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == ZERO_LENGTH_EDGE)
        .expect("the skipped segment is named");
    assert_eq!(note.status, cad_domain::SupportStatus::Partial);
    assert!(
        note.message.contains("1 zero-length"),
        "the note counts what was skipped: {}",
        note.message
    );
    assert!(codes(&output).contains(&EDGE_SEMANTICS_NOT_WRITTEN));
    assert!(matches!(
        output.report.completeness,
        Completeness::Partial(_)
    ));
}

/// A wire body whose segments are **all** zero-length writes no edge at all: the
/// whole entity is a named `Unsupported` drop, not a `Partial` note over an empty
/// file. Without the drop the report would be `Partial` (only `ZERO_LENGTH_EDGE`)
/// while the written model carried nothing — an empty written model with no
/// dropped entity.
#[test]
fn a_wire_body_with_only_zero_length_segments_is_dropped_by_name() {
    let edges = vec![
        vec![point(0.0, 0.0, 0.0), point(0.0, 0.0, 0.0)],
        vec![
            point(1.0, 1.0, 0.0),
            point(1.0, 1.0, 0.0),
            point(1.0, 1.0, 0.0),
        ],
    ];
    let body = display_body(BodyKind::Wire, Mesh::default(), edges);
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    assert_eq!(
        output.report.counts.dropped, 1,
        "an entity with no writable edge is dropped, not silently omitted"
    );
    assert_eq!(output.report.counts.exact, 0);
    assert_eq!(
        output.report.counts.converted, 1,
        "the skipped segments are still counted once by the zero-length note"
    );
    assert_eq!(
        codes(&output),
        [WIRE_BODY_NO_WRITABLE_EDGES, ZERO_LENGTH_EDGE],
        "the entity drop comes first, then the skipped-segment count"
    );
    let entry = output
        .report
        .entries
        .iter()
        .find(|entry| entry.code == WIRE_BODY_NO_WRITABLE_EDGES)
        .expect("the dropped wire body is named");
    assert_eq!(entry.status, cad_domain::SupportStatus::Unsupported);
    assert_eq!(entry.object, Some(ObjectId(1)));
    assert_eq!(entry.type_key.as_deref(), Some("AcDb3dSolid"));
    assert!(
        entry
            .message
            .contains("entity carries only zero-length display edge segments")
            && entry.message.contains("no edge was written"),
        "the message states why nothing was written: {}",
        entry.message
    );
    // Any drop is the most severe verdict: `Missing`, never `Partial`, and the
    // drop's reason is carried in the verdict.
    let Completeness::Missing(reasons) = &output.report.completeness else {
        panic!(
            "a dropped entity must make the verdict Missing, got {:?}",
            output.report.completeness
        );
    };
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("no edge was written")),
        "the missing verdict carries the drop's reason: {reasons:?}"
    );
    let parsed = parsed_model(&output);
    let written = parsed
        .geometry
        .iter()
        .flat_map(|run| {
            [
                run.mesh.faces.len(),
                run.mesh.edges.len(),
                run.mesh.vertices.len(),
            ]
        })
        .sum::<usize>();
    assert_eq!(
        written, 0,
        "the written model carries no face, edge or vertex for the dropped body"
    );
}

/// A valid mesh beside an all-zero wire body: the mesh is written, the wire body
/// is one dropped entity, and the verdict is still `Missing` — a written mesh
/// does not turn a dropped source into `Partial`.
#[test]
fn a_mesh_beside_an_all_zero_wire_body_writes_the_mesh_and_drops_one_entity() {
    let edges = vec![vec![point(0.0, 0.0, 0.0), point(0.0, 0.0, 0.0)]];
    let wire = display_body(BodyKind::Wire, Mesh::default(), edges);
    let database = database(vec![
        (SemanticGeometry::Mesh(triangle()), LayerId(0)),
        (SemanticGeometry::Body3d(Box::new(wire)), LayerId(0)),
    ]);
    let output = export(&database, Some(&meter())).unwrap();

    assert_eq!(output.report.counts.exact, 1, "the mesh is written in full");
    assert_eq!(output.report.counts.dropped, 1, "the wire body is dropped");
    assert_eq!(
        codes(&output),
        [WIRE_BODY_NO_WRITABLE_EDGES, ZERO_LENGTH_EDGE],
        "the dropped entity plus the skipped-segment count"
    );
    assert!(
        matches!(output.report.completeness, Completeness::Missing(_)),
        "one dropped entity keeps the verdict Missing even with written geometry: {:?}",
        output.report.completeness
    );

    let parsed = parsed_model(&output);
    assert_eq!(parsed.geometry.len(), 1, "the one merged root run");
    let run = &parsed.geometry[0];
    assert_eq!(
        run.mesh.faces.len(),
        1,
        "the mesh triangle reaches the file"
    );
    assert_eq!(
        run.mesh.vertices.len(),
        3,
        "the dropped wire adds no vertices"
    );
    assert_eq!(
        canonical_edges(&parsed),
        [(0, 1), (1, 2), (0, 2)],
        "only the triangle's boundaries are written"
    );
    assert!(
        mesh_triangles(&import(output.bytes).unwrap().database) >= 1,
        "the reader gets the mesh triangle back"
    );
}

/// A polyline that retraces the same endpoint pair produces one stored edge
/// record, so the pair is written once (and the source still counts exact).
///
/// Writing the repeated pair once is coordinate normalization: the written edge
/// set is the same one the source names, so this source stays `exact`. Only a
/// *zero-length* segment (real geometry with no SketchUp carrier) makes a wire
/// source inexact.
#[test]
fn a_retraced_wire_segment_is_written_once() {
    let edges = vec![vec![
        point(0.0, 0.0, 0.0),
        point(1.0, 0.0, 0.0),
        point(0.0, 0.0, 0.0),
    ]];
    let body = display_body(BodyKind::Wire, Mesh::default(), edges);
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();

    assert_eq!(output.report.counts.exact, 1);
    let parsed = parsed_model(&output);
    assert_eq!(parsed.geometry[0].mesh.vertices.len(), 3);
    assert_eq!(
        canonical_edges(&parsed),
        [(0, 1)],
        "the retraced segment is written once, not twice"
    );
    assert!(!codes(&output).contains(&ZERO_LENGTH_EDGE));
}

/// Mesh and wire sources share the one root run; the free edge appends its own
/// vertices (no welding), so the face boundary is not duplicated or swallowed.
#[test]
fn a_mesh_and_a_wire_body_share_one_run_without_duplicating_face_boundaries() {
    let wire_edges = vec![vec![point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)]];
    let wire = display_body(BodyKind::Wire, Mesh::default(), wire_edges);
    let database = database(vec![
        (SemanticGeometry::Mesh(triangle()), LayerId(0)),
        (SemanticGeometry::Body3d(Box::new(wire)), LayerId(0)),
    ]);
    let output = export(&database, Some(&meter())).unwrap();

    assert_eq!(output.report.counts.exact, 2);
    let parsed = parsed_model(&output);
    assert_eq!(parsed.geometry.len(), 1);
    let run = &parsed.geometry[0];
    assert_eq!(run.mesh.faces.len(), 1);
    let written = canonical_edges(&parsed);
    assert_eq!(written.len(), 4, "3 triangle boundaries + 1 free edge");
    for boundary in [(0, 1), (1, 2), (0, 2)] {
        assert!(
            written.contains(&boundary),
            "the triangle boundary {boundary:?} is missing from {written:?}"
        );
    }
    assert!(
        written.contains(&(3, 4)),
        "the free edge keeps its own appended vertices: {written:?}"
    );
    let mut unique = written.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        written.len(),
        "no canonical vertex pair is written twice"
    );
    let output_codes = codes(&output);
    assert!(output_codes.contains(&ENTITY_GROUPING_FLATTENED));
    assert!(output_codes.contains(&EDGE_SEMANTICS_NOT_WRITTEN));
}

/// A body with neither triangles nor display edges has nothing to write: a
/// truthful named refusal, not an empty success.
#[test]
fn a_body_with_neither_mesh_nor_edges_is_dropped_by_name() {
    let body = display_body(BodyKind::Unknown, Mesh::default(), Vec::new());
    let database = database(vec![(SemanticGeometry::Body3d(Box::new(body)), LayerId(0))]);
    let output = export(&database, Some(&meter())).unwrap();
    assert_eq!(output.report.counts.dropped, 1);
    assert_eq!(output.report.counts.exact, 0);
    assert_eq!(codes(&output), [UNSUPPORTED_GEOMETRY]);
    assert!(matches!(
        output.report.completeness,
        Completeness::Missing(_)
    ));
}
