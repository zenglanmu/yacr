use super::*;

#[test]
fn offset_line_creates_a_parallel_line_on_the_picked_side() {
    let (mut app, mut session) = drawing_app();
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    // Source line (0,0)->(10,0); pick above => parallel line at y = 3.
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
    assert_eq!(outcome.diagnostics[0].code, "drawing.offset");
    let created = EntityId(outcome.objects[0].0);
    assert_ne!(created, EntityId(1), "fresh id");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 1
    );
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(created)
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!(
                start.x.abs() < 1e-9 && (start.y - 3.0).abs() < 1e-9,
                "{start:?}"
            );
            assert!(
                (end.x - 10.0).abs() < 1e-9 && (end.y - 3.0).abs() < 1e-9,
                "{end:?}"
            );
        }
        other => panic!("expected a line, got {other:?}"),
    }
    // The source is untouched.
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(1))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { end, .. } => {
            assert!((end.x - 10.0).abs() < 1e-9 && end.y.abs() < 1e-9)
        }
        other => panic!("expected a line, got {other:?}"),
    }
    // Exactly one undo step was recorded.
    assert_eq!(
        app.history
            .get(&DocumentId(1))
            .map(|h| h.undo_depth())
            .unwrap_or(0),
        1
    );

    // One undo removes the offset.
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
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before
    );
    // Redo restores the same offset geometry.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(created)
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { end, .. } => assert!((end.y - 3.0).abs() < 1e-9),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn offset_line_on_the_opposite_side() {
    let (mut app, mut session) = drawing_app();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(5.0, -3.0),
        },
    )
    .unwrap();
    let created = EntityId(outcome.objects[0].0);
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(created)
        .unwrap()
        .geometry
    {
        SemanticGeometry::Line { start, end } => {
            assert!((start.y + 3.0).abs() < 1e-9, "{start:?}");
            assert!((end.y + 3.0).abs() < 1e-9, "{end:?}");
        }
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn offset_line_with_a_point_on_the_line_is_refused() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
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
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}

#[test]
fn offset_requires_exactly_one_selection() {
    let (mut app, mut session) = drawing_app();
    let rev = revision(&app);
    for refs in [Vec::new(), vec![ref_of(1), ref_of(2)]] {
        let err = execute(
            &mut app,
            &mut session,
            CommandId::OffsetEntity,
            CommandPayload::Offset {
                refs,
                side_point: point(5.0, 3.0),
            },
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)));
        assert_eq!(revision(&app), rev);
    }
}

#[test]
fn offset_is_refused_in_viewer() {
    let (mut app, _) = drawing_app();
    let mut viewer = SessionState::new(DocumentId(1), AppMode::Viewer);
    execute(
        &mut app,
        &mut viewer,
        CommandId::Select,
        CommandPayload::Selection(vec![ref_of(1)]),
    )
    .unwrap();
    let err = execute(
        &mut app,
        &mut viewer,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(5.0, 3.0),
        },
    )
    .unwrap_err();
    assert_eq!(err, CadError::PermissionDenied);
}

#[test]
fn offset_circle_grows_outward_and_shrinks_inward() {
    let circle_app = || {
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
            },
        );
        app.workspace.viewports.insert(
            ViewportId(1),
            Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
        );
        app
    };

    // Outside the circle grows the radius to |point - centre| = 8.
    let mut app = circle_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(0.0, 8.0),
        },
    )
    .unwrap();
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Circle { radius, .. } => assert!((radius - 8.0).abs() < 1e-9),
        other => panic!("expected a circle, got {other:?}"),
    }

    // Inside shrinks it to |point - centre| = 2.
    let mut app = circle_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![ref_of(1)],
            side_point: point(0.0, 2.0),
        },
    )
    .unwrap();
    match &app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .unwrap()
        .geometry
    {
        SemanticGeometry::Circle { radius, .. } => assert!((radius - 2.0).abs() < 1e-9),
        other => panic!("expected a circle, got {other:?}"),
    }

    // A point at the centre, or on the circle (a zero offset), is refused.
    let mut app = circle_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    for side in [point(0.0, 0.0), point(0.0, 5.0)] {
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::OffsetEntity,
            CommandPayload::Offset {
                refs: vec![ref_of(1)],
                side_point: side,
            },
        )
        .unwrap_err();
        assert!(matches!(err, CadError::InvalidInput(_)), "{side:?}");
        assert_eq!(revision(&app), rev);
    }
}

#[test]
fn offset_refuses_unsupported_geometry() {
    let (mut app, mut session) = drawing_app();
    // Entity 2 is an ARC, entity 3 an INSERT: both are outside the subset.
    let count = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();
    for source in [ref_of(2), ref_of(3)] {
        let rev = revision(&app);
        let err = execute(
            &mut app,
            &mut session,
            CommandId::OffsetEntity,
            CommandPayload::Offset {
                refs: vec![source.clone()],
                side_point: point(5.0, 3.0),
            },
        )
        .unwrap_err();
        assert!(matches!(err, CadError::Unsupported(_)), "{source:?}");
        assert_eq!(revision(&app), rev);
        assert_eq!(
            app.workspace.documents[&DocumentId(1)]
                .drawing
                .entity_count(),
            count,
            "nothing may be created for an unsupported source"
        );
    }
}

#[test]
fn offset_carries_the_source_render_attributes() {
    let attributes = non_default_attributes();
    let mut app = single_line_app(Some(attributes.clone()));
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
    let created = EntityId(outcome.objects[0].0);
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_render_attributes(created),
        attributes,
        "the offset copy carries the source render attributes"
    );
}

/// Wrap a hand-built drawing in a one-document application.
fn app_with_drawing(drawing: DrawingDatabase) -> Application {
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
    app
}

/// A drawing whose source line (id 1) lives on layer 1 and in block space 0,
/// so a layer/space carry bug is observable.
fn layered_block_source_app() -> Application {
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
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    );
    source.layer = LayerId(1);
    source.space = SpaceId::Block(cad_domain::BlockId(0));
    b.insert_entity(source).unwrap();
    app_with_drawing(b.finish().unwrap())
}

#[test]
fn offset_copies_the_source_layer_and_space() {
    let mut app = layered_block_source_app();
    let mut session = SessionState::new(DocumentId(1), AppMode::Work);
    // The session's active layer differs from the source's layer, and the
    // source is in block space, not the default model space: copying
    // `session.active_layer` / `SpaceId::Model` would fail these assertions.
    assert_eq!(session.active_layer, LayerId(0));
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
    let created = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .unwrap();
    assert_eq!(created.layer, LayerId(1));
    assert_eq!(created.space, SpaceId::Block(cad_domain::BlockId(0)));
}

#[test]
fn offset_display_state_survives_undo_redo() {
    let attributes = non_default_attributes();
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // The source is a governed member of a dynamic-visibility state, so its
    // captured display state includes visibility membership.
    b.insert_block(BlockDefinition {
        id: cad_domain::BlockId(0),
        entities: vec![EntityId(1)],
        dynamic_visibility: Some(DynamicBlockVisibility {
            member_entities: vec![EntityId(1)],
            states: vec![DynamicBlockState {
                name: "Visible".into(),
                entities: vec![EntityId(1)],
            }],
            active_state: Some("Visible".into()),
        }),
    })
    .unwrap();
    let mut source = ent(
        1,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(10.0, 0.0),
        },
        0,
    );
    source.space = SpaceId::Block(cad_domain::BlockId(0));
    b.insert_entity(source).unwrap();
    b.set_entity_render_attributes(EntityId(1), attributes.clone())
        .unwrap();

    let mut app = app_with_drawing(b.finish().unwrap());
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
    let copy_id = EntityId(outcome.objects[0].0);
    let membership = {
        let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
        assert_eq!(
            drawing.entity_render_attributes(copy_id),
            attributes,
            "the offset copy carries the source attributes"
        );
        let membership = drawing.entity_visibility_membership(copy_id);
        assert!(
            !membership.visibility.is_empty(),
            "the offset copy joins the dynamic-visibility state"
        );
        membership
    };

    // One undo prunes the copy's render attributes and its membership.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .unwrap();
    {
        let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
        assert!(drawing.entity(copy_id).is_none());
        assert_eq!(
            drawing.entity_render_attributes(copy_id),
            cad_db::EntityRenderAttributes::default()
        );
        assert_eq!(
            drawing.entity_visibility_membership(copy_id),
            cad_db::VisibilityMembership::default()
        );
    }

    // Redo re-applies both the attributes and the membership.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .unwrap();
    {
        let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
        assert_eq!(
            drawing.entity_render_attributes(copy_id),
            attributes,
            "redo restores the render attributes"
        );
        assert_eq!(
            drawing.entity_visibility_membership(copy_id),
            membership,
            "redo restores the dynamic-visibility membership"
        );
    }
}

#[test]
fn offset_reports_sub_element_and_instance_refs() {
    let (mut app, mut session) = drawing_app();
    let mut sub = ref_of(1);
    sub.sub_element = Some(cad_domain::SubElementId {
        source_key: "edge:0".into(),
        topology_revision: Revision(0),
    });
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![sub],
            side_point: point(5.0, 3.0),
        },
    )
    .unwrap();
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "drawing.offset.sub_element"),
        "{:?}",
        outcome.diagnostics
    );

    let mut inst = ref_of(1);
    inst.instance = cad_domain::InstancePath(vec![EntityId(3)]);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::OffsetEntity,
        CommandPayload::Offset {
            refs: vec![inst],
            side_point: point(5.0, 3.0),
        },
    )
    .unwrap();
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "drawing.offset.instance"),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn offset_refuses_a_degenerate_line_source() {
    // The builder does not geometry-validate, so a zero-length line can be
    // the source; OFFSET must refuse it explicitly rather than offset by 0.
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
            end: point(2.0, 2.0),
        },
        0,
    ))
    .unwrap();
    let mut app = app_with_drawing(b.finish().unwrap());
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
    assert!(matches!(err, CadError::InvalidInput(_)));
    assert_eq!(revision(&app), rev);
}
