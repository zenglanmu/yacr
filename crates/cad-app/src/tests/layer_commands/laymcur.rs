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
fn laymcur_refuses_non_addressable_refs() {
    let (mut app, mut session) = app_with(
        vec![(LayerId(0), "0", LayerAttributes::default())],
        vec![line(1, LayerId(0), 0.0)],
    );
    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMCur,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("laymcur.not_addressable:"));
    assert_untouched(&app, before);
}
