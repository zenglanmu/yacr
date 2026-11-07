use super::*;

// ---------------------------------------------------------------------------
// LAYLCK / LAYULK
// ---------------------------------------------------------------------------

#[test]
fn laylck_locks_every_distinct_selected_layer_including_the_current_one() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (
                LayerId(3),
                "WALLS",
                LayerAttributes {
                    frozen_in_new_viewport: true,
                    ..LayerAttributes::default()
                },
            ),
        ],
        vec![line(1, LayerId(0), 0.0), line(2, LayerId(3), 5.0)],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayLck,
        CommandPayload::None,
    )
    .expect("LAYLCK locks the selection's layers");

    assert!(
        layer_attributes(&app, LayerId(0)).locked,
        "the current layer may be locked"
    );
    assert!(layer_attributes(&app, LayerId(3)).locked);
    assert!(
        layer_attributes(&app, LayerId(3)).frozen_in_new_viewport,
        "the other attributes are preserved"
    );
    assert_eq!(revision(&app), Revision(before.0 .0 + 2));
    assert_eq!(
        undo_depth(&app),
        before.1,
        "layer-table edits are not undoable"
    );
    assert_eq!(diagnostic_codes(&outcome), vec!["laylck.applied"]);
    assert!(outcome.diagnostics[0].message.contains('2'));
    assert!(outcome.diagnostics[0].message.contains("not undoable"));
}

#[test]
fn laylck_reports_unchanged_when_every_layer_is_already_locked() {
    let locked = LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", locked),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayLck,
        CommandPayload::None,
    )
    .expect("an all-locked selection is a no-op");
    assert_eq!(state(&app), before);
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["laylck.unchanged"]);
}

#[test]
fn layulk_unlocks_the_selected_layers_and_preserves_other_flags() {
    let locked = LayerAttributes {
        locked: true,
        frozen: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", locked),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayUlk,
        CommandPayload::None,
    )
    .expect("LAYULK unlocks the selection's layer");

    assert!(!layer_attributes(&app, LayerId(3)).locked);
    assert!(
        layer_attributes(&app, LayerId(3)).frozen,
        "unlocking does not thaw"
    );
    assert_eq!(revision(&app), Revision(before.0 .0 + 1));
    assert_eq!(undo_depth(&app), before.1);
    assert_eq!(diagnostic_codes(&outcome), vec!["layulk.applied"]);
}

#[test]
fn layulk_reports_unchanged_when_every_layer_is_already_unlocked() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayUlk,
        CommandPayload::None,
    )
    .expect("an all-unlocked selection is a no-op");
    assert_eq!(state(&app), before);
    assert_eq!(diagnostic_codes(&outcome), vec!["layulk.unchanged"]);
}

#[test]
fn laylck_refuses_empty_and_non_addressable_selections() {
    {
        let id = CommandId::LayLck;
        let (mut app, mut session) = app_with(
            vec![(LayerId(0), "0", LayerAttributes::default())],
            vec![line(1, LayerId(0), 0.0)],
        );
        let before = state(&app);
        let error = execute(&mut app, &mut session, id, CommandPayload::None)
            .expect_err("empty selection is a refusal");
        assert!(refusal(error).starts_with("laylck.selection:"));
        assert_untouched(&app, before);

        let foreign = SelectionRef {
            document: DocumentId(2),
            ..ref_of(1)
        };
        session.selection = SelectionSet::from_refs([foreign]);
        let error = execute(&mut app, &mut session, id, CommandPayload::None)
            .expect_err("another document's ref is refused");
        assert!(refusal(error).starts_with("laylck.not_addressable:"));
        assert_untouched(&app, before);
    }
}

#[test]
fn layulk_refuses_empty_and_non_addressable_selections() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayUlk,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("layulk.selection:"));
    assert_untouched(&app, before);

    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayUlk,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("layulk.not_addressable:"));
    assert_untouched(&app, before);
}
