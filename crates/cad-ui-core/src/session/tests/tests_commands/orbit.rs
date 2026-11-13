//! `ORBIT` contracts: the typed `cad_orbit` payload reaches the **real**
//! application camera through the session funnel, and a 2D orbit is the host's own
//! refusal with the camera left untouched.
//!
//! The wire channel itself (finite check, gate order, queue bound, owner-tick drain)
//! lives in the shared layer and is pinned in `tests_admission`/`tests_wire`; this
//! module pins what `CadSession::orbit` must do once a delta arrives.

use super::super::super::*;
use super::super::test_support::*;

use cad_app::ProjectionKind;

fn demo_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

/// The active viewport's 2D↔3D mode.
fn view_kind(session: &CadSession) -> ProjectionKind {
    let c = session.core.controller.borrow();
    c.application.workspace.viewports[&c.viewport_id]
        .view_mode
        .kind()
}

/// Enter the real 3D view through the ordinary command funnel.
fn enter_3d(session: &CadSession) {
    session
        .command("SWITCH2D3D")
        .expect("SWITCH2D3D is an admitted command");
    assert_eq!(view_kind(session), ProjectionKind::ThreeD);
}

#[test]
fn a_2d_orbit_is_refused_and_leaves_the_camera_unchanged() {
    let session = demo_session();
    assert_eq!(view_kind(&session), ProjectionKind::TwoD);
    let before = viewport_camera(&session);
    let status_before = session.snapshot(None, None, 0);
    assert_eq!(status_before.error, None);

    let error = session.orbit(0.3, 0.2).expect_err("a 2D view has no orbit");
    // The refusal is the host's own `CadError::InvalidInput`, formatted exactly like
    // every other funneled command refusal (no invented marker, no generic "failed").
    assert!(
        error.starts_with("InvalidInput(")
            && error.contains("Orbit is only defined in the three-dimensional view"),
        "{error}"
    );
    assert_eq!(
        viewport_camera(&session),
        before,
        "a refused orbit never moves the 2D camera"
    );
    // The refusal is recorded for `cad_status.error`, exactly like every other
    // funneled command; it is not swallowed.
    let status_after = session.snapshot(None, None, 0);
    assert_eq!(status_after.error.as_deref(), Some(error.as_str()));
    assert_eq!(status_after.camera, status_before.camera);
}

#[test]
fn a_3d_orbit_rotates_the_eye_and_preserves_the_target_distance() {
    let session = demo_session();
    enter_3d(&session);
    let before = viewport_camera(&session);
    let target = before.target;
    let distance = before.distance();
    assert!(distance.is_finite() && distance > 0.0, "{distance}");

    session.orbit(0.3, 0.2).expect("a 3D orbit is applied");

    let after = viewport_camera(&session);
    assert_eq!(after.target, target, "the orbit pivots about the target");
    assert!(
        (after.distance() - distance).abs() < 1e-9,
        "the eye-target distance is preserved: {} -> {}",
        distance,
        after.distance()
    );
    assert_ne!(after.eye, before.eye, "the eye really moved");
    assert_eq!(view_kind(&session), ProjectionKind::ThreeD);
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.error, None, "a real orbit reports no refusal");
}
