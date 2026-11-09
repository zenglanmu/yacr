//! STRETCH crossing-polygon / lasso contract tests.
//!
//! The payload is the free-form boundary vertex list (a lasso is the same
//! containment rule over the same list), so these pin: partial vs whole
//! enclosure, the circle-centre rule, and winding independence of the even-odd
//! point-in-polygon predicate. One transaction / one undo step, exactly like the
//! rectangle path.

use super::*;

/// Build a drawing with layer 0 plus the given entities.
fn build(entities: Vec<DbEntity>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
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
            identity: DocumentIdentity::Sha256([11u8; 32]),
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

fn line(id: u128, start: Point3, end: Point3) -> DbEntity {
    ent(id, SemanticGeometry::Line { start, end }, 0)
}

fn circle(id: u128, center: Point3, radius: f64) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Circle {
            center,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius,
        },
        0,
    )
}

/// The canvas-logical form of one world point (the viewport is the default plan
/// camera at scale 1, the same mapping the host uses to build the payload).
fn logical(world: Point3) -> [f64; 2] {
    let vp = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    vp.camera
        .world_to_screen(world, vp.logical_size)
        .expect("finite world point")
}

fn stretch_polygon(
    app: &mut Application,
    session: &mut SessionState,
    vertices: &[(f64, f64)],
    base: Point3,
    delta: Point3,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::StretchEntities,
        CommandPayload::StretchPolygon(Box::new(StretchPolygonCommand {
            polygon_logical: vertices
                .iter()
                .map(|(x, y)| logical(point(*x, *y)))
                .collect(),
            base,
            delta,
        })),
    )
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

fn circle_of(app: &Application, id: u128) -> (Point3, f64) {
    match geometry_of(app, id) {
        SemanticGeometry::Circle { center, radius, .. } => (center, radius),
        other => panic!("expected a circle, got {other:?}"),
    }
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn near(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
}

fn has_code(outcome: &CommandOutcome, code: &str) -> bool {
    outcome.diagnostics.iter().any(|d| d.code == code)
}

#[test]
fn stretch_polygon_moves_only_the_endpoint_it_encloses() {
    let (mut app, mut session) = build(vec![line(1, point(0.0, 0.0), point(10.0, 0.0))]);
    let before = revision(&app);
    let outcome = stretch_polygon(
        &mut app,
        &mut session,
        &[(-1.0, -1.0), (2.0, -1.0), (2.0, 1.0), (-1.0, 1.0)],
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap();
    let (start, end) = line_of(&app, 1);
    assert!(near(start, point(0.0, 5.0)), "{start:?}");
    assert!(near(end, point(10.0, 0.0)), "{end:?}");
    assert_eq!(outcome.objects.len(), 1);
    assert_eq!(revision(&app).0, before.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "one undo step");
}

#[test]
fn stretch_polygon_fully_enclosing_a_line_moves_it_whole() {
    let (mut app, mut session) = build(vec![line(1, point(0.0, 0.0), point(10.0, 0.0))]);
    stretch_polygon(
        &mut app,
        &mut session,
        &[(-1.0, -1.0), (11.0, -1.0), (11.0, 1.0), (-1.0, 1.0)],
        point(0.0, 0.0),
        point(3.0, 4.0),
    )
    .unwrap();
    let (start, end) = line_of(&app, 1);
    assert!(near(start, point(3.0, 4.0)), "{start:?}");
    assert!(near(end, point(13.0, 4.0)), "{end:?}");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn stretch_polygon_moves_a_circle_only_when_its_centre_is_enclosed() {
    // The crossing polygon hits the circle but does not contain its centre: the
    // circle cannot be stretched, so it stays unchanged with the named reason.
    let (mut app, mut session) = build(vec![circle(1, point(5.0, 5.0), 1.0)]);
    let before = revision(&app);
    let outcome = stretch_polygon(
        &mut app,
        &mut session,
        &[(5.5, 5.5), (6.5, 5.5), (6.5, 6.5), (5.5, 6.5)],
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap();
    assert_eq!(revision(&app), before, "nothing written");
    assert_eq!(undo_depth(&app), 0);
    let (center, radius) = circle_of(&app, 1);
    assert!(near(center, point(5.0, 5.0)) && (radius - 1.0).abs() < 1e-12);
    assert!(
        has_code(&outcome, "drawing.stretch.circle_center_outside"),
        "{:?}",
        outcome.diagnostics
    );

    // A crossing polygon that contains the centre but not the whole circle moves
    // the circle as a whole (the documented rule).
    let (mut app, mut session) = build(vec![circle(1, point(5.0, 5.0), 1.0)]);
    stretch_polygon(
        &mut app,
        &mut session,
        &[(4.0, 4.0), (6.0, 4.0), (6.0, 6.0), (4.0, 6.0)],
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap();
    let (center, radius) = circle_of(&app, 1);
    assert!(near(center, point(5.0, 5.0 + 5.0)), "{center:?}");
    assert!((radius - 1.0).abs() < 1e-12, "radius unchanged: {radius}");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn stretch_polygon_containment_is_winding_independent() {
    let vertices = [(-1.0, -1.0), (2.0, -1.0), (2.0, 1.0), (-1.0, 1.0)];
    let mut reversed: Vec<(f64, f64)> = vertices.to_vec();
    reversed.reverse();

    let run = |vertices: &[(f64, f64)]| {
        let (mut app, mut session) = build(vec![line(1, point(0.0, 0.0), point(10.0, 0.0))]);
        stretch_polygon(
            &mut app,
            &mut session,
            vertices,
            point(0.0, 0.0),
            point(0.0, 5.0),
        )
        .unwrap();
        line_of(&app, 1)
    };

    let forward = run(&vertices);
    let backward = run(&reversed);
    assert!(near(forward.0, backward.0), "{forward:?} vs {backward:?}");
    assert!(near(forward.1, backward.1), "{forward:?} vs {backward:?}");
    assert!(near(forward.0, point(0.0, 5.0)));
}

#[test]
fn stretch_polygon_refuses_a_degenerate_boundary() {
    let (mut app, mut session) = build(vec![line(1, point(0.0, 0.0), point(10.0, 0.0))]);
    let before = revision(&app);
    // Two vertices cannot enclose anything: a named refusal, not a silent no-op.
    let error = stretch_polygon(
        &mut app,
        &mut session,
        &[(0.0, 0.0), (1.0, 1.0)],
        point(0.0, 0.0),
        point(0.0, 5.0),
    )
    .unwrap_err();
    match error {
        CadError::InvalidInput(reason) => {
            assert!(reason.contains("at least three vertices"), "{reason}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert_eq!(revision(&app), before);
    assert_eq!(undo_depth(&app), 0);
}
