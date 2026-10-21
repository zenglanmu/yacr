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

// --- 3D entry framing -------------------------------------------------------

/// Replace the open document's drawing with one polyline spanning `min..max`,
/// so the document has a known 3D extent to frame.
fn set_document_extent(app: &mut Application, min: Point3, max: Point3) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(cad_db::DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(1),
            type_key: "AcDbPolyline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Polyline {
            points: vec![min, max],
            bulges: Vec::new(),
            closed: false,
        },
        draw_order: 0,
    })
    .unwrap();
    let drawing = b.finish().unwrap();
    app.workspace
        .documents
        .get_mut(&DocumentId(1))
        .unwrap()
        .drawing = Arc::new(drawing);
}

fn fov_of(camera: &Camera) -> f64 {
    match camera.projection {
        Projection::Perspective {
            vertical_fov_radians,
        } => vertical_fov_radians,
        Projection::Orthographic { .. } => panic!("expected a perspective camera"),
    }
}

#[test]
fn switch_2d3d_frames_the_document_beyond_the_near_plane() {
    let (mut app, mut session) = application_with_document();
    // A cube spanning [-1, 1]^3, the shape the live defect clipped away.
    let min = Point3 {
        x: -1.0,
        y: -1.0,
        z: -1.0,
    };
    let max = Point3 {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    };
    set_document_extent(&mut app, min, max);
    // Reproduce the measured 2D state: the plan eye sits one unit above the
    // target. `FitDrawing` preserves `eye.z`, so the fit keeps that offset.
    app.workspace
        .viewports
        .get_mut(&ViewportId(1))
        .unwrap()
        .camera
        .eye
        .z = 1.0;
    app.execute(
        &mut session,
        command(CommandId::FitDrawing, CommandPayload::None),
    )
    .unwrap();
    // The plan camera keeps its one-unit eye offset; entering 3D used to inherit
    // exactly that distance, putting the eye on the cube's top face.
    let plan = app.workspace.viewports[&ViewportId(1)].camera;
    assert!(plan.projection.is_orthographic());
    assert!((plan.distance() - 1.0).abs() < 1e-12);

    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    let camera = vp.camera;
    assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);

    // Target is the bounds centre, z included.
    assert!((camera.target.x - 0.0).abs() < 1e-12);
    assert!((camera.target.y - 0.0).abs() < 1e-12);
    assert!((camera.target.z - 0.0).abs() < 1e-12);

    // The eye is outside the model bounds (above the top face), not on it.
    assert!(camera.eye.z > max.z);

    // The view direction is unchanged: entering from plan stays top-down.
    assert!((camera.eye.x - camera.target.x).abs() < 1e-12);
    assert!((camera.eye.y - camera.target.y).abs() < 1e-12);

    // The whole diagonal fits in the visible height at the target, with margin.
    let fov = fov_of(&camera);
    let radius = 0.5 * (12.0f64).sqrt(); // half the cube's diagonal
    let extent = 2.0 * radius;
    let distance = camera.distance();
    let visible_height = 2.0 * distance * (fov * 0.5).tan();
    assert!(
        visible_height >= extent,
        "visible height {visible_height} must cover the extent {extent}"
    );

    // Near/far planes bracket the whole model: nothing is clipped.
    let params = camera.camera3d_params().unwrap();
    assert!(params.near < distance - radius);
    assert!(params.far > distance + radius);
    assert!(params.near > 0.0);
}

#[test]
fn switch_projection_frames_the_document_on_the_way_into_3d() {
    let (mut app, mut session) = application_with_document();
    let min = Point3 {
        x: -2.0,
        y: -1.0,
        z: -3.0,
    };
    let max = Point3 {
        x: 2.0,
        y: 1.0,
        z: 3.0,
    };
    set_document_extent(&mut app, min, max);
    app.workspace
        .viewports
        .get_mut(&ViewportId(1))
        .unwrap()
        .camera
        .eye
        .z = 1.0;
    app.execute(
        &mut session,
        command(CommandId::FitDrawing, CommandPayload::None),
    )
    .unwrap();

    app.execute(
        &mut session,
        command(CommandId::SwitchProjection, CommandPayload::None),
    )
    .unwrap();
    let vp = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);
    let camera = vp.camera;
    // Target is the bounds centre including z; the eye is outside the box.
    assert!((camera.target.z - 0.0).abs() < 1e-12);
    let distance = camera.distance();
    let radius = 0.5 * (16.0f64 + 4.0 + 36.0).sqrt();
    assert!(camera.eye.z > max.z);
    assert!(distance > radius);
    let visible_height = 2.0 * distance * (fov_of(&camera) * 0.5).tan();
    assert!(visible_height >= 2.0 * radius);
    assert!(camera.camera3d_params().unwrap().near < distance - radius);

    // Returning to 2D restores the snapshot exactly.
    let saved = vp.view_mode.saved_2d_camera();
    app.execute(
        &mut session,
        command(CommandId::SwitchProjection, CommandPayload::None),
    )
    .unwrap();
    assert_eq!(app.workspace.viewports[&ViewportId(1)].camera, saved);
}

#[test]
fn switch_2d3d_without_a_usable_extent_keeps_a_finite_positive_distance() {
    // An empty document: the plan camera's unit offset must not survive as a
    // degenerate 3D distance.
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let empty = app.workspace.viewports[&ViewportId(1)].camera;
    let empty_distance = empty.distance();
    assert!(empty_distance.is_finite());
    assert!(empty_distance > 0.0);
    assert!(empty.camera3d_params().is_ok());

    // A dimensionless document (one point, zero extent) hits the same fallback.
    let (mut app, mut session) = application_with_document();
    let point = Point3 {
        x: 5.0,
        y: 5.0,
        z: 5.0,
    };
    set_document_extent(&mut app, point, point);
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let degenerate = app.workspace.viewports[&ViewportId(1)].camera;
    let distance = degenerate.distance();
    assert!(distance.is_finite());
    assert!(distance >= camera::MIN_3D_DISTANCE);
    assert!(degenerate.camera3d_params().is_ok());
    // The target still lands on the only point the drawing has.
    assert!((degenerate.target.x - point.x).abs() < 1e-12);
    assert!((degenerate.target.y - point.y).abs() < 1e-12);
    assert!((degenerate.target.z - point.z).abs() < 1e-12);
}

// --- orbit from the plan view ------------------------------------------------

/// Convex hull (monotone chain) of 2D points, in screen space.
fn hull_len(points: &[[f64; 2]]) -> usize {
    let mut p: Vec<[f64; 2]> = points.to_vec();
    p.sort_by(|a, b| {
        a[0].partial_cmp(&b[0])
            .unwrap()
            .then(a[1].partial_cmp(&b[1]).unwrap())
    });
    p.dedup();
    if p.len() < 3 {
        return p.len();
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 1e-9 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 1e-9 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.len() + upper.len()
}

/// The live 3D defect, reproduced exactly at the app level: entering 3D from
/// the plan view and applying the measured primary drag `dx = +90, dy = +51`
/// (`orbit(yaw = -0.72, pitch = +0.41)`).
///
/// A top-down offset is parallel to the yaw axis, so yaw leaves it unchanged and
/// pitch tilts the eye off `+Z` by exactly the pitch angle: the eye's polar
/// angle must be 0.41 rad (23.5°), not 0. A camera that stays top-down would
/// project the cube to a square; a tilted one must give a 6-vertex silhouette.
#[test]
fn orbit_from_plan_tilts_the_eye_by_the_pitch_angle() {
    let (mut app, mut session) = application_with_document();
    set_document_extent(
        &mut app,
        Point3 {
            x: -1.0,
            y: -1.0,
            z: -1.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
    );
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let framed = app.workspace.viewports[&ViewportId(1)].camera;
    let distance_before = framed.distance();

    app.execute(
        &mut session,
        command(
            CommandId::Orbit,
            CommandPayload::Orbit {
                yaw: -0.72,
                pitch: 0.41,
            },
        ),
    )
    .unwrap();
    let camera = app.workspace.viewports[&ViewportId(1)].camera;

    let offset = camera::sub(camera.eye, camera.target);
    let polar = (offset.z / camera::length3(offset)).clamp(-1.0, 1.0).acos();
    assert!(
        (polar - 0.41).abs() < 1e-9,
        "orbit pitch 0.41 must tilt the eye 0.41 rad off +Z, measured {polar}"
    );
    assert!(
        (camera.distance() - distance_before).abs() < 1e-9,
        "orbit must preserve the eye-target distance ({} -> {})",
        distance_before,
        camera.distance()
    );
    // The yaw is real too: the horizontal component of the offset is non-zero.
    let horizontal = (offset.x * offset.x + offset.y * offset.y).sqrt();
    assert!(
        horizontal > 1e-6,
        "the eye must leave the top-down axis, horizontal offset {horizontal}"
    );
    // The tilt magnitude is what the pitch asked for.
    let tilt = (horizontal / offset.z).atan();
    assert!(
        (tilt - 0.41).abs() < 1e-9,
        "tilt must be 0.41 rad, got {tilt}"
    );

    // And the camera's own projection of the cube's 8 corners is a hexagon.
    let size = [256.0, 256.0];
    let corners = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    let projected: Vec<[f64; 2]> = corners
        .iter()
        .map(|c| {
            camera
                .world_to_screen(
                    Point3 {
                        x: c[0],
                        y: c[1],
                        z: c[2],
                    },
                    size,
                )
                .expect("every cube corner is in front of the camera")
        })
        .collect();
    assert_eq!(
        hull_len(&projected),
        6,
        "an oblique cube projects to a 6-vertex silhouette, projected {projected:?}"
    );
}

/// The host's **real** 3D drag path emits one `Orbit` command per pointer move
/// (`crates/cad-host/src/input/pointer.rs`: `orbit_delta(from, point)` on every
/// move), not one command for the whole gesture. Each increment is small, so the
/// camera must *accumulate* them: a downward drag has to keep tilting the eye in
/// one direction.
///
/// Regression (the measured live defect): the pitch axis was derived as
/// `horizontal × +Z`, which is the negative of screen right. The first increment
/// tilted the eye off `+Z`, and every following increment tilted it back, so the
/// gesture ended a few milliradians off top-down - the cube's `+Z` face rendered
/// as a rotated square with no internal edges. 36 increments of the measured
/// drag (`dx = +90, dy = +51` in the probe) must therefore land on the same
/// 0.41 rad polar angle a single combined orbit produces.
#[test]
fn incremental_orbit_accumulates_the_pitch() {
    let (mut app, mut session) = application_with_document();
    set_document_extent(
        &mut app,
        Point3 {
            x: -1.0,
            y: -1.0,
            z: -1.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
    );
    app.execute(
        &mut session,
        command(CommandId::Switch2d3d, CommandPayload::None),
    )
    .unwrap();
    let distance = app.workspace.viewports[&ViewportId(1)].camera.distance();

    // 90 px right / 51 px down at 0.008 rad/px over 36 moves: exactly what the
    // live primary drag produced.
    const MOVES: usize = 36;
    let total_yaw = -90.0 * 0.008;
    let total_pitch = 51.0 * 0.008;
    for _ in 0..MOVES {
        app.execute(
            &mut session,
            command(
                CommandId::Orbit,
                CommandPayload::Orbit {
                    yaw: total_yaw / MOVES as f64,
                    pitch: total_pitch / MOVES as f64,
                },
            ),
        )
        .unwrap();
    }

    let camera = app.workspace.viewports[&ViewportId(1)].camera;
    let offset = camera::sub(camera.eye, camera.target);
    let polar = (offset.z / camera::length3(offset)).clamp(-1.0, 1.0).acos();
    assert!(
        (polar - total_pitch).abs() < 1e-6,
        "an incremental orbit must accumulate to the full pitch: expected {} rad, measured {polar}",
        total_pitch
    );
    assert!(
        (camera.distance() - distance).abs() < 1e-9,
        "the increments must preserve the eye-target distance"
    );

    // The tilted camera's own projection of the cube is a hexagon, so the frame
    // can no longer be a square face.
    let size = [926.0, 341.0];
    let corners = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    let projected: Vec<[f64; 2]> = corners
        .iter()
        .map(|c| {
            camera
                .world_to_screen(
                    Point3 {
                        x: c[0],
                        y: c[1],
                        z: c[2],
                    },
                    size,
                )
                .expect("every cube corner is in front of the camera")
        })
        .collect();
    assert_eq!(
        hull_len(&projected),
        6,
        "an obliquely orbited cube must project to a hexagon, projected {projected:?}"
    );
    // Nothing degenerates into the camera's own up hint.
    assert!(camera.camera3d_params().is_ok());
    let basis = camera.view_basis().unwrap();
    assert!(basis.is_orthonormal(1e-9));
}
