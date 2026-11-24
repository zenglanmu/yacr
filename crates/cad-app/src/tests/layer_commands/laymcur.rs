use super::*;

// ---------------------------------------------------------------------------
// LAYM C U R
// ---------------------------------------------------------------------------

#[test]
fn laymcur_makes_the_first_selected_objects_layer_current() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(0), 5.0)],
    );
    select(&mut app, &mut session, &[1, 2]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("LAYMCUR reads the first pick");

    assert_eq!(session.active_layer, LayerId(3));
    assert_eq!(state(&app), before, "session-only: no revision, no undo");
    assert!(outcome.changes.is_none());
    assert_eq!(
        diagnostic_codes(&outcome),
        vec!["laymcur.applied", "laymcur.first_pick"]
    );
    assert!(outcome.diagnostics[0].message.contains("WALLS"));

    // The pick order is the contract: reversing the selection changes the answer.
    select(&mut app, &mut session, &[2, 1]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("LAYMCUR reads the new first pick");
    assert_eq!(session.active_layer, LayerId(0));
}

#[test]
fn laymcur_refuses_an_empty_selection() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("laymcur.selection:"));
    assert_untouched(&app, before);
    assert_eq!(session.active_layer, LayerId(0));
}

#[test]
fn laymcur_refuses_a_locked_frozen_or_off_first_layer() {
    for (attributes, code) in [
        (
            LayerAttributes {
                locked: true,
                ..LayerAttributes::default()
            },
            "layer.current_locked",
        ),
        (
            LayerAttributes {
                frozen: true,
                ..LayerAttributes::default()
            },
            "layer.current_frozen",
        ),
        (
            LayerAttributes {
                off: true,
                ..LayerAttributes::default()
            },
            "layer.current_off",
        ),
    ] {
        let (mut app, mut session) = app_with(
            vec![
                (LayerId(0), "0", LayerAttributes::default()),
                (LayerId(3), "WALLS", attributes),
            ],
            vec![line(1, LayerId(3), 0.0)],
        );
        select(&mut app, &mut session, &[1]);
        let before = state(&app);
        let error = execute(
            &mut app,
            &mut session,
            CommandId::LayMCur,
            CommandPayload::None,
        )
        .expect_err("an unusable layer cannot become current");
        assert!(
            refusal(error).starts_with(code),
            "expected {code}, got something else"
        );
        assert_eq!(session.active_layer, LayerId(0), "the session is untouched");
        assert_untouched(&app, before);
    }
}

#[test]
fn laymcur_accepts_paper_space_top_level_inserts_and_nested_picks() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);

    // A paper-space object names its stored layer.
    select(&mut app, &mut session, &[2]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("LAYMCUR reads a paper-space object");
    assert_eq!(session.active_layer, LayerId(5));

    // A top-level INSERT names its own layer.
    select(&mut app, &mut session, &[4]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("LAYMCUR reads a top-level INSERT");
    assert_eq!(session.active_layer, LayerId(5));

    // A nested pick names the member's effective layer: a member on WALLS keeps
    // WALLS, and a member on layer 0 resolves to the INSERT's layer (DOORS).
    select_refs(&mut app, &mut session, vec![nested_ref(4, 5)]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("LAYMCUR reads a nested pick");
    assert_eq!(session.active_layer, LayerId(3));

    select_refs(&mut app, &mut session, vec![nested_ref(4, 6)]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("a layer-0 member resolves to the INSERT layer");
    assert_eq!(session.active_layer, LayerId(5));

    // A block-definition member addressed directly names its stored layer.
    session.selection = SelectionSet::from_refs([ref_of(5)]);
    execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect("LAYMCUR reads a block-definition member");
    assert_eq!(session.active_layer, LayerId(3));
}

#[test]
fn laymcur_refuses_sub_elements_and_unresolvable_paths() {
    let (mut app, mut session) = multi_space_app([Vec::new(), Vec::new()]);
    let before = state(&app);

    let sub_element = SelectionRef {
        sub_element: Some(SubElementId {
            source_key: "face".into(),
            topology_revision: Revision(0),
        }),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([sub_element]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect_err("a sub-element has no whole-object layer");
    assert!(refusal(error).starts_with("laymcur.not_addressable:"));
    assert_eq!(session.active_layer, LayerId(0), "the session is untouched");
    assert_untouched(&app, before);

    session.selection = SelectionSet::from_refs([nested_ref(999, 5)]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect_err("an unresolvable instance path is refused");
    assert!(refusal(error).starts_with("laymcur.not_addressable:"));
    assert_eq!(session.active_layer, LayerId(0));
    assert_untouched(&app, before);
}
