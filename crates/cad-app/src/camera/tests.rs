use super::*;

fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

#[test]
fn projection_rejects_degenerate_fov_and_scale() {
    assert!(Projection::perspective(0.0).is_err());
    assert!(Projection::perspective(-1.0).is_err());
    assert!(Projection::perspective(f64::NAN).is_err());
    assert!(Projection::perspective(std::f64::consts::PI).is_err());
    assert!(Projection::perspective(f64::INFINITY).is_err());
    assert!(Projection::orthographic(0.0).is_err());
    assert!(Projection::orthographic(-1.0).is_err());
    assert!(Projection::orthographic(f64::NAN).is_err());

    // A valid FOV round-trips through validation.
    assert!(Projection::perspective(std::f64::consts::FRAC_PI_4).is_ok());
    assert!(Projection::orthographic(1.0).is_ok());
}

#[test]
fn orthographic_projection_matrix_maps_extents_to_clip() {
    let camera = Camera {
        eye: p(0.0, 0.0, 10.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 1.0, 0.0),
        projection: Projection::Orthographic { scale: 1.0 },
    };
    let m = camera.projection_matrix([800.0, 600.0]).unwrap();
    // World x = ±400 (half width in pixels * scale) maps to clip ±1.
    let clip_x = m[0][0] * 400.0;
    assert!((clip_x - 1.0).abs() < 1e-12);
    // World y = ±300 maps to clip ±1.
    let clip_y = m[1][1] * 300.0;
    assert!((clip_y - 1.0).abs() < 1e-12);
}

#[test]
fn projection_matrix_rejects_degenerate_viewport_size() {
    let camera = Camera::top_view_2d();
    assert!(camera.projection_matrix([0.0, 600.0]).is_err());
    assert!(camera.projection_matrix([800.0, f64::NAN]).is_err());
}

#[test]
fn standard_view_bases_are_right_handed_and_orthonormal() {
    for view in [
        StandardView::Top,
        StandardView::Bottom,
        StandardView::Front,
        StandardView::Back,
        StandardView::Left,
        StandardView::Right,
        StandardView::Isometric,
    ] {
        let offset = view.eye_offset();
        let camera = Camera {
            eye: add(p(0.0, 0.0, 0.0), scale3(offset, 1000.0)),
            target: p(0.0, 0.0, 0.0),
            up: view.up_hint(),
            projection: Projection::orthographic(1.0).unwrap(),
        };
        camera.validate().unwrap();
        let basis = camera.view_basis().unwrap();
        assert!(
            basis.is_orthonormal(1e-9),
            "view {view:?} basis is not orthonormal: {basis:?}"
        );
        // Right-handed: right × up = -forward.
        let expected = scale3(basis.forward, -1.0);
        let n = cross(basis.right, basis.up);
        assert!(
            length3(sub(n, expected)) < 1e-9,
            "view {view:?} basis is not right-handed"
        );
    }
}

#[test]
fn orbit_preserves_distance_and_clamps_pitch_at_poles() {
    let mut camera = Camera {
        eye: p(0.0, 0.0, 1000.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 1.0, 0.0),
        projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
    };
    let distance = camera.distance();
    camera.orbit(0.4, 0.3).unwrap();
    assert!((camera.distance() - distance).abs() < 1e-6);
    // A huge pitch must not collapse the view; the basis stays valid.
    camera.orbit(0.0, std::f64::consts::PI).unwrap();
    let basis = camera.view_basis().unwrap();
    assert!(basis.is_orthonormal(1e-9));
    // A near-plane guard keeps the eye off the target.
    assert!(camera.distance() > 1e-9);
}

#[test]
fn orbit_rejects_non_finite_angles() {
    let mut camera = Camera::top_view_2d();
    assert!(camera.orbit(f64::NAN, 0.0).is_err());
    assert!(camera.orbit(0.0, f64::INFINITY).is_err());
}

#[test]
fn world_screen_round_trip_orthographic() {
    let camera = Camera {
        eye: p(0.0, 0.0, 1000.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 1.0, 0.0),
        projection: Projection::Orthographic { scale: 2.0 },
    };
    let size = [800.0, 600.0];
    let world = p(120.0, -80.0, 0.0);
    let screen = camera.world_to_screen(world, size).unwrap();
    let ray = camera.screen_to_ray(screen, size).unwrap();
    // In orthographic, the ray through the pixel must pass through the world
    // point: project the world point onto the ray.
    let rel = sub(world, ray.origin);
    let t = dot3(rel, ray.direction);
    let closest = add(ray.origin, scale3(ray.direction, t));
    assert!(length3(sub(closest, world)) < 1e-9);
}

#[test]
fn world_screen_round_trip_perspective() {
    let camera = Camera {
        eye: p(0.0, -500.0, 200.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 0.0, 1.0),
        projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
    };
    let size = [1024.0, 768.0];
    let world = p(50.0, 30.0, 10.0);
    let screen = camera.world_to_screen(world, size).unwrap();
    let ray = camera.screen_to_ray(screen, size).unwrap();
    // The world point lies on the ray through its own pixel.
    let rel = sub(world, ray.origin);
    let t = dot3(rel, ray.direction);
    let closest = add(ray.origin, scale3(ray.direction, t));
    assert!(length3(sub(closest, world)) < 1e-9);
}

#[test]
fn world_to_screen_rejects_points_behind_a_perspective_camera() {
    let camera = Camera {
        eye: p(0.0, 0.0, 0.0),
        target: p(0.0, 1.0, 0.0),
        up: p(0.0, 0.0, 1.0),
        projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
    };
    // A point behind the eye (negative depth) is not projectable.
    assert!(camera
        .world_to_screen(p(0.0, -10.0, 0.0), [800.0, 600.0])
        .is_none());
}

#[test]
fn zoom_to_cursor_anchors_in_both_projections() {
    // Orthographic: the world point under the cursor is unchanged.
    let mut ortho = Camera {
        eye: p(0.0, 0.0, 1000.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 1.0, 0.0),
        projection: Projection::Orthographic { scale: 1.0 },
    };
    let cursor = [200.0, 150.0];
    let size = [800.0, 600.0];
    let anchor = ortho.screen_to_plan_world(cursor, size).unwrap();
    ortho.zoom_at(2.0, cursor, size).unwrap();
    let after = ortho.screen_to_plan_world(cursor, size).unwrap();
    assert!(length3(sub(anchor, after)) < 1e-9);

    // Perspective: the world point under the cursor stays on the same ray.
    let mut persp = Camera {
        eye: p(0.0, -500.0, 200.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 0.0, 1.0),
        projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
    };
    let ray_before = persp.screen_to_ray(cursor, size).unwrap();
    let focus_before = persp.cursor_on_target_plane(cursor, size).unwrap();
    persp.zoom_at(1.5, cursor, size).unwrap();
    let ray_after = persp.screen_to_ray(cursor, size).unwrap();
    // Same direction and the focus point still lies on the pixel ray.
    assert!(length3(sub(ray_before.direction, ray_after.direction)) < 1e-9);
    let rel = sub(focus_before, ray_after.origin);
    let t = dot3(rel, ray_after.direction);
    let closest = add(ray_after.origin, scale3(ray_after.direction, t));
    assert!(length3(sub(closest, focus_before)) < 1e-9);
}

#[test]
fn zoom_rejects_non_positive_or_non_finite_factor() {
    let mut camera = Camera::top_view_2d();
    assert!(camera.zoom_at(0.0, [0.0, 0.0], [800.0, 600.0]).is_err());
    assert!(camera.zoom_at(-2.0, [0.0, 0.0], [800.0, 600.0]).is_err());
    assert!(camera
        .zoom_at(f64::NAN, [0.0, 0.0], [800.0, 600.0])
        .is_err());
}

#[test]
fn work_plane_basis_is_orthonormal_and_right_handed() {
    let plane =
        orthonormal_work_plane(p(1.0, 2.0, 3.0), p(1.0, 1.0, 0.0), p(-1.0, 1.0, 0.0)).unwrap();
    assert!((length3(plane.u) - 1.0).abs() < 1e-12);
    assert!((length3(plane.v) - 1.0).abs() < 1e-12);
    assert!(dot3(plane.u, plane.v).abs() < 1e-12);
    // Right-handed normal points +Z for an in-plane basis.
    let n = cross(plane.u, plane.v);
    assert!(n.z > 0.0);
    // A parallel basis is degenerate and rejected.
    assert!(orthonormal_work_plane(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0)).is_err());
}

#[test]
fn camera_validate_rejects_coincident_eye_and_target() {
    let camera = Camera {
        eye: p(0.0, 0.0, 0.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 1.0, 0.0),
        projection: Projection::orthographic(1.0).unwrap(),
    };
    assert!(camera.validate().is_err());
}

#[test]
fn camera2d_params_require_orthographic_and_carry_the_scale() {
    let camera = Camera {
        eye: p(10.0, 20.0, 1000.0),
        target: p(10.0, 20.0, 0.0),
        up: p(0.0, 1.0, 0.0),
        projection: Projection::orthographic(2.5).unwrap(),
    };
    let params = camera.camera2d_params().unwrap();
    assert_eq!(params.center, p(10.0, 20.0, 0.0));
    assert_eq!(params.world_per_px, 2.5);

    let perspective = Camera {
        projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        ..camera
    };
    assert!(perspective.camera2d_params().is_err());
}

#[test]
fn camera3d_params_require_perspective_and_derive_finite_near_far() {
    let camera = Camera {
        eye: p(0.0, -500.0, 200.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 0.0, 1.0),
        projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
    };
    let params = camera.camera3d_params().unwrap();
    assert_eq!(params.eye, p(0.0, -500.0, 200.0));
    assert_eq!(params.target, p(0.0, 0.0, 0.0));
    assert_eq!(params.up, p(0.0, 0.0, 1.0));
    assert!((params.fov_y - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
    assert!(params.near > 0.0 && params.near.is_finite());
    assert!(params.far > params.near && params.far.is_finite());

    // Orthographic cameras are refused on the 3D render path.
    let ortho = Camera {
        projection: Projection::orthographic(1.0).unwrap(),
        ..camera
    };
    assert!(ortho.camera3d_params().is_err());
}

#[test]
fn camera3d_params_reject_a_degenerate_or_pole_locked_camera() {
    let coincident = Camera {
        eye: p(0.0, 0.0, 0.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 1.0, 0.0),
        projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
    };
    assert!(coincident.camera3d_params().is_err());

    // Up parallel to the view direction: no stable basis for the renderer.
    let degenerate_up = Camera {
        eye: p(0.0, 0.0, 10.0),
        target: p(0.0, 0.0, 0.0),
        up: p(0.0, 0.0, 1.0),
        projection: Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
    };
    assert!(degenerate_up.camera3d_params().is_err());
}

#[test]
fn standard_view_all_index_round_trips() {
    assert_eq!(StandardView::ALL.len(), 7);
    for (index, view) in StandardView::ALL.iter().copied().enumerate() {
        assert_eq!(view.index(), index);
        assert_eq!(StandardView::from_index(index as i32), Some(view));
    }
    assert_eq!(StandardView::from_index(-1), None);
    assert_eq!(StandardView::from_index(99), None);
}
