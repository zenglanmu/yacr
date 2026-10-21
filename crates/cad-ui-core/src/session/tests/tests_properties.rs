//! The properties-panel write channel (`cad_set_selected_property`).
//!
//! The admission only carries two strings to the owner thread; the selection,
//! the drawing's own tables and every refusal are the host session's authority
//! on drain. These contracts exercise the **real** session funnel
//! (`CadSession::set_selected_property` → `SETENTITYPROPERTY`), so the next
//! `cad_status` snapshot is the evidence that the write reached the drawing and
//! that the display rows read back the value that was written.

use super::super::*;
use super::test_support::*;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// Select exactly one entity of the session's own document.
fn select(session: &CadSession, entity: u128) {
    let reference = SelectionRef {
        document: session.core.controller.borrow().document_id,
        entity: EntityId(entity),
        instance: InstancePath::default(),
        sub_element: None,
    };
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(vec![reference])
        .expect("the selection");
}

fn row<'a>(
    status: &'a crate::session::CadStatus,
    key: &str,
) -> &'a crate::session::CadSelectionRow {
    status
        .selection
        .rows
        .iter()
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("no '{key}' row in {:?}", status.selection.rows))
}

#[test]
fn the_property_channel_applies_a_layer_change_and_the_next_status_shows_it() {
    let session = fresh_session();
    select(&session, 1);

    // The demo entity 1 is on the real `WALLS` layer; the row is the display
    // text, the writable value is the bare name.
    let before = session.snapshot(None, None, 0);
    assert_eq!(row(&before, "layer").value.as_deref(), Some("WALLS (1)"));
    assert_eq!(row(&before, "layer").kind, "enum");
    assert!(row(&before, "layer").editable);
    assert!(row(&before, "layer").options.contains(&"0".to_string()));

    session
        .set_selected_property("layer", "0")
        .expect("the layer `0` exists in the demo drawing");

    let after = session.snapshot(None, None, 0);
    assert_eq!(row(&after, "layer").value.as_deref(), Some("0 (0)"));
    assert_eq!(
        row(&after, "layer").options,
        vec!["0".to_string(), "WALLS".to_string()],
        "the option list is still the real layer table"
    );
}

#[test]
fn a_written_display_value_reads_back_unchanged() {
    let session = fresh_session();
    select(&session, 1);

    // The display rows are the canonical write grammar: writing the value a row
    // carries is a no-op, and an explicit value reads back as that same string.
    for (key, before, written, after) in [
        ("color", "ByLayer", "aci:1", "aci:1"),
        ("lineweight", "ByLayer", "0.35", "0.35"),
        ("transparency", "0", "60", "60"),
    ] {
        let status = session.snapshot(None, None, 0);
        let current = row(&status, key).value.clone().expect("a value");
        assert_eq!(current, before, "'{key}' starts at the drawing default");
        session
            .set_selected_property(key, &current)
            .unwrap_or_else(|error| {
                panic!("re-writing '{key}={current}' must be a no-op: {error}")
            });
        session
            .set_selected_property(key, written)
            .unwrap_or_else(|error| panic!("'{key}={written}' is writable: {error}"));
        let status = session.snapshot(None, None, 0);
        assert_eq!(row(&status, key).value.as_deref(), Some(after), "'{key}'");
    }
}

#[test]
fn an_unknown_key_is_refused_with_the_stable_property_code() {
    let session = fresh_session();
    select(&session, 1);
    for key in ["bogus", "text", "id", "type"] {
        let error = session.set_selected_property(key, "x").unwrap_err();
        assert!(
            error.contains("property.unsupported_key"),
            "'{key}' must be refused by code, got {error}"
        );
    }
    // Nothing was written: the layer row still carries the entity's real layer.
    assert_eq!(
        row(&session.snapshot(None, None, 0), "layer")
            .value
            .as_deref(),
        Some("WALLS (1)")
    );
}

#[test]
fn a_wrong_value_is_refused_and_leaves_the_entity_unchanged() {
    let session = fresh_session();
    select(&session, 1);
    let before = {
        let controller = session.core.controller.borrow();
        let drawing = controller.drawing().expect("a document");
        (
            drawing.revision(),
            drawing.entity(EntityId(1)).expect("the entity").layer,
        )
    };

    for (key, value) in [
        ("transparency", "91"),
        ("transparency", "abc"),
        ("lineweight", "0.37"),
        ("color", "not-a-colour"),
        ("layer", "NOSUCHLAYER"),
    ] {
        let error = session.set_selected_property(key, value).unwrap_err();
        assert!(
            error.contains("property.invalid_value") || error.contains("property.unknown_layer"),
            "'{key}={value}' must be refused by code, got {error}"
        );
    }

    let controller = session.core.controller.borrow();
    let drawing = controller.drawing().expect("a document");
    assert_eq!(drawing.revision(), before.0, "no transaction was opened");
    assert_eq!(
        drawing.entity(EntityId(1)).expect("the entity").layer,
        before.1,
        "the entity is byte-for-byte unchanged"
    );
    assert!(
        !controller.history_availability().can_undo,
        "a refused write records no undo entry"
    );
}

#[test]
fn the_property_channel_refuses_an_empty_selection_by_name() {
    let session = fresh_session();
    session
        .core
        .controller
        .borrow_mut()
        .set_selection(Vec::new())
        .expect("the empty selection");
    let error = session.set_selected_property("layer", "0").unwrap_err();
    assert!(error.contains("property.selection_empty"), "got {error}");
}
