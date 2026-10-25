//! Host-side grip dragging: arming at press, the marker preview, the release
//! commit and the click-suppression latch.
//!
//! These are synthetic contracts over `HostCore` (no window, no GPU): they drive
//! exactly the request sequence the session emits (`Pointer Down`, `Pointer
//! Move`, `Pointer Up` followed by `SelectAt` for a click) and read the real
//! database, history, selection and `OverlayInputs` afterwards. They are not
//! render/visual acceptance evidence.
use super::*;

fn revision_now(core: &HostCore) -> u64 {
    core.request_context()
        .metrics_revision
        .expect("the host always reports a metrics revision")
}

/// One single-contact primary pointer phase at `position`.
fn primary(core: &HostCore, phase: cad_app::interaction::PointerPhase, position: [f64; 2]) {
    dispatch_pointer(core, mouse(1, phase, position, revision_now(core)));
}

/// The `SelectAt` the session emits as the second half of a click.
fn click_select_at(core: &HostCore, pixel: [f64; 2]) {
    dispatch_pointer(
        core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: pixel,
                metrics_revision: revision_now(core),
            },
            mode: SelectionMode::Replace,
        },
    );
}

fn drawing_revision(core: &HostCore) -> cad_domain::Revision {
    core.controller
        .borrow()
        .drawing()
        .expect("a drawing is open")
        .revision()
}

fn undo_depth(core: &HostCore) -> usize {
    let controller = core.controller.borrow();
    controller
        .application
        .history
        .get(&controller.document_id)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn selected_refs(core: &HostCore) -> Vec<cad_domain::SelectionRef> {
    core.controller.borrow().session.selection.refs().to_vec()
}

/// Create a model-space LINE and return its entity id.
fn create_line_at(core: &HostCore, start: Point3, end: Point3) -> cad_domain::EntityId {
    let mut controller = core.controller.borrow_mut();
    let outcome = controller
        .create_line(start, end)
        .expect("the LINE is created");
    cad_domain::EntityId(outcome.objects[0].0)
}

fn grip_markers(core: &HostCore) -> Vec<Point3> {
    core.render.overlays().grip_markers.clone()
}

/// The world point a canvas-local pixel resolves to (the same seam the host
/// uses: the viewport's `screen_to_world`, never a hand-rolled projection).
fn world_of(core: &HostCore, pixel: [f64; 2]) -> Point3 {
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    viewport
        .screen_to_world(pixel, viewport.logical_size)
        .expect("the pixel resolves onto the work plane")
}

fn assert_close(actual: Point3, expected: Point3, what: &str) {
    for (axis, (actual, expected)) in [
        ("x", (actual.x, expected.x)),
        ("y", (actual.y, expected.y)),
        ("z", (actual.z, expected.z)),
    ] {
        assert!(
            (actual - expected).abs() < 1e-9,
            "{what}: {axis} is {actual} but expected {expected}"
        );
    }
}

/// Whether the last status carries a `grip.*` refusal.
fn status_mentions_grip(core: &HostCore) -> bool {
    match core.last_status() {
        Some(StatusMessage::Catalog { key, args }) => {
            key.contains("grip")
                || args
                    .iter()
                    .any(|arg| arg.value.contains("grip.") || arg.value.contains("grip_"))
        }
        Some(StatusMessage::ExternalText(text)) => text.contains("grip."),
        None => false,
    }
}

// --- 1. press on a handle → drag → release commits one GripStretch -----------

#[test]
fn press_on_a_grip_arms_a_drag_and_release_commits_one_gripstretch() {
    let core = core();
    let (line, other) = add_two_lines(&core);
    core.controller
        .borrow_mut()
        .fit()
        .expect("the new lines fit");
    select_entity(&core, line);
    let (start, end) = line_endpoints(&core, line);
    let (other_start, other_end) = line_endpoints(&core, other);
    let press = screen_of(&core, start);
    // Well beyond the drag threshold, so this is a drag and not a click.
    let release = [
        press[0] + cad_app::DRAG_THRESHOLD_LOGICAL_PX + 40.0,
        press[1] - cad_app::DRAG_THRESHOLD_LOGICAL_PX - 25.0,
    ];
    let revision_before = drawing_revision(&core);
    let undo_before = undo_depth(&core);
    let selection_before = selected_refs(&core);
    // Drain the setup's published deltas so "overlay-only" below is about the
    // drag itself.
    EngineClient::take_updates(&core).expect("the setup state is publishable");

    primary(&core, cad_app::interaction::PointerPhase::Down, press);
    assert!(
        core.grip_drag_active(),
        "a press on a handle of the selection arms a drag"
    );
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "the grip won the press: no box gesture starts"
    );

    primary(&core, cad_app::interaction::PointerPhase::Move, release);
    assert!(
        core.grip_drag_active(),
        "the drag survives the move (serviced before the threshold gate)"
    );
    // The preview signal: the dragged marker's entry is the drag target.
    let expected_target = world_of(&core, release);
    let markers = grip_markers(&core);
    assert_eq!(
        markers.len(),
        3,
        "the selected LINE exposes two endpoints and a midpoint"
    );
    assert!(
        markers
            .iter()
            .any(|marker| (marker.x - expected_target.x).abs() < 1e-9
                && (marker.y - expected_target.y).abs() < 1e-9),
        "the moved marker must be at the drag target, got {markers:?}"
    );
    assert!(
        markers
            .iter()
            .any(|marker| marker.x == end.x && marker.y == end.y),
        "the untouched handle keeps its marker"
    );
    // A grip drag is an overlay-only change: no navigation/full delta is
    // published for the press/move stream.
    let updates = EngineClient::take_updates(&core).expect("the drag state is publishable");
    assert!(
        updates.navigation.is_none(),
        "a grip drag must not publish a navigation delta"
    );
    assert!(
        updates.full.is_none(),
        "a grip drag is overlay-only, not a full state change"
    );

    primary(&core, cad_app::interaction::PointerPhase::Up, release);
    assert!(!core.grip_drag_active(), "the release consumes the drag");

    // Exactly one handle moved, by the released point, in one transaction.
    let (new_start, new_end) = line_endpoints(&core, line);
    assert_close(
        new_start,
        Point3 {
            x: expected_target.x,
            y: expected_target.y,
            z: start.z,
        },
        "the dragged endpoint",
    );
    assert_eq!(new_end, end, "the other endpoint is bit-identical");
    assert_eq!(
        drawing_revision(&core),
        cad_domain::Revision(revision_before.0 + 1),
        "one committed transaction"
    );
    assert_eq!(
        undo_depth(&core),
        undo_before + 1,
        "one undo entry for one grip drag"
    );
    assert_eq!(
        line_endpoints(&core, other),
        (other_start, other_end),
        "no other entity was touched"
    );

    // The click the session emits right after the release is swallowed: a click
    // that lands on a grip must not also select.
    click_select_at(&core, release);
    assert_eq!(
        selected_refs(&core),
        selection_before,
        "the post-release click must not change the selection"
    );
}

// --- 2. press and release in place writes nothing ---------------------------

#[test]
fn a_press_on_a_grip_released_in_place_writes_nothing() {
    let core = core();
    let (line, _other) = add_two_lines(&core);
    core.controller
        .borrow_mut()
        .fit()
        .expect("the new lines fit");
    select_entity(&core, line);
    let (start, end) = line_endpoints(&core, line);
    let pixel = screen_of(&core, start);
    let revision_before = drawing_revision(&core);
    let undo_before = undo_depth(&core);
    let selection_before = selected_refs(&core);
    let status_before = core.last_status();
    EngineClient::take_updates(&core).expect("the setup state is publishable");

    primary(&core, cad_app::interaction::PointerPhase::Down, pixel);
    assert!(core.grip_drag_active(), "the press arms the drag");
    primary(&core, cad_app::interaction::PointerPhase::Up, pixel);
    assert!(!core.grip_drag_active());

    assert_eq!(
        line_endpoints(&core, line),
        (start, end),
        "a drag in place moves nothing"
    );
    assert_eq!(
        drawing_revision(&core),
        revision_before,
        "no transaction, so no revision bump"
    );
    assert_eq!(
        undo_depth(&core),
        undo_before,
        "no empty undo entry for a zero-displacement drag"
    );
    assert!(
        !status_mentions_grip(&core),
        "a zero-displacement release is a silent no-op, never a grip.* error: {:?}",
        core.last_status()
    );
    assert_eq!(
        core.last_status(),
        status_before,
        "the release grades no status of its own"
    );

    // The in-place click is swallowed as well: the press was on a grip.
    click_select_at(&core, pixel);
    assert_eq!(
        selected_refs(&core),
        selection_before,
        "a click on a grip must not select"
    );
}

// --- 3. the latch and the arbiter regressions -------------------------------

#[test]
fn a_later_click_that_does_not_land_on_a_grip_still_selects() {
    let core = core();
    // The demo document fits to a scale where a few world units are a fraction of
    // a pixel, so the two fixtures are placed far apart in world space (≈270
    // logical pixels) to make "not on a grip" unambiguous.
    let near = create_line_at(
        &core,
        Point3 {
            x: 1000.0,
            y: 1000.0,
            z: 0.0,
        },
        Point3 {
            x: 1001.0,
            y: 1000.0,
            z: 0.0,
        },
    );
    let far = create_line_at(
        &core,
        Point3 {
            x: 1000.0,
            y: 2400.0,
            z: 0.0,
        },
        Point3 {
            x: 1001.0,
            y: 2400.0,
            z: 0.0,
        },
    );
    core.controller
        .borrow_mut()
        .fit()
        .expect("the new lines fit");
    select_entity(&core, near);
    let (start, _end) = line_endpoints(&core, near);
    let press = screen_of(&core, start);
    let release = [press[0] + 50.0, press[1] + 20.0];

    // A real grip drag (no `SelectAt` follows a drag), which sets the latch.
    primary(&core, cad_app::interaction::PointerPhase::Down, press);
    assert!(core.grip_drag_active());
    primary(&core, cad_app::interaction::PointerPhase::Move, release);
    primary(&core, cad_app::interaction::PointerPhase::Up, release);
    assert!(!core.grip_drag_active());

    // The next press is not on a grip: its click must select normally, i.e. the
    // stale latch did not survive into this gesture.
    let (far_start, far_end) = line_endpoints(&core, far);
    let midpoint = Point3 {
        x: (far_start.x + far_end.x) * 0.5,
        y: (far_start.y + far_end.y) * 0.5,
        z: far_start.z,
    };
    let click = screen_of(&core, midpoint);
    let expected = picked_ref_at(&core, click);
    assert_eq!(
        expected.entity, far,
        "the click pixel picks the far fixture, not the dragged one"
    );
    primary(&core, cad_app::interaction::PointerPhase::Down, click);
    assert!(
        !core.grip_drag_active(),
        "this press really is away from every handle"
    );
    primary(&core, cad_app::interaction::PointerPhase::Up, click);
    click_select_at(&core, click);
    assert_eq!(
        selected_refs(&core),
        vec![expected.clone()],
        "a press not on a grip still selects the entity under it"
    );
}

#[test]
fn a_drag_from_empty_space_still_box_selects() {
    let core = core();
    let (line, _other) = add_two_lines(&core);
    core.controller
        .borrow_mut()
        .fit()
        .expect("the new lines fit");
    select_entity(&core, line);
    let anchor = [1.0, 1.0];
    assert!(
        picks_nothing(&core, anchor),
        "the chosen corner must really be empty space"
    );
    let current = [
        anchor[0] + cad_app::DRAG_THRESHOLD_LOGICAL_PX + 60.0,
        anchor[1] + cad_app::DRAG_THRESHOLD_LOGICAL_PX + 60.0,
    ];

    primary(&core, cad_app::interaction::PointerPhase::Down, anchor);
    assert!(
        !core.grip_drag_active(),
        "a press on empty space arms nothing"
    );
    primary(&core, cad_app::interaction::PointerPhase::Move, current);
    assert!(
        core.navigation.box_gesture.borrow().is_some(),
        "a primary drag from empty space is still a box selection"
    );
    assert!(!core.grip_drag_active(), "and it never arms a grip drag");
    primary(&core, cad_app::interaction::PointerPhase::Up, current);
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "the release finishes the box gesture"
    );
}
