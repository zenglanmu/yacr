//! The session drafting switches (`SNAP`, `ORTHO`) on a grip drag.
//!
//! The constraint rule is **not** re-implemented in this file: the host routes
//! the drag target through the same session helper the point-capture path uses
//! (`SessionState::constrained_point`), so these contracts pin the *wiring* —
//! which switches apply, in which order, against which origin, and that the
//! preview and the commit read one stored target.
//!
//! Every expectation is stated as an **exact** world point and compared with
//! `==`. That is deliberate: grid snap collapses the pointer's sub-unit error
//! onto a multiple of the spacing, so a fixture can name the committed point
//! literally instead of using a tolerance. The raw pointer point is asserted
//! *not* to be on the grid (and *not* to be axis-aligned), so a test cannot pass
//! by accident.
//!
//! These are synthetic contracts over `HostCore` (no window, no GPU), not
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

fn grip_markers(core: &HostCore) -> Vec<Point3> {
    core.render.overlays().grip_markers.clone()
}

fn selected_refs(core: &HostCore) -> Vec<cad_domain::SelectionRef> {
    core.controller.borrow().session.selection.refs().to_vec()
}

fn drawing_revision(core: &HostCore) -> cad_domain::Revision {
    core.controller
        .borrow()
        .drawing()
        .expect("a drawing is open")
        .revision()
}

/// The world point a canvas-local pixel resolves to, through the **same** seam
/// the host uses (`Viewport::screen_to_world`), never a hand-rolled projection.
fn world_of(core: &HostCore, pixel: [f64; 2]) -> Point3 {
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    viewport
        .screen_to_world(pixel, viewport.logical_size)
        .expect("the pixel resolves onto the work plane")
}

/// Create a model-space LINE and return its entity id.
fn create_line_at(core: &HostCore, start: Point3, end: Point3) -> cad_domain::EntityId {
    let mut controller = core.controller.borrow_mut();
    let outcome = controller
        .create_line(start, end)
        .expect("the LINE is created");
    cad_domain::EntityId(outcome.objects[0].0)
}

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// The shared fixture: one LINE from `(2, 3, 5)` to `(12, 3, 5)`, fitted and
/// selected, plus the pixels the contracts press and release at.
///
/// The drag is deliberately **diagonal** and its X component is the nearer axis,
/// so `ORTHO` must keep the original `y` and grid snap must round both axes.
/// `z = 5` proves a constraint never moves an elevation.
struct Fixture {
    line: cad_domain::EntityId,
    start: Point3,
    end: Point3,
    midpoint: Point3,
    press: [f64; 2],
    release: [f64; 2],
}

fn fixture(core: &HostCore) -> Fixture {
    let line = create_line_at(core, point(2.0, 3.0, 5.0), point(12.0, 3.0, 5.0));
    core.controller
        .borrow_mut()
        .fit()
        .expect("the new line fits");
    select_entity(core, line);
    let (start, end) = line_endpoints(core, line);
    Fixture {
        line,
        start,
        end,
        midpoint: point((start.x + end.x) * 0.5, (start.y + end.y) * 0.5, start.z),
        press: screen_of(core, start),
        release: screen_of(core, point(18.2, 4.4, 5.0)),
    }
}

/// Configure the session switches. Object snap is switched **off** on purpose:
/// it is a different switch that a grip drag does not consult, so leaving it on
/// would not prove anything.
fn switches(core: &HostCore, grid_snap: bool, ortho: bool, spacing: f64) {
    let mut controller = core.controller.borrow_mut();
    controller.set_snap_enabled(false);
    controller.set_snap_grid_enabled(grid_snap);
    controller.set_ortho_enabled(ortho);
    controller
        .set_snap_spacing(spacing)
        .expect("a finite positive spacing is a valid SNAPUNIT");
}

/// Press on the handle and drag once; return the armed target as the overlay
/// preview shows it.
///
/// The moved marker **is** the preview (`OverlayInputs::grip_markers`), so the
/// three grips of the selected LINE must read as the dragged target plus the two
/// untouched handles, and the discarded original start marker must be gone.
fn drag_to(core: &HostCore, fixture: &Fixture) -> Point3 {
    primary(
        core,
        cad_app::interaction::PointerPhase::Down,
        fixture.press,
    );
    assert!(
        core.grip_drag_active(),
        "the press lands on the selected LINE's start handle"
    );
    primary(
        core,
        cad_app::interaction::PointerPhase::Move,
        fixture.release,
    );
    let to = core
        .navigation
        .grip_drag
        .borrow()
        .as_ref()
        .expect("the drag is still armed after the move")
        .to;
    let markers = grip_markers(core);
    assert_eq!(
        markers.len(),
        3,
        "the selected LINE exposes two endpoints and a midpoint, got {markers:?}"
    );
    assert!(
        markers.contains(&to),
        "the dragged handle's marker is the drag preview, got {markers:?}"
    );
    assert!(
        !markers.contains(&fixture.start),
        "the original handle marker is replaced by the drag target, got {markers:?}"
    );
    assert!(
        markers.contains(&fixture.end),
        "the far handle keeps its marker"
    );
    assert!(
        markers.contains(&fixture.midpoint),
        "the midpoint handle keeps its marker"
    );
    to
}

fn release_and_commit(core: &HostCore, fixture: &Fixture) -> (Point3, Point3) {
    primary(
        core,
        cad_app::interaction::PointerPhase::Up,
        fixture.release,
    );
    assert!(!core.grip_drag_active(), "the release consumes the drag");
    line_endpoints(core, fixture.line)
}

// --- 1. ORTHO: the nearer axis only, the other coordinate bit-identical ------

#[test]
fn ortho_commits_the_nearer_axis_only_with_the_other_coordinate_bit_identical() {
    let core = core();
    let fixture = fixture(&core);
    switches(&core, false, true, 10.0);
    let raw = world_of(&core, fixture.release);
    // Guard the fixture: the drag really is diagonal and X really is nearer.
    assert_ne!(
        raw.y, fixture.start.y,
        "the raw pointer must not already be on the horizontal axis: {raw:?}"
    );
    assert!(
        (raw.x - fixture.start.x).abs() > (raw.y - fixture.start.y).abs(),
        "the X component must be the nearer axis for this fixture: {raw:?}"
    );

    let preview = drag_to(&core, &fixture);
    let (committed, other_end) = release_and_commit(&core, &fixture);

    // Horizontal ORTHO keeps the grip's ORIGINAL y (bit-identical) and the raw x.
    let expected = point(raw.x, fixture.start.y, fixture.start.z);
    assert_eq!(
        preview, expected,
        "the dragged marker must preview the ORTHO-constrained target"
    );
    assert_eq!(
        committed, expected,
        "the commit must use the same constrained target the preview showed"
    );
    assert_eq!(
        committed.y, fixture.start.y,
        "the constrained coordinate is the grip's original y, bit for bit"
    );
    assert_eq!(
        committed.x, raw.x,
        "the free axis keeps the raw pointer x, unsnapped (grid snap is off)"
    );
    assert_eq!(
        committed.z, fixture.start.z,
        "ORTHO is a 2D aid: it never moves an elevation"
    );
    assert_eq!(other_end, fixture.end, "the other endpoint is untouched");
}

#[test]
fn ortho_keeps_the_original_x_when_the_vertical_delta_is_nearer() {
    let core = core();
    let fixture = fixture(&core);
    switches(&core, false, true, 10.0);
    // A mostly-vertical drag from the same handle.
    let release = screen_of(&core, point(4.4, 18.2, 5.0));
    let raw = world_of(&core, release);
    assert!(
        (raw.y - fixture.start.y).abs() > (raw.x - fixture.start.x).abs(),
        "the Y component must be the nearer axis for this drag: {raw:?}"
    );

    primary(
        &core,
        cad_app::interaction::PointerPhase::Down,
        fixture.press,
    );
    primary(&core, cad_app::interaction::PointerPhase::Move, release);
    primary(&core, cad_app::interaction::PointerPhase::Up, release);

    let (committed, _other_end) = line_endpoints(&core, fixture.line);
    assert_eq!(
        committed,
        point(fixture.start.x, raw.y, fixture.start.z),
        "a nearer vertical delta keeps the grip's original x and the raw y"
    );
    assert_eq!(
        committed.x, fixture.start.x,
        "the constrained coordinate is the grip's original x, bit for bit"
    );
}

// --- 2. Grid SNAP -----------------------------------------------------------

#[test]
fn grid_snap_quantises_the_drag_target_to_the_session_spacing() {
    let core = core();
    let fixture = fixture(&core);
    let spacing = 10.0;
    switches(&core, true, false, spacing);
    let raw = world_of(&core, fixture.release);
    assert_ne!(
        (raw.x / spacing).fract(),
        0.0,
        "the fixture's raw x must NOT be on the grid: {raw:?}"
    );
    assert_ne!(
        raw.y, 0.0,
        "the fixture's raw y must NOT be on the grid: {raw:?}"
    );

    let preview = drag_to(&core, &fixture);
    let (committed, other_end) = release_and_commit(&core, &fixture);

    // 18.2 → 20 and 4.4 → 0 with a 10-unit spacing about the world origin.
    let expected = point(20.0, 0.0, fixture.start.z);
    assert_eq!(
        preview, expected,
        "the dragged marker must preview the grid-snapped target"
    );
    assert_eq!(
        committed, expected,
        "the commit must use the same snapped target the preview showed"
    );
    assert_eq!(
        committed.x % spacing,
        0.0,
        "the committed x is a multiple of SNAPUNIT"
    );
    assert_eq!(
        committed.y % spacing,
        0.0,
        "the committed y is a multiple of SNAPUNIT"
    );
    assert_eq!(
        committed.z, fixture.start.z,
        "grid snap is a 2D aid: it never moves an elevation"
    );
    assert_eq!(other_end, fixture.end, "the other endpoint is untouched");
}

// --- 3. Both switches: snap first, then ORTHO -------------------------------

#[test]
fn both_switches_apply_snap_then_ortho_to_the_raw_target() {
    let core = core();
    let fixture = fixture(&core);
    switches(&core, true, true, 10.0);
    let raw = world_of(&core, fixture.release);
    assert!(
        (raw.x - fixture.start.x).abs() > (raw.y - fixture.start.y).abs(),
        "the X component must be the nearer axis for this fixture: {raw:?}"
    );

    let preview = drag_to(&core, &fixture);
    let (committed, _other_end) = release_and_commit(&core, &fixture);

    // The capture path's documented order is grid snap, then ORTHO: (18.2, 4.4)
    // snaps to (20, 0), which ORTHO then constrains to the grip's original y.
    let snapped = point(20.0, 0.0, fixture.start.z);
    let expected = point(snapped.x, fixture.start.y, fixture.start.z);
    assert_eq!(
        preview, expected,
        "the preview is snap-then-ortho applied to the raw target"
    );
    assert_eq!(
        committed, expected,
        "the commit agrees with the preview (snap, then ORTHO about the grip's origin)"
    );
    assert_eq!(
        committed.y, fixture.start.y,
        "ORTHO still keeps the grip's original y bit for bit after snapping"
    );
    // The order is observable: ORTHO first would constrain y to 3 and grid snap
    // would then round that back to 0 — the wrong (20, 0) instead of (20, 3).
    assert_ne!(
        committed,
        point(20.0, 0.0, fixture.start.z),
        "the host must snap first and constrain second, like the capture path"
    );
}

// --- 4. Both switches off: the unconstrained regression guard --------------

#[test]
fn both_switches_off_leave_the_diagonal_target_unconstrained() {
    let core = core();
    let fixture = fixture(&core);
    switches(&core, false, false, 10.0);
    let raw = world_of(&core, fixture.release);
    assert_ne!(raw.y, fixture.start.y, "the drag is diagonal: {raw:?}");
    assert_ne!(
        (raw.x / 10.0).fract(),
        0.0,
        "and it is not grid-aligned either: {raw:?}"
    );

    let preview = drag_to(&core, &fixture);
    let (committed, _other_end) = release_and_commit(&core, &fixture);

    let expected = point(raw.x, raw.y, fixture.start.z);
    assert_eq!(
        preview, expected,
        "with both switches off the marker previews the raw pointer target"
    );
    assert_eq!(
        committed, expected,
        "with both switches off the commit is the raw pointer target, byte for byte"
    );
}

// --- 5. A constrained drag owns the pointer like an unconstrained one -------

#[test]
fn a_constrained_drag_still_does_not_navigate_and_still_swallows_the_click() {
    let core = core();
    let fixture = fixture(&core);
    switches(&core, true, true, 10.0);
    let camera_before = active_camera(&core);
    let revision_before = drawing_revision(&core);
    let selection_before = selected_refs(&core);
    // Drain the setup's published deltas so "overlay-only" below is about the
    // drag itself.
    EngineClient::take_updates(&core).expect("the setup state is publishable");

    primary(
        &core,
        cad_app::interaction::PointerPhase::Down,
        fixture.press,
    );
    assert!(core.grip_drag_active());
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "the grip wins the press: no box gesture starts"
    );
    primary(
        &core,
        cad_app::interaction::PointerPhase::Move,
        fixture.release,
    );
    let updates = EngineClient::take_updates(&core).expect("the drag state is publishable");
    assert!(
        updates.navigation.is_none(),
        "a constrained grip drag must not publish a navigation delta"
    );
    assert!(
        updates.full.is_none(),
        "a constrained grip drag is overlay-only, not a full state change"
    );

    primary(
        &core,
        cad_app::interaction::PointerPhase::Up,
        fixture.release,
    );
    assert!(!core.grip_drag_active());
    assert_eq!(
        active_camera(&core),
        camera_before,
        "committing a constrained drag never moves the camera"
    );
    assert_eq!(
        drawing_revision(&core),
        cad_domain::Revision(revision_before.0 + 1),
        "the constrained drag is one committed transaction"
    );

    // The click the session emits right after the release is swallowed.
    dispatch_pointer(
        &core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: fixture.release,
                metrics_revision: revision_now(&core),
            },
            mode: SelectionMode::Replace,
        },
    );
    assert_eq!(
        selected_refs(&core),
        selection_before,
        "the post-release click must not change the selection"
    );
}
