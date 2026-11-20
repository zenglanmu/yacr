//! BOUNDARY (AutoCAD 2026) contract tests.
//!
//! Pins the declared internal-point v1 plus the two `-BOUNDARY` options: one picked
//! point selects the **smallest visible closed boundary** containing it plus nested
//! islands, and every loop becomes one closed, non-associative `AcDbPolyline` on the
//! active layer. Only whole visible circle/full ellipse/closed polyline entities
//! participate; the current selection is deliberately not used. `BoundaryOptions`
//! narrows the search to a named **boundary set** and can switch **island
//! detection** off (which must be named, never silently dropped). On-edge/
//! no-boundary/ambiguous picks, invalid boundary sets and Viewer mode write nothing.

use super::*;

use super::hatch::{circle, entity_count, square, undo_depth};

use crate::command_id::boundary::{BoundaryInput, BoundaryOptions};
use crate::selection::model_ref;

fn boundary_app(entities: Vec<(u128, SemanticGeometry)>) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for (index, (id, geometry)) in entities.into_iter().enumerate() {
        b.insert_entity(ent(id, geometry, index as i64)).unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([7u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
            compat: Vec::new(),
            sources: Vec::new(),
            persistence: crate::PersistenceState::untitled(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

fn run_boundary(
    app: &mut Application,
    session: &mut SessionState,
    seed: Point3,
) -> CadResult<CommandOutcome> {
    run_boundary_with(app, session, seed, BoundaryOptions::default())
}

fn run_boundary_with(
    app: &mut Application,
    session: &mut SessionState,
    seed: Point3,
    options: BoundaryOptions,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Boundary,
        CommandPayload::BoundaryPoint(BoundaryInput { seed, options }),
    )
}

/// The message of the one diagnostic with `code`.
fn diagnostic_message(outcome: &CommandOutcome, code: &str) -> String {
    let messages: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(messages.len(), 1, "expected one {code} diagnostic");
    messages[0].to_string()
}

/// The geometry of one created object id.
fn created_geometry(app: &Application, id: ObjectId) -> &SemanticGeometry {
    &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id.0))
        .unwrap_or_else(|| panic!("created entity {} exists", id.0))
        .geometry
}

fn assert_refused(app: &mut Application, session: &mut SessionState, seed: Point3, needle: &str) {
    assert_refused_with(app, session, seed, BoundaryOptions::default(), needle)
}

fn assert_refused_with(
    app: &mut Application,
    session: &mut SessionState,
    seed: Point3,
    options: BoundaryOptions,
    needle: &str,
) {
    let before_revision = revision(app);
    let before_depth = undo_depth(app);
    let before_count = entity_count(app);
    let error = run_boundary_with(app, session, seed, options).expect_err("BOUNDARY must refuse");
    let text = format!("{error:?}");
    assert!(text.contains(needle), "expected {needle:?} in {text}");
    assert_eq!(revision(app), before_revision, "a refusal must not write");
    assert_eq!(
        undo_depth(app),
        before_depth,
        "a refusal must not open a transaction"
    );
    assert_eq!(entity_count(app), before_count, "a refusal must not insert");
}

#[test]
fn one_internal_point_creates_one_closed_rectangle_polyline() {
    let (mut app, mut session) = boundary_app(vec![(1, square(1, 0.0, 10.0, 0.0).1)]);
    let before_revision = revision(&app);

    let outcome = run_boundary(&mut app, &mut session, p3(5.0, 5.0, 0.0))
        .expect("BOUNDARY succeeds inside the closed polyline");
    assert_eq!(
        outcome.objects.len(),
        1,
        "rectangle -> exactly one polyline"
    );

    match created_geometry(&app, outcome.objects[0]) {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => {
            assert_eq!(points.len(), 4, "the rectangle keeps four vertices");
            assert_eq!(bulges, &vec![0.0; 4], "straight ring stays straight");
            assert!(*closed, "BOUNDARY always writes closed polylines");
            assert_eq!(points[0].z, 0.0);
            assert_eq!(points[1].z, 0.0);
        }
        other => panic!("expected a polyline, got {other:?}"),
    }
    assert_eq!(entity_count(&app), 2, "source boundary is retained");
    assert_ne!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    // The single undo removes every created polyline and keeps the source.
    let created_id = outcome.objects[0];
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 1);
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(created_id.0))
        .is_none());
}

#[test]
fn nested_boundaries_become_one_polyline_per_loop() {
    let (mut app, mut session) = boundary_app(vec![
        (1, square(1, 0.0, 10.0, 0.0).1),
        (2, square(2, 3.0, 7.0, 0.0).1),
    ]);
    let outcome = run_boundary(&mut app, &mut session, p3(1.0, 1.0, 0.0))
        .expect("BOUNDARY succeeds in the annulus");
    assert_eq!(outcome.objects.len(), 2, "outer loop plus one island");
    for id in &outcome.objects {
        match created_geometry(&app, *id) {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
                ..
            } => {
                assert_eq!(points.len(), 4);
                assert_eq!(bulges, &vec![0.0; 4]);
                assert!(*closed);
            }
            other => panic!("expected a polyline, got {other:?}"),
        }
    }
    assert_eq!(entity_count(&app), 4, "both sources retained");
}

#[test]
fn a_circle_boundary_keeps_its_arc_bulges() {
    let (mut app, mut session) = boundary_app(vec![(1, circle(1, 0.0, 0.0, 0.0, 5.0).1)]);
    let outcome = run_boundary(&mut app, &mut session, p3(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let SemanticGeometry::Polyline { points, bulges, .. } =
        created_geometry(&app, outcome.objects[0])
    else {
        panic!("expected a polyline");
    };
    assert_eq!(points.len(), 4, "circle is four quarter-arc vertices");
    let bulge = std::f64::consts::FRAC_PI_8.tan();
    assert_eq!(
        bulges,
        &vec![bulge; 4],
        "bulges are preserved, never sampled"
    );
}

#[test]
fn on_edge_no_boundary_and_ambiguous_picks_write_nothing() {
    let (mut app, mut session) = boundary_app(vec![(1, square(1, 0.0, 10.0, 0.0).1)]);
    assert_refused(
        &mut app,
        &mut session,
        p3(0.0, 0.0, 0.0),
        "boundary.boundary_on_edge",
    );
    assert_refused(
        &mut app,
        &mut session,
        p3(20.0, 20.0, 0.0),
        "boundary.boundary_not_found",
    );

    let (mut app, mut session) = boundary_app(vec![
        (1, square(1, 0.0, 10.0, 0.0).1),
        (2, square(2, 0.0, 10.0, 0.0).1),
    ]);
    assert_refused(
        &mut app,
        &mut session,
        p3(5.0, 5.0, 0.0),
        "boundary.boundary_ambiguous",
    );
}

#[test]
fn boundary_refuses_wrong_payload_non_planar_seed_and_viewer() {
    let (mut app, mut session) = boundary_app(vec![(1, square(1, 0.0, 10.0, 0.0).1)]);
    let before_revision = revision(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Boundary,
        CommandPayload::None,
    )
    .expect_err("a bare BOUNDARY payload is refused");
    assert!(format!("{error:?}").contains("boundary.payload"));
    assert_eq!(revision(&app), before_revision);

    assert_refused(
        &mut app,
        &mut session,
        Point3 {
            x: f64::NAN,
            y: 0.0,
            z: 0.0,
        },
        "boundary.not_planar",
    );

    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    let before_revision = revision(&app);
    let error = run_boundary(&mut app, &mut viewer, p3(5.0, 5.0, 0.0))
        .expect_err("Viewer mode refuses BOUNDARY");
    assert_eq!(error, CadError::PermissionDenied);
    assert_eq!(revision(&app), before_revision);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn boundary_tool_captures_one_internal_point() {
    let mut tool = DrawTool::new(DrawToolKind::Boundary, 0);
    assert_eq!(tool.kind().key(), "boundary");
    assert_eq!(
        DrawToolKind::from_key("boundary"),
        Some(DrawToolKind::Boundary)
    );
    assert_eq!(tool.remaining(), Some(1));
    assert!(!tool.is_ready());
    tool.push_point(p3(1.0, 2.0, 0.0)).unwrap();
    assert!(tool.is_ready());
    assert_eq!(
        tool.intent().unwrap(),
        DrawIntent::Boundary {
            seed: p3(1.0, 2.0, 0.0)
        }
    );
}

#[test]
fn island_detection_names_every_separate_loop_it_writes() {
    // Outer 0..10 with one inner 3..7 and the seed in the annulus.
    let (mut app, mut session) = boundary_app(vec![
        (1, square(1, 0.0, 10.0, 0.0).1),
        (2, square(2, 3.0, 7.0, 0.0).1),
    ]);
    let outcome = run_boundary(&mut app, &mut session, p3(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(outcome.objects.len(), 2, "outer loop plus one island");

    let islands = diagnostic_message(&outcome, "boundary.islands");
    assert!(islands.contains("1 nested loop(s)"), "{islands}");
    assert!(
        islands.contains("no hole list") && islands.contains("diagnostic only"),
        "the hole representation is named, not implied: {islands}"
    );
    let created = diagnostic_message(&outcome, "boundary.created");
    assert!(created.contains("island detection ON"), "{created}");
    assert!(
        created.contains("every visible closed entity"),
        "the whole drawing is the default boundary set: {created}"
    );
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    // The single undo removes both loops and keeps both sources.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 2);
}

#[test]
fn two_inner_islands_create_three_polylines_in_one_undo_step() {
    let (mut app, mut session) = boundary_app(vec![
        (1, square(1, 0.0, 20.0, 0.0).1),
        (2, square(2, 2.0, 4.0, 0.0).1),
        (3, square(3, 12.0, 18.0, 0.0).1),
    ]);
    let outcome = run_boundary(&mut app, &mut session, p3(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(
        outcome.objects.len(),
        3,
        "outer loop plus both inner islands"
    );
    let islands = diagnostic_message(&outcome, "boundary.islands");
    assert!(islands.contains("2 nested loop(s)"), "{islands}");
    assert_eq!(entity_count(&app), 6, "every source is retained");
    assert_eq!(undo_depth(&app), 1, "one transaction / one undo step");

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(entity_count(&app), 3, "one undo removes all three loops");
}

#[test]
fn island_detection_off_writes_only_the_outer_loop_and_names_the_skipped_islands() {
    let (mut app, mut session) = boundary_app(vec![
        (1, square(1, 0.0, 10.0, 0.0).1),
        (2, square(2, 3.0, 7.0, 0.0).1),
    ]);
    let options = BoundaryOptions {
        boundary_set: None,
        island_detection: false,
    };
    let outcome = run_boundary_with(&mut app, &mut session, p3(1.0, 1.0, 0.0), options).unwrap();
    assert_eq!(outcome.objects.len(), 1, "only the outer loop is written");
    let SemanticGeometry::Polyline { points, .. } = created_geometry(&app, outcome.objects[0])
    else {
        panic!("expected a polyline");
    };
    assert!(
        points
            .iter()
            .all(|point| point.x <= 10.0 && point.y <= 10.0),
        "the outer ring is the 0..10 square: {points:?}"
    );
    let skipped = diagnostic_message(&outcome, "boundary.island_detection_off");
    assert!(skipped.contains("1 nested loop(s)"), "{skipped}");
    assert!(
        skipped.contains("NOT created") && skipped.contains("no hole is ever implied"),
        "the omitted island is named, never silently dropped: {skipped}"
    );
    assert!(
        outcome
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "boundary.islands"),
        "island detection off must not claim islands were written"
    );
    let created = diagnostic_message(&outcome, "boundary.created");
    assert!(created.contains("island detection OFF"), "{created}");
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn boundary_set_narrows_the_search_and_replaces_the_whole_drawing_default() {
    // Outer 0..10 (id 1) with an inner 3..7 (id 2); the seed sits in the annulus.
    let (mut app, mut session) = boundary_app(vec![
        (1, square(1, 0.0, 10.0, 0.0).1),
        (2, square(2, 3.0, 7.0, 0.0).1),
    ]);
    // The default boundary set is the whole drawing: outer loop + island.
    let outcome = run_boundary(&mut app, &mut session, p3(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(outcome.objects.len(), 2);

    // Narrowing the set to the outer entity alone drops the island: the island is
    // outside the set, not silently ignored geometry inside it.
    let outer_only = BoundaryOptions {
        boundary_set: Some(vec![model_ref(DocumentId(1), EntityId(1))]),
        island_detection: true,
    };
    let outcome = run_boundary_with(
        &mut app,
        &mut session,
        p3(1.0, 1.0, 0.0),
        outer_only.clone(),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1, "the set holds one candidate");
    let created = diagnostic_message(&outcome, "boundary.created");
    assert!(created.contains("named 1-entity boundary set"), "{created}");

    // A set that does not contain the seed cannot silently widen: it is the
    // shared no-boundary refusal.
    let inner_only = BoundaryOptions {
        boundary_set: Some(vec![model_ref(DocumentId(1), EntityId(2))]),
        island_detection: true,
    };
    assert_refused_with(
        &mut app,
        &mut session,
        p3(1.0, 1.0, 0.0),
        inner_only,
        "boundary.boundary_not_found",
    );
}

#[test]
fn an_invalid_boundary_set_is_a_named_refusal_that_writes_nothing() {
    let (mut app, mut session) = boundary_app(vec![
        (1, square(1, 0.0, 10.0, 0.0).1),
        (
            2,
            SemanticGeometry::Line {
                start: p3(0.0, 0.0, 0.0),
                end: p3(5.0, 5.0, 0.0),
            },
        ),
    ]);
    // An empty set is refused, never widened back to the whole drawing.
    assert_refused_with(
        &mut app,
        &mut session,
        p3(1.0, 1.0, 0.0),
        BoundaryOptions {
            boundary_set: Some(Vec::new()),
            island_detection: true,
        },
        "boundary.set_empty",
    );
    // An open LINE is not a closed candidate.
    assert_refused_with(
        &mut app,
        &mut session,
        p3(1.0, 1.0, 0.0),
        BoundaryOptions {
            boundary_set: Some(vec![model_ref(DocumentId(1), EntityId(2))]),
            island_detection: true,
        },
        "boundary.set_not_candidate",
    );
    // A ref that is not in the drawing is refused by name too.
    assert_refused_with(
        &mut app,
        &mut session,
        p3(1.0, 1.0, 0.0),
        BoundaryOptions {
            boundary_set: Some(vec![model_ref(DocumentId(1), EntityId(99))]),
            island_detection: true,
        },
        "boundary.set_not_candidate",
    );
    // A block-instance member / sub-element ref is not directly addressable.
    assert_refused_with(
        &mut app,
        &mut session,
        p3(1.0, 1.0, 0.0),
        BoundaryOptions {
            boundary_set: Some(vec![SelectionRef {
                document: DocumentId(1),
                entity: EntityId(1),
                instance: InstancePath(vec![EntityId(1)]),
                sub_element: None,
            }]),
            island_detection: true,
        },
        "boundary.set_not_candidate",
    );
}
