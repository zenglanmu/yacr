//! `SETENTITYPROPERTY` host contracts: the typed interaction arm that the
//! properties panel will call.
//!
//! These drive `HostCore` through `EngineClient` (no UI, no GPU) and pin that the
//! request carries no entity id, that a refusal is named and opens no
//! transaction, and that a successful edit is one transaction / one undo step
//! with the selection preserved.

use super::*;

fn dispatch_property(core: &HostCore, key: &str, value: &str) -> Result<(), RequestError> {
    EngineClient::dispatch(
        core,
        EngineRequest::Interaction(InteractionRequest::SetSelectedProperty {
            key: key.into(),
            value: value.into(),
        }),
    )
    .map(|_| ())
}

fn layer_of(core: &HostCore, entity: u128) -> cad_domain::LayerId {
    core.controller
        .borrow()
        .drawing()
        .expect("a document")
        .entity(cad_domain::EntityId(entity))
        .expect("the entity")
        .layer
}

#[test]
fn dispatch_edits_every_selected_entity_and_keeps_the_selection() {
    let core = core();
    let document = core.controller.borrow().document_id;
    core.controller.borrow_mut().session.selection.replace([
        cad_app::selection::model_ref(document, cad_domain::EntityId(1)),
        cad_app::selection::model_ref(document, cad_domain::EntityId(2)),
    ]);
    let before = core.controller.borrow().drawing().unwrap().revision();

    dispatch_property(&core, "layer", "0").expect("layer 0 exists in the demo drawing");

    assert_eq!(layer_of(&core, 1), cad_domain::LayerId(0));
    assert_eq!(layer_of(&core, 2), cad_domain::LayerId(0));
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        cad_domain::Revision(before.0 + 1),
        "one transaction"
    );
    let controller = core.controller.borrow();
    assert_eq!(controller.selection().len(), 2, "selection preserved");
    let properties = controller.selection_properties().expect("properties");
    let layer = properties
        .rows
        .iter()
        .find(|row| row.key == "layer")
        .expect("a layer row");
    assert_eq!(layer.value.as_deref(), Some("0 (0)"));
    assert!(controller.application.can_undo(&document));
}

#[test]
fn dispatch_refusals_are_named_and_leave_no_transaction() {
    let core = core();

    // Nothing selected.
    let error = dispatch_property(&core, "layer", "0").expect_err("nothing is selected");
    match error {
        RequestError::Execution(CadError::InvalidInput(reason)) => {
            assert!(reason.contains("property.selection_empty"), "{reason}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }

    // A bad value on a valid selection.
    select_entity(&core, cad_domain::EntityId(1));
    let before = core.controller.borrow().drawing().unwrap().revision();
    let error = dispatch_property(&core, "transparency", "91").expect_err("out of range");
    match error {
        RequestError::Execution(CadError::InvalidInput(reason)) => {
            assert!(reason.contains("property.invalid_value"), "{reason}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    assert_eq!(
        core.controller.borrow().drawing().unwrap().revision(),
        before,
        "a refusal writes nothing"
    );
    let document = core.controller.borrow().document_id;
    assert!(!core.controller.borrow().application.can_undo(&document));

    // `text` has its own path and is refused here.
    let error = dispatch_property(&core, "text", "NEW").expect_err("text is not a property");
    match error {
        RequestError::Execution(CadError::Unsupported(reason)) => {
            assert!(reason.contains("property.unsupported_key"), "{reason}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
}
