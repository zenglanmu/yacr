//! Object-snap marker overlay contracts (defect #2: screen-space sizing).
//!
//! These drive the real `HostCore` through the interaction surface (no UI, no
//! GPU) and observe the **producer contract** (`OverlayInputs`) plus the derived
//! marker size the scene assembly feeds the renderer.

use super::*;

/// The demo document's circle centre, a real `SnapKind::Center` candidate.
fn demo_circle_centre(core: &HostCore) -> Point3 {
    let drawing = core
        .controller
        .borrow()
        .drawing()
        .expect("the demo document");
    drawing
        .model_space()
        .iter()
        .find_map(|entity| match &entity.geometry {
            cad_domain::SemanticGeometry::Circle { center, .. } => Some(*center),
            _ => None,
        })
        .expect("the demo document has a circle")
}

/// A draw tool whose cursor snaps to a real candidate publishes a non-empty
/// object-snap hint overlay input, and the marker's world half-extent is the
/// fixed logical-pixel half-extent times the view scale — a screen size that
/// does not drift with zoom (defect #2).
#[test]
fn a_snapped_draw_cursor_publishes_a_screen_sized_snap_marker_input() {
    let core = core();
    core.controller.borrow_mut().fit().expect("the demo fits");
    let centre = demo_circle_centre(&core);
    core.controller
        .borrow_mut()
        .begin_draw_tool(cad_app::DrawToolKind::Line)
        .expect("LINE starts");

    dispatch_pointer(
        &core,
        InteractionRequest::MoveCursor(PointInput::World(centre)),
    );

    let overlays = core.render.overlays();
    assert!(
        !overlays.snap_hints_input.is_empty(),
        "the tool cursor snapped to the circle centre: the snap hint input must be fed"
    );
    let scale = overlays
        .view_scale
        .expect("a 2D view publishes the view scale");
    assert!(scale.is_finite() && scale > 0.0);
    // The renderer's marker half-extent is the fixed pixel size times the scale.
    let options = cad_app::render_scene::PreviewOptions::for_view_scale(overlays.view_scale);
    let expected = cad_app::render_scene::SNAP_MARKER_HALF_EXTENT_PIXELS * scale;
    assert!(
        (options.marker_size - expected).abs() < 1e-12,
        "marker half-extent must be pixels × scale: {} vs {expected}",
        options.marker_size
    );
}

/// A `LINE` endpoint directly under the tool cursor resolves as an `endpoint`
/// snap, so the evidence driver's world-targeted click really does exercise the
/// marker (it is not a "no candidate" case).
#[test]
fn a_line_endpoint_under_the_cursor_resolves_as_an_endpoint_snap() {
    let core = core();
    {
        let mut controller = core.controller.borrow_mut();
        controller
            .create_line(p(0.0, 0.0), p(0.0, 1.0))
            .expect("the vertical line is created");
    }
    core.controller.borrow_mut().fit().expect("the demo fits");
    core.controller
        .borrow_mut()
        .begin_draw_tool(cad_app::DrawToolKind::Line)
        .expect("LINE starts");

    dispatch_pointer(
        &core,
        InteractionRequest::MoveCursor(PointInput::World(p(0.0, 0.0))),
    );

    let hints = core.render.overlays().snap_hints_input;
    assert!(
        !hints.is_empty(),
        "a LINE endpoint under the cursor must resolve a snap candidate"
    );
    assert_eq!(
        hints[0].kind.as_str(),
        "endpoint",
        "the endpoint marker kind is the AutoCAD ENDPOINT snap"
    );
}
