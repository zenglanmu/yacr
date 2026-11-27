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
