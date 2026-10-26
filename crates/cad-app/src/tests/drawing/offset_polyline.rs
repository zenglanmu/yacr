//! LWPOLYLINE `OFFSET` contract tests.
//!
//! The straight-segment polyline arm of OFFSET (`app_drawing::offset_polyline`):
//! constant distance to the pick's side, **miter** joins, vertex count and order
//! preserved, a closed polyline staying closed, one transaction / one undo step,
//! and the named refusals (`offset.bulge_segment`, `offset.collapsed`,
//! `offset.non_planar`) that change nothing.
//!
//! Split out of `offset.rs` (which keeps the LINE/CIRCLE arm) so both files stay
//! well inside the 1000-line code-file cap.

use super::offset::app_with_drawing;
use super::*;

/// A drawing whose only entity (id 1) is the given polyline on layer 0.
fn polyline_app(points: Vec<Point3>, bulges: Vec<f64>, closed: bool) -> Application {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        },
        0,
    ))
    .unwrap();
    app_with_drawing(b.finish().unwrap())
}

/// The closed 10 × 5 rectangle `(0,0) (10,0) (10,5) (0,5)` (CCW).
fn rectangle_polyline_app() -> Application {
    polyline_app(
        vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 5.0),
            point(0.0, 5.0),
        ],
        vec![0.0; 4],
        true,
    )
}

/// The offset copy an OFFSET outcome created.
fn created_polyline(app: &Application, outcome: &CommandOutcome) -> (Vec<Point3>, Vec<f64>, bool) {
    let created = EntityId(outcome.objects[0].0);
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(created)
        .expect("the offset copy exists")
        .geometry
    {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points.clone(), bulges.clone(), *closed),
        other => panic!("expected a polyline, got {other:?}"),
    }
}

fn assert_vertex(actual: Point3, x: f64, y: f64) {
    assert!(
        (actual.x - x).abs() < 1e-9 && (actual.y - y).abs() < 1e-9,
        "expected ({x}, {y}), got {actual:?}"
    );
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

#[test]
fn offset_closed_rectangle_outward_moves_every_corner_exactly() {
    let mut app = rectangle_polyline_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let source_before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .cloned()
        .unwrap();

    // The nearest segment is the right edge (x = 10): the pick is 2 units out.
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(12.0, 2.5),
        },
    )
    .unwrap();

    let (points, bulges, closed) = created_polyline(&app, &outcome);
    assert!(closed, "a closed polyline offsets to a closed polyline");
    assert_eq!(points.len(), 4, "n vertices in, n vertices out");
    assert_eq!(bulges.len(), 4, "the straight-segment bulges are carried");
    // Same order as the source, all four mitred corners exact.
    assert_vertex(points[0], -2.0, -2.0);
    assert_vertex(points[1], 12.0, -2.0);
    assert_vertex(points[2], 12.0, 7.0);
    assert_vertex(points[3], -2.0, 7.0);

    // The source is untouched.
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .cloned()
            .unwrap(),
        source_before
    );
}

#[test]
fn offset_closed_rectangle_inward_moves_every_corner_exactly() {
    let mut app = rectangle_polyline_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(8.0, 2.5),
        },
    )
    .unwrap();
    let (points, _, closed) = created_polyline(&app, &outcome);
    assert!(closed);
    assert_eq!(points.len(), 4);
    assert_vertex(points[0], 2.0, 2.0);
    assert_vertex(points[1], 8.0, 2.0);
    assert_vertex(points[2], 8.0, 3.0);
    assert_vertex(points[3], 2.0, 3.0);
}

#[test]
fn offset_open_polyline_mitres_the_middle_vertex_and_keeps_the_ends() {
    // (0,0) -> (10,0) -> (10,10), picked 3 units above the first segment.
    let mut app = polyline_app(
        vec![point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)],
        vec![0.0; 3],
        false,
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(5.0, 3.0),
        },
    )
    .unwrap();
    let (points, _, closed) = created_polyline(&app, &outcome);
    assert!(
        !closed,
        "an open polyline stays open: no closing segment invented"
    );
    assert_eq!(points.len(), 3, "n vertices in, n vertices out");
    // The end vertices move with their own segment (no end cap); the middle
    // vertex is the miter point of the two offset lines.
    assert_vertex(points[0], 0.0, 3.0);
    assert_vertex(points[1], 7.0, 3.0);
    assert_vertex(points[2], 7.0, 10.0);
}

#[test]
fn offset_polyline_pick_side_is_symmetric() {
    // A miter offset is affine in the signed distance, so the two opposite sides
    // are exact reflections through every source vertex: q⁻ᵢ = 2·pᵢ − q⁺ᵢ.
    let source = vec![point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)];
    let offset_to = |side: Point3| {
        let mut app = polyline_app(source.clone(), vec![0.0; 3], false);
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::OffsetEntity,
            CommandPayload::Offset {
                refs: vec![ref_of(1)],
                side_point: side,
            },
        )
        .unwrap();
        created_polyline(&app, &outcome).0
    };
    let above = offset_to(point(5.0, 3.0));
    let below = offset_to(point(5.0, -3.0));
    assert_eq!(above.len(), below.len());
    for (index, (plus, minus)) in above.iter().zip(below.iter()).enumerate() {
        let mirror = point(
            2.0 * source[index].x - plus.x,
            2.0 * source[index].y - plus.y,
        );
        assert!(
            (mirror.x - minus.x).abs() < 1e-9 && (mirror.y - minus.y).abs() < 1e-9,
            "vertex {index}: {minus:?} is not the point reflection of {plus:?}"
        );
    }
}

#[test]
fn offset_polyline_with_a_bulge_segment_is_refused_by_name() {
    let mut app = polyline_app(
        vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 5.0),
            point(0.0, 5.0),
        ],
        vec![0.0, 0.5, 0.0, 0.0],
        true,
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(12.0, 2.5),
        },
    )
    .unwrap_err();
    match err {
        CadError::Unsupported(reason) => assert!(
            reason.contains("offset.bulge_segment"),
            "the refusal must name the arc-segment gap: {reason}"
        ),
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev, "no database write");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1,
        "nothing may be created"
    );
    assert_eq!(undo_depth(&app), 0, "zero undo steps");
}

#[test]
fn offset_polyline_refuses_a_collapsed_result() {
    // A 10 × 1 rectangle picked on its centre line: the inward offset distance
    // (0.5) reaches the other side, so the corners meet and the offset segments
    // collapse. Refused as `offset.collapsed`, never emitted as a bow-tie.
    let mut app = polyline_app(
        vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 1.0),
            point(0.0, 1.0),
        ],
        vec![0.0; 4],
        true,
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(5.0, 0.5),
        },
    )
    .unwrap_err();
    match err {
        CadError::Unsupported(reason) => assert!(
            reason.contains("offset.collapsed"),
            "the refusal must name the collapse: {reason}"
        ),
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1
    );
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn offset_polyline_refuses_a_non_planar_vertex() {
    let mut app = polyline_app(
        vec![
            point(0.0, 0.0),
            point(10.0, 0.0),
            point(10.0, 10.0),
            p3(0.0, 10.0, 4.0),
        ],
        vec![0.0; 4],
        false,
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(5.0, 3.0),
        },
    )
    .unwrap_err();
    match err {
        CadError::Unsupported(reason) => assert!(
            reason.contains("offset.non_planar"),
            "the refusal must name the planarity gap: {reason}"
        ),
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    }
    assert_eq!(revision(&app), rev);
}

#[test]
fn offset_polyline_pick_on_the_polyline_is_refused() {
    let mut app = rectangle_polyline_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    // A point on the bottom edge: zero distance, so there is no side and no
    // offset distance — the same refusal the LINE branch gives.
    let err = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(5.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(revision(&app), rev);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn offset_polyline_is_one_undo_step() {
    let mut app = rectangle_polyline_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(12.0, 2.5),
        },
    )
    .unwrap();
    let copy = EntityId(outcome.objects[0].0);
    assert_eq!(undo_depth(&app), 1);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        2
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(copy)
        .is_none());
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        1,
        "one undo removes the whole polyline offset"
    );
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_some());

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    let (points, _, _) = created_polyline(&app, &outcome);
    assert_vertex(points[1], 12.0, -2.0);
}
