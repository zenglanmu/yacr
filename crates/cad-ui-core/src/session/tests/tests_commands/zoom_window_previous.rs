//! `ZOOM WINDOW` and `ZOOM PREVIOUS` through the UI session.
//!
//! The two word arguments ride `CommandId::Zoom` with their own payloads, so these
//! contracts pin: the grammar (`WINDOW`/`PREVIOUS`, full words only), the arming
//! of the host capture without moving the camera, the 3D refusal by name, the
//! restore of a previous discrete view, and one end-to-end path — typed command,
//! real pointer drag, camera framed, `ZOOM PREVIOUS` back.

use super::super::super::*;
use super::super::test_support::*;
use crate::protocol::InputKind;

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// The projected 2D camera scale (world units per logical pixel).
fn camera_scale(session: &CadSession) -> f64 {
    session
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan")
        .scale
}

/// Assert the session recorded no undo/redo step (a view command is not an edit).
fn assert_no_history(session: &CadSession) {
    let c = session.core.controller.borrow();
    assert!(!c.application.can_undo(&c.document_id));
    assert!(!c.application.can_redo(&c.document_id));
}

/// One primary press/drag/release over the canvas.
fn drag(session: &CadSession, from: [f64; 2], to: [f64; 2]) {
    session
        .handle_input(&pointer_at(InputKind::PointerDown, 1, from, None))
        .expect("the press is admitted");
    session
        .handle_input(&pointer_at(InputKind::PointerMove, 2, to, None))
        .expect("the move is admitted");
    session
        .handle_input(&pointer_at(InputKind::PointerUp, 3, to, None))
        .expect("the release is admitted");
}

#[test]
fn the_two_word_arguments_are_part_of_the_zoom_grammar() {
    assert_eq!(parse_zoom_argument("WINDOW"), Some(ZoomArgument::Window));
    assert_eq!(parse_zoom_argument("window"), Some(ZoomArgument::Window));
    assert_eq!(
        parse_zoom_argument("  Window  "),
        Some(ZoomArgument::Window)
    );
    assert_eq!(
        parse_zoom_argument("PREVIOUS"),
        Some(ZoomArgument::Previous)
    );
    assert_eq!(
        parse_zoom_argument("previous"),
        Some(ZoomArgument::Previous)
    );

    // Full words only: the router never guesses at an abbreviation.
    for refused in ["W", "P", "PREV", "WINDOW 1", "PREVIOUS 2"] {
        assert_eq!(parse_zoom_argument(refused), None, "{refused:?}");
    }

    assert_eq!(
        typed_command_argument("ZOOM WINDOW"),
        Some(("ZOOM", "WINDOW"))
    );
    assert_eq!(
        typed_command_argument("zoom previous"),
        Some(("ZOOM", "previous"))
    );
    // `ZOOM EXTENTS` keeps its pre-existing alias onto FIT.
    assert_eq!(canonical_command_name("ZOOM EXTENTS"), Some("FIT"));
}

#[test]
fn an_out_of_grammar_argument_names_the_new_words() {
    let session = fresh_session();
    let error = session
        .command("ZOOM 0")
        .expect_err("ZOOM 0 is still refused");
    for word in ["factor", "ALL", "EXTENTS", "WINDOW", "PREVIOUS"] {
        assert!(
            error.contains(word),
            "the refusal must name {word}: {error}"
        );
    }
    assert!(
        error.contains("ZOOM 0"),
        "the typed line is echoed: {error}"
    );
}

#[test]
fn zoom_window_arms_the_capture_without_moving_the_camera() {
    let session = fresh_session();
    let base = camera_scale(&session);

    session
        .command("ZOOM WINDOW")
        .expect("ZOOM WINDOW is admitted in 2D");
    assert_eq!(
        camera_scale(&session),
        base,
        "arming a capture must not move the camera"
    );
    assert!(
        session.core.zoom_window_armed(),
        "the host capture is armed and waits for the second corner"
    );
    assert_eq!(session.snapshot(None, None, 0).error, None);
    assert_no_history(&session);
}

#[test]
fn zoom_window_refuses_a_three_d_view_by_name() {
    let session = fresh_session();
    session
        .command("SWITCH2D3D")
        .expect("SWITCH2D3D is admitted");
    assert_eq!(
        session.snapshot(None, None, 0).camera,
        None,
        "a 3D view has no 2D camera block"
    );

    let error = session
        .command("ZOOM WINDOW")
        .expect_err("ZOOM WINDOW is 2D-only");
    assert!(
        error.contains("zoom.window.requires_2d"),
        "the refusal is named: {error}"
    );
    assert_eq!(
        session.snapshot(None, None, 0).error.as_deref(),
        Some(error.as_str()),
        "the refusal is visible in cad_status.error"
    );
    assert!(
        !session.core.zoom_window_armed(),
        "a refused arm must not leave a dead capture behind"
    );
}

#[test]
fn zoom_previous_restores_the_view_before_a_zoom_step() {
    let session = fresh_session();
    let base = camera_scale(&session);
    session.command("ZOOM 2").expect("ZOOM 2 is admitted");
    let zoomed = camera_scale(&session);
    assert!((zoomed - base / 2.0).abs() < 1e-9);

    session
        .command("ZOOM PREVIOUS")
        .expect("ZOOM PREVIOUS is admitted");
    assert!(
        (camera_scale(&session) - base).abs() < 1e-9,
        "PREVIOUS restores the framing before the zoom step"
    );
    assert_no_history(&session);

    // Walking past the recorded history is a non-error no-op, never a refusal.
    session
        .command("ZOOM PREVIOUS")
        .expect("the empty case is not an error");
    assert_eq!(
        session.snapshot(None, None, 0).error,
        None,
        "an empty ZOOM PREVIOUS reports no error"
    );
}

#[test]
fn a_typed_window_capture_frames_the_drag_and_previous_restores_it() {
    let session = fresh_session();
    let base = session
        .snapshot(None, None, 0)
        .camera
        .expect("the demo view is a 2D plan");
    session
        .command("ZOOM WINDOW")
        .expect("ZOOM WINDOW starts the capture");

    // A right-to-left, bottom-to-top drag over the middle half of the canvas.
    drag(&session, [600.0, 450.0], [200.0, 150.0]);

    let windowed = camera_scale(&session);
    assert!(
        (windowed - base.scale * 0.5).abs() < 1e-9,
        "the drag frames exactly half the canvas: {} -> {windowed}",
        base.scale
    );
    assert!(
        !session.core.zoom_window_armed(),
        "the one-shot capture is consumed by the completed drag"
    );
    let camera = session
        .snapshot(None, None, 0)
        .camera
        .expect("the window keeps the 2D plan");
    assert!(
        (camera.center[0] - base.center[0]).abs() < 1e-9
            && (camera.center[1] - base.center[1]).abs() < 1e-9,
        "a rect symmetric about the canvas centre keeps the viewport centre: {camera:?}"
    );
    assert_no_history(&session);

    session
        .command("ZOOM PREVIOUS")
        .expect("PREVIOUS is admitted");
    assert!(
        (camera_scale(&session) - base.scale).abs() < 1e-9,
        "PREVIOUS restores the pre-window framing"
    );
}

#[test]
fn the_window_drag_does_not_select_anything() {
    let session = fresh_session();
    let selection_before = session.snapshot(None, None, 0).selection.count;
    session.command("ZOOM WINDOW").expect("armed");
    drag(&session, [0.0, 0.0], [800.0, 600.0]);
    assert_eq!(
        session.snapshot(None, None, 0).selection.count,
        selection_before,
        "a zoom window must not leak into the selection"
    );
}
