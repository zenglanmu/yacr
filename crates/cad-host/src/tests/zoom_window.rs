//! The `ZOOM WINDOW` capture: arming, the box-gesture path, refusals.
//!
//! The contract pinned here: the typed-funnel arm ([`HostCore::begin_zoom_window`])
//! refuses a 3D view / a busy pointer by name, an armed drag dispatches exactly
//! one `ZoomWindow` payload whose corners are canonicalised logical pixels, a
//! degenerate drag dispatches nothing, and the gesture never leaks into the
//! selection or into the document history. The wheel's continuous-zoom rule
//! (`ZOOM PREVIOUS` records nothing for a notch) is pinned here too, because the
//! distinction only exists in the host.
use super::*;

fn modifiers() -> cad_app::interaction::Modifiers {
    cad_app::interaction::Modifiers::default()
}

fn pointer_request(
    core: &HostCore,
    phase: cad_app::interaction::PointerPhase,
    position: [f64; 2],
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(1),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button: Some(PointerButton::Primary),
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: core.request_context().metrics_revision.unwrap(),
        },
        modifiers: modifiers(),
    }
}

fn press(core: &HostCore, phase: cad_app::interaction::PointerPhase, position: [f64; 2]) {
    dispatch_pointer(core, pointer_request(core, phase, position));
}

/// One complete primary drag: down at `from`, one move past the threshold at
/// `to`, up at `to`.
fn drag(core: &HostCore, from: [f64; 2], to: [f64; 2]) {
    press(core, cad_app::interaction::PointerPhase::Down, from);
    press(core, cad_app::interaction::PointerPhase::Move, to);
    press(core, cad_app::interaction::PointerPhase::Up, to);
}

fn session_camera(core: &HostCore) -> cad_app::Camera {
    let c = core.controller.borrow();
    c.application.workspace.viewports[&c.viewport_id].camera
}

/// The projected 2D camera: centre plus world units per logical pixel.
fn camera_2d(core: &HostCore) -> (cad_domain::Point3, f64) {
    let camera = session_camera(core);
    let cad_app::Projection::Orthographic { scale } = camera.projection else {
        panic!("the plan view is orthographic");
    };
    (camera.target, scale)
}

fn selected_refs(core: &HostCore) -> Vec<cad_domain::SelectionRef> {
    core.controller.borrow().session.selection.refs().to_vec()
}

fn zoom_steps(core: &HostCore) -> usize {
    let c = core.controller.borrow();
    c.application.workspace.viewports[&c.viewport_id].zoom_history_len()
}

fn revision(core: &HostCore) -> cad_domain::Revision {
    core.controller.borrow().drawing().unwrap().revision()
}

fn set_three_d(core: &HostCore) {
    let (viewport_id, camera) = {
        let c = core.controller.borrow();
        (
            c.viewport_id,
            c.application.workspace.viewports[&c.viewport_id].camera,
        )
    };
    core.controller
        .borrow_mut()
        .application
        .workspace
        .viewports
        .get_mut(&viewport_id)
        .unwrap()
        .view_mode = cad_app::ViewMode2d3d::ThreeD { saved_2d: camera };
}

#[test]
fn arming_refuses_a_three_d_view_by_name() {
    let core = core();
    set_three_d(&core);
    let error = core
        .begin_zoom_window()
        .expect_err("a 3D view cannot capture a window");
    assert!(
        error.to_string().contains("zoom.window.requires_2d"),
        "the refusal is named: {error}"
    );
    assert!(!core.zoom_window_armed(), "a refused arm arms nothing");
}

#[test]
fn arming_refuses_while_another_capture_owns_the_pointer() {
    let core = core();
    // Enter the real LINE capture through the same host entry the funnel uses.
    core.controller
        .borrow_mut()
        .begin_draw_tool(cad_app::DrawToolKind::Line)
        .unwrap();
    let error = core
        .begin_zoom_window()
        .expect_err("a live draw capture owns the drag");
    assert!(
        error.to_string().contains("zoom.window.capture_busy"),
        "the refusal is named: {error}"
    );
    assert!(!core.zoom_window_armed());
}

#[test]
fn an_armed_drag_dispatches_one_zoom_window_with_canonical_corners() {
    let core = core();
    core.begin_zoom_window().expect("a 2D window arms");
    assert!(core.zoom_window_armed());
    let (centre_before, scale_before) = camera_2d(&core);
    let selection_before = selected_refs(&core);
    let revision_before = revision(&core);
    let steps_before = zoom_steps(&core);

    // Right-to-left and bottom-to-top: the corners are deliberately NOT in
    // `[min, max]` order, and a selection box would have been a CROSSING one.
    let anchor = [600.0, 450.0];
    let opposite = [200.0, 150.0];
    drag(&core, anchor, opposite);

    let (centre_after, scale_after) = camera_2d(&core);
    // 800x600 canvas at scale 1.0: the rect is 400 x 300 world units, i.e. half
    // the width and half the height, so the exact window scale is 0.5.
    assert!(
        (scale_after - 0.5 * scale_before).abs() < 1e-9,
        "a window frames the rect exactly: {scale_before} -> {scale_after}"
    );
    assert_eq!(centre_after.x, centre_before.x);
    assert_eq!(centre_after.y, centre_before.y);

    assert_eq!(
        selected_refs(&core),
        selection_before,
        "a window drag must not leak into the selection"
    );
    assert_eq!(revision(&core), revision_before);
    assert!(!core.zoom_window_armed(), "the capture is one-shot");
    assert_eq!(
        zoom_steps(&core),
        steps_before + 1,
        "ZOOM WINDOW records one ZOOM PREVIOUS step"
    );
    assert!(!core
        .controller
        .borrow()
        .application
        .can_undo(&cad_domain::DocumentId(1)));
}

#[test]
fn a_degenerate_drag_dispatches_nothing_and_consumes_the_arm() {
    let core = core();
    core.begin_zoom_window().unwrap();
    let (centre_before, scale_before) = camera_2d(&core);
    let selection_before = selected_refs(&core);

    // 50 px wide but one logical pixel tall: the drag really is a gesture (it
    // crosses the drag threshold) yet it is under the 2 px window extent, so the
    // capture refuses it by name instead of dispatching a degenerate rectangle.
    drag(&core, [400.0, 300.0], [450.0, 301.0]);

    assert_eq!(camera_2d(&core).1, scale_before, "nothing zoomed");
    assert_eq!(camera_2d(&core).0, centre_before);
    assert_eq!(selected_refs(&core), selection_before);
    assert!(!core.zoom_window_armed(), "the one-shot arm is consumed");
    let status = core.last_status().expect("the refusal is reported");
    let text = format!("{status:?}");
    assert!(
        text.contains("zoom.window.degenerate_rect"),
        "the refusal is named: {text}"
    );
}

#[test]
fn a_click_while_armed_does_not_select_and_keeps_the_arm() {
    let core = core();
    let pixel = screen_of(&core, demo_circle_rim(&core));
    let picked = picked_ref_at(&core, pixel);
    core.begin_zoom_window().unwrap();
    {
        let mut controller = core.controller.borrow_mut();
        controller.session.selection = cad_app::SelectionSet::new();
    }

    // A press/release with no drag is a click (the session emits `SelectAt`).
    press(&core, cad_app::interaction::PointerPhase::Down, pixel);
    press(&core, cad_app::interaction::PointerPhase::Up, pixel);

    assert!(
        !selected_refs(&core).contains(&picked),
        "a click while ZOOM WINDOW waits for corners must not select"
    );
    assert!(
        core.zoom_window_armed(),
        "a click is not a corner: the capture stays armed"
    );
}

#[test]
fn the_window_band_is_a_solid_rectangle_even_right_to_left() {
    let core = core();
    core.begin_zoom_window().unwrap();
    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        [600.0, 450.0],
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        [200.0, 150.0],
    );
    let preview = core
        .box_selection_preview()
        .expect("the armed drag has a live band");
    assert!(!preview.crossing, "a zoom window is never a CROSSING band");
    assert!(!preview.segments.is_empty());
}

#[test]
fn after_the_capture_a_drag_selects_again() {
    let core = core();
    core.begin_zoom_window().unwrap();
    drag(&core, [600.0, 450.0], [200.0, 150.0]);
    let (_, scale_after_window) = camera_2d(&core);

    // The arm is gone: the next drag is an ordinary box selection and must not
    // zoom again.
    drag(&core, [100.0, 100.0], [700.0, 500.0]);
    assert_eq!(
        camera_2d(&core).1,
        scale_after_window,
        "a later drag must not zoom"
    );
}

#[test]
fn a_wheel_notch_is_continuous_navigation_and_records_no_previous_step() {
    let core = core();
    let steps_before = zoom_steps(&core);
    let scale_before = camera_2d(&core).1;
    EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::Scroll {
            delta_logical: [0.0, 120.0],
            position: PointInput::CanvasLocal {
                logical_position: [400.0, 300.0],
                metrics_revision: core.request_context().metrics_revision.unwrap(),
            },
            modifiers: modifiers(),
        }),
    )
    .unwrap();
    assert!(
        camera_2d(&core).1 < scale_before,
        "the wheel really zoomed in"
    );
    assert_eq!(
        zoom_steps(&core),
        steps_before,
        "a wheel notch must not be restorable by ZOOM PREVIOUS"
    );
}

#[test]
fn a_pan_records_no_previous_step_either() {
    let core = core();
    let steps_before = zoom_steps(&core);
    let camera_before = session_camera(&core);
    // A middle-button drag is the pan gesture (a primary drag boxes).
    let mut request = pointer_request(
        &core,
        cad_app::interaction::PointerPhase::Down,
        [400.0, 300.0],
    );
    if let InteractionRequest::Pointer { button, .. } = &mut request {
        *button = Some(PointerButton::Middle);
    }
    dispatch_pointer(&core, request);
    let mut request = pointer_request(
        &core,
        cad_app::interaction::PointerPhase::Move,
        [500.0, 380.0],
    );
    if let InteractionRequest::Pointer { button, .. } = &mut request {
        *button = Some(PointerButton::Middle);
    }
    dispatch_pointer(&core, request);
    let mut request = pointer_request(
        &core,
        cad_app::interaction::PointerPhase::Up,
        [500.0, 380.0],
    );
    if let InteractionRequest::Pointer { button, .. } = &mut request {
        *button = Some(PointerButton::Middle);
    }
    dispatch_pointer(&core, request);

    assert_ne!(
        session_camera(&core),
        camera_before,
        "the middle drag panned"
    );
    assert_eq!(
        zoom_steps(&core),
        steps_before,
        "a pan is continuous navigation: it records no ZOOM PREVIOUS step"
    );
}
