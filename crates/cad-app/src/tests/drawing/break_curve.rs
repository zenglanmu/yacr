//! `BREAK` edit-command contract tests.
//!
//! Pins the two-point break semantics for LINE/ARC/CIRCLE/SPLINE: the removed
//! span between the two projected picks, the kept complementary piece(s), the
//! source-retained/fresh-id transaction shape (one transaction / one undo step)
//! and every named refusal (nothing written, no revision bump, no undo entry).

use super::explode::part;
use super::*;
use cad_db::{DrawingDatabaseBuilder, Layer};

const TAU: f64 = std::f64::consts::TAU;

fn line_geometry(x0: f64, y0: f64, x1: f64, y1: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: point(x0, y0),
        end: point(x1, y1),
    }
}

fn arc_geometry(center: Point3, radius: f64, start: f64, sweep: f64) -> SemanticGeometry {
    SemanticGeometry::Arc {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
        start,
        sweep,
    }
}

fn circle_geometry(center: Point3, radius: f64) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center,
        normal: p3(0.0, 0.0, 1.0),
        radius,
    }
}

/// A work-mode app with layers 0/WALLS and the given model entities.
fn app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    build(entities, AppMode::Work)
}

fn viewer_app_with(entities: Vec<DbEntity>) -> (Application, SessionState) {
    build(entities, AppMode::Viewer)
}

fn build(entities: Vec<DbEntity>, mode: AppMode) -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(7), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
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
            identity: DocumentIdentity::Sha256([41u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), mode))
}

fn line_entity(id: u128, layer: LayerId, geometry: SemanticGeometry, order: i64) -> DbEntity {
    part(id, geometry, layer, SpaceId::Model, "AcDbLine", order)
}

fn arc_entity(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    part(id, geometry, LayerId(0), SpaceId::Model, "AcDbArc", order)
}

fn circle_entity(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    part(
        id,
        geometry,
        LayerId(0),
        SpaceId::Model,
        "AcDbCircle",
        order,
    )
}

fn spline_entity(id: u128, geometry: SemanticGeometry, order: i64) -> DbEntity {
    part(
        id,
        geometry,
        LayerId(0),
        SpaceId::Model,
        "AcDbSpline",
        order,
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

fn entity(app: &Application, id: u128) -> DbEntity {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id))
        .cloned()
        .unwrap_or_else(|| panic!("entity {id} should exist"))
}

fn model_geometry(app: &Application) -> Vec<(EntityId, SemanticGeometry)> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|entity| (entity.id, entity.geometry.clone()))
        .collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn break_at(
    app: &mut Application,
    session: &mut SessionState,
    target: u128,
    first: Point3,
    second: Point3,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Break,
        CommandPayload::Break {
            target: ref_of(target),
            first,
            second,
        },
    )
}

/// The stable refusal code prefix of a `CadError`, or the whole debug form.
fn refusal_message(error: CadError) -> String {
    match error {
        CadError::InvalidInput(message) | CadError::Unsupported(message) => message,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

fn assert_invalid(error: CadError, code: &str) {
    match error {
        CadError::InvalidInput(message) => assert!(
            message.starts_with(code),
            "expected refusal starting with {code}, got {message}"
        ),
        other => panic!("expected InvalidInput {code}, got {other:?}"),
    }
}

fn assert_unsupported(error: CadError, needle: &str) {
    match error {
        CadError::Unsupported(message) => assert!(
            message.contains(needle),
            "expected Unsupported containing {needle}, got {message}"
        ),
        other => panic!("expected Unsupported containing {needle}, got {other:?}"),
    }
}

fn assert_unchanged(app: &Application, expected_revision: Revision, expected_count: usize) {
    assert_eq!(revision(app), expected_revision, "no revision bump");
    assert_eq!(entity_count(app), expected_count, "no entity change");
    assert_eq!(undo_depth(app), 0, "no undo entry");
}

#[test]
fn line_break_retains_the_source_as_the_first_piece_and_inserts_the_second() {
    let source = line_entity(1, LayerId(7), line_geometry(0.0, 0.0, 10.0, 0.0), 7);
    let (mut app, mut session) = app_with(vec![source]);
    let before_revision = revision(&app);
    let before_count = entity_count(&app);

    let outcome = break_at(&mut app, &mut session, 1, point(3.0, 0.0), point(7.0, 0.0))
        .expect("the line breaks at u = 0.3 / 0.7");
    assert_eq!(outcome.objects.len(), 2, "source + one new piece");
    assert_eq!(
        EntityId(outcome.objects[0].0),
        EntityId(1),
        "the source id is retained as the first piece"
    );
    let new_id = EntityId(outcome.objects[1].0);
    assert_ne!(new_id, EntityId(1), "the second piece gets a fresh id");

    let first = entity(&app, 1);
    match &first.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(
                near(start.x, 0.0) && near(start.y, 0.0) && near(end.x, 3.0),
                "source becomes [0, 0.3]: {start:?} → {end:?}"
            );
        }
        other => panic!("expected the first LINE piece, got {other:?}"),
    }
    assert_eq!(first.layer, LayerId(7), "layer preserved");
    assert_eq!(first.space, SpaceId::Model, "space preserved");

    let second = entity(&app, new_id.0);
    match &second.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(
                near(start.x, 7.0) && near(end.x, 10.0) && near(end.y, 0.0),
                "new piece is [0.7, 1]: {start:?} → {end:?}"
            );
        }
        other => panic!("expected the second LINE piece, got {other:?}"),
    }
    assert_eq!(second.layer, LayerId(7), "new piece keeps the layer");
    assert_eq!(second.space, SpaceId::Model, "new piece keeps the space");
    assert_eq!(
        second.draw_order, first.draw_order,
        "the new piece keeps the source draw order"
    );

    // Removed span 3..7 (length 4); the two kept pieces are 3 + 3.
    let total: f64 = model_geometry(&app)
        .iter()
        .map(|(_, geometry)| match geometry {
            SemanticGeometry::Line { start, end } => (end.x - start.x).hypot(end.y - start.y),
            other => panic!("expected LINE pieces, got {other:?}"),
        })
        .sum();
    assert!(near(total, 6.0), "kept lengths sum to 6, got {total}");

    assert_eq!(entity_count(&app), before_count + 1, "one fresh entity");
    assert_eq!(revision(&app).0, before_revision.0 + 1, "one transaction");
    assert_eq!(undo_depth(&app), 1, "exactly one undo step");

    // One undo restores the original geometry and removes the new piece.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("the break is undoable");
    assert_eq!(entity_count(&app), before_count);
    assert_eq!(undo_depth(&app), 0);
    let restored = entity(&app, 1);
    match &restored.geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(near(start.x, 0.0) && near(end.x, 10.0));
        }
        other => panic!("expected the original LINE, got {other:?}"),
    }
}

#[test]
fn break_at_an_endpoint_keeps_exactly_one_piece() {
    let source = line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0);
    let (mut app, mut session) = app_with(vec![source]);
    let before_count = entity_count(&app);

    let outcome = break_at(&mut app, &mut session, 1, point(0.0, 0.0), point(3.0, 0.0))
        .expect("one pick at an endpoint leaves a single kept piece");
    assert_eq!(outcome.objects, vec![ObjectId(1)], "no second entity");
    assert_eq!(entity_count(&app), before_count);
    match &entity(&app, 1).geometry {
        SemanticGeometry::Line { start, end } => {
            assert!(
                near(start.x, 3.0) && near(end.x, 10.0),
                "the remaining [0.3, 1] piece is kept"
            );
        }
        other => panic!("expected one LINE, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn arc_break_leaves_two_arcs_with_complementary_sweeps() {
    let source = arc_entity(
        1,
        arc_geometry(point(0.0, 0.0), 5.0, 0.0, std::f64::consts::PI),
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    let original_sweep = std::f64::consts::PI;

    // Parameters 0.25 / 0.75 of a π sweep.
    let first = point(
        5.0 * (original_sweep * 0.25).cos(),
        5.0 * (original_sweep * 0.25).sin(),
    );
    let second = point(
        5.0 * (original_sweep * 0.75).cos(),
        5.0 * (original_sweep * 0.75).sin(),
    );
    let outcome = break_at(&mut app, &mut session, 1, first, second).expect("the arc breaks");
    assert_eq!(outcome.objects.len(), 2, "two ARC pieces");

    let first_arc = entity(&app, 1);
    let second_id = EntityId(outcome.objects[1].0);
    let second_arc = entity(&app, second_id.0);
    for piece in [&first_arc, &second_arc] {
        match &piece.geometry {
            SemanticGeometry::Arc { radius, .. } => assert!(near(*radius, 5.0)),
            other => panic!("expected an ARC, got {other:?}"),
        }
        assert_eq!(piece.object.type_key, "AcDbArc");
    }
    let (first_sweep, second_sweep) = match (&first_arc.geometry, &second_arc.geometry) {
        (
            SemanticGeometry::Arc { sweep: first, .. },
            SemanticGeometry::Arc { sweep: second, .. },
        ) => (*first, *second),
        _ => unreachable!(),
    };
    assert!(
        near(first_sweep, original_sweep * 0.25) && near(second_sweep, original_sweep * 0.25),
        "complementary quarter-pieces: {first_sweep} / {second_sweep}"
    );
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn circle_break_turns_the_source_into_one_arc_complement() {
    let source = circle_entity(1, circle_geometry(point(0.0, 0.0), 5.0), 4);
    let (mut app, mut session) = app_with(vec![source]);
    let before_count = entity_count(&app);

    // Remove the first quarter (angles 0 → π/2); the kept complement is 3π/2.
    let outcome = break_at(&mut app, &mut session, 1, point(5.0, 0.0), point(0.0, 5.0))
        .expect("the circle breaks");
    assert_eq!(
        outcome.objects,
        vec![ObjectId(1)],
        "the source stays the one object"
    );
    assert_eq!(entity_count(&app), before_count, "no extra entity");

    let after = entity(&app, 1);
    assert_eq!(after.object.type_key, "AcDbArc", "CIRCLE becomes an ARC");
    match &after.geometry {
        SemanticGeometry::Arc {
            radius,
            start,
            sweep,
            ..
        } => {
            assert!(near(*radius, 5.0));
            assert!(near(*start, std::f64::consts::FRAC_PI_2), "starts at u2");
            assert!(
                near(*sweep, TAU * 0.75),
                "keeps the 1 - removed complement, got {sweep}"
            );
        }
        other => panic!("expected the kept ARC complement, got {other:?}"),
    }
    assert_eq!(undo_depth(&app), 1);
}

#[test]
fn spline_break_leaves_two_splines_or_a_named_refusal_that_writes_nothing() {
    let fit = vec![
        point(0.0, 0.0),
        point(1.0, 2.0),
        point(2.5, 2.5),
        point(4.0, 1.0),
        point(5.5, -1.0),
    ];
    let (knots, control_points) =
        crate::draw_tool::spline_control_points(&fit).expect("a valid fit spline");
    let weights = vec![1.0; control_points.len()];
    let spline = SemanticGeometry::Spline {
        degree: cad_geometry::FIT_SPLINE_DEGREE,
        knots,
        control_points,
        weights,
    };
    let source = spline_entity(1, spline, 0);
    let (mut app, mut session) = app_with(vec![source]);

    match break_at(&mut app, &mut session, 1, point(1.0, 2.0), point(4.0, 1.0)) {
        Ok(outcome) => {
            assert_eq!(outcome.objects.len(), 2, "two SPLINE pieces");
            match &entity(&app, 1).geometry {
                SemanticGeometry::Spline { degree, .. } => assert_eq!(*degree, 3),
                other => panic!("expected first SPLINE piece, got {other:?}"),
            }
            let second = entity(&app, outcome.objects[1].0);
            match &second.geometry {
                SemanticGeometry::Spline { degree, .. } => assert_eq!(*degree, 3),
                other => panic!("expected second SPLINE piece, got {other:?}"),
            }
            assert_eq!(undo_depth(&app), 1);
        }
        Err(error) => {
            let message = refusal_message(error);
            assert!(
                message.starts_with("break.degenerate_piece"),
                "only the documented with_span refusal is allowed, got {message}"
            );
            assert_eq!(undo_depth(&app), 0, "a refused break writes nothing");
            assert_eq!(entity_count(&app), 1);
        }
    }
}

#[test]
fn zero_span_is_a_named_refusal_that_writes_nothing() {
    let source = line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0);
    let (mut app, mut session) = app_with(vec![source]);
    let before_revision = revision(&app);
    let error = break_at(&mut app, &mut session, 1, point(5.0, 0.0), point(5.0, 0.0))
        .expect_err("same point twice is a zero span");
    assert_invalid(error, "break.zero_span");
    assert_unchanged(&app, before_revision, 1);
}

#[test]
fn whole_curve_is_a_named_refusal_that_writes_nothing() {
    let source = line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0);
    let (mut app, mut session) = app_with(vec![source]);
    let before_revision = revision(&app);
    let error = break_at(&mut app, &mut session, 1, point(0.0, 0.0), point(10.0, 0.0))
        .expect_err("the picks span the whole curve");
    assert_invalid(error, "break.whole_curve");
    assert_unchanged(&app, before_revision, 1);
}

#[test]
fn a_pick_outside_the_target_span_is_a_named_refusal() {
    let source = line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0);
    let (mut app, mut session) = app_with(vec![source]);
    let before_revision = revision(&app);
    let error = break_at(&mut app, &mut session, 1, point(5.0, 0.0), point(20.0, 0.0))
        .expect_err("the second pick is beyond the line's own span");
    assert_invalid(error, "break.pick_outside_span");
    assert_unchanged(&app, before_revision, 1);
}

#[test]
fn a_polyline_target_is_the_shared_unsupported_refusal() {
    let source = part(
        1,
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(10.0, 0.0), point(10.0, 5.0)],
            bulges: vec![0.0; 3],
            closed: false,
        },
        LayerId(0),
        SpaceId::Model,
        "AcDbPolyline",
        0,
    );
    let (mut app, mut session) = app_with(vec![source]);
    let before_revision = revision(&app);
    let error = break_at(&mut app, &mut session, 1, point(2.0, 0.0), point(8.0, 0.0))
        .expect_err("LWPOLYLINE is not supported in v1");
    assert_unsupported(error, "LINE, ARC, CIRCLE or SPLINE");
    assert_unchanged(&app, before_revision, 1);
}

#[test]
fn a_block_member_or_sub_element_target_is_not_addressable() {
    let source = line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0);
    let (mut app, mut session) = app_with(vec![source]);
    let before_revision = revision(&app);

    for target in [
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath(vec![EntityId(1)]),
            sub_element: None,
        },
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: Some(SubElementId {
                source_key: "edge".into(),
                topology_revision: Revision(0),
            }),
        },
    ] {
        let error = execute(
            &mut app,
            &mut session,
            CommandId::Break,
            CommandPayload::Break {
                target,
                first: point(2.0, 0.0),
                second: point(8.0, 0.0),
            },
        )
        .expect_err("a block member / sub-element is not addressable");
        assert_invalid(error, "break.not_addressable");
    }
    assert_unchanged(&app, before_revision, 1);
}

#[test]
fn a_missing_target_is_a_named_refusal() {
    let source = line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0);
    let (mut app, mut session) = app_with(vec![source]);
    let before_revision = revision(&app);
    let error = break_at(&mut app, &mut session, 99, point(2.0, 0.0), point(8.0, 0.0))
        .expect_err("the target id does not exist");
    assert_invalid(error, "break.target_missing");
    assert_unchanged(&app, before_revision, 1);
}

#[test]
fn a_non_finite_break_point_is_a_named_refusal() {
    let source = line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0);
    let (mut app, mut session) = app_with(vec![source]);
    let before_revision = revision(&app);
    let error = break_at(
        &mut app,
        &mut session,
        1,
        Point3 {
            x: f64::NAN,
            y: 0.0,
            z: 0.0,
        },
        point(8.0, 0.0),
    )
    .expect_err("NaN is not a finite break point");
    assert_invalid(error, "break.invalid_point");
    assert_unchanged(&app, before_revision, 1);
}

#[test]
fn viewer_mode_refuses_break() {
    let source = line_entity(1, LayerId(0), line_geometry(0.0, 0.0, 10.0, 0.0), 0);
    let (mut app, mut session) = viewer_app_with(vec![source]);
    let before_revision = revision(&app);
    let error = break_at(&mut app, &mut session, 1, point(2.0, 0.0), point(8.0, 0.0))
        .expect_err("BREAK is Work-only");
    assert!(
        matches!(error, CadError::PermissionDenied),
        "expected PermissionDenied, got {error:?}"
    );
    assert_eq!(revision(&app), before_revision);
    assert_eq!(undo_depth(&app), 0);
}
