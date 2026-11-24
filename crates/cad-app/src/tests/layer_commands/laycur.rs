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
fn laycur_refuses_foreign_and_nested_refs() {
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
}

#[test]
fn laycur_moves_paper_space_entities_and_top_level_inserts() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    // Entities 2 (paper space) and 4 (a top-level INSERT) are both ordinary
    // top-level rows; only their own `layer` may move.
    session.active_layer = LayerId(3);
    select(&mut app, &mut session, &[2, 4]);
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect("LAYCUR moves paper-space and INSERT objects");

    assert_eq!(entity_layer(&app, 2), LayerId(3), "the paper entity moved");
    assert_eq!(entity_layer(&app, 4), LayerId(3), "the INSERT itself moved");
    assert_eq!(
        entity_layer(&app, 6),
        LayerId(0),
        "the block member is untouched"
    );
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 1),
        "one revision bump"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(outcome.objects.len(), 2);
    assert_eq!(diagnostic_codes(&outcome), vec!["laycur.applied"]);
}

#[test]
fn laycur_refuses_a_block_definition_member_by_name() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    session.active_layer = LayerId(3);
    session.selection = SelectionSet::from_refs([ref_of(5)]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayCur,
        CommandPayload::None,
    )
    .expect_err("a block-definition member has no top-level identity");
    assert!(refusal(error).starts_with("laycur.block_space:"));
    assert_untouched(&app, before);
    assert_eq!(entity_layer(&app, 5), LayerId(3), "nothing was rewritten");
}
