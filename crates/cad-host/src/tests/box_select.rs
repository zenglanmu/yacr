//! Gesture arbitration and box-selection contracts (ADR 0014 decision B).
//!
//! The single winner per single-contact gesture is pinned here: a 2D primary
//! drag boxes, a middle drag pans in 2D and 3D, a 3D primary drag orbits, a
//! draw/measure capture owns the pointer, and a sub-threshold press+release is
//! still a click. The rubber band is a transient render overlay input and the
//! selection itself is computed by the real `SelectInBox` command.
use super::*;

/// Two synthetic entities plus the document, positioned so one is fully inside
/// the test rectangle and the other only crosses its border.
struct BoxFixture {
    document: cad_domain::DocumentId,
    inside: cad_domain::EntityId,
    crossing: cad_domain::EntityId,
}

/// A line fully inside the `(-40,-20)..(-12,-10)` world rectangle.
const INSIDE_LINE: [[f64; 2]; 2] = [[-30.0, -15.0], [-15.0, -15.0]];
/// A vertical line that only crosses the rectangle's lower/upper border.
const CROSSING_LINE: [[f64; 2]; 2] = [[-20.0, -25.0], [-20.0, 5.0]];
/// The world rectangle both directions drag over.
const BOX_MIN: (f64, f64) = (-40.0, -20.0);
const BOX_MAX: (f64, f64) = (-12.0, -10.0);

fn box_fixture(core: &HostCore) -> BoxFixture {
    let mut controller = core.controller.borrow_mut();
    let document = controller.document_id;
    let inside = controller
        .create_line(
            p(INSIDE_LINE[0][0], INSIDE_LINE[0][1]),
            p(INSIDE_LINE[1][0], INSIDE_LINE[1][1]),
        )
        .unwrap();
    let crossing = controller
        .create_line(
            p(CROSSING_LINE[0][0], CROSSING_LINE[0][1]),
            p(CROSSING_LINE[1][0], CROSSING_LINE[1][1]),
        )
        .unwrap();
    BoxFixture {
        document,
        inside: cad_domain::EntityId(inside.objects[0].0),
        crossing: cad_domain::EntityId(crossing.objects[0].0),
    }
}

fn modifiers(shift: bool) -> cad_app::interaction::Modifiers {
    cad_app::interaction::Modifiers {
        shift,
        ..cad_app::interaction::Modifiers::default()
    }
}

fn pointer_request(
    core: &HostCore,
    phase: cad_app::interaction::PointerPhase,
    button: PointerButton,
    position: [f64; 2],
    shift: bool,
) -> InteractionRequest {
    InteractionRequest::Pointer {
        id: PointerId(1),
        kind: cad_app::interaction::PointerKind::Mouse,
        phase,
        button: Some(button),
        position: PointInput::CanvasLocal {
            logical_position: position,
            metrics_revision: core.request_context().metrics_revision.unwrap(),
        },
        modifiers: modifiers(shift),
    }
}

fn press(
    core: &HostCore,
    phase: cad_app::interaction::PointerPhase,
    button: PointerButton,
    position: [f64; 2],
    shift: bool,
) {
    dispatch_pointer(core, pointer_request(core, phase, button, position, shift));
}

/// The screen corner for a world rectangle corner.
fn corner(core: &HostCore, world: (f64, f64)) -> [f64; 2] {
    screen_of(core, p(world.0, world.1))
}

fn selected_refs(core: &HostCore) -> Vec<cad_domain::SelectionRef> {
    core.controller.borrow().session.selection.refs().to_vec()
}

fn drawing_revision(core: &HostCore) -> cad_domain::Revision {
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

// --- 2D primary drag: box selection ----------------------------------------

#[test]
fn two_d_primary_drag_left_to_right_is_a_window_selection() {
    let core = core();
    let fixture = box_fixture(&core);
    let revision = drawing_revision(&core);
    let camera_before = active_camera(&core);
    // Left edge first: left-to-right is WINDOW.
    let anchor = corner(&core, BOX_MIN);
    let current = corner(&core, BOX_MAX);
    assert!(anchor[0] < current[0], "the fixture drags left to right");

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        anchor,
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        current,
        false,
    );

    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "a 2D primary drag must start a box gesture, not a pan"
    );
    assert_eq!(
        active_camera(&core),
        camera_before,
        "the camera must not move during a box drag"
    );
    assert!(
        selected_refs(&core).is_empty(),
        "nothing is selected until the box is released"
    );
    let overlay = core
        .render
        .overlays()
        .selection_box
        .expect("the live rubber band reaches the overlay inputs");
    assert!(
        !overlay.crossing,
        "left-to-right is WINDOW (crossing false)"
    );
    assert_eq!(
        overlay.segments.len(),
        4,
        "a WINDOW rectangle is four solid edges"
    );

    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Primary,
        current,
        false,
    );

    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "the gesture is cleared on release"
    );
    assert!(
        core.render.overlays().selection_box.is_none(),
        "no rubber band survives the release"
    );
    let refs = selected_refs(&core);
    let inside = cad_app::selection::model_ref(fixture.document, fixture.inside);
    let crossing = cad_app::selection::model_ref(fixture.document, fixture.crossing);
    assert!(
        refs.contains(&inside),
        "WINDOW selects an entity fully inside the rectangle"
    );
    assert!(
        !refs.contains(&crossing),
        "WINDOW ignores an entity that only crosses the border"
    );
    assert_eq!(
        drawing_revision(&core),
        revision,
        "a box selection is read-only: no ChangeSet, no revision, no history"
    );
}

#[test]
fn two_d_primary_drag_right_to_left_is_a_crossing_selection() {
    let core = core();
    let fixture = box_fixture(&core);
    let revision = drawing_revision(&core);
    // Right edge first: right-to-left is CROSSING.
    let anchor = corner(&core, BOX_MAX);
    let current = corner(&core, BOX_MIN);
    assert!(current[0] < anchor[0], "the fixture drags right to left");

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        anchor,
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        current,
        false,
    );

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

    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Primary,
        current,
        false,
    );

    let refs = selected_refs(&core);
    let inside = cad_app::selection::model_ref(fixture.document, fixture.inside);
    let crossing = cad_app::selection::model_ref(fixture.document, fixture.crossing);
    assert!(
        refs.contains(&inside),
        "CROSSING still selects a fully contained entity"
    );
    assert!(
        refs.contains(&crossing),
        "CROSSING selects an entity that crosses the border too"
    );
    assert_eq!(drawing_revision(&core), revision, "no ChangeSet/history");
}

// --- middle drag: pan, in 2D and 3D -----------------------------------------

#[test]
fn two_d_middle_drag_pans_and_dispatches_no_box() {
    let core = core();
    let _ = box_fixture(&core);
    let probe = [100.0, 100.0];
    let world_before = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        viewport
            .screen_to_world(probe, viewport.logical_size)
            .unwrap()
    };

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Middle,
        probe,
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Middle,
        [160.0, 100.0],
        false,
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "a middle drag must never start a box"
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Middle,
        [160.0, 100.0],
        false,
    );

    let world_after = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        viewport
            .screen_to_world(probe, viewport.logical_size)
            .unwrap()
    };
    assert_ne!(
        world_before, world_after,
        "a middle drag pans the world point under the pointer"
    );
    assert!(selected_refs(&core).is_empty(), "a pan selects nothing");
    assert!(core.render.overlays().selection_box.is_none());
}

#[test]
fn three_d_middle_drag_pans_and_dispatches_no_box() {
    let core = core();
    let _ = box_fixture(&core);
    set_three_d(&core);
    let camera_before = active_camera(&core);

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Middle,
        [100.0, 100.0],
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Middle,
        [160.0, 100.0],
        false,
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "middle drag is a pan in 3D too, never a box"
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Middle,
        [160.0, 100.0],
        false,
    );

    let camera_after = active_camera(&core);
    assert_ne!(camera_after, camera_before, "the 3D middle drag pans");
    assert_ne!(
        camera_after.target, camera_before.target,
        "pan translates the camera target"
    );
    assert!(core.render.overlays().selection_box.is_none());
}

// --- 3D primary drag: still orbits, never boxes -----------------------------

#[test]
fn three_d_primary_drag_orbits_and_never_boxes() {
    let core = core();
    let _ = box_fixture(&core);
    set_three_d(&core);
    let camera_before = active_camera(&core);
    let distance_before = camera_before.distance();

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        [100.0, 100.0],
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        [160.0, 120.0],
        false,
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "3D primary drag is an orbit, never a box"
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Primary,
        [160.0, 120.0],
        false,
    );

    let camera_after = active_camera(&core);
    assert_ne!(camera_after, camera_before, "3D primary drag still orbits");
    assert_eq!(
        camera_after.target, camera_before.target,
        "orbit preserves the camera target (a pan would translate it)"
    );
    assert!(
        (camera_after.distance() - distance_before).abs() < 1e-9,
        "orbit preserves the eye-target distance"
    );
    assert!(
        selected_refs(&core).is_empty(),
        "a 3D orbit never selects a box"
    );
    assert!(core.render.overlays().selection_box.is_none());
}

// --- sub-threshold press+release: still a single pick -----------------------

#[test]
fn primary_press_release_below_the_threshold_still_picks_a_point() {
    let core = core();
    let fixture = box_fixture(&core);
    let revision = drawing_revision(&core);
    // A pixel on the inside line's midpoint, jittered by less than the 4px
    // drag threshold.
    let pixel = screen_of(&core, p(-22.5, -15.0));
    let jitter = [pixel[0] + 2.0, pixel[1]];

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        pixel,
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        jitter,
        false,
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "a sub-threshold move must not start a box"
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Primary,
        jitter,
        false,
    );

    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "no box gesture survives a click"
    );
    assert_eq!(
        drawing_revision(&core),
        revision,
        "the pointer phases alone write nothing"
    );
    assert!(
        selected_refs(&core).is_empty(),
        "the pointer path itself does not select; the UI's click does"
    );

    // The UI's click then resolves a single point pick.
    dispatch_pointer(
        &core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: jitter,
                metrics_revision: core.request_context().metrics_revision.unwrap(),
            },
            mode: SelectionMode::Replace,
        },
    );
    let refs = selected_refs(&core);
    assert_eq!(refs.len(), 1, "a click selects exactly one object");
    assert_eq!(
        refs[0],
        cad_app::selection::model_ref(fixture.document, fixture.inside),
        "the click picks the entity under the cursor"
    );
}

// --- Shift merges, plain replaces -------------------------------------------

#[test]
fn shift_merges_the_box_selection_while_a_plain_drag_replaces() {
    // Shift held on the press: the previous selection survives the box.
    let merged = core();
    let fixture = box_fixture(&merged);
    select_entity(&merged, fixture.crossing);
    let anchor = corner(&merged, BOX_MIN);
    let current = corner(&merged, BOX_MAX);
    press(
        &merged,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        anchor,
        true,
    );
    press(
        &merged,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        current,
        true,
    );
    press(
        &merged,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Primary,
        current,
        true,
    );
    let refs = selected_refs(&merged);
    assert!(
        refs.contains(&cad_app::selection::model_ref(
            fixture.document,
            fixture.crossing
        )),
        "Shift merges: the previous selection survives"
    );
    assert!(
        refs.contains(&cad_app::selection::model_ref(
            fixture.document,
            fixture.inside
        )),
        "Shift adds the box matches to the previous selection"
    );

    // No Shift: the same drag replaces the previous selection.
    let replacement = core();
    let fixture = box_fixture(&replacement);
    select_entity(&replacement, fixture.crossing);
    let anchor = corner(&replacement, BOX_MIN);
    let current = corner(&replacement, BOX_MAX);
    press(
        &replacement,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        anchor,
        false,
    );
    press(
        &replacement,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        current,
        false,
    );
    press(
        &replacement,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Primary,
        current,
        false,
    );
    let refs = selected_refs(&replacement);
    assert!(
        refs.contains(&cad_app::selection::model_ref(
            fixture.document,
            fixture.inside
        )),
        "a plain box drag still selects its matches"
    );
    assert!(
        !refs.contains(&cad_app::selection::model_ref(
            fixture.document,
            fixture.crossing
        )),
        "without Shift the previous selection is replaced"
    );
}

// --- a capture owns the gesture; hover does not compete ---------------------

#[test]
fn a_draw_capture_owns_the_gesture_and_boxes_nothing() {
    let core = core();
    let _ = box_fixture(&core);
    core.controller
        .borrow_mut()
        .begin_draw_tool(DrawToolKind::Line)
        .unwrap();
    let anchor = corner(&core, BOX_MIN);
    let current = corner(&core, BOX_MAX);

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        anchor,
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        current,
        false,
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "an active draw capture owns the pointer: no box gesture"
    );
    assert!(core.render.overlays().selection_box.is_none());
    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        PointerButton::Primary,
        current,
        false,
    );

    assert!(
        matches!(core.controller.borrow().session.tool, ToolState::Drawing(_)),
        "the capture is untouched by the pointer phases"
    );
    assert!(
        selected_refs(&core).is_empty(),
        "no box selection runs under a capture"
    );
}

#[test]
fn a_box_gesture_suppresses_hover_and_tint() {
    let core = hover_core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let anchor = corner(&core, BOX_MIN);
    let current = corner(&core, BOX_MAX);

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        anchor,
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        current,
        false,
    );
    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "the drag is a live box gesture"
    );
    assert!(
        hovered_refs(&core).is_empty(),
        "hover must not compete with a live box gesture"
    );
    // A hover probe delivered anyway must not tint during the box.
    let queries_before = core.hover_query_count();
    dispatch_pointer(&core, hover_cursor(&core, current));
    assert_eq!(
        core.hover_query_count(),
        queries_before,
        "no hover pick runs while a box gesture owns the pointer"
    );
    assert!(hovered_refs(&core).is_empty());
}

// --- cancel clears the gesture and the preview ------------------------------

#[test]
fn pointer_cancel_clears_the_box_and_dispatches_nothing() {
    let core = core();
    let _ = box_fixture(&core);
    let revision = drawing_revision(&core);
    let anchor = corner(&core, BOX_MIN);
    let current = corner(&core, BOX_MAX);

    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        PointerButton::Primary,
        anchor,
        false,
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        PointerButton::Primary,
        current,
        false,
    );
    assert!(core.render.overlays().selection_box.is_some());

    dispatch_pointer(
        &core,
        InteractionRequest::Pointer {
            id: PointerId(1),
            kind: cad_app::interaction::PointerKind::Mouse,
            phase: cad_app::interaction::PointerPhase::Cancel,
            button: None,
            position: PointInput::CanvasLocal {
                logical_position: current,
                metrics_revision: core.request_context().metrics_revision.unwrap(),
            },
            modifiers: cad_app::interaction::Modifiers::default(),
        },
    );

    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "a cancel clears the box gesture"
    );
    assert!(
        core.render.overlays().selection_box.is_none(),
        "a cancel clears the rubber band"
    );
    assert!(
        selected_refs(&core).is_empty(),
        "a cancel dispatches no box selection"
    );
    assert_eq!(drawing_revision(&core), revision, "a cancel writes nothing");
}

// --- a canvas resize reported together with the press ------------------------

/// The pre-layout `.native-input` rect of the real GUI run.
const OLD_CANVAS: [f64; 2] = [626.0, 341.0];
/// The rect reported by the first pointer event after a panel closed.
const NEW_CANVAS: [f64; 2] = [926.0, 341.0];

/// A platform surface whose logical size the test can move, so the real
/// layout-reported metrics path (`HostCore::tick` -> `metrics`) can be driven.
struct MutableSurface(std::rc::Rc<std::cell::Cell<[f64; 2]>>);

impl cad_platform::SurfaceSource for MutableSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        let size = self.0.get();
        Some(SurfaceMetrics {
            logical: size,
            scale: 1.0,
            safe_insets: [0.0; 4],
        })
    }
}

fn plan_camera() -> cad_app::Camera {
    cad_app::Camera {
        eye: cad_domain::Point3 {
            x: 1.5,
            y: 0.5,
            z: 1000.0,
        },
        target: p(1.5, 0.5),
        up: p(0.0, 1.0),
        projection: cad_app::Projection::Orthographic { scale: 0.005032 },
    }
}

/// A core over the pre-resize canvas, four vertical lines at x = 0..3 spanning
/// y = 0..1, and the evidence run's plan camera (1.5, 0.5) at 0.005032
/// world units per logical pixel.
///
/// The demo document's own entities sit near x = 2000, far outside this view, so
/// the box result is exactly the four lines.
fn resize_core(size: std::rc::Rc<std::cell::Cell<[f64; 2]>>) -> HostCore {
    let platform = HostPlatform {
        files: std::rc::Rc::new(FakeFiles::default()),
        config_store: std::rc::Rc::new(FakeConfig),
        surface: std::rc::Rc::new(MutableSurface(size)),
        resources: std::rc::Rc::new(UnsupportedResources),
        capabilities: capabilities(true),
    };
    let mut opts = options();
    opts.size = OLD_CANVAS;
    let core = HostCore::new(opts, platform, RenderSessionHandle::default()).unwrap();
    core.tick().unwrap();
    {
        let mut c = core.controller.borrow_mut();
        for x in [0.0, 1.0, 2.0, 3.0] {
            c.create_line(p(x, 0.0), p(x, 1.0)).unwrap();
        }
        let id = c.viewport_id;
        let viewport = c
            .application
            .workspace
            .viewports
            .get_mut(&id)
            .expect("active viewport");
        viewport.camera = plan_camera();
    }
    core
}

/// A window drag whose *press* is reported in the same batch as the canvas
/// resize must still select.
///
/// This is the regression for the real GUI run: the first pointer event after
/// a layout change reports the new `.native-input` rect **and** the press
/// together, so the host applies the new canvas metrics in the same tick that
/// already admitted the press. That metrics change used to clear the just-made
/// press, and the following move then degenerated to a zero-area gesture: the
/// drag dispatched no `SelectInBox` at all (WINDOW read 0 on a rectangle that
/// fully contains the drawing). The canvas-local anchor is still a valid corner
/// after a resize, so the stationary press must survive it.
#[test]
fn a_window_drag_survives_a_canvas_resize_reported_with_the_press() {
    let size = std::rc::Rc::new(std::cell::Cell::new(OLD_CANVAS));
    let core = resize_core(size.clone());
    let initial_revision = core.request_context().metrics_revision.unwrap();
    assert_eq!(
        core.controller.borrow().application.workspace.viewports
            [&core.controller.borrow().viewport_id]
            .logical_size,
        OLD_CANVAS
    );

    // The resize and the press arrive together: 2% -> 98% of the new rect.
    size.set(NEW_CANVAS);
    let down = mouse(
        1,
        cad_app::interaction::PointerPhase::Down,
        [18.52, 6.82],
        initial_revision,
    );
    EngineClient::submit(
        &core,
        core.request_context(),
        EngineRequest::Interaction(down),
    )
    .unwrap();
    core.pump().unwrap();
    // The host tick then applies the reported canvas metrics.
    core.tick().unwrap();
    let revision = core.request_context().metrics_revision.unwrap();
    assert!(
        revision > initial_revision,
        "the resize advances the metrics revision"
    );

    let moved = mouse(
        1,
        cad_app::interaction::PointerPhase::Move,
        [907.48, 334.18],
        revision,
    );
    EngineClient::submit(
        &core,
        core.request_context(),
        EngineRequest::Interaction(moved),
    )
    .unwrap();
    core.pump().unwrap();
    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "the press survives the resize, so the move starts the box gesture"
    );

    let up = mouse(
        1,
        cad_app::interaction::PointerPhase::Up,
        [907.48, 334.18],
        revision,
    );
    EngineClient::submit(
        &core,
        core.request_context(),
        EngineRequest::Interaction(up),
    )
    .unwrap();
    core.pump().unwrap();

    assert_eq!(
        core.controller.borrow().session.selection.len(),
        4,
        "a near-full-canvas WINDOW must select the four lines it contains"
    );
}
