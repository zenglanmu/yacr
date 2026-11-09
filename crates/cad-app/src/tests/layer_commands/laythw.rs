use super::*;

// ---------------------------------------------------------------------------
// LAYTHW
// ---------------------------------------------------------------------------

#[test]
fn laythw_thaws_every_frozen_layer_and_leaves_the_viewport_flag_alone() {
    let frozen = LayerAttributes {
        frozen: true,
        ..LayerAttributes::default()
    };
    let frozen_new_viewport = LayerAttributes {
        frozen: true,
        frozen_in_new_viewport: true,
        ..LayerAttributes::default()
    };
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", frozen.clone()),
            (LayerId(3), "WALLS", frozen_new_viewport),
            (LayerId(4), "DETAIL", LayerAttributes::default()),
        ],
        vec![line(1, LayerId(3), 0.0)],
    );
    // LAYTHW takes no selection: an existing selection is ignored, never refused.
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::None,
    )
    .expect("LAYTHW thaws all frozen layers");

    assert!(
        !layer_attributes(&app, LayerId(0)).frozen,
        "the current layer is thawed too"
    );
    assert!(!layer_attributes(&app, LayerId(3)).frozen);
    assert!(!layer_attributes(&app, LayerId(4)).frozen);
    assert!(
        layer_attributes(&app, LayerId(3)).frozen_in_new_viewport,
        "the new-viewport flag is not touched"
    );
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 2),
        "one bump per changed layer"
    );
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "one undo step for the whole thaw batch"
    );
    assert_eq!(diagnostic_codes(&outcome), vec!["laythw.applied"]);
    assert!(outcome.diagnostics[0].message.contains("viewport-level"));
    assert!(outcome.diagnostics[0].message.contains("one transaction"));

    // Undo re-freezes the two changed rows exactly; redo thaws them again.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYTHW");
    assert!(layer_attributes(&app, LayerId(0)).frozen);
    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert!(layer_attributes(&app, LayerId(3)).frozen_in_new_viewport);
    assert!(!layer_attributes(&app, LayerId(4)).frozen);
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYTHW");
    assert!(!layer_attributes(&app, LayerId(0)).frozen);
    assert!(!layer_attributes(&app, LayerId(3)).frozen);
}

#[test]
fn laythw_refuses_a_payload_and_changes_nothing() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (
                LayerId(3),
                "WALLS",
                LayerAttributes {
                    frozen: true,
                    ..LayerAttributes::default()
                },
            ),
        ],
        vec![line(1, LayerId(0), 0.0)],
    );
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::Points(vec![point(0.0, 0.0)]),
    )
    .expect_err("LAYTHW takes no payload");
    assert!(refusal(error).starts_with("laythw.payload:"));
    assert!(layer_attributes(&app, LayerId(3)).frozen);
    assert_untouched(&app, before);
}

#[test]
fn laythw_reports_unchanged_when_nothing_is_frozen() {
    let (mut app, mut session) = app_with(
        vec![
            (LayerId(0), "0", LayerAttributes::default()),
            (LayerId(3), "WALLS", LayerAttributes::default()),
        ],
        Vec::new(),
    );
    let before = state(&app);
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayThw,
        CommandPayload::None,
    )
    .expect("nothing to thaw is a no-op");
    assert_eq!(state(&app), before);
    assert!(outcome.changes.is_none());
    assert_eq!(diagnostic_codes(&outcome), vec!["laythw.unchanged"]);
}
