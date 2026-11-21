//! `cad_select_entities` host wiring: a component's geometry rows replace the
//! current selection — atomically, or not at all.
//!
//! The wire contract (`cad-ui-core`) pins the request shape and the gates; the
//! BIM query backend pins what a row's `entityIds` mean. What no unit test on
//! either side can see is the host's own resolution against the open document:
//! that the ids become real `SelectionRef`s bound to the current document, that
//! the properties projection really reports the new selection, and that an
//! unknown id refuses the **whole** request instead of selecting the subset that
//! still exists.
//!
//! These are synthetic contracts over `HostCore` (no window, no GPU): they are not
//! render/visual acceptance evidence.

use super::*;
use crate::{SELECTION_NO_DOCUMENT, SELECTION_UNKNOWN_ENTITY};
use cad_domain::EntityId;

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A blank document with two disjoint CIRCLEs, returning their entity ids.
fn two_circles() -> (HostCore, Vec<EntityId>) {
    let core = core();
    core.controller
        .borrow_mut()
        .new_blank_document()
        .expect("a blank document is installed");
    core.note_document_replaced();
    let mut ids = Vec::new();
    for x in [0.0_f64, 10.0] {
        let outcome = core
            .controller
            .borrow_mut()
            .create_circle(point(x, 0.0, 0.0), point(x + 2.0, 0.0, 0.0))
            .expect("the CIRCLE is created");
        ids.push(EntityId(outcome.objects[0].0));
    }
    (core, ids)
}

/// The selected entity ids, in selection order.
fn selected_ids(core: &HostCore) -> Vec<EntityId> {
    core.controller
        .borrow()
        .selection()
        .refs()
        .iter()
        .map(|reference| reference.entity)
        .collect()
}

#[test]
fn selecting_two_ids_replaces_the_selection_and_the_projection_reports_both() {
    let (core, ids) = two_circles();
    core.select_entities(&ids).expect("both ids exist");

    assert_eq!(
        selected_ids(&core),
        ids,
        "the selection is exactly the requested ids, in order"
    );
    let properties = core
        .controller
        .borrow()
        .selection_properties()
        .expect("a document is open");
    assert_eq!(properties.count, 2, "the panel sees two selected objects");
    assert!(!properties.empty);
    assert!(
        properties.rows.iter().any(|row| row.key == "type"),
        "the projection is the real one, not an empty shell"
    );
}

#[test]
fn a_later_request_replaces_rather_than_extends_the_selection() {
    let (core, ids) = two_circles();
    core.select_entities(&ids).expect("both ids exist");
    core.select_entities(&ids[1..]).expect("one real id");

    assert_eq!(
        selected_ids(&core),
        vec![ids[1]],
        "the previous selection is replaced, not appended to"
    );
}

#[test]
fn an_unknown_id_refuses_the_whole_request_and_leaves_the_selection_untouched() {
    let (core, ids) = two_circles();
    core.select_entities(&ids).expect("both ids exist");

    let unknown = EntityId(99_999_999);
    let error = core
        .select_entities(&[ids[0], unknown])
        .expect_err("an id outside the document is refused");
    match &error {
        CadError::Unsupported(reason) => assert_eq!(
            reason,
            &format!("{SELECTION_UNKNOWN_ENTITY}: {}", unknown.0),
            "the marker names the missing id"
        ),
        other => panic!("expected the stable marker, got {other:?}"),
    }
    assert_eq!(
        selected_ids(&core),
        ids,
        "a refused request never partially selects"
    );
}

#[test]
fn a_host_with_no_document_refuses_by_marker() {
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(FakeConfig),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let mut options = options();
    options.document_at_start = StartDocument::Empty;
    let core =
        HostCore::new(options, platform, RenderSessionHandle::default()).expect("empty host core");
    assert!(core.controller.borrow().drawing().is_none());

    let error = core
        .select_entities(&[EntityId(1)])
        .expect_err("no document means no entity to select");
    match &error {
        CadError::Unsupported(reason) => assert_eq!(reason, SELECTION_NO_DOCUMENT),
        other => panic!("expected the stable marker, got {other:?}"),
    }
}
