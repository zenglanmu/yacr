//! `CAMERA` (Position Camera) selector, capture route and typed refusals.
//!
//! The bare selector starts the two-point eye/target capture; the reference
//! tool's two-point-perspective and walk modes are refused by name. Contract
//! level: no GUI, no GPU.

use super::super::*;
use super::test_support::*;
use crate::session::commands::{parse_camera_argument, CameraArgument};

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn the_camera_selector_starts_the_two_point_placement() {
    assert!(admissible_names().contains(&"CAMERA"));
    assert_eq!(canonical_command_name("CAMERA"), Some("CAMERA"));
    assert_eq!(normalize_command("camera"), Some(CommandId::PositionCamera));
    assert_eq!(command_name(CommandId::PositionCamera), Some("CAMERA"));
    assert!(matches!(
        commands::interaction_request_for(CommandId::PositionCamera),
        Some(cad_app::interaction::InteractionRequest::BeginDraw(
            cad_app::DrawToolKind::PositionCamera
        ))
    ));
    let session = fresh_session();
    session.command("CAMERA").expect("CAMERA is admitted");
    let tool = session.snapshot(None, None, 0).tool.expect("a tool");
    assert_eq!(tool.key, "draw.positioncamera");
    assert_eq!(tool.points, 0);
    assert!(!tool.ready);
}

#[test]
fn the_unsupported_camera_modes_are_refused_by_name() {
    assert_eq!(
        parse_camera_argument("two-point"),
        Some(CameraArgument::TwoPoint)
    );
    assert_eq!(parse_camera_argument("WALK"), Some(CameraArgument::Walk));
    assert_eq!(parse_camera_argument("BOGUS"), None);

    let session = fresh_session();
    let error = session
        .command("CAMERA TWO-POINT")
        .expect_err("two-point perspective is not modeled");
    assert!(
        error.contains("camera.two_point_not_implemented"),
        "{error}"
    );
    let error = session
        .command("CAMERA WALK")
        .expect_err("walk mode is not modeled");
    assert!(error.contains("camera.walk_not_implemented"), "{error}");
    let error = session
        .command("CAMERA BOGUS")
        .expect_err("outside the grammar");
    assert!(error.contains("unsupported CAD command"), "{error}");
    assert!(session.snapshot(None, None, 0).tool.is_none());
}
