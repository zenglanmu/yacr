//! `ID` point-coordinate host contract: a snapped pick's provenance reaches the
//! typed command echo.
//!
//! `cad-app` pins the report itself (`tests::id_report`); this file pins the
//! **wiring** that no unit test on either side can see: the host resolves the
//! object snap at capture time, the session stores the provenance beside the
//! captured point, the command layer reports it and the host's command echo
//! (`commandLog`) carries the stable `SnapKind` token — not the diagnostic text.
//!
//! These are synthetic contracts over `HostCore` (no window, no GPU): they are
//! not render/visual acceptance evidence.

use super::*;

fn revision_now(core: &HostCore) -> u64 {
    core.request_context()
        .metrics_revision
        .expect("the host always reports a metrics revision")
}

/// The canvas-local logical pixel a world point projects to in the active view.
fn screen_of(core: &HostCore, world: Point3) -> [f64; 2] {
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    viewport
        .camera
        .world_to_screen(world, viewport.logical_size)
        .expect("the 2D plan view projects the world point")
}

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn dist(a: Point3, b: Point3) -> f64 {
    ((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y) + (a.z - b.z) * (a.z - b.z)).sqrt()
}

/// The object-snap aperture in world units, from the same two inputs the capture
/// path resolves with (used only to guard the fixture's distance).
fn aperture(core: &HostCore) -> f64 {
    let policy = core.tolerance_policy();
    let c = core.controller.borrow();
    let viewport = &c.application.workspace.viewports[&c.viewport_id];
    policy.interaction_logical_pixels * viewport.world_per_px()
}

/// A blank document with one model LINE `(0,0,0)->(10,0,0)`, fitted, with object
/// snap on and grid snap/ORTHO off.
fn fixture() -> HostCore {
    let core = core();
    core.controller
        .borrow_mut()
        .new_blank_document()
        .expect("a blank document is installed");
    core.note_document_replaced();
    core.controller
        .borrow_mut()
        .create_line(point(0.0, 0.0, 0.0), point(10.0, 0.0, 0.0))
        .expect("the LINE is created");
    core.controller.borrow_mut().fit().expect("the scene fits");
    {
        let mut controller = core.controller.borrow_mut();
        controller.set_snap_enabled(true);
        controller.set_snap_grid_enabled(false);
        controller.set_ortho_enabled(false);
    }
    core
}

/// Dispatch one interaction through the real host boundary.
fn interact(core: &HostCore, request: InteractionRequest) {
    dispatch_pointer(core, request);
}

/// The retained echo entries, oldest first.
fn entries(core: &HostCore) -> Vec<CommandLogEntry> {
    core.command_log_entries()
}

#[test]
fn a_snapped_id_pick_echoes_its_snap_kind_space_and_precision() {
    let core = fixture();
    // Just past the LINE's start endpoint, inside the aperture: the perpendicular
    // foot is rejected (its parameter is negative, i.e. beyond the start), so the
    // endpoint is the only candidate in reach.
    let pixel = screen_of(&core, point(-0.02, 0.06, 0.0));
    let raw = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        viewport
            .screen_to_world(pixel, viewport.logical_size)
            .expect("the pixel resolves onto the work plane")
    };
    assert!(
        raw.x < 0.0,
        "the fixture's pick must be beyond the start endpoint: {raw:?}"
    );
    let expected = point(0.0, 0.0, 0.0);
    assert!(
        dist(raw, expected) < aperture(&core),
        "the fixture's pick must be inside the object-snap aperture: {} vs {}",
        dist(raw, expected),
        aperture(&core)
    );
    assert_ne!(
        raw, expected,
        "the fixture must not pick the exact endpoint"
    );

    // The confirmation the host performs inside the interaction funnel restores
    // nothing here, so the fixture really exercises the capture path.
    core.record_command("ID");
    interact(
        &core,
        InteractionRequest::BeginMeasurement(cad_app::MeasurementToolKind::Id),
    );
    interact(
        &core,
        InteractionRequest::CapturePoint(PointInput::CanvasLocal {
            logical_position: pixel,
            metrics_revision: revision_now(&core),
        }),
    );

    let entries = entries(&core);
    // `ID` (admission), then the auto-completed confirm's `Result`.
    let result = entries
        .iter()
        .find(|entry| entry.point.is_some())
        .expect("the snapped pick produced a point echo");
    assert_eq!(result.kind, CommandLogKind::Result);
    assert!(
        result.measurement.is_none(),
        "an ID result never fabricates a measurement"
    );
    let echo = result.point.as_ref().unwrap();
    assert_eq!(
        echo.position,
        [expected.x, expected.y, expected.z],
        "the echoed coordinate is the snapped point, not the raw pixel point"
    );
    assert_eq!(echo.space, "model");
    assert_eq!(
        echo.snap,
        Some("endpoint"),
        "the resolved SnapKind token reaches the echo"
    );
    assert_eq!(echo.precision, Some("analytic"));
    assert_eq!(
        echo.unit, None,
        "the blank document has no unit assignment: drawing units stay null"
    );
    let pixel_distance = echo
        .pixel_distance
        .expect("a snapped pick carries its logical-pixel distance");
    assert!(
        pixel_distance.is_finite() && pixel_distance > 0.0,
        "the distance from the raw pick is real: {pixel_distance}"
    );
}

#[test]
fn an_unsnapped_id_pick_echoes_a_null_snap_and_the_raw_point() {
    let core = fixture();
    // Far from every candidate: object snap resolves nothing, so the report is
    // the raw pick with `snap: null` — never a fabricated provenance.
    let target = point(5.0, 4.0, 0.0);
    let pixel = screen_of(&core, target);
    let raw = {
        let c = core.controller.borrow();
        let viewport = &c.application.workspace.viewports[&c.viewport_id];
        viewport
            .screen_to_world(pixel, viewport.logical_size)
            .expect("the pixel resolves onto the work plane")
    };
    assert!(
        dist(raw, point(5.0, 0.0, 0.0)) > aperture(&core),
        "the fixture must keep the pick away from the LINE"
    );

    core.record_command("ID");
    interact(
        &core,
        InteractionRequest::BeginMeasurement(cad_app::MeasurementToolKind::Id),
    );
    interact(
        &core,
        InteractionRequest::CapturePoint(PointInput::CanvasLocal {
            logical_position: pixel,
            metrics_revision: revision_now(&core),
        }),
    );

    let echo = entries(&core)
        .into_iter()
        .find_map(|entry| entry.point)
        .expect("the bare pick produced a point echo");
    assert_eq!(echo.snap, None);
    assert_eq!(echo.position, [raw.x, raw.y, raw.z]);
    assert_eq!(echo.pixel_distance, None);
}

#[test]
fn id_writes_nothing_and_records_no_history_entry() {
    let core = fixture();
    let before = {
        let c = core.controller.borrow();
        let document = c.document_id;
        let drawing = c.drawing().expect("a drawing is open");
        let undo_depth = c
            .application
            .history
            .get(&document)
            .map(|history| history.undo_depth())
            .unwrap_or(0);
        (
            drawing.revision(),
            drawing.entity_count(),
            document,
            undo_depth,
        )
    };
    core.record_command("ID");
    interact(
        &core,
        InteractionRequest::BeginMeasurement(cad_app::MeasurementToolKind::Id),
    );
    interact(
        &core,
        InteractionRequest::CapturePoint(PointInput::CanvasLocal {
            logical_position: screen_of(&core, point(-0.02, 0.06, 0.0)),
            metrics_revision: revision_now(&core),
        }),
    );

    let c = core.controller.borrow();
    let drawing = c.drawing().expect("a drawing is open");
    assert_eq!(drawing.revision(), before.0, "no revision bump");
    assert_eq!(drawing.entity_count(), before.1, "no entity change");
    assert_eq!(
        c.application
            .history
            .get(&before.2)
            .map(|history| history.undo_depth())
            .unwrap_or(0),
        before.3,
        "a coordinate query records no undo entry"
    );
}
