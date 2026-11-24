//! `LAYERSTATE` (AutoCAD 2026): bounded named layer states.
//!
//! A snapshot records per-layer attributes, the active layer id and the session
//! visibility overrides. `SAVE`/`LIST`/`DELETE` are session-only; `RESTORE` writes
//! the layer table and records **one** undo step with every changed row (undo
//! restores the attributes; the restored active layer/overrides are session state
//! and are not undone). Refusals: an unknown name, a bare selector, and a snapshot
//! naming a layer that no longer exists.

use super::*;
use cad_db::DrawingDatabaseBuilder;

fn layers() -> Vec<(LayerId, &'static str, LayerAttributes)> {
    vec![
        (LayerId(0), "0", LayerAttributes::default()),
        (LayerId(3), "WALLS", LayerAttributes::default()),
        (LayerId(5), "DOORS", LayerAttributes::default()),
    ]
}

fn save(
    app: &mut Application,
    session: &mut SessionState,
    name: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::LayerState,
        CommandPayload::LayerState(LayerStateCommand::Save(name.into())),
    )
}

fn restore(
    app: &mut Application,
    session: &mut SessionState,
    name: &str,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::LayerState,
        CommandPayload::LayerState(LayerStateCommand::Restore(name.into())),
    )
}

#[test]
fn layerstate_save_and_restore_round_trip_attributes_active_layer_and_overrides() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    session.active_layer = LayerId(3);
    session.layer_overrides.set(LayerId(5), false);

    let outcome = save(&mut app, &mut session, "Plan A").expect("SAVE");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerstate.saved"]);
    assert_eq!(session.layer_states.len(), 1);
    assert!(
        session.layer_states.resolve("plan a").is_some(),
        "case-insensitive"
    );

    // Change the drawing and the session after the snapshot.
    session.active_layer = LayerId(0);
    session.layer_overrides.clear();
    execute(
        &mut app,
        &mut session,
        CommandId::SetLayerProperty,
        CommandPayload::SetLayerProperty {
            layer: "WALLS".into(),
            key: "locked".into(),
            value: "true".into(),
        },
    )
    .expect("write a property after the save");
    assert!(layer_attributes(&app, LayerId(3)).locked);

    let before = state(&app);
    let outcome = restore(&mut app, &mut session, "Plan A").expect("RESTORE");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerstate.restored"]);
    assert!(
        !layer_attributes(&app, LayerId(3)).locked,
        "the saved attribute set is restored"
    );
    assert_eq!(session.active_layer, LayerId(3));
    assert_eq!(session.layer_overrides.get(LayerId(5)), Some(false));
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "RESTORE records one undo step for every changed row"
    );

    // Undo restores the saved attributes; the restored active layer/override
    // choice is session state and stays as RESTORE left it.
    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("undo the RESTORE");
    assert!(
        layer_attributes(&app, LayerId(3)).locked,
        "undo restores the pre-restore locked attribute"
    );
    assert_eq!(
        session.active_layer,
        LayerId(3),
        "the session active layer is not part of the layer-table undo step"
    );
    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("redo the RESTORE");
    assert!(!layer_attributes(&app, LayerId(3)).locked);
}

#[test]
fn layerstate_refuses_an_unknown_name_and_a_bare_selector() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    let before = state(&app);
    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayerState,
        CommandPayload::None,
    )
    .expect_err("bare LAYERSTATE is refused");
    assert!(refusal(error).starts_with("layerstate.needs_option:"));
    assert_untouched(&app, before);

    let error = restore(&mut app, &mut session, "missing").expect_err("unknown name");
    assert!(refusal(error).starts_with("layerstate.unknown_name:"));
    assert_untouched(&app, before);

    let error = execute(
        &mut app,
        &mut session,
        CommandId::LayerState,
        CommandPayload::LayerState(LayerStateCommand::Delete("missing".into())),
    )
    .expect_err("unknown delete");
    assert!(refusal(error).starts_with("layerstate.unknown_name:"));
    assert_untouched(&app, before);
}

#[test]
fn layerstate_refuses_restoring_a_snapshot_whose_layer_is_gone() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    save(&mut app, &mut session, "S1").expect("SAVE");

    // Replace the drawing with one that lost layer 3.
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(builder.finish().unwrap());

    let before = state(&app);
    let error = restore(&mut app, &mut session, "S1").expect_err("dangling layer");
    assert!(refusal(error).starts_with("layerstate.dangling_layer:"));
    assert_untouched(&app, before);
}

#[test]
fn layerstate_restore_writes_every_changed_row_as_one_undo_step() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    save(&mut app, &mut session, "Plan A").expect("SAVE");

    for (layer, key, value) in [("WALLS", "locked", "true"), ("DOORS", "color", "aci:4")] {
        execute(
            &mut app,
            &mut session,
            CommandId::SetLayerProperty,
            CommandPayload::SetLayerProperty {
                layer: layer.into(),
                key: key.into(),
                value: value.into(),
            },
        )
        .expect("write an attribute after the save");
    }
    let before = state(&app);
    restore(&mut app, &mut session, "Plan A").expect("RESTORE");
    assert!(!layer_attributes(&app, LayerId(3)).locked);
    assert_eq!(
        layer_attributes(&app, LayerId(5)).color,
        LayerAttributes::default().color
    );
    assert_eq!(
        undo_depth(&app),
        before.1 + 1,
        "two changed rows are still one undo step"
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Undo,
        CommandPayload::None,
    )
    .expect("one undo");
    assert!(layer_attributes(&app, LayerId(3)).locked);
    assert_ne!(
        layer_attributes(&app, LayerId(5)).color,
        LayerAttributes::default().color
    );

    execute(
        &mut app,
        &mut session,
        CommandId::Redo,
        CommandPayload::None,
    )
    .expect("one redo");
    assert!(!layer_attributes(&app, LayerId(3)).locked);
    assert_eq!(
        layer_attributes(&app, LayerId(5)).color,
        LayerAttributes::default().color
    );
}

#[test]
fn layerstate_list_delete_and_the_bounded_cap() {
    let (mut app, mut session) = app_with(layers(), Vec::new());
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayerState,
        CommandPayload::LayerState(LayerStateCommand::List),
    )
    .expect("LIST");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerstate.list_empty"]);

    save(&mut app, &mut session, "A").unwrap();
    save(&mut app, &mut session, "B").unwrap();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayerState,
        CommandPayload::LayerState(LayerStateCommand::List),
    )
    .expect("LIST");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerstate.list"]);
    assert!(outcome.diagnostics[0].message.contains("A, B"));

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::LayerState,
        CommandPayload::LayerState(LayerStateCommand::Delete("A".into())),
    )
    .expect("DELETE");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerstate.deleted"]);
    assert!(session.layer_states.resolve("A").is_none());
    assert!(session.layer_states.resolve("B").is_some());

    // The store drops the oldest state at the documented cap.
    let mut states = LayerStates::new();
    assert_eq!(states.cap(), LAYER_STATE_CAP);
    for index in 0..(LAYER_STATE_CAP + 1) {
        states.save(
            &format!("STATE{index}"),
            std::collections::BTreeMap::new(),
            LayerId(0),
            crate::layers::LayerOverrideSet::new(),
        );
    }
    assert_eq!(states.len(), LAYER_STATE_CAP);
    assert!(
        states.resolve("STATE0").is_none(),
        "the oldest state is dropped"
    );
    assert!(states.resolve(&format!("STATE{LAYER_STATE_CAP}")).is_some());
}

// ---------------------------------------------------------------------------
// `LAYERSTATESAVE`: the dialog spelling's typed form (2026-11-24)
// ---------------------------------------------------------------------------

fn layerstatesave(
    app: &mut Application,
    session: &mut SessionState,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    execute(
        app,
        session,
        CommandId::Extended(crate::ExtendedCommandId::LayerStateSave),
        payload,
    )
}

#[test]
fn layerstatesave_saves_and_lists_through_the_one_layerstate_store() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    session.active_layer = LayerId(3);
    session.layer_overrides.set(LayerId(5), false);
    let before = state(&app);

    let outcome = layerstatesave(
        &mut app,
        &mut session,
        CommandPayload::LayerState(LayerStateCommand::Save("Plan A".into())),
    )
    .expect("LAYERSTATESAVE <name> saves");
    // Exactly `LAYERSTATE SAVE`'s report and store: one implementation.
    assert_eq!(diagnostic_codes(&outcome), vec!["layerstate.saved"]);
    assert!(outcome.changes.is_none(), "session state only");
    let saved = session
        .layer_states
        .resolve("plan a")
        .expect("the name is stored case-insensitively");
    assert_eq!(saved.active_layer, LayerId(3));
    assert_eq!(saved.attributes.len(), 3);
    assert_eq!(state(&app), before, "no drawing write, no revision bump");
    assert_eq!(undo_depth(&app), 0);

    let outcome = layerstatesave(
        &mut app,
        &mut session,
        CommandPayload::LayerState(LayerStateCommand::List),
    )
    .expect("LAYERSTATESAVE ? lists");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerstate.list"]);
    assert!(outcome.diagnostics[0].message.contains("Plan A"));
    assert_eq!(state(&app), before);

    // A duplicate name replaces the same snapshot (the store's rule), it is not
    // a refusal and it does not grow the store.
    let outcome = layerstatesave(
        &mut app,
        &mut session,
        CommandPayload::LayerState(LayerStateCommand::Save("plan a".into())),
    )
    .expect("a duplicate name replaces");
    assert_eq!(diagnostic_codes(&outcome), vec!["layerstate.saved"]);
    assert!(outcome.diagnostics[0].message.contains("replaced"));
    assert_eq!(session.layer_states.len(), 1);
    assert_eq!(state(&app), before);
    assert_eq!(undo_depth(&app), 0);
}

#[test]
fn layerstatesave_refuses_restore_delete_and_empty_payloads_without_writing() {
    let (mut app, mut session) = app_with(layers(), vec![line(1, LayerId(3), 0.0)]);
    layerstatesave(
        &mut app,
        &mut session,
        CommandPayload::LayerState(LayerStateCommand::Save("Keep".into())),
    )
    .expect("seed one state");
    let before = state(&app);
    let undo = undo_depth(&app);

    for (payload, expected) in [
        (
            LayerStateCommand::Restore("Keep".into()),
            "layerstatesave.option:",
        ),
        (
            LayerStateCommand::Delete("Keep".into()),
            "layerstatesave.option:",
        ),
    ] {
        let error = layerstatesave(&mut app, &mut session, CommandPayload::LayerState(payload))
            .expect_err("RESTORE/DELETE are LAYERSTATE's own options");
        assert!(refusal(error).starts_with(expected));
        assert_eq!(state(&app), before, "nothing was written");
        assert_eq!(undo_depth(&app), undo, "no undo step");
        assert_eq!(session.layer_states.len(), 1, "the store is untouched");
    }

    // The bare selector keeps the named missing-dialog report and writes nothing.
    let outcome = layerstatesave(&mut app, &mut session, CommandPayload::None)
        .expect("the bare form is the named report");
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "layerstatesave.needs_name"));
    assert_eq!(state(&app), before);
    assert_eq!(undo_depth(&app), undo);
    assert_eq!(session.layer_states.len(), 1);
}
