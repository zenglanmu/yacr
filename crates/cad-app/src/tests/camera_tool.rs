//! `CAMERA` (Position Camera) contracts: a two-point perspective placement,
//! view-only, with named refusals. No GUI or GPU is involved.

use super::*;

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map_or(0, cad_history::History::undo_depth)
}

fn place(
    app: &mut Application,
    session: &mut SessionState,
    eye: Point3,
    target: Point3,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(
            CommandId::PositionCamera,
            CommandPayload::PositionCamera { eye, target },
        ),
    )
}

#[test]
fn position_camera_sets_a_perspective_view_and_records_zoom_previous() {
    let (mut app, mut session) = application_with_document();
    let eye = Point3 {
        x: 10.0,
        y: -20.0,
        z: 15.0,
    };
    let target = Point3 {
        x: 1.0,
        y: 2.0,
        z: 3.0,
    };
    let before = revision(&app);
    let outcome = place(&mut app, &mut session, eye, target).expect("CAMERA");
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|row| row.code == "camera.positioned")
        .expect("the placement is named");
    assert!(
        diagnostic.message.contains("model is unchanged"),
        "{diagnostic:?}"
    );
    let viewport = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(viewport.camera.eye, eye);
    assert_eq!(viewport.camera.target, target);
    assert!(viewport.camera.projection.is_orthographic().eq(&false));
    assert_eq!(viewport.view_mode.kind(), ProjectionKind::ThreeD);
    assert_eq!(viewport.zoom_history_len(), 1, "ZOOM PREVIOUS can undo it");
    assert_eq!(revision(&app), before, "a camera move writes no document");
    assert_eq!(undo_depth(&app), 0, "no drawing undo step");
}

#[test]
fn position_camera_refuses_bad_points_by_name() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    let error = place(
        &mut app,
        &mut session,
        Point3 {
            x: f64::NAN,
            y: 0.0,
            z: 0.0,
        },
        point(0.0, 0.0),
    )
    .expect_err("non-finite eye");
    assert!(matches!(error, CadError::InvalidInput(text) if text.contains("camera.invalid_point")));
    let error = place(
        &mut app,
        &mut session,
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
        Point3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        },
    )
    .expect_err("coincident eye/target");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("camera.eye_target_coincident")),
        "{error:?}"
    );
    assert_eq!(revision(&app), before);
    assert_eq!(
        app.workspace.viewports[&ViewportId(1)].view_mode.kind(),
        ProjectionKind::TwoD,
        "a refused placement leaves the view alone"
    );
}

#[test]
fn position_camera_is_legal_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    place(
        &mut app,
        &mut session,
        Point3 {
            x: 10.0,
            y: 10.0,
            z: 10.0,
        },
        point(0.0, 0.0),
    )
    .expect("a view-only command is Viewer-legal");
}

fn perspective(app: &mut Application, session: &mut SessionState) {
    place(
        app,
        session,
        Point3 {
            x: 10.0,
            y: -20.0,
            z: 15.0,
        },
        Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        },
    )
    .expect("the perspective fixture");
}

fn walk(
    app: &mut Application,
    session: &mut SessionState,
    action: crate::WalkCommand,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Walk, CommandPayload::Walk(action)),
    )
}

fn look(
    app: &mut Application,
    session: &mut SessionState,
    action: crate::LookAroundCommand,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::LookAround, CommandPayload::LookAround(action)),
    )
}

#[test]
fn walk_moves_the_eye_horizontally_and_keeps_its_height() {
    let (mut app, mut session) = application_with_document();
    perspective(&mut app, &mut session);
    let before = app.workspace.viewports[&ViewportId(1)].camera;
    let revision_before = revision(&app);
    let destination = Point3 {
        x: 0.0,
        y: 0.0,
        z: 999.0,
    };
    let outcome =
        walk(&mut app, &mut session, crate::WalkCommand::To(destination)).expect("WALK to a point");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|row| row.code == "walk.moved"),
        "{:?}",
        outcome.diagnostics
    );
    let viewport = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(viewport.camera.eye.x, 0.0);
    assert_eq!(viewport.camera.eye.y, 0.0);
    assert_eq!(
        viewport.camera.eye.z, before.eye.z,
        "walking never changes the eye height"
    );
    // The target moved by the same horizontal delta, so the view direction is
    // unchanged.
    assert!((viewport.camera.target.x - (before.target.x - 10.0)).abs() < 1e-9);
    assert!((viewport.camera.target.y - (before.target.y + 20.0)).abs() < 1e-9);
    assert_eq!(viewport.zoom_history_len(), 2, "ZOOM PREVIOUS can undo it");
    assert_eq!(revision(&app), revision_before, "walk writes no document");

    // `By` walks along the horizontal view direction and is signed.
    let forward = Point3 {
        x: viewport.camera.target.x - viewport.camera.eye.x,
        y: viewport.camera.target.y - viewport.camera.eye.y,
        z: 0.0,
    };
    let length = (forward.x * forward.x + forward.y * forward.y).sqrt();
    let before_by = viewport.camera.eye;
    walk(&mut app, &mut session, crate::WalkCommand::By(5.0)).expect("WALK by 5");
    let after = app.workspace.viewports[&ViewportId(1)].camera.eye;
    assert!((after.x - (before_by.x + forward.x / length * 5.0)).abs() < 1e-9);
    assert!((after.y - (before_by.y + forward.y / length * 5.0)).abs() < 1e-9);
    assert_eq!(after.z, before_by.z);
}

#[test]
fn walk_refuses_plan_views_and_degenerate_steps() {
    let (mut app, mut session) = application_with_document();
    // The default fixture is the 2D plan view.
    let error = walk(&mut app, &mut session, crate::WalkCommand::By(1.0))
        .expect_err("a plan view cannot walk");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("walk.requires_perspective")),
        "{error:?}"
    );

    perspective(&mut app, &mut session);
    let error = walk(&mut app, &mut session, crate::WalkCommand::By(0.0)).expect_err("a zero walk");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("walk.no_distance")),
        "{error:?}"
    );
    let error = walk(&mut app, &mut session, crate::WalkCommand::By(f64::NAN))
        .expect_err("a non-finite walk");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("walk.invalid_distance")),
        "{error:?}"
    );

    // Looking straight down leaves no horizontal forward direction.
    place(
        &mut app,
        &mut session,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 10.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    )
    .expect("a vertical camera");
    let error = walk(&mut app, &mut session, crate::WalkCommand::By(1.0))
        .expect_err("no horizontal forward");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("walk.degenerate_view")),
        "{error:?}"
    );
}

#[test]
fn look_around_turns_the_fixed_eye() {
    let (mut app, mut session) = application_with_document();
    perspective(&mut app, &mut session);
    let eye = app.workspace.viewports[&ViewportId(1)].camera.eye;
    let revision_before = revision(&app);
    let outcome = look(
        &mut app,
        &mut session,
        crate::LookAroundCommand::At(Point3 {
            x: 10.0,
            y: 0.0,
            z: 15.0,
        }),
    )
    .expect("LOOKAROUND at a point");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|row| row.code == "lookaround.turned"),
        "{:?}",
        outcome.diagnostics
    );
    let viewport = &app.workspace.viewports[&ViewportId(1)];
    assert_eq!(viewport.camera.eye, eye, "the eye does not move");
    assert!((viewport.camera.target.x - eye.x).abs() < 1e-9);
    assert!(viewport.camera.target.y > eye.y);
    assert_eq!(viewport.zoom_history_len(), 2);
    assert_eq!(revision(&app), revision_before);

    // A yaw turns about world +Z and a pitch tilts the result.
    let before = viewport.camera;
    look(
        &mut app,
        &mut session,
        crate::LookAroundCommand::By {
            yaw_degrees: 90.0,
            pitch_degrees: 10.0,
        },
    )
    .expect("LOOKAROUND by yaw/pitch");
    let after = app.workspace.viewports[&ViewportId(1)].camera;
    assert_eq!(after.eye, before.eye);
    assert!(
        (after.target.z - after.eye.z).abs() > 1.0,
        "a positive pitch lifts the target: {:?}",
        after.target
    );
}

#[test]
fn look_around_refuses_bad_inputs_and_plan_views() {
    let (mut app, mut session) = application_with_document();
    let error = look(
        &mut app,
        &mut session,
        crate::LookAroundCommand::By {
            yaw_degrees: 10.0,
            pitch_degrees: 0.0,
        },
    )
    .expect_err("a plan view cannot look around");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("lookaround.requires_perspective")),
        "{error:?}"
    );

    perspective(&mut app, &mut session);
    let eye = app.workspace.viewports[&ViewportId(1)].camera.eye;
    let error = look(&mut app, &mut session, crate::LookAroundCommand::At(eye))
        .expect_err("looking at the eye has no direction");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("lookaround.no_direction")),
        "{error:?}"
    );
    // A horizontal view pitched by 90° is exactly vertical: a pole.
    place(
        &mut app,
        &mut session,
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 10.0,
        },
        Point3 {
            x: 10.0,
            y: 0.0,
            z: 10.0,
        },
    )
    .expect("a horizontal camera");
    let error = look(
        &mut app,
        &mut session,
        crate::LookAroundCommand::By {
            yaw_degrees: 0.0,
            pitch_degrees: 90.0,
        },
    )
    .expect_err("a vertical view is a pole");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("lookaround.pole")),
        "{error:?}"
    );
    let error = look(
        &mut app,
        &mut session,
        crate::LookAroundCommand::By {
            yaw_degrees: f64::NAN,
            pitch_degrees: 0.0,
        },
    )
    .expect_err("non-finite angles");
    assert!(
        matches!(error, CadError::InvalidInput(ref text) if text.contains("lookaround.invalid_angle")),
        "{error:?}"
    );
}

#[test]
fn walk_and_look_around_are_legal_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    perspective(&mut app, &mut session);
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    walk(&mut app, &mut session, crate::WalkCommand::By(1.0))
        .expect("a view-only walk is Viewer-legal");
    look(
        &mut app,
        &mut session,
        crate::LookAroundCommand::By {
            yaw_degrees: 5.0,
            pitch_degrees: 0.0,
        },
    )
    .expect("a view-only turn is Viewer-legal");
}
