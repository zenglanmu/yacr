//! Render-publication batch contracts at the host boundary.
//!
//! A batch defers only the derived render publication: every business step still
//! runs in order, the interaction tolerance is still installed, and a pending
//! incremental edit receipt is stashed rather than dropped.

use super::*;

/// A pan command through the ordinary host dispatch funnel (which ends in the
/// same post-business `sync_render` every real command path uses).
fn pan(core: &HostCore, delta: Point3) {
    EngineClient::dispatch(
        core,
        EngineRequest::Command(command(
            core,
            CommandId::Pan,
            CommandPayload::Points(vec![delta]),
        )),
    )
    .expect("the pan command is admitted");
}

/// Counters observed after one settled publication.
fn counters(core: &HostCore) -> (u64, u64) {
    let diagnostics = core.render.diagnostics();
    (diagnostics.view_revision, diagnostics.requested_token)
}

#[test]
fn a_render_batch_defers_publication_to_one_flush() {
    let core = core();
    // Zeroed settle timeouts make every publication immediate and observable.
    core.render
        .set_grid_settle_timeout(std::time::Duration::ZERO);
    core.render
        .set_navigation_settle_timeout(std::time::Duration::ZERO);

    let before = counters(&core);
    core.begin_render_batch();
    assert_eq!(core.render_batch_depth(), 1);

    for step in 1..=4 {
        pan(&core, p(f64::from(step), 0.0));
    }
    assert_eq!(
        counters(&core),
        before,
        "no publication happens while the batch is open"
    );

    core.end_render_batch().expect("the flush publishes");
    let flushed = counters(&core);
    assert_eq!(
        flushed.0,
        before.0 + 1,
        "one view publication for the whole batch"
    );
    assert!(
        flushed.1 > before.1,
        "the single flush publishes real preparation work"
    );
    assert_eq!(core.render_batch_depth(), 0);
}

#[test]
fn nested_batches_publish_only_at_the_outermost_close() {
    let core = core();
    core.render
        .set_grid_settle_timeout(std::time::Duration::ZERO);
    core.render
        .set_navigation_settle_timeout(std::time::Duration::ZERO);
    let before = counters(&core);

    core.begin_render_batch();
    core.begin_render_batch();
    pan(&core, p(3.0, 0.0));
    core.end_render_batch().expect("the inner close is a no-op");
    assert_eq!(core.render_batch_depth(), 1);
    assert_eq!(counters(&core), before, "the inner close must not publish");

    core.end_render_batch().expect("the outer close publishes");
    assert_eq!(core.render_batch_depth(), 0);
    assert_eq!(counters(&core).0, before.0 + 1);
}

#[test]
fn a_pending_edit_receipt_survives_the_batch_and_flushes_once() {
    let core = hover_core();
    core.render
        .set_grid_settle_timeout(std::time::Duration::ZERO);
    core.render
        .set_navigation_settle_timeout(std::time::Duration::ZERO);
    {
        let mut controller = core.controller.borrow_mut();
        controller.set_snap_enabled(false);
        controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    }

    core.begin_render_batch();
    dispatch_pointer(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(0.0, 0.0))),
    );
    dispatch_pointer(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(10.0, 0.0))),
    );
    let input_before_confirm = core.render.diagnostics().input_revision;
    dispatch_pointer(&core, InteractionRequest::Confirm);

    assert!(
        core.last_edit_receipt_for_test.get(),
        "the trusted confirm funnel still built the incremental receipt"
    );
    assert!(
        core.deferred_edit.borrow().is_some(),
        "the receipt is stashed, never dropped, while the batch is open"
    );
    assert_eq!(
        core.render.diagnostics().input_revision,
        input_before_confirm,
        "the committed drawing is not published before the batch closes"
    );

    // A `None` sync inside the same batch must not erase the stashed receipt.
    core.sync_render().expect("a no-edit sync is accepted");
    assert!(
        core.deferred_edit.borrow().is_some(),
        "a later edit-free sync must not overwrite a pending edit"
    );

    core.end_render_batch()
        .expect("the flush publishes the edit");
    assert!(
        core.deferred_edit.borrow().is_none(),
        "the receipt is consumed exactly once at the flush"
    );
    assert!(
        core.render.diagnostics().input_revision > input_before_confirm,
        "the flush published the committed drawing"
    );
    assert_eq!(core.render_batch_depth(), 0);
}
