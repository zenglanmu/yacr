//! The layer-panel read projection and the three layer mutations.
//!
//! These contracts exercise the **real** session funnel (`CadSession::layers` /
//! `set_layer_visibility` / `restore_layers` / `set_active_layer` →
//! `ToggleLayer` / `RestoreLayers` / `SetActiveLayer`) against the demo drawing,
//! so the projected rows are evidence that the panel reads the drawing's own
//! layer table plus the session override set — never a second visibility state
//! machine and never a database write.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

fn row<'a>(layers: &'a CadLayers, name: &str) -> &'a CadLayer {
    layers
        .layers
        .iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no '{name}' row in {:?}", layers.layers))
}

#[test]
fn layers_project_the_drawing_rows_with_exactly_one_current() {
    let session = fresh_session();
    let layers = session.layers().expect("the demo drawing layers");

    // The demo drawing's real layer table: `0` and `WALLS`, both stored visible.
    assert_eq!(
        layers
            .layers
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        vec!["0", "WALLS"]
    );
    assert_eq!(layers.active.as_deref(), Some("0"));
    assert_eq!(layers.overridden, 0);
    for row in &layers.layers {
        assert!(row.visible);
        assert!(row.database_visible);
        assert!(!row.overridden);
    }
    // Exactly one row is the active layer, and it is the session's own layer.
    let current: Vec<&str> = layers
        .layers
        .iter()
        .filter(|row| row.current)
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(current, vec!["0"]);
}

#[test]
fn every_row_carries_the_stored_attributes_in_the_write_grammar() {
    let session = fresh_session();
    let layers = session.layers().expect("the demo drawing layers");

    // The fixture's rows carry no attributes record, so the documented default
    // is projected: ACI 7 white, `Continuous`, ByLayer weight, opaque, on,
    // thawed, unlocked and plottable. Nothing is invented on top of it.
    for row in &layers.layers {
        assert_eq!(row.color.aci, Some(7));
        assert_eq!(row.color.rgb, "#ffffff");
        assert_eq!(row.linetype, "Continuous");
        assert_eq!(row.lineweight, "ByLayer");
        assert_eq!(row.transparency, "0");
        assert!(!row.locked);
        assert!(!row.frozen);
        assert!(!row.off);
        assert!(row.plottable);
    }
}

#[test]
fn a_layer_attribute_write_reads_back_through_the_projection() {
    let session = fresh_session();
    let before = revision(&session);

    // Storage-only keys: they appear on the row and change nothing else.
    session
        .set_layer_property("WALLS", "color", "aci:30")
        .expect("a palette index");
    session
        .set_layer_property("WALLS", "lineweight", "35")
        .expect("0.35 mm in 1/100 mm");
    session
        .set_layer_property("WALLS", "transparency", "40")
        .expect("40 percent transparent");
    session
        .set_layer_property("WALLS", "locked", "true")
        .expect("a legal flag");
    session
        .set_layer_property("WALLS", "plottable", "false")
        .expect("a legal flag");

    let layers = session.layers().expect("layers");
    let walls = row(&layers, "WALLS");
    assert_eq!(walls.color.aci, Some(30));
    assert_eq!(walls.color.rgb, "#ff7f00");
    // The demo drawing has no linetype table, so this is the documented default
    // name, not a reference this build resolved. A successful `linetype` write is
    // pinned in `cad-app` (where the fixture has a real table); here the missing
    // table is the refusal case below.
    assert_eq!(walls.linetype, "Continuous");
    assert_eq!(walls.lineweight, "35");
    assert_eq!(walls.transparency, "40");
    assert!(walls.locked);
    assert!(!walls.plottable);
    assert!(walls.visible, "locking and not plotting hide nothing");
    // Five real changes, five revision bumps, still no undo entry.
    assert_eq!(revision(&session), cad_domain::Revision(before.0 + 5));
    assert!(!can_undo(&session));
}

#[test]
fn off_and_frozen_hide_the_layer_through_the_projection() {
    let session = fresh_session();

    session
        .set_layer_property("WALLS", "frozen", "1")
        .expect("a legal flag");

    let layers = session.layers().expect("layers");
    let walls = row(&layers, "WALLS");
    assert!(walls.frozen);
    assert!(!walls.off);
    // The database's own flag moved (not a temporary override): the row is
    // hidden in the layer table, so no override is reported.
    assert!(!walls.visible);
    assert!(!walls.database_visible);
    assert!(!walls.overridden);

    // Clearing the flag shows it again; an `off` layer stays hidden until both
    // are cleared.
    session
        .set_layer_property("WALLS", "off", "true")
        .expect("a legal flag");
    session
        .set_layer_property("WALLS", "frozen", "false")
        .expect("a legal flag");
    assert!(!row(&session.layers().expect("layers"), "WALLS").visible);
    session
        .set_layer_property("WALLS", "off", "false")
        .expect("a legal flag");
    assert!(row(&session.layers().expect("layers"), "WALLS").visible);
}

#[test]
fn a_refused_layer_attribute_write_changes_no_row_and_no_revision() {
    let session = fresh_session();
    let before_rows = session.layers().expect("layers");
    let before_revision = revision(&session);

    for (key, value) in [
        ("color", "ByLayer"),
        ("lineweight", "500"),
        ("transparency", "101"),
        ("plotstyle", "Normal"),
        ("hatch", "x"),
    ] {
        let error = session
            .set_layer_property("WALLS", key, value)
            .expect_err("this key/value pair must be refused");
        assert!(error.contains("layer."), "{key}={value}: {error}");
    }
    let error = session
        .set_layer_property("ROOF", "locked", "true")
        .expect_err("ROOF does not exist");
    assert!(error.contains("layer.unknown"), "{error}");

    assert_eq!(session.layers().expect("layers"), before_rows);
    assert_eq!(revision(&session), before_revision);
}

#[test]
fn hiding_a_layer_by_name_is_a_temporary_override_only() {
    let session = fresh_session();
    let revision_before = session
        .core
        .controller
        .borrow()
        .drawing()
        .expect("the demo drawing")
        .revision();

    session
        .set_layer_visibility("WALLS", false)
        .expect("WALLS exists in the demo drawing");

    let layers = session.layers().expect("the layers read");
    let walls = row(&layers, "WALLS");
    assert!(!walls.visible, "the scene honours the override");
    assert!(
        walls.database_visible,
        "the DWG layer flag is never modified by the UI"
    );
    assert!(walls.overridden);
    assert_eq!(layers.overridden, 1);
    // The other layer is untouched, and it is still the active one.
    let zero = row(&layers, "0");
    assert!(zero.visible);
    assert!(!zero.overridden);
    assert!(zero.current);

    // The override is session state only: the drawing itself is unchanged.
    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("the demo drawing");
    assert_eq!(drawing.revision(), revision_before);
    let stored = drawing
        .layers()
        .find(|layer| layer.name == "WALLS")
        .expect("the stored layer");
    assert!(stored.visible, "the stored flag is still visible");
}

#[test]
fn restore_layers_drops_every_override() {
    let session = fresh_session();
    session
        .set_layer_visibility("WALLS", false)
        .expect("WALLS exists");
    session
        .set_layer_visibility("0", false)
        .expect("layer 0 exists");
    assert_eq!(session.layers().expect("layers").overridden, 2);

    session.restore_layers().expect("the restore");

    let layers = session.layers().expect("layers");
    assert_eq!(layers.overridden, 0);
    for row in &layers.layers {
        assert_eq!(row.visible, row.database_visible);
        assert!(!row.overridden);
    }
}

#[test]
fn set_active_layer_moves_current() {
    let session = fresh_session();
    assert_eq!(
        session.layers().expect("layers").active.as_deref(),
        Some("0")
    );

    session.set_active_layer("WALLS").expect("WALLS exists");

    let layers = session.layers().expect("layers");
    assert_eq!(layers.active.as_deref(), Some("WALLS"));
    assert!(row(&layers, "WALLS").current);
    assert!(!row(&layers, "0").current);
    // Session state only: no override was created by making a layer current.
    assert_eq!(layers.overridden, 0);
}

#[test]
fn a_case_insensitive_name_resolves_to_the_drawing_layer() {
    let session = fresh_session();
    session
        .set_layer_visibility("walls", false)
        .expect("layer names are case-insensitive");
    let layers = session.layers().expect("layers");
    assert!(!row(&layers, "WALLS").visible);
    assert_eq!(layers.overridden, 1);
}

#[test]
fn an_unknown_layer_name_is_refused_without_any_state_change() {
    let session = fresh_session();
    let before = session.layers().expect("layers");

    let error = session
        .set_layer_visibility("ROOF", false)
        .expect_err("ROOF does not exist");
    assert!(
        error.contains("ROOF"),
        "the refusal names the layer: {error}"
    );
    assert!(error.contains("layer.unknown"), "{error}");

    let error = session
        .set_active_layer("doors")
        .expect_err("doors does not exist");
    assert!(
        error.contains("doors"),
        "the refusal names the layer: {error}"
    );
    assert!(error.contains("layer.unknown"), "{error}");

    let after = session.layers().expect("layers");
    assert_eq!(before, after, "a refused name changes nothing");
}

#[test]
fn no_document_projects_an_empty_answer_and_refuses_the_mutations() {
    let session = fresh_session();
    // Remove the authoritative document to reach the honest "nothing is open"
    // state (there is no host CLOSE command in this build).
    let document = session.core.controller.borrow().document_id;
    session
        .core
        .controller
        .borrow_mut()
        .application
        .workspace
        .documents
        .remove(&document);

    let layers = session.layers().expect("no document is an empty answer");
    assert_eq!(layers, CadLayers::empty());
    assert!(layers.layers.is_empty());
    assert_eq!(layers.active, None);
    assert_eq!(layers.overridden, 0);

    for error in [
        session.set_layer_visibility("0", false).unwrap_err(),
        session.restore_layers().unwrap_err(),
        session.layer_previous().unwrap_err(),
        session.set_active_layer("0").unwrap_err(),
    ] {
        assert!(
            error.contains("layer.no_document"),
            "an accepted mutation without a drawing would be a fake success: {error}"
        );
    }
}

#[test]
fn the_layer_write_admission_applies_gates_before_the_queue() {
    let mut queue: std::collections::VecDeque<CadLayerWrite> = std::collections::VecDeque::new();
    let write = CadLayerWrite::SetVisible {
        name: "WALLS".into(),
        visible: false,
    };

    // Modal popup gate first, then probe mode, exactly like the other CAD
    // mutation channels.
    let error = admit_layer_write(&mut queue, true, true, write.clone()).unwrap_err();
    assert!(error.contains("popup.modal_open"), "{error}");
    let error = admit_layer_write(&mut queue, false, false, write.clone()).unwrap_err();
    assert!(error.contains("probe mode"), "{error}");
    assert!(queue.is_empty());

    admit_layer_write(&mut queue, true, false, write).expect("an admitted write is queued");
    assert_eq!(queue.len(), 1);
}

#[test]
fn a_layer_property_key_is_checked_statically_before_the_queue() {
    // The key is the one part of the request that needs no session state, so the
    // admission refuses an unknown or unimplemented key by name.
    assert_eq!(
        parse_layer_property_write("WALLS", "color", "aci:1"),
        Ok((
            "WALLS".to_string(),
            "color".to_string(),
            "aci:1".to_string()
        ))
    );
    for key in ["plotstyle", "hatch"] {
        let error = parse_layer_property_write("WALLS", key, "x")
            .expect_err("this key must be refused before the queue");
        assert!(error.contains("layer.property.unsupported"), "{error}");
        assert!(error.contains(key), "the refusal names the key: {error}");
    }
    // The value is deliberately **not** validated here: the grammar can depend on
    // the drawing's own tables, so it is the owner thread's call.
    assert!(parse_layer_property_write("WALLS", "linetype", "PHANTOM").is_ok());
}

#[test]
fn a_drained_layer_write_is_applied_through_the_session() {
    let session = fresh_session();
    let mut queue = std::collections::VecDeque::from([
        CadLayerWrite::SetVisible {
            name: "WALLS".into(),
            visible: false,
        },
        CadLayerWrite::SetActive {
            name: "WALLS".into(),
        },
        // An unknown name is a real refusal, returned (not swallowed) and it
        // changes nothing; the writes before it still applied.
        CadLayerWrite::SetVisible {
            name: "ROOF".into(),
            visible: false,
        },
    ]);
    let refusals = drain_layer_writes(&mut queue, |write| write.apply(&session));
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].contains("ROOF"), "{refusals:?}");
    assert!(queue.is_empty());

    let layers = session.layers().expect("layers");
    assert!(!row(&layers, "WALLS").visible);
    assert!(row(&layers, "WALLS").current);
    assert_eq!(layers.overridden, 1);
}

// ---------------------------------------------------------------------------
// LAYERP: the previous layer state
// ---------------------------------------------------------------------------

#[test]
fn layerp_restores_the_previous_layer_state_in_a_viewer_session() {
    // `LAYERP` reads/writes session view state only, so a **Viewer** session must
    // be able to run it through the real selector funnel.
    let session = fresh_session();
    session
        .core
        .controller
        .borrow_mut()
        .set_mode(cad_app::AppMode::Viewer)
        .expect("switch to Viewer");

    // An empty history is a no-op, not an error, and changes nothing.
    let before = session.layers().expect("layers");
    session
        .command("LAYERP")
        .expect("LAYERP is admitted in Viewer");
    assert_eq!(session.layers().expect("layers"), before);

    // Two real overrides, then LAYERP steps back to the state before the **last**
    // change (layer 0's hide is undone; WALLS stays hidden) — not clear everything.
    session
        .set_layer_visibility("WALLS", false)
        .expect("WALLS exists");
    session
        .set_layer_visibility("0", false)
        .expect("layer 0 exists");
    let layers = session.layers().expect("layers");
    assert_eq!(layers.overridden, 2);

    session.command("LAYERP").expect("LAYERP in Viewer");
    let layers = session.layers().expect("layers");
    assert!(
        !row(&layers, "WALLS").visible,
        "the previous state still had WALLS hidden"
    );
    assert!(row(&layers, "0").visible, "layer 0's hide was undone");
    assert_eq!(
        layers.overridden, 1,
        "one step back, not a clear-all restore"
    );

    // ...and one more step back reaches the original, fully restored state.
    session.command("layerp").expect("case-insensitive");
    let layers = session.layers().expect("layers");
    assert_eq!(layers.overridden, 0);
    assert!(layers.layers.iter().all(|row| row.visible));

    // Exhausted history: another LAYERP is a no-op (no ping-pong) and the layers
    // are unchanged.
    session
        .command("LAYERP")
        .expect("an empty history is not an error");
    assert_eq!(session.layers().expect("layers"), layers);
}

#[test]
fn layerp_and_restore_layers_are_different_commands() {
    // `RESTORELAYERS`/`LAYUNISO`/`LAYON` clear every override; `LAYERP` steps back
    // one recorded state. Both must work through the selector funnel, and they must
    // not be the same name.
    assert_eq!(normalize_command("LAYERP"), Some(CommandId::LayerPrevious));
    assert_eq!(
        normalize_command("LAYUNISO"),
        Some(CommandId::RestoreLayers)
    );

    let session = fresh_session();
    session
        .set_layer_visibility("WALLS", false)
        .expect("WALLS exists");
    session
        .set_layer_visibility("0", false)
        .expect("layer 0 exists");

    // RESTORELAYERS clears both overrides (and records one LAYERP step).
    session.command("LAYUNISO").expect("LAYUNISO clears");
    assert_eq!(session.layers().expect("layers").overridden, 0);

    // LAYERP brings the cleared pair *back*: that is the previous state. The typed
    // panel entrance and the selector reach the same command.
    session.layer_previous().expect("LAYERP restores");
    assert_eq!(
        session.layers().expect("layers").overridden,
        2,
        "LAYERP is not an alias of the clear-all restore"
    );
}

// ---------------------------------------------------------------------------
// Layer table structural writes: create / rename / delete
// ---------------------------------------------------------------------------

fn revision(session: &CadSession) -> cad_domain::Revision {
    session
        .core
        .controller
        .borrow()
        .drawing()
        .expect("the demo drawing")
        .revision()
}

fn can_undo(session: &CadSession) -> bool {
    let controller = session.core.controller.borrow();
    controller.application.can_undo(&controller.document_id)
}

#[test]
fn create_layer_adds_a_row_in_one_revision_bump_and_no_undo_entry() {
    let session = fresh_session();
    let before_rows = session.layers().expect("layers").layers.len();
    let before_revision = revision(&session);

    session.create_layer("DOORS").expect("a fresh legal name");

    let layers = session.layers().expect("layers");
    assert_eq!(layers.layers.len(), before_rows + 1);
    let doors = row(&layers, "DOORS");
    assert!(doors.visible, "a new layer is stored visible");
    assert!(doors.database_visible);
    assert!(!doors.overridden, "a create adds no temporary override");
    // A real database write: exactly one revision bump, no undo entry.
    assert_eq!(
        revision(&session),
        cad_domain::Revision(before_revision.0 + 1)
    );
    assert!(!can_undo(&session), "layer edits are not undoable");
}

#[test]
fn create_layer_refuses_a_duplicate_and_an_illegal_name() {
    let session = fresh_session();
    let before = session.layers().expect("layers");
    let before_revision = revision(&session);

    let error = session
        .create_layer("walls")
        .expect_err("WALLS exists; names are case-insensitive");
    assert!(error.contains("layer.duplicate_name"), "{error}");

    let error = session
        .create_layer("A*B")
        .expect_err("'*' is not legal in a layer name");
    assert!(error.contains("layer.name_illegal_char"), "{error}");

    assert_eq!(
        session.layers().expect("layers"),
        before,
        "a refused create changes no row"
    );
    assert_eq!(revision(&session), before_revision);
}

#[test]
fn rename_layer_resolves_under_the_new_name_and_not_the_old_one() {
    let session = fresh_session();

    session
        .rename_layer("WALLS", "PARTITIONS")
        .expect("WALLS exists and PARTITIONS is free");

    let layers = session.layers().expect("layers");
    row(&layers, "PARTITIONS");
    assert!(
        layers.layers.iter().all(|row| row.name != "WALLS"),
        "the old name is gone from the table"
    );

    // The old name no longer resolves; the new one resolves case-insensitively.
    let error = session
        .set_layer_visibility("WALLS", false)
        .expect_err("WALLS no longer exists");
    assert!(error.contains("layer.unknown"), "{error}");
    session
        .set_layer_visibility("partitions", false)
        .expect("the new name resolves");
    assert!(!row(&session.layers().expect("layers"), "PARTITIONS").visible);
}

#[test]
fn delete_layer_in_use_is_refused_without_changing_the_projection() {
    let session = fresh_session();
    let before = session.layers().expect("layers");
    let before_revision = revision(&session);

    // The demo drawing's WALLS layer carries the room-outline entities.
    let error = session
        .delete_layer("WALLS")
        .expect_err("WALLS is referenced by entities");
    let text = error;
    assert!(text.contains("layer.in_use"), "{text}");
    assert!(text.contains('4'), "the refusal names the count: {text}");

    assert_eq!(session.layers().expect("layers"), before);
    assert_eq!(revision(&session), before_revision);
}

#[test]
fn deleting_the_active_layer_makes_zero_current_again() {
    let session = fresh_session();
    session.create_layer("DOORS").expect("a fresh legal name");
    session
        .set_active_layer("DOORS")
        .expect("the new layer can be made current");
    assert_eq!(
        session.layers().expect("layers").active.as_deref(),
        Some("DOORS")
    );

    session
        .delete_layer("DOORS")
        .expect("nothing references DOORS");

    let layers = session.layers().expect("layers");
    assert_eq!(
        layers.active.as_deref(),
        Some("0"),
        "the session never keeps a dangling active layer"
    );
    assert!(row(&layers, "0").current);
    assert!(layers.layers.iter().all(|row| row.name != "DOORS"));
}

#[test]
fn delete_of_the_default_layer_is_refused() {
    let session = fresh_session();
    let before = session.layers().expect("layers");

    let error = session
        .delete_layer("0")
        .expect_err("layer 0 is the default layer");
    assert!(error.contains("layer.default_undeletable"), "{error}");
    assert_eq!(session.layers().expect("layers"), before);
}

#[test]
fn the_structural_layer_writes_apply_through_the_same_queue() {
    let session = fresh_session();
    let rows_before = session.layers().expect("layers").layers.len();
    let mut queue = std::collections::VecDeque::from([
        CadLayerWrite::Create {
            name: "DOORS".into(),
        },
        CadLayerWrite::Rename {
            layer: "DOORS".into(),
            name: "WINDOWS".into(),
        },
        // The attribute write rides the **same** queue: no second channel.
        CadLayerWrite::SetProperty {
            layer: "WINDOWS".into(),
            key: "color".into(),
            value: "aci:1".into(),
        },
        CadLayerWrite::Delete {
            layer: "WINDOWS".into(),
        },
    ]);

    let refusals = drain_layer_writes(&mut queue, |write| write.apply(&session));
    assert!(refusals.is_empty(), "{refusals:?}");

    let layers = session.layers().expect("layers");
    assert_eq!(layers.layers.len(), rows_before, "created then deleted");
    assert!(layers.layers.iter().all(|row| row.name != "WINDOWS"));
    assert!(layers.layers.iter().all(|row| row.name != "DOORS"));
}

#[test]
fn the_structural_layer_writes_are_refused_without_a_document() {
    let session = fresh_session();
    let document = session.core.controller.borrow().document_id;
    session
        .core
        .controller
        .borrow_mut()
        .application
        .workspace
        .documents
        .remove(&document);

    for error in [
        session.create_layer("DOORS").unwrap_err(),
        session.rename_layer("WALLS", "PARTITIONS").unwrap_err(),
        session.delete_layer("WALLS").unwrap_err(),
        session
            .set_layer_property("WALLS", "locked", "true")
            .unwrap_err(),
    ] {
        assert!(
            error.contains("layer.no_document"),
            "an accepted write without a drawing would be a fake success: {error}"
        );
    }
}
