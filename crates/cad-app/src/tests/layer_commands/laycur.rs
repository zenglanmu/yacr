use super::*;

// ---------------------------------------------------------------------------
// LAYCUR
// ---------------------------------------------------------------------------

#[test]
fn laycur_moves_every_selected_entity_to_the_current_layer() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(3), 5.0)],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect("LAYCUR moves the selection");

    assert_eq!(entity_layer(&app, 1), LayerId(0));
    assert_eq!(entity_layer(&app, 2), LayerId(0));
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 1),
        "one revision bump"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(outcome.objects.len(), 2);
    assert!(outcome.changes.is_some());
    assert_eq!(
        diagnostic_codes(&outcome),
        vec!["laycur.applied"],
        "the success diagnostic names the layer and count"
    );
    assert!(outcome.diagnostics[0].message.contains("'0'"));
    assert!(outcome.diagnostics[0].message.contains('2'));
}

#[test]
fn laycur_is_a_noop_when_every_entity_is_already_on_the_current_layer() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(0), 0.0), line(2, LayerId(0), 5.0)],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect("LAYCUR reports the no-op");

    assert_eq!(state(&app), before, "no transaction, no undo entry");
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["laycur.unchanged"]);
}

#[test]
fn laycur_refuses_an_empty_selection_without_touching_anything() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("laycur.selection:"));
    assert_untouched(&app, before);
}

#[test]
fn laycur_refuses_a_missing_current_layer() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    session.active_layer = LayerId(99);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("a missing active layer is a refusal");
    assert!(refusal(error).starts_with("laycur.active_layer_missing:"));
    assert_untouched(&app, before);
}

#[test]
fn laycur_refuses_moving_entities_onto_a_locked_layer() {
    let locked = LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", locked),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("the database refuses a write on a locked layer");
    assert!(refusal(error).contains("layer.locked"));
    assert_eq!(entity_layer(&app, 1), LayerId(3), "nothing moved");
    assert_untouched(&app, before);
}

#[test]
fn laycur_refuses_foreign_instance_and_non_model_refs() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(0), 0.0)],
    );
    session.active_layer = LayerId(3);

    let foreign = SelectionRef {
        document: DocumentId(2),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([foreign]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("another document's ref is refused");
    assert!(refusal(error).starts_with("laycur.not_addressable:"));
    assert_untouched(&app, before);

    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("laycur.not_addressable:"));
    assert_untouched(&app, before);

    let paper = DbEntity {
        space: SpaceId::Paper(LayoutId(1)),
        ..line(1, LayerId(0), 0.0)
    };
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new({
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
        builder
            .insert_layer(Layer {
                id: LayerId(3),
                name: "WALLS".into(),
                visible: true,
            })
            .unwrap();
        builder
            .insert_layout(Layout {
                id: LayoutId(1),
                name: "Sheet1".into(),
                viewports: Vec::new(),
            })
            .unwrap();
        builder.insert_entity(paper).unwrap();
        builder.finish().unwrap()
    });
    session.selection = SelectionSet::from_refs([ref_of(1)]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("a paper-space entity is refused");
    assert!(refusal(error).starts_with("laycur.not_addressable:"));
    assert_untouched(&app, before);
}
