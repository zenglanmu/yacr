//! `CAMERA` on the host boundary: the two captured points become the active
//! viewport's perspective camera, and the drawing is untouched. No GUI or GPU is
//! involved.
use super::*;

#[test]
fn the_position_camera_command_sets_the_viewport_camera() {
    let core = core();
    let eye = Point3 {
        x: 10.0,
        y: -20.0,
        z: 15.0,
    };
    let target = p(1.0, 2.0);
    let revision_before = core.controller.borrow().drawing().unwrap().revision();
    core.command(
        CommandId::PositionCamera,
        CommandPayload::PositionCamera { eye, target },
    )
    .expect("CAMERA is admitted");
    let controller = core.controller.borrow();
    let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
    assert_eq!(viewport.camera.eye, eye);
    assert_eq!(viewport.camera.target, target);
    assert_eq!(viewport.view_mode.kind(), cad_app::ProjectionKind::ThreeD);
    assert!(
        !viewport.camera.projection.is_orthographic(),
        "position camera enters a perspective view"
    );
    assert_eq!(
        controller.drawing().unwrap().revision(),
        revision_before,
        "a camera placement writes no document"
    );
}
