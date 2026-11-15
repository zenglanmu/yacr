//! The STRETCH **crossing polygon/lasso** window on the real host gesture path.
//!
//! The contract pinned here: a STRETCH capture started in polygon mode collects
//! its boundary as successive clicks (closed by `CONFIRM` or by a click on its
//! first vertex), a capture started in lasso mode hands its freehand **drag** path
//! over as the closed ring, both reach the existing `StretchPolygon` payload
//! through the ordinary tool confirmation, and every refused/aborted capture
//! writes nothing and leaves the document and history untouched. The rectangle
//! window keeps the four-point click capture (`zoom_window.rs`/`cad-app`'s own
//! contracts pin the rest of the gesture arbitration).
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

/// Start a STRETCH capture in `mode` through the one parameter channel the UI's
/// capture-mode chooser uses, with object snap off so the captures below are the
/// exact points the test names.
fn begin_stretch(core: &HostCore, mode: f64) {
    let mut controller = core.controller.borrow_mut();
    controller
        .session
        .set_tool_parameter(
            cad_app::ToolParameter::StretchWindow,
            cad_app::ToolValue::Number(mode),
        )
        .unwrap();
    controller
        .begin_draw_tool(cad_app::DrawToolKind::Stretch)
        .unwrap();
    controller.set_snap_enabled(false);
}

/// One world point click through the ordinary capture request.
fn capture_point(core: &HostCore, world: Point3) {
    EngineClient::dispatch(
        core,
        EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(world))),
    )
    .unwrap();
}

/// `CONFIRM` through the ordinary interaction request.
fn confirm(core: &HostCore) {
    EngineClient::dispatch(
        core,
        EngineRequest::Interaction(InteractionRequest::Confirm),
    )
    .unwrap();
}

fn revision(core: &HostCore) -> cad_domain::Revision {
    core.controller.borrow().drawing().unwrap().revision()
}

fn undo_depth(core: &HostCore) -> usize {
    let c = core.controller.borrow();
    c.application
        .history
        .get(&c.document_id)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

fn window_step(core: &HostCore) -> Option<cad_app::StretchWindowStep> {
    core.controller.borrow().session.draw_window_step()
}

fn ring_len(core: &HostCore) -> usize {
    core.controller.borrow().session.draw_window_ring_len()
}

/// A freehand drag around the room's `(0,0)` corner: down on the lower-left,
/// four moves around a 60 × 60 logical-pixel square, and the release back on the
/// start point (the lasso's closing edge).
fn lasso_around_origin(core: &HostCore) {
    let origin = screen_of(core, p(0.0, 0.0));
    let lower_left = [origin[0] - 30.0, origin[1] + 30.0];
    press(core, cad_app::interaction::PointerPhase::Down, lower_left);
    for offset in [[30.0, 0.0], [30.0, -30.0], [0.0, -30.0], [-30.0, 0.0]] {
        press(
            core,
            cad_app::interaction::PointerPhase::Move,
            [
                lower_left[0] + offset[0] * 2.0,
                lower_left[1] + offset[1] * 2.0,
            ],
        );
    }
    press(core, cad_app::interaction::PointerPhase::Up, lower_left);
}

#[test]
fn a_freehand_drag_is_the_lasso_ring_of_the_running_stretch_capture() {
    let core = core();
    let revision_before = revision(&core);
    let selection_before = core.controller.borrow().session.selection.refs().to_vec();
    begin_stretch(&core, cad_app::STRETCH_WINDOW_LASSO);
    assert_eq!(
        window_step(&core),
        Some(cad_app::StretchWindowStep::LassoTrace),
        "the capture asks the host for a freehand lasso"
    );

    // A click is not a lasso: the capture refuses it by name and keeps waiting.
    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::CapturePoint(PointInput::World(p(
            0.0, 0.0,
        )))),
    )
    .expect_err("a click cannot define a lasso");
    let text = format!("{error:?}");
    assert!(
        text.contains("stretch.window.lasso_needs_drag"),
        "the refusal is named: {text}"
    );
    assert_eq!(ring_len(&core), 0);
    assert_eq!(
        window_step(&core),
        Some(cad_app::StretchWindowStep::LassoTrace)
    );

    lasso_around_origin(&core);

    assert_eq!(
        window_step(&core),
        None,
        "the release closed the ring: the window step is over"
    );
    assert!(
        ring_len(&core) >= 3,
        "the traced path became the ring ({} vertices)",
        ring_len(&core)
    );
    // The trail is gone with the gesture, and the closed ring is capture state
    // (no rubber band survives it).
    assert!(core.box_selection_preview().is_none());
    assert_eq!(revision(&core), revision_before, "capture writes nothing");
    assert_eq!(
        core.controller.borrow().session.selection.refs().to_vec(),
        selection_before,
        "a lasso drag must not leak into the selection"
    );
}

#[test]
fn a_lasso_window_stretches_with_one_transaction_and_one_undo_step() {
    let core = core();
    let revision_before = revision(&core);
    let undo_before = undo_depth(&core);
    begin_stretch(&core, cad_app::STRETCH_WINDOW_LASSO);
    lasso_around_origin(&core);

    // The two displacement picks follow the closed window as ordinary clicks.
    capture_point(&core, p(0.0, 0.0));
    capture_point(&core, p(0.0, 5.0));
    confirm(&core);

    assert_eq!(
        revision(&core).0,
        revision_before.0 + 1,
        "exactly one database transaction"
    );
    assert_eq!(undo_depth(&core), undo_before + 1, "one undo step");
    let drawing = core.controller.borrow().drawing().unwrap();
    // The room's two `(0,0)` endpoints moved; the rest of the geometry did not.
    match &drawing.entity(cad_domain::EntityId(1)).unwrap().geometry {
        cad_domain::SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, p(0.0, 5.0));
            assert_eq!(*end, p(4000.0, 0.0));
        }
        other => panic!("expected the room's bottom line, got {other:?}"),
    }
}

#[test]
fn polygon_vertices_are_clicks_and_the_first_vertex_closes_the_ring() {
    let core = core();
    let revision_before = revision(&core);
    begin_stretch(&core, cad_app::STRETCH_WINDOW_POLYGON);
    assert_eq!(
        window_step(&core),
        Some(cad_app::StretchWindowStep::PolygonVertex)
    );

    // A ring large enough on screen that "click the first vertex" is a real click
    // on it and not a pick-box coincidence (the default plan view is 1 px/unit).
    let vertices = [
        p(-20.0, -20.0),
        p(20.0, -20.0),
        p(20.0, 20.0),
        p(-20.0, 20.0),
    ];
    for vertex in vertices {
        capture_point(&core, vertex);
    }
    assert_eq!(ring_len(&core), 4);
    assert_eq!(
        window_step(&core),
        Some(cad_app::StretchWindowStep::PolygonVertex),
        "an open ring still accepts vertices"
    );

    // A drag in polygon mode is not a lasso: it adds no vertex and captures
    // nothing (the click path is the only vertex source there).
    let origin = screen_of(&core, p(0.0, 0.0));
    press(&core, cad_app::interaction::PointerPhase::Down, origin);
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        [origin[0] + 40.0, origin[1] + 40.0],
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Up,
        [origin[0] + 40.0, origin[1] + 40.0],
    );
    assert_eq!(ring_len(&core), 4, "a polygon drag is not a vertex source");

    // AutoCAD's rule: clicking the **first vertex** closes the ring, and that
    // click is not an extra vertex.
    capture_point(&core, p(-20.0, -20.0));
    assert_eq!(window_step(&core), None, "the click closed the ring");
    assert_eq!(ring_len(&core), 4);

    capture_point(&core, p(0.0, 0.0));
    capture_point(&core, p(0.0, 5.0));
    assert_eq!(revision(&core), revision_before, "capture writes nothing");
    confirm(&core);
    assert_eq!(revision(&core).0, revision_before.0 + 1);
    assert_eq!(undo_depth(&core), 1, "one undo step");
}

#[test]
fn confirm_closes_an_open_polygon_ring_instead_of_committing() {
    let core = core();
    let revision_before = revision(&core);
    begin_stretch(&core, cad_app::STRETCH_WINDOW_POLYGON);
    for vertex in [
        p(-20.0, -20.0),
        p(20.0, -20.0),
        p(20.0, 20.0),
        p(-20.0, 20.0),
    ] {
        capture_point(&core, vertex);
    }
    // `CONFIRM` at a vertex prompt closes the window; it does not commit a
    // stretch whose displacement points do not exist yet.
    confirm(&core);
    assert_eq!(window_step(&core), None);
    assert_eq!(revision(&core), revision_before, "no transaction");
    assert_eq!(undo_depth(&core), 0);
}

#[test]
fn a_ring_with_fewer_than_three_vertices_is_refused_by_name() {
    let core = core();
    let revision_before = revision(&core);
    begin_stretch(&core, cad_app::STRETCH_WINDOW_POLYGON);
    capture_point(&core, p(-1.0, -1.0));
    capture_point(&core, p(2.0, -1.0));
    // Closing a two-vertex ring cannot define a window: the refusal is named and
    // the ring stays open so the user can keep clicking.
    let error = EngineClient::dispatch(
        &core,
        EngineRequest::Interaction(InteractionRequest::Confirm),
    )
    .expect_err("two vertices cannot enclose anything");
    let text = format!("{error:?}");
    assert!(
        text.contains("stretch.window.too_few_vertices"),
        "the refusal is named: {text}"
    );
    assert_eq!(
        window_step(&core),
        Some(cad_app::StretchWindowStep::PolygonVertex)
    );
    assert_eq!(ring_len(&core), 2, "nothing was dropped");
    assert_eq!(revision(&core), revision_before);
    assert_eq!(undo_depth(&core), 0);
}

#[test]
fn a_click_without_a_drag_is_refused_as_a_degenerate_lasso() {
    let core = core();
    let revision_before = revision(&core);
    begin_stretch(&core, cad_app::STRETCH_WINDOW_LASSO);
    let origin = screen_of(&core, p(0.0, 0.0));
    // A press/release in place is a click, not a lasso.
    press(&core, cad_app::interaction::PointerPhase::Down, origin);
    press(&core, cad_app::interaction::PointerPhase::Up, origin);

    assert_eq!(ring_len(&core), 0, "nothing was captured");
    assert_eq!(
        window_step(&core),
        Some(cad_app::StretchWindowStep::LassoTrace),
        "the capture still waits for a real lasso"
    );
    let status = core.last_status().expect("the refusal is reported");
    let text = format!("{status:?}");
    assert!(
        text.contains("stretch.window.lasso_degenerate"),
        "the refusal is named: {text}"
    );
    assert_eq!(revision(&core), revision_before);
    assert_eq!(undo_depth(&core), 0);
}

#[test]
fn a_cancelled_lasso_changes_nothing() {
    let core = core();
    let revision_before = revision(&core);
    begin_stretch(&core, cad_app::STRETCH_WINDOW_LASSO);
    let origin = screen_of(&core, p(0.0, 0.0));
    press(
        &core,
        cad_app::interaction::PointerPhase::Down,
        [origin[0] - 20.0, origin[1] - 20.0],
    );
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        [origin[0] + 40.0, origin[1] + 40.0],
    );
    press(&core, cad_app::interaction::PointerPhase::Cancel, origin);

    assert_eq!(ring_len(&core), 0, "a cancel hands nothing over");
    // The existing pointer-cancel contract also ends the capture, so no half-open
    // tool survives the OS/browser cancel.
    assert_eq!(window_step(&core), None);
    assert_eq!(revision(&core), revision_before, "nothing was written");
    assert_eq!(undo_depth(&core), 0);
}

#[test]
fn the_live_lasso_trail_is_the_crossing_rubber_band() {
    let core = core();
    begin_stretch(&core, cad_app::STRETCH_WINDOW_LASSO);
    let origin = screen_of(&core, p(0.0, 0.0));
    let lower_left = [origin[0] - 30.0, origin[1] + 30.0];
    press(&core, cad_app::interaction::PointerPhase::Down, lower_left);
    press(
        &core,
        cad_app::interaction::PointerPhase::Move,
        [lower_left[0] + 60.0, lower_left[1]],
    );
    // The trail reaches the renderer through the one transient overlay input the
    // box-selection band already uses, painted as a CROSSING boundary.
    let preview = core
        .box_selection_preview()
        .expect("the traced lasso has a live band");
    assert!(preview.crossing, "a lasso is always a CROSSING window");
    assert_eq!(preview.segments.len(), 1, "one move, one segment");
    press(&core, cad_app::interaction::PointerPhase::Cancel, origin);
}
