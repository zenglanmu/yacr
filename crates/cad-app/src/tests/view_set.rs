//! `cad_set_view` placement contracts: fitted presets and absolute cameras,
//! mode/projection agreement, history and the empty-space refusal.
use super::*;

fn viewport(app: &Application) -> &Viewport {
    &app.workspace.viewports[&ViewportId(1)]
}

fn run(
    app: &mut Application,
    session: &mut SessionState,
    placement: ViewPlacement,
) -> CadResult<()> {
    app.execute(
        session,
        command(CommandId::SetView, CommandPayload::SetView(placement)),
    )
    .map(|_| ())
}

fn perspective(fov: f64) -> Projection {
    Projection::perspective(fov).unwrap()
}

#[test]
fn an_overview_preset_frames_the_bounds_and_enters_3d() {
    let (mut app, mut session) = application_with_bounds(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 10.0,
            y: 20.0,
            z: 30.0,
        },
    );
    let before = viewport(&app).camera;
    run(
        &mut app,
        &mut session,
        ViewPlacement::Preset(ViewPreset::Overview),
    )
    .unwrap();

    let vp = viewport(&app);
    assert_eq!(vp.view_mode.kind(), ProjectionKind::ThreeD);
    assert!(!vp.camera.projection.is_orthographic());
    assert_eq!(
        vp.camera.target,
        Point3 {
            x: 5.0,
            y: 10.0,
            z: 15.0
        },
        "the preset targets the bounds centre, z included"
    );
    let forward = camera::normalize3(camera::sub(vp.camera.eye, vp.camera.target)).unwrap();
    let expected = StandardView::Isometric.eye_offset();
    assert!((forward.x - expected.x).abs() < 1e-9);
    assert!((forward.y - expected.y).abs() < 1e-9);
    assert!((forward.z - expected.z).abs() < 1e-9);
    assert_eq!(vp.camera.up, StandardView::Isometric.up_hint());
    assert!(vp.camera.distance() > 0.0 && vp.camera.distance().is_finite());
    assert_eq!(
        vp.view_mode.saved_2d_camera(),
        before,
        "entering 3D keeps the exact 2D camera for the return trip"
    );
}

#[test]
fn a_plan_preset_returns_to_the_fitted_2d_top_view() {
    let (mut app, mut session) = application_with_bounds(
        Point3 {
            x: -5.0,
            y: -5.0,
            z: 0.0,
        },
        Point3 {
            x: 5.0,
            y: 5.0,
            z: 0.0,
        },
    );
    run(
        &mut app,
        &mut session,
        ViewPlacement::Preset(ViewPreset::Overview),
    )
    .unwrap();
    run(
        &mut app,
        &mut session,
        ViewPlacement::Preset(ViewPreset::Plan),
    )
    .unwrap();

    let vp = viewport(&app);
    assert_eq!(vp.view_mode.kind(), ProjectionKind::TwoD);
    assert!(vp.camera.projection.is_orthographic());
    assert_eq!(
        vp.camera.target,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0
        }
    );
    assert_eq!(
        vp.camera.up,
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0
        }
    );
    match vp.view_mode {
        ViewMode2d3d::TwoD { saved } => assert_eq!(saved, vp.camera),
        _ => panic!("plan must be 2D"),
    }
}

#[test]
fn an_absolute_camera_is_applied_verbatim_and_can_be_walked_back() {
    let (mut app, mut session) = application_with_document();
    let before = viewport(&app).camera;
    let camera = Camera {
        eye: Point3 {
            x: 1.0,
            y: 2.0,
            z: 12.0,
        },
        target: Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        },
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        projection: perspective(0.7),
    };
    run(
        &mut app,
        &mut session,
        ViewPlacement::Camera {
            camera,
            mode: ViewModeRequest::ThreeD,
        },
    )
    .unwrap();
    assert_eq!(viewport(&app).camera, camera);
    assert_eq!(viewport(&app).view_mode.kind(), ProjectionKind::ThreeD);

    app.execute(
        &mut session,
        command(CommandId::Zoom, CommandPayload::ZoomPrevious),
    )
    .unwrap();
    assert_eq!(
        viewport(&app).camera,
        before,
        "one ZOOM PREVIOUS step restores the old view"
    );
    assert_eq!(viewport(&app).view_mode.kind(), ProjectionKind::TwoD);
}

#[test]
fn an_identical_placement_is_a_no_op_without_a_history_step() {
    let (mut app, mut session) = application_with_document();
    let camera = viewport(&app).camera;
    run(
        &mut app,
        &mut session,
        ViewPlacement::Camera {
            camera,
            mode: ViewModeRequest::Plan,
        },
    )
    .unwrap();
    // ZOOM PREVIOUS with no recorded step is a named no-op.
    let outcome = app
        .execute(
            &mut session,
            command(CommandId::Zoom, CommandPayload::ZoomPrevious),
        )
        .unwrap();
    assert!(outcome
        .diagnostics
        .iter()
        .any(|row| row.code == "zoom.previous.empty"));
}

#[test]
fn mode_and_projection_must_agree() {
    let (mut app, mut session) = application_with_document();
    let orthographic = Camera {
        eye: Point3 {
            x: 0.0,
            y: 0.0,
            z: 10.0,
        },
        target: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        projection: Projection::orthographic(1.0).unwrap(),
    };
    let error = run(
        &mut app,
        &mut session,
        ViewPlacement::Camera {
            camera: orthographic,
            mode: ViewModeRequest::ThreeD,
        },
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("view.set.mode_projection_mismatch"));

    let perspective = Camera {
        projection: perspective(0.7),
        ..orthographic
    };
    let error = run(
        &mut app,
        &mut session,
        ViewPlacement::Camera {
            camera: perspective,
            mode: ViewModeRequest::Plan,
        },
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("view.set.mode_projection_mismatch"));
}

#[test]
fn a_preset_on_an_empty_space_refuses_by_name() {
    let (mut app, mut session) = application_with_document();
    let error = run(
        &mut app,
        &mut session,
        ViewPlacement::Preset(ViewPreset::Overview),
    )
    .unwrap_err();
    assert!(error.to_string().contains("view.set.empty"));
    // The refused preset changed no camera state.
    assert_eq!(viewport(&app).view_mode.kind(), ProjectionKind::TwoD);
}

#[test]
fn non_finite_or_coincident_cameras_are_refused() {
    let (mut app, mut session) = application_with_document();
    let base = viewport(&app).camera;
    for camera in [
        Camera {
            eye: Point3 {
                x: f64::NAN,
                ..base.eye
            },
            ..base
        },
        Camera {
            eye: base.target,
            ..base
        },
    ] {
        let error = run(
            &mut app,
            &mut session,
            ViewPlacement::Camera {
                camera,
                mode: ViewModeRequest::Plan,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("view.set.invalid_camera"));
    }
}
