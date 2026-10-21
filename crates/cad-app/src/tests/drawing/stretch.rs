//! STRETCH edit-command contract tests (ADR 0016 decision E).
//!
//! Pins the crossing-window enclosure classification, the per-vertex stretch
//! (including the UNVERIFIED bulge-preservation rule), the move-only kinds, the
//! whole-command TEXT refusal, the explicit refusal set, and the
//! one-transaction/one-undo-step shape.

use super::*;

/// Build a drawing with layer 0 plus the given entities/blocks.
fn build(entities: Vec<DbEntity>, blocks: Vec<BlockDefinition>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for block in blocks {
        b.insert_block(block).unwrap();
    }
    for entity in entities {
        b.insert_entity(entity).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([9u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

/// A model-space entity in the given space (block members use `SpaceId::Block`).
fn ent_in(id: u128, space: SpaceId, geometry: SemanticGeometry, order: i64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry,
        draw_order: order,
    }
}

/// The two opposite corners of a crossing window, as the **canvas logical**
/// corners the payload expects, from the two world corners a test thinks in.
///
/// The viewport is `Viewport::new` (the default plan camera, scale 1), so this
/// uses the same `world_to_screen` the host uses when it builds the payload.
fn window(a: Point3, b: Point3) -> [[f64; 2]; 2] {
    let vp = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    let size = vp.logical_size;
    let to_logical = |world: Point3| {
        vp.camera
            .world_to_screen(world, size)
            .expect("finite world point")
    };
    [to_logical(a), to_logical(b)]
}

fn stretch(
    app: &mut Application,
    session: &mut SessionState,
    rect: [[f64; 2]; 2],
    base: Point3,
    delta: Point3,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::StretchEntities,
        CommandPayload::Stretch {
            rect_logical: rect,
            base,
            delta,
        },
    )
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn geometry_of(app: &Application, id: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .unwrap()
        .geometry
        .clone()
}

fn line_of(app: &Application, id: u128) -> (Point3, Point3) {
    match geometry_of(app, id) {
        SemanticGeometry::Line { start, end } => (start, end),
        other => panic!("expected a line, got {other:?}"),
    }
}

fn polyline_of(app: &Application, id: u128) -> (Vec<Point3>, Vec<f64>, bool) {
    match geometry_of(app, id) {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points, bulges, closed),
        other => panic!("expected a polyline, got {other:?}"),
    }
}

fn circle_of(app: &Application, id: u128) -> (Point3, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Circle { center, radius, .. } => (center, radius),
        other => panic!("expected a circle, got {other:?}"),
    }
}

fn arc_of(app: &Application, id: u128) -> (Point3, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Arc { center, radius, .. } => (center, radius),
        other => panic!("expected an arc, got {other:?}"),
    }
}

fn insert_transform(app: &Application, id: u128) -> Transform3 {
    match geometry_of(app, id) {
        SemanticGeometry::Insert { transform, .. } => transform,
        other => panic!("expected an insert, got {other:?}"),
    }
}

fn near(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
}

fn line(id: u128, start: Point3, end: Point3, order: i64) -> DbEntity {
    ent(id, SemanticGeometry::Line { start, end }, order)
}

fn has_code(outcome: &CommandOutcome, code: &str) -> bool {
    outcome.diagnostics.iter().any(|d| d.code == code)
}

#[test]
fn stretch_fully_enclosed_line_equals_move_result() {
    // A window that fully encloses the line: the whole entity is translated,
    // which must equal MOVE by the same displacement.
    let (mut app, mut session) = build(
        vec![line(1, point(0.0, 0.0), point(10.0, 0.0), 0)],
        Vec::new(),
    );
    let before = revision(&app);
    stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -1.0), point(11.0, 1.0)),
        point(0.0, 0.0),
        point(3.0, 4.0),
    )
    .unwrap();
    let stretched = line_of(&app, 1);
    assert!(near(stretched.0, point(3.0, 4.0)), "{:?}", stretched.0);
    assert!(near(stretched.1, point(13.0, 4.0)), "{:?}", stretched.1);
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");

    // MOVE the same entity by the same displacement gives identical coordinates.
    let (mut move_app, mut move_session) = build(
        vec![line(1, point(0.0, 0.0), point(10.0, 0.0), 0)],
        Vec::new(),
    );
    execute(
        &mut move_app,
        &mut move_session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1)],
            delta: point(3.0, 4.0),
        },
    )
    .unwrap();
    assert_eq!(stretched, line_of(&move_app, 1));
}

#[test]
fn stretch_window_crossing_a_segment_with_no_endpoint_inside_changes_nothing_with_diagnostic() {
    // The window crosses the segment (x in [4, 6]) but neither endpoint is
    // inside: unchanged plus an explicit diagnostic, never silent.
    let (mut app, mut session) = build(
        vec![line(1, point(0.0, 0.0), point(10.0, 0.0), 0)],
        Vec::new(),
    );
    let before = revision(&app);
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(4.0, -1.0), point(6.0, 1.0)),
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap();
    assert!(
        outcome.changes.is_none(),
        "no transaction when nothing changes"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
    assert_eq!(entity_count(&app), 1);
    let (start, end) = line_of(&app, 1);
    assert!(near(start, point(0.0, 0.0)) && near(end, point(10.0, 0.0)));
    assert!(
        has_code(&outcome, "drawing.stretch.no_vertex"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_moves_only_the_line_endpoint_inside_the_window() {
    let (mut app, mut session) = build(
        vec![line(1, point(0.0, 0.0), point(10.0, 0.0), 0)],
        Vec::new(),
    );
    stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -1.0), point(2.0, 1.0)),
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap();
    let (start, end) = line_of(&app, 1);
    assert!(near(start, point(0.0, 5.0)), "{start:?}");
    assert!(near(end, point(10.0, 0.0)), "{end:?}");
}

#[test]
fn stretch_moves_exactly_the_polyline_vertices_inside_the_window() {
    let (mut app, mut session) = build(
        vec![ent(
            1,
            SemanticGeometry::Polyline {
                points: vec![
                    point(0.0, 0.0),
                    point(1.0, 0.0),
                    point(2.0, 0.0),
                    point(5.0, 0.0),
                ],
                bulges: vec![0.0; 4],
                closed: false,
            },
            0,
        )],
        Vec::new(),
    );
    stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -1.0), point(3.0, 1.0)),
        point(0.0, 0.0),
        point(0.0, 2.0),
    )
    .unwrap();
    let (points, bulges, closed) = polyline_of(&app, 1);
    assert_eq!(
        points,
        vec![
            point(0.0, 2.0),
            point(1.0, 2.0),
            point(2.0, 2.0),
            point(5.0, 0.0),
        ]
    );
    assert_eq!(bulges, vec![0.0; 4], "straight segments keep zero bulges");
    assert!(!closed);
}

#[test]
fn stretch_moves_a_bulge_polyline_vertex_and_preserves_the_bulge_value() {
    // UNVERIFIED rule (ADR 0016 decision E): a bulge-segment endpoint moves while
    // the stored bulge value is preserved. Pinned numerically.
    let (mut app, mut session) = build(
        vec![ent(
            1,
            SemanticGeometry::Polyline {
                points: vec![point(0.0, 0.0), point(4.0, 0.0), point(8.0, 0.0)],
                bulges: vec![0.5, 0.0, 0.0],
                closed: false,
            },
            0,
        )],
        Vec::new(),
    );
    stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -1.0), point(5.0, 1.0)),
        point(0.0, 0.0),
        point(0.0, 3.0),
    )
    .unwrap();
    let (points, bulges, closed) = polyline_of(&app, 1);
    assert_eq!(
        points,
        vec![point(0.0, 3.0), point(4.0, 3.0), point(8.0, 0.0)]
    );
    assert_eq!(bulges, vec![0.5, 0.0, 0.0], "the bulge value is preserved");
    assert!(!closed);
}

/// The STRETCH "fully enclosed" rule is **two-way** (ADR 0016 decision E): a
/// straight LINE or a straight-segment LWPOLYLINE is fully enclosed iff every
/// vertex is inside the window, but a polyline **with a bulge segment** uses the
/// conservative world AABB (which includes the arc's sagitta). A bulge arc can
/// bow out of the window while both its endpoints are inside; that boundary case
/// is our declared (conservative) choice.
///
/// This test pins the observable consequence: a third vertex just outside the
/// window stays put when the entity is only partially enclosed (per-vertex
/// stretch) and moves when the entity is fully enclosed (whole-entity move).
///
/// **Observability note**: when *every* vertex is inside the window but the arc
/// bows outside, the conservative AABB classifies the entity as partial whereas
/// the straight-polyline vertex rule would call it fully enclosed — but because
/// every vertex is inside, per-vertex stretch and whole-entity move are
/// numerically identical for a translation, so that specific divergence is not
/// observable in the stored coordinates. The cases below therefore keep an
/// out-of-window vertex so partial vs full is visible.
#[test]
fn stretch_bulge_polyline_conservative_bounds_rule_pins_partial_and_full() {
    // Polyline (0,0)-(2,0)-(5,0) with bulge 1 on the first segment: the
    // (0,0)-(2,0) chord bulges to y = -1 (sagitta |1| * 2 / 2 = 1), so the
    // conservative AABB is x in [0, 5], y in [-1, 0].
    let polyline = || SemanticGeometry::Polyline {
        points: vec![point(0.0, 0.0), point(2.0, 0.0), point(5.0, 0.0)],
        bulges: vec![1.0, 0.0, 0.0],
        closed: false,
    };

    // (a) Both bulge endpoints are inside the window, but the arc bows outside it
    // (and the third vertex is outside too): the conservative AABB is not
    // contained => **partial** => per-vertex. Only the two in-window vertices move;
    // the bulge value is preserved.
    let (mut app, mut session) = build(vec![ent(1, polyline(), 0)], Vec::new());
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(-0.5, -0.1), point(3.0, 0.5)),
        point(0.0, 0.0),
        point(0.0, 3.0),
    )
    .unwrap();
    assert!(
        outcome.changes.is_some(),
        "a partial stretch still changes the polyline"
    );
    let (points, bulges, closed) = polyline_of(&app, 1);
    assert_eq!(
        points,
        vec![point(0.0, 3.0), point(2.0, 3.0), point(5.0, 0.0)],
        "partial: only the in-window vertices move; the outside vertex is put"
    );
    assert_eq!(bulges, vec![1.0, 0.0, 0.0], "the bulge value is preserved");
    assert!(!closed);

    // (b) The converse: the window contains every vertex AND the arc's bulge
    // (AABB y in [-1, 0]), so the entity is **fully enclosed** => whole move:
    // every vertex, including the one left put in (a), moves.
    let (mut app, mut session) = build(vec![ent(1, polyline(), 0)], Vec::new());
    stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -2.0), point(6.0, 1.0)),
        point(0.0, 0.0),
        point(0.0, 3.0),
    )
    .unwrap();
    let (points, bulges, _) = polyline_of(&app, 1);
    assert_eq!(
        points,
        vec![point(0.0, 3.0), point(2.0, 3.0), point(5.0, 3.0)],
        "fully enclosed: the whole entity is moved"
    );
    assert_eq!(bulges, vec![1.0, 0.0, 0.0], "the bulge value is preserved");
}

#[test]
fn stretch_fully_enclosed_circle_is_moved_and_partially_enclosed_circle_is_unchanged() {
    let circle = || SemanticGeometry::Circle {
        center: point(5.0, 5.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 3.0,
    };
    // Fully enclosed: the whole circle is moved.
    let (mut app, mut session) = build(vec![ent(1, circle(), 0)], Vec::new());
    stretch(
        &mut app,
        &mut session,
        window(point(0.0, 0.0), point(10.0, 10.0)),
        point(0.0, 0.0),
        point(2.0, 3.0),
    )
    .unwrap();
    let (center, radius) = circle_of(&app, 1);
    assert!(near(center, point(7.0, 8.0)), "{center:?}");
    assert!((radius - 3.0).abs() < 1e-9, "radius is unchanged");

    // Partially enclosed: unchanged plus a diagnostic (a circle cannot stretch).
    let (mut app2, mut session2) = build(vec![ent(1, circle(), 0)], Vec::new());
    let before = revision(&app2);
    let outcome = stretch(
        &mut app2,
        &mut session2,
        window(point(0.0, 0.0), point(6.0, 6.0)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app2), before);
    let (center, radius) = circle_of(&app2, 1);
    assert!(near(center, point(5.0, 5.0)), "{center:?}");
    assert!((radius - 3.0).abs() < 1e-9);
    assert!(
        has_code(&outcome, "drawing.stretch.not_stretchable"),
        "{:?}",
        outcome.diagnostics
    );
}

/// A drawing with an INSERT (id 1) of a block whose member is a unit line, placed
/// at `(5, 5)`.
fn insert_app() -> (Application, SessionState) {
    let member = ent_in(
        100,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        0,
    );
    let insert = ent(
        1,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
        },
        1,
    );
    build(
        vec![member, insert],
        vec![BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        }],
    )
}

#[test]
fn stretch_fully_enclosed_insert_is_moved_and_partially_enclosed_insert_is_unchanged() {
    // Fully enclosed: the insert's placement is translated by the displacement.
    let (mut app, mut session) = insert_app();
    stretch(
        &mut app,
        &mut session,
        window(point(0.0, 0.0), point(10.0, 10.0)),
        point(0.0, 0.0),
        point(2.0, 3.0),
    )
    .unwrap();
    let transform = insert_transform(&app, 1);
    assert!(
        near(transform.apply_point(point(0.0, 0.0)), point(7.0, 8.0)),
        "{transform:?}"
    );

    // Partially enclosed: an insert cannot stretch, so it is unchanged with a
    // diagnostic.
    let (mut app2, mut session2) = insert_app();
    let before = revision(&app2);
    let outcome = stretch(
        &mut app2,
        &mut session2,
        window(point(0.0, 0.0), point(5.5, 5.5)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app2), before);
    let transform = insert_transform(&app2, 1);
    assert!(
        near(transform.apply_point(point(0.0, 0.0)), point(5.0, 5.0)),
        "{transform:?}"
    );
    assert!(
        has_code(&outcome, "drawing.stretch.not_stretchable"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_fully_enclosed_arc_is_moved_and_partially_enclosed_arc_is_unchanged() {
    let arc = || SemanticGeometry::Arc {
        center: point(5.0, 5.0),
        normal: p3(0.0, 0.0, 1.0),
        radius: 3.0,
        start: 0.0,
        sweep: std::f64::consts::PI,
    };
    let (mut app, mut session) = build(vec![ent(1, arc(), 0)], Vec::new());
    stretch(
        &mut app,
        &mut session,
        window(point(0.0, 0.0), point(10.0, 10.0)),
        point(0.0, 0.0),
        point(2.0, 3.0),
    )
    .unwrap();
    let (center, radius) = arc_of(&app, 1);
    assert!(near(center, point(7.0, 8.0)), "{center:?}");
    assert!((radius - 3.0).abs() < 1e-9);

    let (mut app2, mut session2) = build(vec![ent(1, arc(), 0)], Vec::new());
    let before = revision(&app2);
    let outcome = stretch(
        &mut app2,
        &mut session2,
        window(point(0.0, 0.0), point(6.0, 6.0)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap();
    assert!(outcome.changes.is_none());
    assert_eq!(revision(&app2), before);
    let (center, _) = arc_of(&app2, 1);
    assert!(near(center, point(5.0, 5.0)), "{center:?}");
    assert!(
        has_code(&outcome, "drawing.stretch.not_stretchable"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn stretch_fails_the_whole_command_for_text_in_the_window_and_writes_nothing() {
    let text = SemanticGeometry::Text {
        text: "AB".into(),
        position: point(5.0, 5.0),
        style: StyleId(0),
        height: 1.0,
        rotation: 0.0,
        font: None,
        h_align: Default::default(),
        v_align: Default::default(),
        wrap_width: None,
    };
    // A fully enclosed TEXT still fails the whole command. ADR 0019 decision B
    // keeps this behaviour: TEXT accepts a similarity transform now, but STRETCH's
    // per-vertex semantics do not apply to a text run and Autodesk does not
    // document its behaviour, so the refusal is the restricted transform, not
    // "text cannot be transformed".
    let (mut app, mut session) = build(vec![ent(1, text, 0)], Vec::new());
    let before = revision(&app);
    let error = stretch(
        &mut app,
        &mut session,
        window(point(0.0, 0.0), point(10.0, 10.0)),
        point(0.0, 0.0),
        point(1.0, 1.0),
    )
    .unwrap_err();
    match error {
        CadError::Unsupported(reason) => {
            assert!(
                reason.contains("similarity-only transform"),
                "the reason must state the restricted transform: {reason}"
            );
            assert!(
                !reason.contains("transform surface refuses"),
                "the stale reason must be gone: {reason}"
            );
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
    assert_eq!(revision(&app), before, "a refusal changes nothing");
    assert_eq!(entity_count(&app), 1);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn stretch_refuses_non_finite_degenerate_and_zero_displacement_inputs_without_change() {
    let cases: [(&str, [[f64; 2]; 2], Point3, Point3); 5] = [
        (
            "non-finite window corner",
            [[0.0, 0.0], [f64::NAN, 1.0]],
            point(0.0, 0.0),
            point(1.0, 1.0),
        ),
        (
            "non-finite base",
            window(point(-1.0, -1.0), point(2.0, 1.0)),
            point(f64::INFINITY, 0.0),
            point(1.0, 1.0),
        ),
        (
            "non-finite displacement",
            window(point(-1.0, -1.0), point(2.0, 1.0)),
            point(0.0, 0.0),
            point(f64::NAN, 1.0),
        ),
        (
            "degenerate rectangle (zero width)",
            window(point(0.0, 0.0), point(0.0, 5.0)),
            point(0.0, 0.0),
            point(1.0, 1.0),
        ),
        (
            "zero displacement",
            window(point(-1.0, -1.0), point(2.0, 1.0)),
            point(0.0, 0.0),
            point(0.0, 0.0),
        ),
    ];
    for (label, rect, base, delta) in cases {
        let (mut app, mut session) = build(
            vec![line(1, point(0.0, 0.0), point(10.0, 0.0), 0)],
            Vec::new(),
        );
        let before = revision(&app);
        let error = stretch(&mut app, &mut session, rect, base, delta).unwrap_err();
        assert!(
            matches!(error, CadError::InvalidInput(_)),
            "{label}: {error:?}"
        );
        assert_eq!(revision(&app), before, "{label} must change nothing");
        assert_eq!(entity_count(&app), 1, "{label}");
        assert_eq!(undo_depth(&app), 0, "{label} must record no history");
        let (start, end) = line_of(&app, 1);
        assert!(
            near(start, point(0.0, 0.0)) && near(end, point(10.0, 0.0)),
            "{label}: the geometry must be untouched"
        );
    }
}

#[test]
fn stretch_is_one_transaction_one_undo_step_and_undo_restores_exactly() {
    let (mut app, mut session) = build(
        vec![
            line(1, point(0.0, 0.0), point(10.0, 0.0), 0),
            line(2, point(0.0, 3.0), point(10.0, 3.0), 1),
        ],
        Vec::new(),
    );
    let original_1 = line_of(&app, 1);
    let original_2 = line_of(&app, 2);
    let before = revision(&app);
    let outcome = stretch(
        &mut app,
        &mut session,
        window(point(-1.0, -1.0), point(2.0, 4.0)),
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap();
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step covers both entities");
    assert_eq!(outcome.objects.len(), 2, "both entities changed");
    assert_ne!(line_of(&app, 1), original_1);
    assert_ne!(line_of(&app, 2), original_2);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(line_of(&app, 1), original_1);
    assert_eq!(line_of(&app, 2), original_2);
    assert_eq!(undo_depth(&app), 0);
}
