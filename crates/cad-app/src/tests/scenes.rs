//! `SCENE SAVE|RESTORE|LIST|DELETE` command contracts.
//!
//! A scene is presentation state: saving/deleting writes one `tables/scenes.json`
//! revision and one undo step; restoring applies camera/mode/space/style without
//! writing the document (it records a `ZOOM PREVIOUS` snapshot, like
//! `VIEW RESTORE`).

use super::*;
use cad_domain::scene::{markers, SceneVisualStyle};

fn scene(
    app: &mut Application,
    session: &mut SessionState,
    option: SceneCommand,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Scene, CommandPayload::Scene(option)),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn scenes(app: &Application) -> cad_domain::scene::SceneTable {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .scenes()
        .clone()
}

fn camera_eye(app: &Application) -> Point3 {
    app.workspace.viewports[&ViewportId(1)].camera.eye
}

fn move_camera(app: &mut Application) {
    app.workspace
        .viewports
        .get_mut(&ViewportId(1))
        .expect("the fixture viewport")
        .camera
        .eye
        .x += 10.0;
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

#[test]
fn a_save_captures_the_camera_space_and_style_and_writes_one_step() {
    let (mut app, mut session) = application_with_document();
    session
        .view_extras
        .set_visual_style("WIREFRAME")
        .expect("a real style");
    let before = revision(&app);
    let outcome = scene(
        &mut app,
        &mut session,
        SceneCommand::Save("Overview".into()),
    )
    .expect("save");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    let table = scenes(&app);
    let saved = table.resolve("Overview").expect("the row exists");
    assert_eq!(saved.style, SceneVisualStyle::Wireframe);
    assert_eq!(saved.view.space, SpaceId::Model);
    let diagnostic = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "scene.saved")
        .expect("the success is named");
    assert!(
        diagnostic.message.contains("Overview"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn an_identical_save_writes_nothing() {
    let (mut app, mut session) = application_with_document();
    scene(&mut app, &mut session, SceneCommand::Save("A".into())).expect("save");
    let before = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();
    let outcome = scene(&mut app, &mut session, SceneCommand::Save("A".into())).expect("no-op");
    assert_eq!(revision(&app), before, "no revision");
    assert_eq!(
        app.history[&DocumentId(1)].undo_depth(),
        depth,
        "no undo step"
    );
    assert!(outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "scene.unchanged"));
}

#[test]
fn a_restore_applies_the_camera_and_style_without_a_document_write() {
    let (mut app, mut session) = application_with_document();
    session.view_extras.set_visual_style("WIREFRAME").unwrap();
    let saved_camera = camera_eye(&app);
    scene(&mut app, &mut session, SceneCommand::Save("Start".into())).expect("save");

    // Move the camera and switch the style.
    move_camera(&mut app);
    assert_ne!(camera_eye(&app), saved_camera);
    session.view_extras.set_visual_style("SHADED").unwrap();
    let before = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();

    scene(
        &mut app,
        &mut session,
        SceneCommand::Restore("start".into()),
    )
    .expect("restore resolves case-insensitively");
    assert_eq!(camera_eye(&app), saved_camera, "the camera is back");
    assert_eq!(
        session.view_extras.visual_style.name(),
        "WIREFRAME",
        "the style is back"
    );
    assert_eq!(revision(&app), before, "restore never writes the document");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), depth);
    // `VIEWBACK` can undo the camera move the restore made.
    app.execute(
        &mut session,
        command(CommandId::ViewBack, CommandPayload::None),
    )
    .expect("view back");
    assert_ne!(camera_eye(&app), saved_camera);
}

#[test]
fn delete_and_undo_round_trip_the_scene_table() {
    let (mut app, mut session) = application_with_document();
    scene(&mut app, &mut session, SceneCommand::Save("A".into())).expect("save");
    let painted = scenes(&app);
    scene(&mut app, &mut session, SceneCommand::Delete("A".into())).expect("delete");
    assert!(scenes(&app).is_empty());
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert_eq!(scenes(&app), painted);
    app.execute(&mut session, command(CommandId::Redo, CommandPayload::None))
        .expect("redo");
    assert!(scenes(&app).is_empty());
}

#[test]
fn list_and_unknown_names_are_named() {
    let (mut app, mut session) = application_with_document();
    let empty = scene(&mut app, &mut session, SceneCommand::List).expect("list");
    assert!(empty
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "scene.list"
            && diagnostic.message.contains("no scenes")));

    scene(&mut app, &mut session, SceneCommand::Save("A".into())).expect("save");
    let listed = scene(&mut app, &mut session, SceneCommand::List).expect("list");
    let report = listed
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "scene.list")
        .expect("the report is named");
    assert!(report.message.contains("A"), "{}", report.message);

    let text = refusal_text(
        scene(
            &mut app,
            &mut session,
            SceneCommand::Restore("missing".into()),
        )
        .unwrap_err(),
    );
    assert!(text.starts_with("scene.unknown:"), "{text}");
    let text =
        refusal_text(scene(&mut app, &mut session, SceneCommand::Save("   ".into())).unwrap_err());
    assert!(text.starts_with("scene.name_required:"), "{text}");
    assert!(markers::SCENE_UNKNOWN.starts_with("scene."));
}

#[test]
fn viewer_mode_refuses_save_and_delete_but_allows_restore_and_list() {
    let (mut app, mut session) = application_with_document();
    scene(&mut app, &mut session, SceneCommand::Save("A".into())).expect("save");
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    assert!(matches!(
        scene(&mut app, &mut session, SceneCommand::Save("B".into())).unwrap_err(),
        CadError::PermissionDenied
    ));
    assert!(matches!(
        scene(&mut app, &mut session, SceneCommand::Delete("A".into())).unwrap_err(),
        CadError::PermissionDenied
    ));
    scene(&mut app, &mut session, SceneCommand::Restore("A".into())).expect("restore");
    scene(&mut app, &mut session, SceneCommand::List).expect("list");
}

#[test]
fn a_document_store_round_trip_keeps_the_scene_table() {
    use cad_domain::DocumentIdentity;
    let (mut app, mut session) = application_with_document();
    scene(&mut app, &mut session, SceneCommand::Save("Plan".into())).expect("save");
    let document = &app.workspace.documents[&DocumentId(1)];
    let snapshot = document
        .drawing
        .snapshot(
            document.units.clone(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            document.drawing.revision(),
        )
        .expect("snapshot");
    assert!(snapshot.has_scenes());
    let loaded = cad_db::DrawingDatabase::from_snapshot(&snapshot).expect("reopen");
    assert_eq!(loaded.scenes().resolve("Plan").unwrap().name(), "Plan");
}
