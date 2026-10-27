//! ARC `OFFSET` contract tests.
//!
//! The ARC arm of OFFSET (`app_drawing::offset_arc`) uses the **same** distance
//! model as the CIRCLE arm — the new radius is the 3D distance
//! `|side_point − centre|` — so an arc and a circle at the same centre/radius can
//! never disagree about what a pick means. Only the radius changes: `center`,
//! `normal`, `start` and `sweep` are stored byte-identically (a negative sweep
//! stays negative). The named refusals (degenerate stored radius, a pick at the
//! centre, a pick on the arc's own circle) change nothing and record no undo
//! step, and an ELLIPSE is refused by its own name because a uniform offset of an
//! ellipse is not an ellipse.
//!
//! Split out of `offset.rs` (which keeps the LINE/CIRCLE arm) so both files stay
//! well inside the 1000-line code-file cap.

use super::offset::app_with_drawing;
use super::*;

/// A drawing whose only entity (id 1) is an ARC on layer 0, in model space.
///
/// The builder does not geometry-validate, so a degenerate radius (zero or
/// non-finite) reaches OFFSET exactly like an imported one can.
fn arc_app(radius: f64, start: f64, sweep: f64) -> Application {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius,
            start,
            sweep,
        },
        0,
    ))
    .unwrap();
    app_with_drawing(b.finish().unwrap())
}

/// The stored geometry of `id`, cloned out of the document.
fn geometry_of(app: &Application, id: EntityId) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(id)
        .unwrap_or_else(|| panic!("entity {id:?} does not exist"))
        .geometry
        .clone()
}

/// Destructure a stored ARC into `(center, normal, radius, start, sweep)`.
fn arc_frame(app: &Application, id: EntityId) -> (Point3, Point3, f64, f64, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => (center, normal, radius, start, sweep),
        other => panic!("expected an arc, got {other:?}"),
    }
}

fn entity_count(app: &Application) -> usize {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count()
}

/// The message of an `InvalidInput` refusal (any other error kind is a test bug).
fn invalid_input(err: CadError) -> String {
    match err {
        CadError::InvalidInput(message) => message,
        other => panic!("expected an InvalidInput refusal, got {other:?}"),
    }
}

fn offset(
    app: &mut Application,
    session: &mut SessionState,
    side: Point3,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: side,
        },
    )
}

#[test]
fn offset_arc_outward_grows_the_radius_and_keeps_the_span() {
    let mut app = arc_app(5.0, 0.25, 1.5);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let before = entity_count(&app);

    let outcome = offset(&mut app, &mut session, point(0.0, 8.0)).unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.offset");
    let created = EntityId(outcome.objects[0].0);
    assert_ne!(created, EntityId(1), "fresh id");
    assert_eq!(entity_count(&app), before + 1, "exactly one entity added");

    let (center, normal, radius, start, sweep) = arc_frame(&app, created);
    // The picked distance is the new radius: |(0, 8) − (0, 0)| = 8.
    assert!((radius - 8.0).abs() < 1e-12, "new radius {radius}");
    // Everything angular is the source's, byte-identical.
    assert_eq!(center, point(0.0, 0.0));
    assert_eq!(normal, p3(0.0, 0.0, 1.0));
    assert_eq!(start, 0.25);
    assert_eq!(sweep, 1.5);
    assert_eq!(
        geometry_of(&app, EntityId(1)),
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.25,
            sweep: 1.5,
        },
        "the source arc is untouched"
    );

    // Exactly one undo step, and it removes exactly the one created entity.
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        1
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
        .entity(created)
        .is_none());
    assert_eq!(entity_count(&app), before);
}

#[test]
fn offset_arc_inward_shrinks_the_radius() {
    let mut app = arc_app(5.0, 0.0, 1.0);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    // A point between the centre and the arc: |(0, 2) − (0, 0)| = 2.
    let outcome = offset(&mut app, &mut session, point(0.0, 2.0)).unwrap();
    let (_, _, radius, start, sweep) = arc_frame(&app, EntityId(outcome.objects[0].0));
    assert!((radius - 2.0).abs() < 1e-12, "new radius {radius}");
    assert_eq!(start, 0.0);
    assert_eq!(sweep, 1.0);
}

#[test]
fn offset_arc_keeps_a_clockwise_sweep() {
    let mut app = arc_app(5.0, 0.5, -1.5);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = offset(&mut app, &mut session, point(0.0, -7.0)).unwrap();
    let (center, normal, radius, start, sweep) = arc_frame(&app, EntityId(outcome.objects[0].0));
    assert!((radius - 7.0).abs() < 1e-12, "new radius {radius}");
    assert_eq!(center, point(0.0, 0.0));
    assert_eq!(normal, p3(0.0, 0.0, 1.0));
    assert_eq!(start, 0.5, "the start angle is unchanged");
    assert_eq!(sweep, -1.5, "a clockwise sweep stays clockwise");
}

#[test]
fn offset_arc_refuses_a_pick_at_the_centre_or_on_the_circle() {
    let mut app = arc_app(5.0, 0.0, 1.0);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let before = entity_count(&app);
    let rev = revision(&app);
    // The centre is a zero-distance pick; (0, 5) is *on* the arc's circle, so
    // the offset would be zero.
    for side in [point(0.0, 0.0), point(0.0, 5.0)] {
        let err = offset(&mut app, &mut session, side).unwrap_err();
        let message = invalid_input(err);
        assert!(message.starts_with("OFFSET arc:"), "{message}");
        assert_eq!(revision(&app), rev, "no write for {side:?}");
        assert_eq!(entity_count(&app), before, "no entity for {side:?}");
        assert!(!app.can_undo(&DocumentId(1)), "no undo record for {side:?}");
    }
}

#[test]
fn offset_arc_refuses_degenerate_stored_radii() {
    for radius in [0.0, f64::NAN] {
        let mut app = arc_app(radius, 0.0, 1.0);
        let mut session = SessionState::new(DocumentId(1), AppMode::Work);
        let rev = revision(&app);
        let err = offset(&mut app, &mut session, point(0.0, 8.0)).unwrap_err();
        let message = invalid_input(err);
        assert!(
            message.starts_with("OFFSET arc: source radius"),
            "radius {radius}: {message}"
        );
        assert_eq!(revision(&app), rev, "no write for radius {radius}");
        assert_eq!(entity_count(&app), 1, "no entity for radius {radius}");
        assert!(
            !app.can_undo(&DocumentId(1)),
            "no undo record for radius {radius}"
        );
    }
}

#[test]
fn offset_refuses_an_ellipse_by_name() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
        0,
    ))
    .unwrap();
    let mut app = app_with_drawing(b.finish().unwrap());
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let before = entity_count(&app);
    let rev = revision(&app);

    let err = offset(&mut app, &mut session, point(0.0, 8.0)).unwrap_err();
    let message = match err {
        CadError::Unsupported(message) => message,
        other => panic!("expected an Unsupported refusal, got {other:?}"),
    };
    assert!(
        message.starts_with("OFFSET ellipse:"),
        "the refusal names the ellipse: {message}"
    );
    assert!(
        message.contains("not an ellipse"),
        "the refusal states why it is not implemented: {message}"
    );
    assert_eq!(revision(&app), rev, "no write");
    assert_eq!(entity_count(&app), before, "no entity");
    assert!(!app.can_undo(&DocumentId(1)), "no undo record");
}

#[test]
fn offset_arc_copies_the_source_layer_and_space() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(1), "1")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(0),
        entities: vec![EntityId(1)],
        dynamic_visibility: None,
    })
    .unwrap();
    let mut source = ent(
        1,
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: 1.0,
        },
        0,
    );
    source.layer = LayerId(1);
    source.space = SpaceId::Block(cad_domain::BlockId(0));
    b.insert_entity(source).unwrap();
    let mut app = app_with_drawing(b.finish().unwrap());
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    // The session's active layer differs from the source's, and the source is in
    // block space: copying `session.active_layer` / `SpaceId::Model` would fail.
    assert_eq!(session.active_layer, LayerId(0));

    let outcome = offset(&mut app, &mut session, point(0.0, 8.0)).unwrap();
    let created = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .unwrap();
    assert_eq!(created.layer, LayerId(1));
    assert_eq!(created.space, SpaceId::Block(cad_domain::BlockId(0)));
}
