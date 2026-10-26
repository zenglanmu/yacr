//! Object snap (OSNAP) on a grip drag.
//!
//! The rule is **not** re-implemented here and neither is it in the drag: the
//! host routes the drag target through the capture path's one resolver
//! (`cad_app`'s `HostController::resolve_snap`) and then through the capture
//! path's one constraint helper (`SessionState::constrained_point`). These
//! contracts pin the *wiring*: which candidates participate, which gate applies,
//! which aperture, the order of object snap / grid snap / ORTHO, the self-snap
//! question, and that the preview and the commit still read one stored target.
//!
//! Every snapped expectation is an **exact** world point compared with `==`: a
//! candidate point is stored geometry, so a fixture can name it literally. The
//! unsnapped expectations are the point the test itself resolves from the pixel
//! through the same `Viewport::screen_to_world` seam the host uses, and every
//! such test also guards the fixture (the raw point must differ from the
//! candidate, and the distance must be on the intended side of the aperture), so
//! no case can pass by accident.
//!
//! The document is blanked first: the demo drawing's geometry would contribute
//! its own candidates (and its 4000-unit extents would change the fitted
//! `world_per_px`, hence the aperture). These are synthetic contracts over
//! `HostCore` (no window, no GPU), not render/visual acceptance evidence.
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

fn dist(a: Point3, b: Point3) -> f64 {
    ((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y) + (a.z - b.z) * (a.z - b.z)).sqrt()
}

/// The object-snap aperture in world units, from the **same** two inputs the
/// capture path resolves with: the configured aperture in logical pixels and the
/// viewport's `world_per_px`. Used only to guard a fixture's distances (the
/// expectations themselves are exact points).
fn aperture(core: &HostCore) -> f64 {
    let policy = core.tolerance_policy();
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    policy.interaction_logical_pixels * viewport.world_per_px()
}

/// Configure the session switches. `osnap` is the object-snap gate, `grid_snap`
/// the F9 grid quantiser and `ortho` the F8 constraint.
fn switches(core: &HostCore, osnap: bool, grid_snap: bool, ortho: bool, spacing: f64) {
    let mut controller = core.controller.borrow_mut();
    controller.set_snap_enabled(osnap);
    controller.set_snap_grid_enabled(grid_snap);
    controller.set_ortho_enabled(ortho);
    controller
        .set_snap_spacing(spacing)
        .expect("a finite positive spacing is a valid SNAPUNIT");
}

/// The fixture: a blank document, the dragged LINE `(0,0,0)->(10,0,0)` selected
/// (so its three handles are drawn), an optional second LINE that provides the
/// object-snap candidates, and the fitted press pixel on the dragged start
/// handle.
///
/// `0..20 / 0..12` world extents on the fake 800×600 surface put the aperture at
/// roughly 0.3 world units, so the offsets below are unambiguous.
struct Scene {
    dragged: cad_domain::EntityId,
    /// The second entity whose handle is the object-snap candidate (if any).
    target: Option<cad_domain::EntityId>,
    grip: Point3,
    press: [f64; 2],
}

fn scene(core: &HostCore, target: Option<(Point3, Point3)>) -> Scene {
    core.controller
        .borrow_mut()
        .new_blank_document()
        .expect("a blank document is installed");
    core.note_document_replaced();
    let dragged = create_line_at(core, point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0));
    let target_entity = target.map(|(start, end)| create_line_at(core, start, end));
    core.controller.borrow_mut().fit().expect("the scene fits");
    select_entity(core, dragged);
    let grip = point(0.0, 0.0, 0.0);
    Scene {
        dragged,
        target: target_entity,
        grip,
        press: screen_of(core, grip),
    }
}

/// Press on the dragged handle, move once and return the armed target the
/// overlay preview shows (the moved grip marker **is** the preview).
fn drag_to(core: &HostCore, scene: &Scene, pixel: [f64; 2]) -> Point3 {
    primary(core, cad_app::interaction::PointerPhase::Down, scene.press);
    assert!(
        core.grip_drag_active(),
        "the press lands on the selected LINE's start handle"
    );
    primary(core, cad_app::interaction::PointerPhase::Move, pixel);
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
    to
}

fn release_at(core: &HostCore, pixel: [f64; 2]) {
    primary(core, cad_app::interaction::PointerPhase::Up, pixel);
    assert!(!core.grip_drag_active(), "the release consumes the drag");
}

// --- 1. a candidate inside the aperture replaces the raw target --------------

#[test]
fn an_osnap_candidate_replaces_the_raw_pointer_target() {
    let core = core();
    // The target entity: a vertical LINE whose start handle at (20, 4) is the
    // only candidate in reach of the drag below.
    let scene = scene(&core, Some((point(20.0, 4.0, 0.0), point(20.0, 12.0, 0.0))));
    switches(&core, true, false, false, 10.0);
    // The pointer sits just **outside** the target's start handle (below it), so
    // the segment's perpendicular-foot candidate is rejected (its parameter is
    // negative) and the endpoint is the nearest candidate.
    let move_pixel = screen_of(&core, point(20.1, 3.9, 0.0));
    let raw = world_of(&core, move_pixel);
    let expected = point(20.0, 4.0, 0.0);
    assert_ne!(
        raw, expected,
        "the fixture's raw pointer must not already be the candidate: {raw:?}"
    );
    assert!(
        dist(raw, expected) < aperture(&core),
        "the fixture's raw pointer must be inside the object-snap aperture: {} vs {}",
        dist(raw, expected),
        aperture(&core)
    );

    let preview = drag_to(&core, &scene, move_pixel);
    assert_eq!(
        preview, expected,
        "the drag target is the snapped candidate, not the raw pointer point"
    );
    let markers = grip_markers(&core);
    assert!(
        markers.contains(&expected),
        "the drag preview marker sits on the snapped point, got {markers:?}"
    );
    assert!(
        !markers.contains(&raw),
        "the drag preview marker is not left at the raw pointer point, got {markers:?}"
    );

    release_at(&core, move_pixel);
    let (committed, other) = line_endpoints(&core, scene.dragged);
    assert_eq!(
        committed, expected,
        "the committed handle is the snapped point the preview showed"
    );
    assert_eq!(
        other,
        point(10.0, 0.0, 0.0),
        "the other endpoint is untouched"
    );
}

// --- 2. OSNAP off: byte-identical to the pre-OSNAP behaviour ----------------

#[test]
fn with_object_snap_off_the_drag_commits_the_raw_pointer_target() {
    let core = core();
    let scene = scene(&core, Some((point(20.0, 4.0, 0.0), point(20.0, 12.0, 0.0))));
    // The same geometry and the same pixels as case 1; only the gate differs.
    switches(&core, false, false, false, 10.0);
    let move_pixel = screen_of(&core, point(20.1, 3.9, 0.0));
    let raw = world_of(&core, move_pixel);
    let snapped = point(20.0, 4.0, 0.0);
    assert_ne!(
        raw, snapped,
        "the raw pointer is not the candidate: {raw:?}"
    );

    let preview = drag_to(&core, &scene, move_pixel);
    assert_eq!(
        preview, raw,
        "with OSNAP off the target is the raw pointer point, byte for byte"
    );
    release_at(&core, move_pixel);
    let (committed, _other) = line_endpoints(&core, scene.dragged);
    assert_eq!(
        committed, raw,
        "with OSNAP off the commit is the raw pointer point"
    );
    assert_ne!(
        committed, snapped,
        "the OSNAP-off commit must differ from the snapped point case 1 commits"
    );
}

// --- 3. a candidate outside the aperture falls back -------------------------

#[test]
fn a_candidate_outside_the_aperture_falls_back_to_the_raw_target() {
    let core = core();
    let scene = scene(&core, Some((point(20.0, 4.0, 0.0), point(20.0, 12.0, 0.0))));
    switches(&core, true, false, false, 10.0);
    let tolerance = aperture(&core);
    // Twice the aperture away from the candidate, and still below it so the
    // segment's perpendicular foot stays out of the segment.
    let move_pixel = screen_of(&core, point(20.1, 4.0 - 2.0 * tolerance, 0.0));
    let raw = world_of(&core, move_pixel);
    let candidate = point(20.0, 4.0, 0.0);
    assert!(
        dist(raw, candidate) > tolerance,
        "the fixture's raw pointer must be outside the aperture: {} vs {tolerance}",
        dist(raw, candidate)
    );

    let preview = drag_to(&core, &scene, move_pixel);
    assert_eq!(
        preview, raw,
        "fall back to an unconstrained target: no candidate, no replacement"
    );
    release_at(&core, move_pixel);
    let (committed, _other) = line_endpoints(&core, scene.dragged);
    assert_eq!(
        committed, raw,
        "fall back to the unconstrained target: no candidate, no replacement"
    );
    assert_ne!(
        committed, candidate,
        "the out-of-aperture candidate must not be used"
    );
}

// --- 4. the documented order: object snap, grid snap, ORTHO ------------------

#[test]
fn object_snap_then_grid_then_ortho_is_the_documented_order() {
    let core = core();
    // The candidate's x straddles a grid boundary (spacing 10) while the pointer
    // sits just outside the candidate: the candidate quantises to (0, 10) while
    // the raw pointer quantises to (10, 10), so the two orders are
    // distinguishable.
    let scene = scene(
        &core,
        Some((point(4.99, 14.99, 0.0), point(4.99, 22.0, 0.0))),
    );
    switches(&core, true, true, true, 10.0);
    // Below the candidate (the target segment runs upwards), so the segment's
    // perpendicular-foot candidate is rejected (its parameter is negative).
    let move_pixel = screen_of(&core, point(5.01, 14.95, 0.0));
    let raw = world_of(&core, move_pixel);
    let candidate = point(4.99, 14.99, 0.0);
    assert!(
        dist(raw, candidate) < aperture(&core),
        "the fixture's raw pointer must be inside the aperture: {} vs {}",
        dist(raw, candidate),
        aperture(&core)
    );
    assert!(
        raw.y < candidate.y,
        "the raw pointer must be beyond the candidate along the segment: {raw:?}"
    );
    assert!(
        raw.x > 5.0,
        "the raw pointer's x must lie on the other side of the grid boundary: {raw:?}"
    );

    let preview = drag_to(&core, &scene, move_pixel);
    release_at(&core, move_pixel);
    let (committed, _other) = line_endpoints(&core, scene.dragged);

    // Capture-path order: object snap (4.99, 14.99) → grid snap (0, 10) → ORTHO
    // about the grip's origin (the nearer axis is vertical, so from.x = 0 is
    // kept).
    let expected = point(0.0, 10.0, 0.0);
    assert_eq!(
        preview, expected,
        "the preview is object snap, then grid snap, then ORTHO"
    );
    assert_eq!(
        committed, expected,
        "the commit agrees with the preview on the same single target"
    );
    // Grid snap runs AFTER the object snap, so it re-quantises the snapped point
    // and the handle lands on the grid rather than on the candidate. That is the
    // capture path's order, pinned here on purpose.
    assert_ne!(
        committed, candidate,
        "grid snap re-quantises the object-snapped point (documented order)"
    );
    // Grid-and-ORTHO applied to the raw pointer instead would land on (10, 0).
    assert_ne!(
        committed,
        point(10.0, 0.0, 0.0),
        "the grid must quantise the snapped point, not the raw pointer"
    );
    assert_eq!(
        committed.z, scene.grip.z,
        "no drafting switch moves an elevation"
    );
}

// --- 4b. a candidate's own elevation is never adopted -----------------------

#[test]
fn a_candidate_at_another_elevation_contributes_only_its_plan_position() {
    let core = core();
    // The target handle sits 0.1 above the dragged handle's plane: inside the
    // 3D probe distance (so it really is a candidate) but not the handle's
    // elevation.
    let scene = scene(&core, Some((point(20.0, 4.0, 0.1), point(20.0, 12.0, 0.1))));
    switches(&core, true, false, false, 10.0);
    let move_pixel = screen_of(&core, point(20.1, 3.9, 0.0));
    let raw = world_of(&core, move_pixel);
    let resolved = core
        .controller
        .borrow()
        .resolve_snap(raw)
        .expect("the snap query resolves")
        .expect("the elevated handle resolves as a candidate");
    assert_eq!(resolved.point, point(20.0, 4.0, 0.1));
    assert!(
        dist(raw, resolved.point) < aperture(&core),
        "the fixture's candidate must be inside the aperture: {} vs {}",
        dist(raw, resolved.point),
        aperture(&core)
    );

    let preview = drag_to(&core, &scene, move_pixel);
    release_at(&core, move_pixel);
    let (committed, _other) = line_endpoints(&core, scene.dragged);
    assert_eq!(
        preview,
        point(20.0, 4.0, 0.0),
        "the snapped target is the candidate's plan position with the handle's z"
    );
    assert_eq!(
        committed,
        point(20.0, 4.0, 0.0),
        "the commit keeps the handle's own elevation: it is not the candidate's"
    );
    assert_eq!(
        committed.z, scene.grip.z,
        "a plan-view drag never changes an elevation (3D grip editing is NOT IMPLEMENTED)"
    );
}

// --- 5. a candidate on the dragged entity itself is a normal candidate ------

#[test]
fn a_candidate_on_the_dragged_entity_itself_is_used_like_the_capture_path() {
    let core = core();
    // Only the dragged LINE exists, so the only candidates near the pointer are
    // its own handles.
    let scene = scene(&core, None);
    switches(&core, true, false, false, 10.0);
    // Just beyond the dragged start handle (x < 0), so the segment's
    // perpendicular foot is rejected and the own endpoint is the nearest
    // candidate.
    let move_pixel = screen_of(&core, point(-0.02, 0.02, 0.0));
    let raw = world_of(&core, move_pixel);
    assert!(
        raw.x < scene.grip.x,
        "the fixture's raw pointer must be beyond the handle: {raw:?}"
    );
    assert!(
        dist(raw, scene.grip) < aperture(&core),
        "the own handle must be inside the aperture: {} vs {}",
        dist(raw, scene.grip),
        aperture(&core)
    );

    // The shared rule, named directly: the capture path's resolver has **no**
    // self-exclusion (the candidate generator walks every model-space entity),
    // so the dragged entity's own endpoint is a normal candidate — and the drag
    // must agree with it rather than inventing a grip-only exclusion.
    let resolved = core
        .controller
        .borrow()
        .resolve_snap(raw)
        .expect("the snap query resolves")
        .expect("the own handle resolves as a candidate");
    assert_eq!(
        resolved.source.entity, scene.dragged,
        "the candidate comes from the dragged entity itself"
    );
    assert_eq!(resolved.kind.as_str(), "endpoint");
    assert_eq!(resolved.point, scene.grip);

    let revision_before = drawing_revision(&core);
    let preview = drag_to(&core, &scene, move_pixel);
    assert_eq!(
        preview, scene.grip,
        "the drag target snaps onto the dragged handle's own position"
    );
    release_at(&core, move_pixel);
    let (committed, other) = line_endpoints(&core, scene.dragged);
    assert_eq!(
        committed, scene.grip,
        "a drag in place is the model's zero-displacement no-op"
    );
    assert_eq!(other, point(10.0, 0.0, 0.0));
    assert_eq!(
        drawing_revision(&core),
        revision_before,
        "a drag that lands back on its own handle dispatches no command at all"
    );
}

// --- 6. a snapping drag still owns the pointer like any other drag ----------

#[test]
fn a_snapping_drag_still_does_not_navigate_and_still_swallows_the_click() {
    let core = core();
    let scene = scene(&core, Some((point(20.0, 4.0, 0.0), point(20.0, 12.0, 0.0))));
    switches(&core, true, false, false, 10.0);
    let move_pixel = screen_of(&core, point(20.1, 3.9, 0.0));
    // The post-release click lands on the **second** entity's rim: without the
    // swallow it would replace the selection, so the assertion below is a real
    // one (not a click that would have missed anyway).
    let click_pixel = screen_of(&core, point(20.0, 8.0, 0.0));
    let camera_before = active_camera(&core);
    let revision_before = drawing_revision(&core);
    let selection_before = selected_refs(&core);
    // Drain the setup's published deltas so "overlay-only" below is about the
    // drag itself.
    EngineClient::take_updates(&core).expect("the setup state is publishable");

    primary(&core, cad_app::interaction::PointerPhase::Down, scene.press);
    assert!(core.grip_drag_active());
    assert!(
        core.navigation.box_gesture.borrow().is_none(),
        "the grip wins the press: no box gesture starts"
    );
    primary(&core, cad_app::interaction::PointerPhase::Move, move_pixel);
    let updates = EngineClient::take_updates(&core).expect("the drag state is publishable");
    assert!(
        updates.navigation.is_none(),
        "a snapping grip drag must not publish a navigation delta"
    );
    assert!(
        updates.full.is_none(),
        "a snapping grip drag is overlay-only, not a full state change"
    );

    primary(&core, cad_app::interaction::PointerPhase::Up, move_pixel);
    assert!(!core.grip_drag_active());
    assert_eq!(
        active_camera(&core),
        camera_before,
        "committing a snapping drag never moves the camera"
    );
    assert_eq!(
        drawing_revision(&core),
        cad_domain::Revision(revision_before.0 + 1),
        "the snapping drag is one committed transaction"
    );

    // The click the session emits right after the release is swallowed.
    dispatch_pointer(
        &core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: click_pixel,
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

    // ... and the same pixel really is a live pick: the latch is one-shot, so the
    // very next click on the second entity's rim replaces the selection. Without
    // this, the assertion above could pass on a click that would have missed.
    dispatch_pointer(
        &core,
        InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: click_pixel,
                metrics_revision: revision_now(&core),
            },
            mode: SelectionMode::Replace,
        },
    );
    let document = core.controller.borrow().document_id;
    assert_eq!(
        selected_refs(&core),
        vec![cad_app::selection::model_ref(
            document,
            scene.target.expect("the scene has a target entity")
        )],
        "the swallowed pixel is a real pick on the second entity"
    );
}
