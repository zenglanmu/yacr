//! The status `view` / `theme` / `navigation` projections.
//!
//! `view` is host truth from the authoritative viewport camera: a 2D plan view
//! reports its real orthographic basis, a 3D orbit view its perspective basis, and
//! a session with no document reports `null` — never a fabricated orientation.

use super::super::super::*;
use super::super::test_support::*;

fn viewport_camera(session: &CadSession) -> (cad_app::Camera, cad_app::ProjectionKind) {
    let controller = session.core.controller.borrow();
    let viewport = &controller.application.workspace.viewports[&controller.viewport_id];
    (viewport.camera, viewport.view_mode.kind())
}

#[test]
fn a_2d_plan_view_reports_its_real_orthographic_basis() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    let (camera, kind) = viewport_camera(&session);
    assert_eq!(kind, cad_app::ProjectionKind::TwoD);
    let view = session
        .snapshot(None, None, 0)
        .view
        .expect("a 2D plan view has a real camera basis");
    assert_eq!(view.mode, "2d");
    assert_eq!(view.projection, "orthographic");
    assert_eq!(view.eye, [camera.eye.x, camera.eye.y, camera.eye.z]);
    assert_eq!(
        view.target,
        [camera.target.x, camera.target.y, camera.target.z]
    );
    assert_eq!(view.up, [camera.up.x, camera.up.y, camera.up.z]);
    assert!(view
        .eye
        .iter()
        .chain(view.target.iter())
        .all(|v| v.is_finite()));
    assert!(view.up.iter().all(|v| v.is_finite()));
}

#[test]
fn a_3d_orbit_view_reports_its_perspective_basis() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    session
        .command("SWITCH2D3D")
        .expect("SWITCH2D3D is admitted");
    let (camera, kind) = viewport_camera(&session);
    assert_eq!(kind, cad_app::ProjectionKind::ThreeD);
    let view = session
        .snapshot(None, None, 0)
        .view
        .expect("a 3D view has a real camera basis");
    assert_eq!(view.mode, "3d");
    assert_eq!(view.projection, "perspective");
    assert_eq!(view.eye, [camera.eye.x, camera.eye.y, camera.eye.z]);
    assert!(view
        .eye
        .iter()
        .chain(view.target.iter())
        .chain(view.up.iter())
        .all(|component| component.is_finite()));
}

#[test]
fn a_closed_document_reports_no_view() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    session.command("CLOSE").expect("CLOSE is admitted");
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.document_label, None);
    assert_eq!(
        status.view, None,
        "no authoritative document means no trustworthy orientation"
    );
    // The host preferences are still real and present.
    assert_eq!(status.theme, "dark");
    assert_eq!(status.navigation, CadNavigation::default());
}

#[test]
fn the_theme_and_navigation_switches_are_host_truth() {
    let session = session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }));
    assert_eq!(session.snapshot(None, None, 0).theme, "dark");
    session
        .apply_setting(
            CadSettingKey::Theme,
            CadSettingValue::Text("light".to_string()),
        )
        .expect("the theme write is accepted");
    session
        .apply_setting(CadSettingKey::NavbarVisible, CadSettingValue::Bool(false))
        .expect("the navbar write is accepted");
    session
        .apply_setting(CadSettingKey::NavvcubeDisplay, CadSettingValue::Number(2))
        .expect("the mask write is accepted");
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.theme, "light");
    assert_eq!(
        status.navigation,
        CadNavigation {
            navbar_visible: false,
            view_cube_display: 2,
            view_cube_in_2d: true,
            view_cube_in_3d: true,
        }
    );
}
