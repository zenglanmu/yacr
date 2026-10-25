//! `ORTHO` (F8) at the host request path.
//!
//! The constraint rule is a pure `cad-app` function; these contracts only prove
//! the host routes point captures through the one session capture path (so
//! `ORTHO` is applied there, after the host's own snap resolution) and that the
//! overlay input the renderer draws carries the constrained preview. No second
//! copy of the rule lives here.

use super::*;
use cad_app::DrawToolKind;

#[test]
fn ortho_constrains_a_captured_point_through_the_real_host_path() {
    let core = hover_core();
    {
        let mut controller = core.controller.borrow_mut();
        controller.set_snap_enabled(false);
        controller.begin_draw_tool(DrawToolKind::Line).unwrap();
    }
    dispatch_pointer(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(0.0, 0.0))),
    );
    core.command(
        cad_app::CommandId::ToggleOrtho,
        cad_app::CommandPayload::None,
    )
    .expect("ORTHO is a session/display command");
    dispatch_pointer(
        &core,
        InteractionRequest::CapturePoint(PointInput::World(p(10.0, 3.0))),
    );

    let preview = core
        .controller
        .borrow()
        .session
        .draw_preview()
        .expect("the Line capture still has a preview");
    assert_eq!(
        preview.points,
        vec![p(0.0, 0.0), p(10.0, 0.0)],
        "a host point capture funnels through `SessionState::capture_point`, where ORTHO applies"
    );
}

#[test]
fn the_render_overlay_preview_carries_the_constrained_cursor() {
    let core = hover_core();
    {
        let mut controller = core.controller.borrow_mut();
        controller.set_snap_enabled(false);
        controller.begin_draw_tool(DrawToolKind::Line).unwrap();
        controller.capture_tool_point(p(0.0, 0.0)).unwrap();
        controller.set_ortho_enabled(true);
        // The host's own cursor path stores the snapped/raw point; the session
        // preview projection is what applies ORTHO, so the rubber band the
        // renderer draws must be the constrained segment.
        controller.move_tool_cursor(p(10.0, 3.0)).unwrap();
    }
    core.sync_render().expect("the synthetic session syncs");

    let preview = core
        .render
        .overlays()
        .draw_preview
        .expect("the overlay input carries the live capture");
    assert_eq!(preview.points, vec![p(0.0, 0.0)]);
    assert_eq!(
        preview.cursor,
        Some(p(10.0, 0.0)),
        "the preview the renderer draws agrees with the constrained commit"
    );
}
