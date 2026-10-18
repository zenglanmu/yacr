use super::*;

#[test]
fn screen_center_maps_to_camera_target_and_corners_are_symmetric() {
    let mut viewport = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    viewport.camera.target = Point3 {
        x: 100.0,
        y: 50.0,
        z: 0.0,
    };
    viewport.camera.projection = Projection::Orthographic { scale: 2.0 };

    let center = viewport.screen_to_world([400.0, 300.0], [800.0, 600.0]);
    assert_eq!(
        center,
        Some(Point3 {
            x: 100.0,
            y: 50.0,
            z: 0.0
        })
    );

    // Top-left is left of and above the centre; world y flips relative to
    // the downward-growing screen y.
    let top_left = viewport
        .screen_to_world([0.0, 0.0], [800.0, 600.0])
        .unwrap();
    assert!((top_left.x - (100.0 - 400.0 * 2.0)).abs() < 1e-9);
    assert!((top_left.y - (50.0 + 300.0 * 2.0)).abs() < 1e-9);
}

#[test]
fn screen_to_world_rejects_degenerate_input() {
    let viewport = Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]);
    assert!(viewport
        .screen_to_world([f64::NAN, 0.0], [800.0, 600.0])
        .is_none());
    assert!(viewport.screen_to_world([0.0, 0.0], [0.0, 600.0]).is_none());
    assert!(viewport
        .screen_to_world([0.0, 0.0], [800.0, -1.0])
        .is_none());
}

#[test]
fn switch_2d3d_and_orbit_are_real_transitions() {
    let (mut app, mut session) = application_with_document();
    // Start in 2D orthographic.
    let before = app.workspace.viewports[&ViewportId(1)].camera;
    assert!(before.projection.is_orthographic());

    // Switch to 3D: perspective, still 3D mode.
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert!(!vp.camera.projection.is_orthographic());
    assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);

    // Orbit in 3D succeeds and preserves the eye-target distance.
    let d = app.workspace.viewports[&ViewportId(1)].camera.distance();
    app.execute(
        &mut session,
        command(
            CommandId::Orbit,
            CommandPayload::Orbit {
                yaw: 0.3,
                pitch: 0.2,
            },
        ),
    )
    .unwrap();
    let after_orbit = app.workspace.viewports[&ViewportId(1)].camera.distance();
    assert!((after_orbit - d).abs() < 1e-9);

    // Switch back restores the exact 2D camera.
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(vp.camera, before);
    assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
}

#[test]
fn orbit_outside_3d_is_rejected_without_changing_the_camera() {
    let (mut app, mut session) = application_with_document();
    let before = app.workspace.viewports[&ViewportId(1)].camera;
    let result = app.execute(
        &mut session,
        command(
            CommandId::Orbit,
            CommandPayload::Orbit {
                yaw: 0.1,
                pitch: 0.1,
            },
        ),
    );
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
    assert_eq!(app.workspace.viewports[&ViewportId(1)].camera, before);
}

#[test]
fn orbit_without_payload_is_rejected() {
    let (mut app, mut session) = application_with_document();
    assert!(matches!(
        app.execute(
            &mut session,
            command(CommandId::Orbit, CommandPayload::None)
        ),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn standard_view_is_a_real_projection_and_plan_returns_to_2d() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::StandardView,
            CommandPayload::StandardView(StandardView::Front),
        ),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);
    let basis = vp.camera.view_basis().unwrap();
    assert!(basis.is_orthonormal(1e-9));

    // Top (plan) returns to 2D orthographic.
    app.execute(
        &mut session,
        command(
            CommandId::StandardView,
            CommandPayload::StandardView(StandardView::Top),
        ),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
    assert!(vp.camera.projection.is_orthographic());
}

#[test]
fn zoom_at_anchors_the_cursor_in_2d() {
    let (mut app, mut session) = application_with_document();
    let cursor = [200.0, 150.0];
    let size = [800.0, 600.0];
    let world_before = app.workspace.viewports[&ViewportId(1)]
        .camera
        .screen_to_plan_world(cursor, size)
        .unwrap();
    app.execute(
        &mut session,
        command(
            CommandId::Zoom,
            CommandPayload::ZoomAt {
                factor: 2.0,
                cursor,
            },
        ),
    )
    .unwrap();
    let world_after = app.workspace.viewports[&ViewportId(1)]
        .camera
        .screen_to_plan_world(cursor, size)
        .unwrap();
    assert!((world_before.x - world_after.x).abs() < 1e-9);
    assert!((world_before.y - world_after.y).abs() < 1e-9);
    // The scale actually changed.
    assert!(app.workspace.viewports[&ViewportId(1)].world_per_px() < 1.0);
}

#[test]
fn work_plane_is_right_handed_and_orthonormal() {
    let plane = cam_xy_work_plane(0.0);
    assert!((cad_app_len(plane.u) - 1.0).abs() < 1e-12);
    assert!((cad_app_len(plane.v) - 1.0).abs() < 1e-12);
    assert!(cad_app_dot(plane.u, plane.v).abs() < 1e-12);
    let n = camera::cross(plane.u, plane.v);
    assert!(n.z > 0.0, "normal points +Z in the plan view");
}
