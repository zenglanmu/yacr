use super::*;

// ---------------------------------------------------------------------------
// LAYFRZ
// ---------------------------------------------------------------------------

#[test]
fn layfrz_freezes_every_distinct_selected_layer_and_preserves_other_attributes() {
    let walls = LayerAttributes {
        locked: true,
        frozen_in_new_viewport: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", walls),
            (LayerId(4), "DETAIL", LayerAttributes::default()),
        ],
        vec![
            line(1, LayerId(3), 0.0),
            line(2, LayerId(4), 5.0),
            line(3, LayerId(3), 10.0),
        ],
    );
    select(&mut app, &mut session, &[1, 2, 3]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("LAYFRZ freezes the selection's layers");

    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert!(
        layer_attributes(&app, LayerId(4)).frozen,
        "one write per distinct layer"
    );
    assert!(
        layer_attributes(&app, LayerId(3)).locked,
        "the other flags are preserved"
    );
    assert!(
        layer_attributes(&app, LayerId(3)).frozen_in_new_viewport,
        "the all-viewports freeze does not touch the new-viewport flag"
    );
    assert!(!layer_attributes(&app, LayerId(0)).frozen);
    // One `set_layer_attributes` per changed layer: two revision bumps, but the
    // whole command is one undo step.
    assert_eq!(revision(&app), Revision(before.0 .0 + 2));
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "one undo step for the whole layer batch"
    );
    assert!(outcome.changes.is_some());
    assert_eq!(diagnostic_codes(&outcome), vec!["layfrz.applied"]);
    assert!(outcome.diagnostics[0].message.contains('2'));
    assert!(outcome.diagnostics[0].message.contains("one transaction"));

    // Undo restores the exact before attributes (frozen back to false, other
    // flags untouched); redo freezes again.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYFRZ");
    assert_eq!(revision(&app), Revision(before.0 .0 + 3));
    assert!(!layer_attributes(&app, LayerId(3)).frozen);
    assert!(!layer_attributes(&app, LayerId(4)).frozen);
    assert!(layer_attributes(&app, LayerId(3)).locked);
    assert!(layer_attributes(&app, LayerId(3)).frozen_in_new_viewport);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYFRZ");
    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert!(layer_attributes(&app, LayerId(4)).frozen);
}

#[test]
fn layfrz_refuses_the_whole_command_when_the_current_layer_is_selected() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
            (LayerId(4), "DETAIL", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(4), 5.0)],
    );
    session.active_layer = LayerId(3);
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect_err("the current layer cannot be frozen");
    assert!(refusal(error).starts_with("layfrz.current_layer:"));
    assert!(
        !layer_attributes(&app, LayerId(4)).frozen,
        "no partial freeze"
    );
    assert_untouched(&app, before);
}

#[test]
fn layfrz_reports_unchanged_when_every_layer_is_already_frozen() {
    let frozen = LayerAttributes {
        frozen: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", frozen),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect("an already-frozen selection is a no-op");
    assert_eq!(state(&app), before);
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["layfrz.unchanged"]);
}

#[test]
fn layfrz_refuses_empty_and_non_addressable_selections() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("layfrz.selection:"));
    assert_untouched(&app, before);

    let sub = SelectionRef {
        sub_element: Some(SubElementId {
            source_key: "edge".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([sub]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayFrz,
        CommandPayload::None,
    )
    .expect_err("a sub-element is refused");
    assert!(refusal(error).starts_with("layfrz.not_addressable:"));
    assert_untouched(&app, before);
}
