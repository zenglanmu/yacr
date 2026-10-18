//! Box-selection hardening contracts (ADR 0014 decision B, gate P1 findings).
//!
//! These pin the edges the core gesture tests in `box_select.rs` do not cover:
//! a capture or a modal pan that takes the pointer mid-drag must not leave a
//! frozen band or a silently refused release; a partially released contact must
//! not inherit the released contact's button/modifiers; a large CROSSING drag
//! must stay closed within the segment bound; and a degenerate rectangle must
//! surface its refusal rather than drop it silently.
use super::*;

/// A pointer request with explicit kind/button/modifiers, tagged with the host's
/// current metrics revision.
fn contact(
    core: &HostCore,
    id: u64,
    kind: cad_app::interaction::PointerKind,
    phase: cad_app::interaction::PointerPhase,
    button: Option<PointerButton>,
    position: [f64; 2],
    modifiers: cad_app::interaction::Modifiers,
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(id),
        kind,
        phase,
        button,
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: core.request_context().metrics_revision.unwrap(),
        },
        modifiers,
    }
}

fn shift_modifiers(shift: bool) -> cad_app::interaction::Modifiers {
    cad_app::interaction::Modifiers {
        shift,
        ..cad_app::interaction::Modifiers::default()
    }
}

/// A single-contact (id 1, mouse, primary) pointer phase.
fn primary(core: &HostCore, phase: cad_app::interaction::PointerPhase, position: [f64; 2]) {
    dispatch_pointer(
        core,
        contact(
            core,
            1,
            cad_app::interaction::PointerKind::Mouse,
            phase,
            Some(PointerButton::Primary),
            position,
            cad_app::interaction::Modifiers::default(),
        ),
    );
}

fn selected_refs(core: &HostCore) -> Vec<cad_domain::SelectionRef> {
    core.controller.borrow().session.selection.refs().to_vec()
}

fn drawing_revision(core: &HostCore) -> cad_domain::Revision {
    core.controller.borrow().drawing().unwrap().revision()
}

// --- a capture activated mid-drag ends the gesture --------------------------

#[test]
fn a_capture_activated_mid_box_drag_ends_the_gesture_and_dispatches_nothing() {
    let core = core();
    let anchor = [100.0, 100.0];
    let current = [200.0, 180.0];

    primary(&core, cad_app::interaction::PointerPhase::Down, anchor);
    primary(&core, cad_app::interaction::PointerPhase::Move, current);
    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "the 2D primary drag starts a box gesture"
    );

    // A draw capture becomes active while the box gesture is still live.
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();

    // The next move must end the box gesture and stop drawing the band.
    primary(&core, cad_app::interaction::PointerPhase::Move, current);
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "a capture owns the pointer: the live box gesture is ended"
    );
    assert!(
        core.render.overlays().selection_box.is_none(),
        "no stale frozen band is rendered after the capture takes over"
    );

    // The release must not dispatch a SelectInBox the tool layer would refuse.
    let status_before = core.last_status();
    primary(&core, cad_app::interaction::PointerPhase::Up, current);
    assert!(core.navigation.box_gesture.borrow().is_none());
    assert!(core.render.overlays().selection_box.is_none());
    assert!(
        selected_refs(&core).is_empty(),
        "no box selection runs under a capture"
    );
    assert_eq!(
        core.last_status(),
        status_before,
        "the release dispatches no refused SelectInBox (which would grade `command.failed`)"
    );
}

// --- a modal pan owns the pointer -------------------------------------------

#[test]
fn a_begin_draw_ends_a_live_box_gesture_without_an_intervening_move() {
    let core = core();
    let anchor = [100.0, 100.0];
    let current = [200.0, 180.0];
    primary(&core, cad_app::interaction::PointerPhase::Down, anchor);
    primary(&core, cad_app::interaction::PointerPhase::Move, current);
    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "the 2D primary drag starts a box gesture"
    );

    // The capture starts through the real interaction path, with no pointer move
    // in between; the live gesture and its band must end with it.
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::BeginDraw(DrawToolKind::Line)),
    )
    .unwrap();
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "starting a capture ends the live box gesture"
    );
    assert!(
        core.render.overlays().selection_box.is_none(),
        "the band is cleared immediately, not on the next move"
    );

    // The subsequent release must not dispatch a refused SelectInBox.
    let status_before = core.last_status();
    primary(&core, cad_app::interaction::PointerPhase::Up, current);
    assert!(selected_refs(&core).is_empty());
    assert_eq!(core.last_status(), status_before);
}

#[test]
fn a_modal_pan_does_not_start_a_box_on_a_primary_drag() {
    let core = core();
    // A typed `BeginPan` enters the modal pan, which owns the pointer.
    core.controller.borrow_mut().session.begin_pan();

    let anchor = [100.0, 100.0];
    let current = [160.0, 140.0];
    primary(&core, cad_app::interaction::PointerPhase::Down, anchor);
    primary(&core, cad_app::interaction::PointerPhase::Move, current);
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "a modal pan owns the pointer: a 2D primary drag starts no box"
    );
    assert!(core.render.overlays().selection_box.is_none());

    primary(&core, cad_app::interaction::PointerPhase::Up, current);
    assert!(
        selected_refs(&core).is_empty(),
        "a modal pan's primary drag never selects a box"
    );
}

// --- a surviving contact must not inherit the released one ------------------

#[test]
fn a_partially_released_contact_does_not_inherit_button_or_modifiers() {
    let core = core();
    let mouse_kind = cad_app::interaction::PointerKind::Mouse;
    let touch_kind = cad_app::interaction::PointerKind::Touch;
    // Contact 1 goes down with Primary + Shift; those decide box-vs-pan and
    // Replace-vs-Add.
    dispatch_pointer(
        &core,
        contact(
            &core,
            1,
            mouse_kind,
            cad_app::interaction::PointerPhase::Down,
            Some(PointerButton::Primary),
            [100.0, 100.0],
            shift_modifiers(true),
        ),
    );
    // Contact 2 (touch) goes down: the gesture becomes multi-contact navigation.
    dispatch_pointer(
        &core,
        contact(
            &core,
            2,
            touch_kind,
            cad_app::interaction::PointerPhase::Down,
            None,
            [200.0, 200.0],
            cad_app::interaction::Modifiers::default(),
        ),
    );
    // Contact 1 (the one that recorded Primary + Shift) is released.
    dispatch_pointer(
        &core,
        contact(
            &core,
            1,
            mouse_kind,
            cad_app::interaction::PointerPhase::Up,
            Some(PointerButton::Primary),
            [100.0, 100.0],
            shift_modifiers(true),
        ),
    );

    assert!(
        core.navigation.active.borrow().contains(&PointerId(2)),
        "contact 2 survives the partial release"
    );
    assert!(
        core.navigation.button.get().is_none(),
        "the surviving contact must not inherit the released contact's button"
    );
    assert!(
        core.navigation.press_modifiers.get().is_none(),
        "the surviving contact must not inherit the released contact's Shift"
    );

    // Release the survivor to leave a clean gesture state.
    dispatch_pointer(
        &core,
        contact(
            &core,
            2,
            touch_kind,
            cad_app::interaction::PointerPhase::Up,
            None,
            [200.0, 200.0],
            cad_app::interaction::Modifiers::default(),
        ),
    );
    assert!(core.navigation.active.borrow().is_empty());
}

// --- a large CROSSING drag stays closed within the cap ----------------------

#[test]
fn a_large_crossing_drag_stays_closed_within_the_segment_cap() {
    let core = core();
    // Right to left is CROSSING. The rectangle's logical perimeter is
    // 2 * (2000 + 1500) = 7000 px, far beyond the old fixed 5+5 px pattern's
    // ~5120 px reach, which used to drop trailing edges and draw an open box.
    let anchor = [2000.0, 1500.0];
    let current = [0.0, 0.0];
    primary(&core, cad_app::interaction::PointerPhase::Down, anchor);
    primary(&core, cad_app::interaction::PointerPhase::Move, current);

    let overlay = core
        .render
        .overlays()
        .selection_box
        .expect("the live rubber band reaches the overlay inputs");
    assert!(
        overlay.crossing,
        "right-to-left is CROSSING (crossing true)"
    );
    assert!(
        !overlay.segments.is_empty(),
        "a CROSSING rectangle is tessellated into dashes"
    );
    assert!(
        overlay.segments.len() <= 512,
        "the segment count stays within the cap, got {}",
        overlay.segments.len()
    );
    // Every edge is covered and the loop closes, so the final segment's end is
    // exactly the first segment's start (the anchor corner): the rectangle is
    // drawn closed, never truncated.
    let first = overlay.segments.first().unwrap();
    let last = overlay.segments.last().unwrap();
    assert_eq!(
        last[1], first[0],
        "the last segment must return to the start point (closed rectangle)"
    );
}

// --- a degenerate rectangle refuses visibly, never silently ------------------

#[test]
fn a_degenerate_box_drag_dispatches_nothing_and_surfaces_the_refusal() {
    let core = core();
    let revision_before = drawing_revision(&core);
    // A zero-width vertical drag crosses the threshold, so it starts a box
    // gesture whose rectangle is degenerate (ADR 0014 B.6).
    let anchor = [100.0, 100.0];
    let current = [100.0, 200.0];
    primary(&core, cad_app::interaction::PointerPhase::Down, anchor);
    primary(&core, cad_app::interaction::PointerPhase::Move, current);
    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "the drag crosses the threshold and starts a box gesture"
    );
    primary(&core, cad_app::interaction::PointerPhase::Up, current);

    assert!(core.navigation.box_gesture.borrow().is_none());
    assert!(core.render.overlays().selection_box.is_none());
    assert!(
        selected_refs(&core).is_empty(),
        "a degenerate drag selects nothing"
    );
    assert_eq!(
        drawing_revision(&core),
        revision_before,
        "a degenerate drag writes nothing"
    );
    // The refusal is visible, not silent (ADR 0014 B.6: the command API refuses
    // the same rectangle with `InvalidInput`). A dispatched-and-refused
    // `SelectInBox` would have graded `command.failed`, so this also proves no
    // command was dispatched.
    let status = core
        .last_status()
        .expect("the refusal is visible, never silent");
    match status {
        StatusMessage::ExternalText(text) => assert!(
            text.contains("degenerate"),
            "the refusal names the reason, got {text}"
        ),
        other => panic!("expected an explicit refusal status, got {other:?}"),
    }
}
