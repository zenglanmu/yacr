//! Per-tick input batching contracts.
//!
//! `CadSession::handle_inputs` must apply every input in order (no phase
//! dropped, no wheel delta merged) but publish the derived render state **once**
//! for the whole tick. `handle_input` keeps its per-call publication, because
//! many callers read the overlay immediately after one input.

use std::time::Duration;

use super::super::*;
use super::test_support::*;

/// A navigation-only session with the concrete settle deferrals and the
/// hover/snap transient producers off.
///
/// The settle timeouts are zeroed so each render publication is immediate and
/// observable (`view_revision`/content token advance in the same call) instead
/// of being deferred by the camera-settle timer; hover/snap are off so a pan
/// sequence changes nothing but the camera.
fn navigation_session() -> CadSession {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    session.core.render.set_grid_settle_timeout(Duration::ZERO);
    session
        .core
        .render
        .set_navigation_settle_timeout(Duration::ZERO);
    {
        let mut controller = session.core.controller.borrow_mut();
        controller.set_snap_enabled(false);
        controller.session.set_hover_preview(false);
    }
    // Install the authoritative canvas metrics/view/overlay snapshot once, so
    // the observed counter deltas belong to the inputs under test and not to
    // the session's first sync.
    session
        .set_canvas_metrics([800.0, 600.0], 1.0)
        .expect("the canvas metrics are accepted");
    session.core.sync_view().expect("the initial view syncs");
    session
}

/// `(view_revision, requested_token)` of the session's render handle.
fn publication_counters(session: &CadSession) -> (u64, u64) {
    let diagnostics = session.core.render.diagnostics();
    (diagnostics.view_revision, diagnostics.requested_token)
}

fn press(session: &CadSession, position: [f64; 2]) {
    session
        .handle_input(&pointer_at(
            InputKind::PointerDown,
            1,
            position,
            Some(BUTTON_MIDDLE),
        ))
        .expect("the press is admitted");
}

/// The moving half of a middle-button drag: six moves past the drag threshold
/// and the release.
fn drag_moves(start: [f64; 2]) -> Vec<ProbeInput> {
    let mut inputs = Vec::new();
    for step in 1..=6 {
        inputs.push(pointer_at(
            InputKind::PointerMove,
            1 + step,
            [start[0] + 10.0 * step as f64, start[1] + 4.0 * step as f64],
            None,
        ));
    }
    inputs.push(pointer_at(
        InputKind::PointerUp,
        8,
        [start[0] + 60.0, start[1] + 24.0],
        Some(BUTTON_MIDDLE),
    ));
    inputs
}

#[test]
fn one_batch_publishes_the_same_camera_with_one_publication() {
    let start = [200.0, 200.0];
    let individual = navigation_session();
    let batched = navigation_session();
    // The press is common to both runs; the compared span is the moving drag.
    press(&individual, start);
    press(&batched, start);
    let inputs = drag_moves(start);

    let individual_before = publication_counters(&individual);
    for input in &inputs {
        individual
            .handle_input(input)
            .unwrap_or_else(|error| panic!("individual {:?} refused: {error}", input.kind));
    }
    let individual_after = publication_counters(&individual);

    let batched_before = publication_counters(&batched);
    batched
        .handle_inputs(&inputs)
        .expect("the whole tick is admitted");
    let batched_after = publication_counters(&batched);

    // Business result is identical: same authoritative camera and the same
    // derived overlay inputs (the deferred publication loses no state).
    assert_eq!(
        viewport_camera(&individual),
        viewport_camera(&batched),
        "batching must not change the final camera"
    );
    assert_eq!(
        individual.core.render.overlays(),
        batched.core.render.overlays(),
        "batching must not change the final overlay inputs"
    );

    let individual_views = individual_after.0 - individual_before.0;
    let batched_views = batched_after.0 - batched_before.0;
    let individual_tokens = individual_after.1 - individual_before.1;
    let batched_tokens = batched_after.1 - batched_before.1;

    assert_eq!(
        individual_views, 6,
        "the individual path publishes once per moving input"
    );
    assert_eq!(
        batched_views, 1,
        "one view publication for the whole tick, not {individual_views}"
    );
    assert!(
        batched_tokens > 0,
        "the single flush really publishes (a real preparation content advance)"
    );
    assert!(
        batched_tokens < individual_tokens,
        "the batch advances the content token with one flush ({batched_tokens}), not per input ({individual_tokens})"
    );
    assert_eq!(
        batched.core.render_batch_depth(),
        0,
        "the batch guard is always closed"
    );
}

#[test]
fn handle_input_keeps_publishing_per_call() {
    let session = navigation_session();
    let start = [200.0, 200.0];
    press(&session, start);
    let before = publication_counters(&session);

    session
        .handle_input(&pointer_at(
            InputKind::PointerMove,
            2,
            [start[0] + 40.0, start[1]],
            None,
        ))
        .expect("the drag is admitted");

    // The move published before `handle_input` returned: no tick-end flush is
    // required for callers that read the overlays immediately.
    assert_eq!(
        publication_counters(&session).0,
        before.0 + 1,
        "one individual call publishes its own camera change"
    );
    assert_eq!(session.core.render_batch_depth(), 0);
}

#[test]
fn a_refused_input_still_closes_the_batch_and_keeps_the_first_error() {
    let start = [200.0, 200.0];
    let session = navigation_session();
    let batched = navigation_session();
    press(&session, start);
    press(&batched, start);

    // An undefined pointer button is refused by the planner (a stable reason)
    // while the neighbouring input stays valid.
    let refused = pointer_at(InputKind::PointerDown, 2, [10.0, 20.0], Some(42));
    let pointer = pointer_at(InputKind::PointerMove, 3, [start[0] + 40.0, start[1]], None);
    let inputs = vec![refused, pointer.clone()];

    let error = session
        .handle_inputs(&inputs)
        .expect_err("the undefined button must be refused");
    assert!(
        error.contains("unsupported pointer button"),
        "the first real refusal is returned: {error}"
    );
    assert_eq!(
        session.core.render_batch_depth(),
        0,
        "a refusal must not leave the batch guard set"
    );

    // The valid input in the same tick still executed (the batch keeps the
    // first error, it does not abort the ordered stream).
    batched
        .handle_input(&pointer)
        .expect("the valid move is admitted");
    assert_eq!(
        viewport_camera(&session),
        viewport_camera(&batched),
        "the valid input after a refused one was still applied"
    );
    // And a later tick can publish normally (the guard is not stuck).
    session
        .handle_inputs(&drag_moves(start))
        .expect("the next tick's batch is applied");
}
