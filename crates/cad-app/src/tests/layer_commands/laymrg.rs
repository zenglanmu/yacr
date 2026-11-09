//! `LAYMRG` (AutoCAD 2026): merge the selection's layers into the active layer.
//!
//! Declared deviation: sources come from the current selection and the target is
//! the session's active layer (no interactive source/target prompts). Every
//! entity on a source layer is rewritten to the active layer and the source
//! layer rows are deleted in one transaction / one undo step; undo restores the
//! rows and the entities' original layers. Refusals: empty selection, a payload,
//! a source equal to the active layer, and the default layer `0`.

use super::*;

fn layers() -> Vec<(LayerId, &'static str, LayerAttributes)> {
    vec![
        (LayerId(0), "0", LayerAttributes::default()),
        (LayerId(3), "WALLS", LayerAttributes::default()),
        (LayerId(5), "DOORS", LayerAttributes::default()),
    ]
}

#[test]
fn laymrg_moves_every_entity_from_the_selected_layers_into_the_active_layer() {
    let (mut app, mut session) = app_with(
        layers(),
        vec![
            line(1, LayerId(3), 0.0),
            line(2, LayerId(3), 5.0),
            line(3, LayerId(5), 10.0),
        ],
    );
    session.active_layer = LayerId(5);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect("LAYMRG merges the selected layer");

    assert_eq!(entity_layer(&app, 1), LayerId(5));
    assert_eq!(
        entity_layer(&app, 2),
        LayerId(5),
        "the whole source layer moves"
    );
    assert_eq!(entity_layer(&app, 3), LayerId(5));
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert!(
        drawing.layer(LayerId(3)).is_none(),
        "the source row is deleted"
    );
    assert!(drawing.layer(LayerId(5)).is_some());
    assert_eq!(
        revision(&app),
        Revision(before.0 .0 + 2),
        "one entity transaction plus one layer deletion"
    );
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "one undo step covers both the entities and the source layer row"
    );
    assert!(outcome.changes.is_some());
    assert_eq!(diagnostic_codes(&outcome), vec!["laymrg.applied"]);

    // Undo restores the source layer row and puts its entities back on it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo LAYMRG");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(revision(&app), Revision(before.0 .0 + 3));
    assert_eq!(
        drawing.layer(LayerId(3)).map(|row| row.name.as_str()),
        Some("WALLS")
    );
    assert_eq!(entity_layer(&app, 1), LayerId(3));
    assert_eq!(entity_layer(&app, 2), LayerId(3));
    assert_eq!(entity_layer(&app, 3), LayerId(5));

    // Redo re-applies the merge and removes the source row again.
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo LAYMRG");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(revision(&app), Revision(before.0 .0 + 4));
    assert!(drawing.layer(LayerId(3)).is_none());
    assert_eq!(entity_layer(&app, 1), LayerId(5));
    assert_eq!(entity_layer(&app, 2), LayerId(5));
    assert_eq!(entity_layer(&app, 3), LayerId(5));
}

#[test]
fn laymrg_refuses_a_source_that_is_the_active_layer() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    session.active_layer = LayerId(3);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect_err("merging a layer into itself is refused");
    assert!(refusal(error).starts_with("laymrg.source_is_active:"));
    assert_untouched(&app, before);
}

#[test]
fn laymrg_refuses_the_default_layer_as_a_source() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(0), 0.0)]);
    session.active_layer = LayerId(5);
    select(&mut app, &mut session, &[1]);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect_err("layer 0 cannot be merged away");
    assert!(refusal(error).starts_with("laymrg.default_layer:"));
    assert_untouched(&app, before);
}

#[test]
fn laymrg_refuses_an_empty_selection_a_payload_and_instance_refs() {
    let (mut app, mut session) = app_with(
        layers(),
        vec![line(1, LayerId(3), 0.0), line(2, LayerId(5), 5.0)],
    );
    session.active_layer = LayerId(5);
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect_err("empty selection is a refusal");
    assert!(refusal(error).starts_with("laymrg.selection:"));
    assert_untouched(&app, before);

    select(&mut app, &mut session, &[1]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::LayerVisibilities(vec![(LayerId(0), false)]),
    )
    .expect_err("a payload is refused");
    assert!(refusal(error).starts_with("laymrg.payload:"));
    assert_untouched(&app, before);

    let instance = SelectionRef {
        instance: InstancePath(vec![EntityId(1)]),
        ..ref_of(1)
    };
    session.selection = SelectionSet::from_refs([instance]);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayMrg,
        CommandPayload::None,
    )
    .expect_err("a block-instance member is refused");
    assert!(refusal(error).starts_with("laymrg.not_addressable:"));
    assert_untouched(&app, before);
}
