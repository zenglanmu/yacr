//! Host contracts for the incremental drawing-edit receipt.
//!
//! These prove the host boundaries only: a successful LINE confirmation binds
//! an admitted receipt through the trusted `cad-app` funnel and forwards it to
//! the render sync; a failed confirmation mutates nothing, preserves the
//! capture and supplies no receipt. They are not GPU or performance evidence.

use super::*;

fn begin_line(core: &HostCore) {
    let mut controller = core.controller.borrow_mut();
    controller.set_snap_enabled(false);
    controller.begin_draw_tool(DrawToolKind::Line).unwrap();
}

#[test]
fn a_successful_line_confirm_supplies_an_incremental_receipt() {
    let core = hover_core();
    begin_line(&core);
    dispatch_pointer(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(0.0, 0.0))),
    );
    dispatch_pointer(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(10.0, 0.0))),
    );
    let before = core.controller.borrow().drawing().unwrap();

    dispatch_pointer(&core, InteractionRequest::Confirm);

    let after = core.controller.borrow().drawing().unwrap();
    assert!(
        !std::sync::Arc::ptr_eq(&before, &after),
        "the LINE commit must publish a new authoritative drawing"
    );
    assert_eq!(
        after.entity_count(),
        before.entity_count() + 1,
        "exactly the one confirmed LINE is inserted"
    );
    assert!(
        core.last_edit_receipt_for_test.get(),
        "the host must forward the receipt built by the trusted confirm funnel"
    );
    assert!(
        core.controller.borrow().session.draw_preview().is_none(),
        "a successful confirm clears the capture"
    );
}

#[test]
fn a_failed_line_confirm_preserves_the_capture_and_supplies_no_receipt() {
    let core = hover_core();
    begin_line(&core);
    dispatch_pointer(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(0.0, 0.0))),
    );
    let before = core.controller.borrow().drawing().unwrap();

    // One captured point cannot produce a LINE: the confirm fails inside the
    // command layer, the capture stays and the database is untouched.
    let refused = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::Confirm),
    );
    assert!(refused.is_err(), "an incomplete LINE must be refused");

    let after = core.controller.borrow().drawing().unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&before, &after),
        "a refused confirm must not publish a drawing"
    );
    assert_eq!(after.entity_count(), before.entity_count());
    assert_eq!(after.revision(), before.revision());
    assert!(
        core.controller.borrow().session.draw_preview().is_some(),
        "the failed confirm preserves the draw capture"
    );
    assert!(
        !core.last_edit_receipt_for_test.get(),
        "a failed confirm must not supply a receipt"
    );
}
