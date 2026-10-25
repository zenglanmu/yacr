use super::*;

#[test]
fn create_arc_through_three_points_sets_the_circumcircle() {
    let mut app = single_line_app(None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Points(vec![point(0.0, 0.0), point(5.0, 5.0), point(10.0, 0.0)]),
    )
    .unwrap();
    let arc_id = EntityId(outcome.objects[0].0);
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(arc_id)
        .unwrap()
        .geometry
    {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => {
            assert!((center.x - 5.0).abs() < 1e-9, "{center:?}");
            assert!((center.y - 0.0).abs() < 1e-9, "{center:?}");
            assert!((radius - 5.0).abs() < 1e-9, "{radius}");
            assert!((start - std::f64::consts::PI).abs() < 1e-9, "{start}");
            assert!((sweep + std::f64::consts::PI).abs() < 1e-9, "{sweep}");
        }
        other => panic!("expected an arc, got {other:?}"),
    }
}

#[test]
fn create_arc_refuses_collinear_points_without_change() {
    let mut app = single_line_app(None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .len();
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0), point(2.0, 0.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .model_space()
            .len(),
        before,
        "no entity may be created for a degenerate arc"
    );
}

#[test]
fn create_arc_is_one_undo_step() {
    let mut app = single_line_app(None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Points(vec![point(0.0, 0.0), point(5.0, 5.0), point(10.0, 0.0)]),
    )
    .unwrap();
    let arc_id = EntityId(outcome.objects[0].0);
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(arc_id)
        .is_some());
    assert!(app.can_undo(&DocumentId(1)));
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(arc_id)
            .is_none(),
        "one undo removes the arc"
    );
}

#[test]
fn extend_line_reaches_the_nearest_boundary_beyond_the_pick_end() {
    let boundary = ent(
        50,
        SemanticGeometry::Line {
            start: point(20.0, -5.0),
            end: point(20.0, 5.0),
        },
        1,
    );
    let mut app = line_and_boundary_app(boundary, None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(50)],
            pick_point: point(9.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(outcome.diagnostics[0].code, "drawing.extend");
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x - 0.0).abs() < 1e-9, "{start:?}");
            assert!((end.x - 20.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }

    // One undo restores the original finite line.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x - 0.0).abs() < 1e-9, "{start:?}");
            assert!((end.x - 10.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn extend_undo_restores_the_original_line_and_attributes() {
    let attributes = non_default_attributes();
    let boundary = ent(
        53,
        SemanticGeometry::Line {
            start: point(20.0, -5.0),
            end: point(20.0, 5.0),
        },
        1,
    );
    let mut app = line_and_boundary_app(boundary, Some(attributes.clone()));
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(53)],
            pick_point: point(9.0, 0.0),
        },
    )
    .unwrap();
    // The update path preserves the target's display attributes.
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(EntityId(1)),
        attributes
    );
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(EntityId(1)),
        attributes,
        "undo must not lose the render attributes"
    );
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x - 0.0).abs() < 1e-9, "{start:?}");
            assert!((end.x - 10.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn extend_refuses_a_target_already_touching_the_boundary() {
    let boundary = ent(
        51,
        SemanticGeometry::Line {
            start: point(10.0, -5.0),
            end: point(10.0, 5.0),
        },
        1,
    );
    let mut app = line_and_boundary_app(boundary, None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(51)],
            pick_point: point(9.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}

#[test]
fn extend_refuses_a_pick_whose_projection_is_outside_the_arc_span() {
    // ARC targets are supported since the curved-cut widening. The pick is
    // *projected* onto the target (so a wrong radius is fine), but the projection
    // must land on the target's own span: the seed arc spans angles 0..1 rad, so
    // (0, 3) — angle PI/2 on the same circle — projects outside it and is refused
    // by name instead of being clamped to the nearest end.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(2),
            boundary: vec![ref_of(1)],
            pick_point: point(0.0, 3.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert!(
        err.to_string().contains("projects outside the target arc"),
        "{err:?}"
    );
    assert_eq!(revision(&app), rev);
}

#[test]
fn extend_refuses_an_unsupported_target_and_a_circle_target() {
    // INSERT stays unsupported; a CIRCLE has no endpoint to grow.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(3),
            boundary: vec![ref_of(1)],
            pick_point: point(5.0, 5.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    assert!(err.to_string().contains("AcDbBlockReference"), "{err:?}");
    assert_eq!(revision(&app), rev);

    let (mut app, mut session) = drawing_app();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        2,
        SemanticGeometry::Line {
            start: point(20.0, -5.0),
            end: point(20.0, 5.0),
        },
        1,
    ))
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(2)],
            pick_point: point(5.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    assert!(err.to_string().contains("has no endpoint"), "{err:?}");
    assert_eq!(revision(&app), rev);
}

#[test]
fn extend_refuses_when_the_boundary_never_meets_the_target_line() {
    // A boundary parallel to the target line: their infinite lines never cross.
    let boundary = ent(
        52,
        SemanticGeometry::Line {
            start: point(0.0, 5.0),
            end: point(10.0, 5.0),
        },
        1,
    );
    let mut app = line_and_boundary_app(boundary, None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(52)],
            pick_point: point(9.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert!(err
        .to_string()
        .to_ascii_lowercase()
        .contains("intersection"));
    assert_eq!(revision(&app), rev);
}

#[test]
fn extend_grows_the_start_end_when_the_pick_is_near_the_start() {
    // Boundary left of the start: the picked start endpoint grows to it.
    let boundary = ent(
        54,
        SemanticGeometry::Line {
            start: point(-10.0, -5.0),
            end: point(-10.0, 5.0),
        },
        1,
    );
    let mut app = line_and_boundary_app(boundary, None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(54)],
            // Near the start (t_pick = 0.1) so the start end grows.
            pick_point: point(1.0, 0.0),
        },
    )
    .unwrap();
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x + 10.0).abs() < 1e-9, "{start:?}");
            assert!((end.x - 10.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn extend_chooses_the_nearest_of_several_crossings() {
    // Two vertical boundaries beyond the end: the nearer (x=15) wins.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    ))
    .unwrap();
    for (id, x) in [(60u128, 15.0f64), (61, 20.0)] {
        b.insert_entity(ent(
            id,
            SemanticGeometry::Line {
                start: point(x, -5.0),
                end: point(x, 5.0),
            },
            1,
        ))
        .unwrap();
    }
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([5u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(60), ref_of(61)],
            pick_point: point(9.0, 0.0),
        },
    )
    .unwrap();
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x - 0.0).abs() < 1e-9, "{start:?}");
            assert!((end.x - 15.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn extend_refuses_a_target_already_crossing_the_boundary() {
    // The boundary passes through the middle of the target (t = 0.5): the
    // target already crosses it, so it is not extendable.
    let boundary = ent(
        55,
        SemanticGeometry::Line {
            start: point(5.0, -5.0),
            end: point(5.0, 5.0),
        },
        1,
    );
    let mut app = line_and_boundary_app(boundary, None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(55)],
            pick_point: point(9.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}

#[test]
fn extend_accepts_a_straight_lwpolyline_boundary() {
    // A straight LWPOLYLINE with two segments: the vertical one at x=20
    // extends the target's end.
    let boundary = ent(
        62,
        SemanticGeometry::Polyline {
            points: vec![point(20.0, -5.0), point(20.0, 5.0), point(25.0, 5.0)],
            bulges: vec![0.0, 0.0],
            closed: false,
        },
        1,
    );
    let mut app = line_and_boundary_app(boundary, None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(62)],
            pick_point: point(9.0, 0.0),
        },
    )
    .unwrap();
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!((start.x - 0.0).abs() < 1e-9, "{start:?}");
            assert!((end.x - 20.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn extend_refuses_an_unsupported_boundary_geometry() {
    // A CIRCLE boundary, a bulged LWPOLYLINE boundary and (since the sampled-cut
    // widening) a SPLINE boundary are supported — see `trim_extend_curves.rs` and
    // `trim_spline_boundary.rs`. A POINT is still an explicit `Unsupported`
    // refusal that changes nothing.
    let point_boundary = ent(65, SemanticGeometry::Point(point(20.0, 0.0)), 1);
    let mut app = line_and_boundary_app(point_boundary, None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let before = format!(
        "{:?}",
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .unwrap()
    );
    let err = execute(
        &mut app,
        &mut session,
        CommandId::ExtendEntity,
        CommandPayload::Extend {
            target: ref_of(1),
            boundary: vec![ref_of(65)],
            pick_point: point(9.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    // The refusal names the offending kind and the accepted set.
    assert!(err.to_string().contains("AcDbPoint"), "{err:?}");
    assert!(err.to_string().contains("SPLINE"), "{err:?}");
    assert_eq!(revision(&app), rev);
    assert_eq!(
        before,
        format!(
            "{:?}",
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(EntityId(1))
                .unwrap()
        ),
        "a refused EXTEND must not touch the target"
    );
}

#[test]
fn create_arc_accepts_a_geometry_arc_payload() {
    let mut app = single_line_app(None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Geometry(Box::new(SemanticGeometry::Arc {
            center: point(1.0, 2.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 3.0,
            start: 0.5,
            sweep: 1.25,
        })),
    )
    .unwrap();
    let arc_id = EntityId(outcome.objects[0].0);
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(arc_id)
        .unwrap()
        .geometry
    {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => {
            assert!((center.x - 1.0).abs() < 1e-12);
            assert!((center.y - 2.0).abs() < 1e-12);
            assert!((radius - 3.0).abs() < 1e-12);
            assert!((start - 0.5).abs() < 1e-12);
            assert!((sweep - 1.25).abs() < 1e-12);
        }
        other => panic!("expected an arc, got {other:?}"),
    }
}

#[test]
fn create_arc_supports_a_positive_sweep_through_the_middle_point() {
    // (0,0) -> (5,-5) -> (10,0) is the lower semicircle; traversed
    // counter-clockwise (increasing angle), so the sweep is positive.
    let mut app = single_line_app(None);
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateArc,
        CommandPayload::Points(vec![point(0.0, 0.0), point(5.0, -5.0), point(10.0, 0.0)]),
    )
    .unwrap();
    let arc_id = EntityId(outcome.objects[0].0);
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(arc_id)
        .unwrap()
        .geometry
    {
        SemanticGeometry::Arc {
            center,
            radius,
            sweep,
            ..
        } => {
            assert!((center.x - 5.0).abs() < 1e-9, "{center:?}");
            assert!((center.y - 0.0).abs() < 1e-9, "{center:?}");
            assert!((radius - 5.0).abs() < 1e-9, "{radius}");
            assert!((sweep - std::f64::consts::PI).abs() < 1e-9, "{sweep}");
        }
        other => panic!("expected an arc, got {other:?}"),
    }
}
