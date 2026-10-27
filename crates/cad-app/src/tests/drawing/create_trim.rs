use super::*;

#[test]
fn create_line_inserts_one_entity_and_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 4.0)]),
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 1);
    let oid = outcome.objects[0];
    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.before, before);
    assert_eq!(cs.after.0, before.0 + 1);
    assert_eq!(cs.changes.len(), 1);
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
        .expect("new entity");
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(0.0, 0.0));
            assert_eq!(*end, point(3.0, 4.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(entity.layer, LayerId(0));
    assert_eq!(entity.space, SpaceId::Model);
    assert_eq!(entity.draw_order, 4, "next draw order after the seed set");
    assert!(app.can_undo(&DocumentId(1)));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // Undo removes the entity exactly, redo restores it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
        .is_none());
    assert!(app.can_redo(&DocumentId(1)));
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
        .is_some());
}

#[test]
fn create_circle_uses_edge_distance_and_rejects_zero_radius() {
    let (mut app, mut session) = drawing_app();
    let before = revision(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle,
        CommandPayload::Points(vec![point(0.0, 0.0), point(3.0, 4.0)]),
    )
    .unwrap();
    let oid = outcome.objects[0];
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(oid.0))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Circle { radius, .. } => assert!((radius - 5.0).abs() < 1e-12),
        other => panic!("expected a circle, got {other:?}"),
    }
    assert_eq!(revision(&app).0, before.0 + 1);

    // A zero radius is an explicit error and changes nothing.
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle,
        CommandPayload::Points(vec![point(1.0, 1.0), point(1.0, 1.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev, "revision unchanged");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
}

#[test]
fn create_on_missing_layer_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    execute(
        &mut app,
        &mut session,
        CommandId::SetActiveLayer,
        CommandPayload::ActiveLayer(LayerId(99)),
    )
    .unwrap();
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0)]),
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn move_lines_updates_geometry_and_is_undoable_in_one_step() {
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1)],
            delta: p3(2.0, 3.0, 0.0),
        },
    )
    .unwrap();
    let cs = outcome.changes.expect("change set");
    assert_eq!(cs.changes.len(), 1, "one transaction for the selection");
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap();
    match &entity.geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(2.0, 3.0));
            assert_eq!(*end, point(12.0, 3.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

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
            assert_eq!(*start, point(0.0, 0.0));
            assert_eq!(*end, point(10.0, 0.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn move_insert_composes_its_transform() {
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(3)],
            delta: p3(1.0, 2.0, 0.0),
        },
    )
    .unwrap();
    let entity = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(3))
        .unwrap();
    match &entity.geometry {
        SemanticGeometry::Insert { transform, .. } => {
            // Original translation (5,5,0) composed with (1,2,0) = (6,7,0).
            let moved = transform.apply_point(p3(0.0, 0.0, 0.0));
            assert!((moved.x - 6.0).abs() < 1e-9, "{moved:?}");
            assert!((moved.y - 7.0).abs() < 1e-9, "{moved:?}");
        }
        other => panic!("expected an insert, got {other:?}"),
    }
}

#[test]
fn move_multiple_entities_is_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: vec![ref_of(1), ref_of(2)],
            delta: p3(1.0, 0.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_some());
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(2))
        .is_some());
}

#[test]
fn trim_line_against_line_keeps_the_pick_side() {
    let (mut app, mut session) = drawing_app();
    // Target line entity 1: (0,0)->(10,0). Boundary: a vertical line at x=4.
    let boundary = ent(
        50,
        SemanticGeometry::Line {
            start: point(4.0, -1.0),
            end: point(4.0, 1.0),
        },
        5,
    );
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for e in app.workspace.documents[&DocumentId(1)].drawing.entities() {
        b.insert_entity(e.clone()).unwrap();
    }
    b.insert_entity(boundary).unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());

    // Pick on the right half: keep (4,0)->(10,0).
    execute(
        &mut app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(1),
            boundary: vec![ref_of(50)],
            pick_point: point(8.0, 0.0),
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
            assert!((start.x - 4.0).abs() < 1e-6, "{start:?}");
            assert!((end.x - 10.0).abs() < 1e-6, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn trim_deletes_a_line_fully_inside_a_closed_boundary() {
    let (mut app, mut session) = drawing_app();
    // A closed square boundary from (0,0) to (10,10), target is a small
    // line entirely inside it.
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
            start: point(2.0, 2.0),
            end: point(4.0, 2.0),
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(
        60,
        SemanticGeometry::Polyline {
            points: vec![
                point(0.0, 0.0),
                point(10.0, 0.0),
                point(10.0, 10.0),
                point(0.0, 10.0),
            ],
            bulges: vec![0.0, 0.0, 0.0, 0.0],
            closed: true,
        },
        1,
    ))
    .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());

    execute(
        &mut app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(1),
            boundary: vec![ref_of(60)],
            pick_point: point(3.0, 2.0),
        },
    )
    .unwrap();
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(1))
            .is_none(),
        "fully contained line is removed"
    );

    // Undo restores the deleted line.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .is_some());
}

#[test]
fn trim_arc_with_no_interior_cut_is_refused_and_changes_nothing() {
    // ARC targets are a supported TRIM target since the curved-cut widening. The
    // seed arc (centre (0,0), r 5, start 0, sweep 1 rad) is met by the seed line
    // only at its own start point (5,0), which is an endpoint rather than an
    // interior cut, so this still refuses — now as a plan-level `InvalidInput`,
    // not as an unsupported target type.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let depth = app
        .history
        .get(&DocumentId(1))
        .map(|h| h.undo_depth())
        .unwrap_or(0);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(2),
            boundary: vec![ref_of(1)],
            pick_point: point(1.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)), "{err:?}");
    assert_eq!(revision(&app), rev);
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        depth
    );
}

#[test]
fn trim_refuses_an_unsupported_target_geometry() {
    // INSERT / SPLINE / Opaque targets stay an explicit `Unsupported` refusal.
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let before = format!(
        "{:?}",
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity(EntityId(3))
            .unwrap()
    );
    let err = execute(
        &mut app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(3),
            boundary: vec![ref_of(1)],
            pick_point: point(5.0, 5.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "{err:?}");
    assert!(err.to_string().contains("AcDbBlockReference"), "{err:?}");
    assert_eq!(revision(&app), rev);
    assert_eq!(
        before,
        format!(
            "{:?}",
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity(EntityId(3))
                .unwrap()
        ),
        "an unsupported target must not be touched"
    );
}

#[test]
fn trim_with_no_intersection_is_reported_and_changes_nothing() {
    let (mut app, mut session) = drawing_app();
    // A parallel boundary far away: no cut, no containment.
    let boundary = ent(
        70,
        SemanticGeometry::Line {
            start: point(0.0, 5.0),
            end: point(10.0, 5.0),
        },
        5,
    );
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for e in app.workspace.documents[&DocumentId(1)].drawing.entities() {
        b.insert_entity(e.clone()).unwrap();
    }
    b.insert_entity(boundary).unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(b.finish().unwrap());
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::TrimEntity,
        CommandPayload::Trim {
            target: ref_of(1),
            boundary: vec![ref_of(70)],
            pick_point: point(5.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}

#[test]
fn viewer_mode_rejects_all_drawing_commands_without_change() {
    let (mut app, mut session) = drawing_app();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    let rev = revision(&app);
    let cases = [
        (
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        ),
        (
            CommandId::CreateCircle,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        ),
        (
            CommandId::CreateArc,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0), point(2.0, 0.0)]),
        ),
        (
            CommandId::MoveEntities,
            CommandPayload::Move {
                refs: vec![ref_of(1)],
                delta: p3(1.0, 0.0, 0.0),
            },
        ),
        (
            CommandId::TrimEntity,
            CommandPayload::Trim {
                target: ref_of(1),
                boundary: vec![ref_of(2)],
                pick_point: point(1.0, 0.0),
            },
        ),
        (
            CommandId::ExtendEntity,
            CommandPayload::Extend {
                target: ref_of(1),
                boundary: vec![ref_of(2)],
                pick_point: point(9.0, 0.0),
            },
        ),
        (CommandId::EraseEntities, CommandPayload::None),
        (
            CommandId::CopyEntities,
            CommandPayload::Copy {
                refs: vec![ref_of(1)],
                delta: p3(1.0, 0.0, 0.0),
            },
        ),
        (
            CommandId::RotateEntities,
            CommandPayload::Rotate {
                refs: vec![ref_of(1)],
                base: point(0.0, 0.0),
                angle: std::f64::consts::FRAC_PI_2,
            },
        ),
        (
            CommandId::ScaleEntities,
            CommandPayload::Scale {
                refs: vec![ref_of(1)],
                base: point(0.0, 0.0),
                factor: 2.0,
            },
        ),
        (
            CommandId::MirrorEntities,
            CommandPayload::Mirror {
                refs: vec![ref_of(1)],
                axis_start: point(0.0, 0.0),
                axis_end: point(1.0, 1.0),
            },
        ),
        (CommandId::CutToClipboard, CommandPayload::None),
        (
            CommandId::PasteFromClipboard,
            CommandPayload::PasteClip {
                insertion: point(0.0, 0.0),
            },
        ),
        (CommandId::PasteOriginalCoordinates, CommandPayload::None),
        (
            CommandId::CutToClipboardBase,
            CommandPayload::CutBase {
                refs: vec![ref_of(1)],
                base: point(0.0, 0.0),
            },
        ),
        (
            CommandId::OffsetEntity,
            CommandPayload::Offset {
                refs: vec![ref_of(1)],
                side_point: point(0.0, 1.0),
            },
        ),
    ];
    for (id, payload) in cases {
        let err = execute(&mut app, &mut viewer, id, payload).unwrap_err();
        assert_eq!(err, CadError::PermissionDenied, "{id:?}");
    }
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
    // Silence the unused-mut warning on the shared session.
    let _ = &mut session;
}

#[test]
fn drawing_undo_redo_round_trips() {
    let (mut app, mut session) = drawing_app();
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    // One drawing create, then a second, then undo both in reverse order.
    execute(
        &mut app,
        &mut session,
        CommandId::CreateLine,
        CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 1.0)]),
    )
    .unwrap();
    let oid = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .iter()
        .map(|e| e.id)
        .max()
        .unwrap();
    execute(
        &mut app,
        &mut session,
        CommandId::CreateCircle,
        CommandPayload::Points(vec![point(5.0, 5.0), point(6.0, 5.0)]),
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 2
    );

    // Undo the circle first, then the line create.
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
            .entity_count(),
        before + 1
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
        .entity(oid)
        .is_none());
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before
    );

    // Redo restores them in order.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(oid)
        .is_some());
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 2
    );
}

#[test]
fn drawing_commands_are_work_only_in_the_catalogue() {
    for id in [
        CommandId::CreateLine,
        CommandId::CreateCircle,
        CommandId::CreateArc,
        CommandId::MoveEntities,
        CommandId::TrimEntity,
        CommandId::ExtendEntity,
        CommandId::EraseEntities,
        CommandId::Join,
        CommandId::CopyEntities,
        CommandId::RotateEntities,
        CommandId::ScaleEntities,
        CommandId::MirrorEntities,
        CommandId::CutToClipboard,
        CommandId::PasteFromClipboard,
        CommandId::CutToClipboardBase,
        CommandId::PasteOriginalCoordinates,
        CommandId::OffsetEntity,
        CommandId::SetActiveLayer,
    ] {
        assert!(id.requires_work_mode(), "{id:?} must be Work-only");
    }
}

#[test]
fn two_creates_are_two_transactions_not_one_or_zero() {
    let (mut app, mut session) = drawing_app();
    for _ in 0..2 {
        execute(
            &mut app,
            &mut session,
            CommandId::CreateLine,
            CommandPayload::Points(vec![point(0.0, 0.0), point(1.0, 0.0)]),
        )
        .unwrap();
    }
    // Each command is exactly one transaction and one undo step: never a
    // merged pair (one step) and never a silent empty success (zero steps).
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 2);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .model_space()
            .len(),
        6,
        "the five seed model entities plus two created lines"
    );
}

#[test]
fn empty_move_selection_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::MoveEntities,
        CommandPayload::Move {
            refs: Vec::new(),
            delta: p3(1.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn erase_selection_deletes_exactly_those_entities_in_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1), ref_of(2)]),
    )
    .unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap();
    assert_eq!(outcome.changes.as_ref().unwrap().changes.len(), 2);
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_none());
    assert!(drawing.entity(EntityId(2)).is_none());
    assert!(
        drawing.entity(EntityId(3)).is_some(),
        "an unselected entity must stay"
    );
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // One undo restores both erased entities.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(drawing.entity(EntityId(1)).is_some());
    assert!(drawing.entity(EntityId(2)).is_some());
}

#[test]
fn erase_empty_selection_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::EraseEntities,
        CommandPayload::None,
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn copy_selection_duplicates_with_translation_in_one_undo_step() {
    let (mut app, mut session) = drawing_app();
    let before_count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .model_space()
        .len();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::CopyEntities,
        CommandPayload::Copy {
            refs: vec![ref_of(1), ref_of(2)],
            delta: p3(5.0, 1.0, 0.0),
        },
    )
    .unwrap();
    assert_eq!(outcome.objects.len(), 2, "one copy per selected entity");

    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.model_space().len(), before_count + 2);
    // The originals are untouched.
    match &drawing.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(0.0, 0.0));
            assert_eq!(*end, point(10.0, 0.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
    // The copies are new ids with translated geometry, same layer/space.
    let copy_line = drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("copy 1");
    match &copy_line.geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(5.0, 1.0));
            assert_eq!(*end, point(15.0, 1.0));
        }
        other => panic!("expected a line, got {other:?}"),
    }
    assert_eq!(copy_line.layer, LayerId(0));
    assert_eq!(copy_line.space, SpaceId::Model);
    let copy_arc = drawing
        .entity(EntityId(outcome.objects[1].0))
        .expect("copy 2");
    match &copy_arc.geometry {
        SemanticGeometry::Arc { center, .. } => assert_eq!(*center, point(5.0, 1.0)),
        other => panic!("expected an arc, got {other:?}"),
    }
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);

    // One undo removes every copy and keeps the originals.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(drawing.model_space().len(), before_count);
    assert!(drawing.entity(EntityId(outcome.objects[0].0)).is_none());
    assert!(drawing.entity(EntityId(outcome.objects[1].0)).is_none());
    assert!(drawing.entity(EntityId(1)).is_some());
}

#[test]
fn copy_empty_selection_is_refused_without_change() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CopyEntities,
        CommandPayload::Copy {
            refs: Vec::new(),
            delta: p3(1.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn copy_in_viewer_mode_is_refused() {
    let (mut app, _work) = drawing_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Viewer);
    execute(
        &mut app,
        &mut session,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CopyEntities,
        CommandPayload::Copy {
            refs: vec![ref_of(1)],
            delta: p3(1.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::PermissionDenied));
    assert_eq!(revision(&app), rev);
}

#[test]
fn copy_untransformable_entity_is_refused_without_change() {
    // An Opaque entity cannot be translated; the whole copy must refuse
    // rather than silently dropping it.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        1,
        SemanticGeometry::Opaque {
            type_key: "ACAD_PROXY_ENTITY".into(),
            version: 1,
            payload: Vec::new(),
        },
        0,
    ))
    .unwrap();
    let drawing = b.finish().unwrap();
    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([2u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let rev = revision(&app);
    let err = execute(
        &mut app,
        &mut session,
        CommandId::CopyEntities,
        CommandPayload::Copy {
            refs: vec![ref_of(1)],
            delta: p3(1.0, 0.0, 0.0),
        },
    )
    .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)));
    assert_eq!(revision(&app), rev);
    assert!(!app.can_undo(&DocumentId(1)));
}
