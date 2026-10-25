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
