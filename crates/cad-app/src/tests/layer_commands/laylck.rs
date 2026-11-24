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
        before.1 + 1,
        "one undo step for the whole layer batch"
    );
    assert_eq!(diagnostic_codes(&outcome), vec!["laylck.applied"]);
    assert!(outcome.diagnostics[0].message.contains('2'));
    assert!(outcome.diagnostics[0].message.contains("one transaction"));

    // Undo unlocks both rows (exact attributes), redo locks them again.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYLCK");
    assert!(!layer_attributes(&app, LayerId(0)).locked);
    assert!(!layer_attributes(&app, LayerId(3)).locked);
    assert!(layer_attributes(&app, LayerId(3)).frozen_in_new_viewport);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYLCK");
    assert!(layer_attributes(&app, LayerId(0)).locked);
    assert!(layer_attributes(&app, LayerId(3)).locked);
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
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(diagnostic_codes(&outcome), vec!["layulk.applied"]);

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYULK");
    assert!(layer_attributes(&app, LayerId(3)).locked);
    assert!(layer_attributes(&app, LayerId(3)).frozen);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYULK");
    assert!(!layer_attributes(&app, LayerId(3)).locked);
    assert!(layer_attributes(&app, LayerId(3)).frozen);
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

/// The same real-machine case as `LAYFRZ`: locking only writes the layer row, so
/// a paper-space pick and a block member (direct or nested) are legal inputs.
#[test]
fn laylck_and_layulk_accept_paper_space_and_block_member_selections() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    // Paper-space member (DOORS) plus a nested block member (WALLS).
    select_refs(&mut app, &mut session, vec![ref_of(2), nested_ref(4, 5)]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayLck,
        CommandPayload::None,
    )
    .expect("a paper-space pick and a nested pick both name a layer");

    assert!(
        layer_attributes(&app, LayerId(5)).locked,
        "the paper-space member's layer"
    );
    assert!(
        layer_attributes(&app, LayerId(3)).locked,
        "the nested block member's layer"
    );
    assert!(
        !layer_attributes(&app, LayerId(0)).locked,
        "layer 0 is untouched"
    );
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 2),
        "one write per changed layer"
    );
    assert_eq!(undo_depth(&app), before.1 + 1, "one undo step");
    assert_eq!(diagnostic_codes(&outcome), vec!["laylck.applied"]);

    // LAYULK on the same shapes unlocks both in one step.
    execute(
        &mut app,
        &mut session,
        CommandId::LayUlk,
        CommandPayload::None,
    )
    .expect("the paper-space and nested picks unlock");
    assert!(!layer_attributes(&app, LayerId(5)).locked);
    assert!(!layer_attributes(&app, LayerId(3)).locked);

    // A member drawn on layer `0` inside the DOORS INSERT is displayed on DOORS,
    // so that is the layer the lock lands on.
    select_refs(&mut app, &mut session, vec![nested_ref(4, 6)]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayLck,
        CommandPayload::None,
    )
    .expect("a layer-0 block member follows its INSERT's layer");
    assert!(layer_attributes(&app, LayerId(5)).locked);
    assert!(!layer_attributes(&app, LayerId(0)).locked);

    // A direct block-definition member names the layer it is stored on.
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    select(&mut app, &mut session, &[5]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayLck,
        CommandPayload::None,
    )
    .expect("a block-definition member is a legal input");
    assert!(layer_attributes(&app, LayerId(3)).locked);
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

    // A nested pick whose `INSERT` chain does not resolve names no layer, so it
    // stays a named refusal — the walk never guesses a layer for it.
    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(999)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayUlk,
        CommandPayload::None,
    )
    .expect_err("an unresolvable block-instance path is refused");
    assert!(refusal(error).starts_with("layulk.not_addressable:"));
    assert_untouched(&app, before);

    // A sub-element is not a layer input in this build.
    session.selection = SelectionSet::from_refs([SelectionRef {
        sub_element: Some(SubElementId {
            source_key: "face:0".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(1)
    }]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayUlk,
        CommandPayload::None,
    )
    .expect_err("a sub-element is refused");
    assert!(refusal(error).starts_with("layulk.not_addressable:"));
    assert_untouched(&app, before);
}
