//! `STANDARDVIEW` contracts: the typed payload reaches the **real** application
//! camera through the session funnel, and the plan view returns to the saved 2D
//! camera.
//!
//! The channel itself (name parsing, queue admission, owner-tick drain) lives in
//! the Linux host (`cad_standard_view`); this module pins what the funnel must do
//! once the request arrives, so the assertions are on the application/session
//! state and never on the admission ack.

use std::collections::VecDeque;

use super::super::super::*;
use super::super::test_support::*;

use cad_app::{Projection, ProjectionKind, StandardView};

/// The demo document's model-space bounds — the same bounds the host frames.
fn demo_bounds(session: &CadSession) -> (cad_domain::Point3, cad_domain::Point3) {
    let c = session.core.controller.borrow();
    c.drawing()
        .expect("the demo document")
        .bounds()
        .expect("the demo document has bounds")
}

/// Whether `point` lies outside the axis-aligned box `min..max`.
fn outside_bounds(
    point: cad_domain::Point3,
    min: cad_domain::Point3,
    max: cad_domain::Point3,
) -> bool {
    point.x < min.x
        || point.x > max.x
        || point.y < min.y
        || point.y > max.y
        || point.z < min.z
        || point.z > max.z
}

/// The active viewport's 2D↔3D mode.
fn view_kind(session: &CadSession) -> ProjectionKind {
    let c = session.core.controller.borrow();
    c.application.workspace.viewports[&c.viewport_id]
        .view_mode
        .kind()
}

fn demo_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn isometric_standard_view_enters_a_framed_perspective_3d_view() {
    let session = demo_session();
    let (min, max) = demo_bounds(&session);
    let before = session.snapshot(None, None, 0);
    assert!(
        before.camera.is_some(),
        "a fresh session reports the 2D camera"
    );
    assert_eq!(before.error, None);
    let camera_before = viewport_camera(&session);
    assert!(camera_before.projection.is_orthographic());

    session
        .standard_view(StandardView::Isometric)
        .expect("the funnel admits isometric");

    let after = session.snapshot(None, None, 0);
    assert_eq!(after.error, None, "a real view never reports a refusal");
    assert!(
        after.camera.is_none(),
        "a 3D observation projects no 2D camera block"
    );
    assert_eq!(view_kind(&session), ProjectionKind::ThreeD);

    let camera_after = viewport_camera(&session);
    assert!(
        matches!(camera_after.projection, Projection::Perspective { .. }),
        "isometric is a perspective observation"
    );
    assert_eq!(
        camera_after.target, camera_before.target,
        "the standard view re-orients around the current target"
    );
    let distance = camera_after.distance();
    assert!(distance.is_finite() && distance > 0.0, "{distance}");
    // The eye is the target plus the isometric unit direction at that distance.
    let offset = StandardView::Isometric.eye_offset();
    for (delta, expected) in [
        (camera_after.eye.x - camera_after.target.x, offset.x),
        (camera_after.eye.y - camera_after.target.y, offset.y),
        (camera_after.eye.z - camera_after.target.z, offset.z),
    ] {
        assert!(
            (delta / distance - expected).abs() < 1e-9,
            "eye direction {delta} at distance {distance} is not the isometric offset {expected}"
        );
    }
    assert!(
        outside_bounds(camera_after.eye, min, max),
        "the framed eye {:?} is outside the model bounds {min:?}..{max:?}",
        camera_after.eye
    );
}

#[test]
fn two_queued_views_apply_in_order_and_the_last_one_wins() {
    // The owner tick drains the typed queue in admission order, so the last
    // admitted view is the one left on the camera. This drives the same funnel
    // the tick drives and asserts the resulting **state**, not an ack.
    let session = demo_session();
    let saved = viewport_camera(&session);
    let mut queue = VecDeque::from([
        StandardView::Isometric,
        StandardView::Front,
        StandardView::Top,
    ]);

    let mut applied = Vec::new();
    while let Some(view) = queue.pop_front() {
        applied.push(view);
        session
            .standard_view(view)
            .expect("the funnel admits every view");
    }

    assert_eq!(
        applied,
        vec![
            StandardView::Isometric,
            StandardView::Front,
            StandardView::Top
        ]
    );
    assert!(queue.is_empty(), "the queue drained completely");
    assert_eq!(
        view_kind(&session),
        ProjectionKind::TwoD,
        "the last admitted view is the one left applied"
    );
    assert_eq!(viewport_camera(&session), saved);
    assert!(session.snapshot(None, None, 0).camera.is_some());
}

#[test]
fn top_standard_view_returns_to_the_saved_2d_orthographic_camera() {
    let session = demo_session();
    let saved = viewport_camera(&session);
    let saved_status = session.snapshot(None, None, 0);
    assert!(saved_status.camera.is_some());

    session
        .standard_view(StandardView::Isometric)
        .expect("the funnel admits isometric");
    assert_eq!(view_kind(&session), ProjectionKind::ThreeD);

    session
        .standard_view(StandardView::Top)
        .expect("the funnel admits the plan view");
    assert_eq!(view_kind(&session), ProjectionKind::TwoD);
    let camera = viewport_camera(&session);
    assert!(camera.projection.is_orthographic());
    assert_eq!(
        camera, saved,
        "the plan view restores the 2D camera saved when 3D was entered"
    );
    let status = session.snapshot(None, None, 0);
    assert_eq!(status.camera, saved_status.camera);
    assert_eq!(status.error, None);
}
