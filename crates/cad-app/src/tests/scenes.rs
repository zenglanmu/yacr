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

#[test]
fn a_scene_captures_each_layers_effective_visibility() {
    let (mut app, mut session) = application_with_document();
    let (layer, name, database_visible) = {
        let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
        let layer = drawing.layers().next().expect("the default layer");
        (layer.id, layer.name.clone(), layer.visible)
    };
    // A session-only override is part of the effective visibility a scene stores.
    session.layer_overrides.set(layer, !database_visible);
    scene(&mut app, &mut session, SceneCommand::Save("Tags".into())).expect("save");
    let table = scenes(&app);
    let saved = table.resolve("Tags").expect("the row");
    let tags = saved.tags.as_ref().expect("tag visibility is captured");
    let tag = tags
        .iter()
        .find(|tag| tag.name == name)
        .expect("the layer is captured by name");
    assert_eq!(tag.visible, !database_visible);
}

#[test]
fn restoring_a_scene_reapplies_the_saved_tag_visibility_without_writing() {
    let (mut app, mut session) = application_with_document();
    let (layer, database_visible) = {
        let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
        let layer = drawing.layers().next().expect("the default layer");
        (layer.id, layer.visible)
    };
    session.layer_overrides.set(layer, !database_visible);
    scene(&mut app, &mut session, SceneCommand::Save("Tags".into())).expect("save");

    // Change the override after saving.
    session.layer_overrides.set(layer, database_visible);
    let before = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();
    scene(&mut app, &mut session, SceneCommand::Restore("Tags".into())).expect("restore");
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        session.layer_overrides.is_layer_visible(drawing, layer),
        !database_visible,
        "the saved effective visibility is back"
    );
    assert_eq!(revision(&app), before, "restore never writes the document");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), depth);
}

#[test]
fn a_scene_saved_without_a_session_override_still_captures_the_database_flag() {
    let (mut app, mut session) = application_with_document();
    let (layer, name, database_visible) = {
        let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
        let layer = drawing.layers().next().expect("the default layer");
        (layer.id, layer.name.clone(), layer.visible)
    };
    scene(&mut app, &mut session, SceneCommand::Save("Plain".into())).expect("save");
    let table = scenes(&app);
    let saved = table.resolve("Plain").expect("the row");
    let tags = saved.tags.as_ref().expect("captured");
    let tag = tags.iter().find(|tag| tag.name == name).expect("the layer");
    assert_eq!(tag.visible, database_visible);
    // Nothing was pinned in the session just by saving.
    assert!(session.layer_overrides.is_empty());
    let drawing = &app.workspace.documents[&DocumentId(1)].drawing;
    assert_eq!(
        session.layer_overrides.is_layer_visible(drawing, layer),
        database_visible
    );
}

fn set_display(
    app: &mut Application,
    session: &mut SessionState,
    option: DisplayCommand,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Shadows, CommandPayload::Display(option)),
    )
}

fn display_settings(app: &Application) -> cad_domain::display::DisplaySettings {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .display_settings()
}

#[test]
fn a_save_captures_the_document_shadows_and_fog_record() {
    let (mut app, mut session) = application_with_document();
    set_display(&mut app, &mut session, DisplayCommand::Shadows(true)).expect("shadows");
    set_display(
        &mut app,
        &mut session,
        DisplayCommand::Sun {
            azimuth: 210.0,
            elevation: 30.0,
        },
    )
    .expect("sun");
    set_display(
        &mut app,
        &mut session,
        DisplayCommand::FogRange {
            start: 50.0,
            end: 900.0,
        },
    )
    .expect("fog");
    let current = display_settings(&app);
    scene(&mut app, &mut session, SceneCommand::Save("Look".into())).expect("save");
    let table = scenes(&app);
    let saved = table.resolve("Look").expect("the row");
    assert_eq!(saved.display, Some(current));
}

#[test]
fn restoring_a_scene_reapplies_a_changed_display_record_as_one_step() {
    let (mut app, mut session) = application_with_document();
    scene(&mut app, &mut session, SceneCommand::Save("Look".into())).expect("save");
    let saved_display = display_settings(&app);
    set_display(&mut app, &mut session, DisplayCommand::Shadows(true)).expect("shadows");
    assert!(display_settings(&app).shadows.enabled);
    let before = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();
    let outcome =
        scene(&mut app, &mut session, SceneCommand::Restore("Look".into())).expect("restore");
    assert_eq!(display_settings(&app), saved_display, "the record is back");
    assert_eq!(revision(&app), Revision(before.0 + 1), "one display write");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), depth + 1);
    let report = outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "scene.restored")
        .expect("the report is named");
    assert!(report.message.contains("shadows/fog"), "{}", report.message);

    // The display write is its own undo step and restores the pre-restore record.
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert!(display_settings(&app).shadows.enabled);
}

#[test]
fn a_viewer_restore_with_a_differing_display_record_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    scene(&mut app, &mut session, SceneCommand::Save("Look".into())).expect("save");
    set_display(&mut app, &mut session, DisplayCommand::Shadows(true)).expect("shadows");
    let display = display_settings(&app);
    let camera = camera_eye(&app);
    let before = revision(&app);
    session
        .switch_mode(AppMode::Viewer)
        .expect("Viewer is legal");
    assert!(matches!(
        scene(&mut app, &mut session, SceneCommand::Restore("Look".into())).unwrap_err(),
        CadError::PermissionDenied
    ));
    assert_eq!(display_settings(&app), display, "the display is untouched");
    assert_eq!(camera_eye(&app), camera, "the camera is untouched");
    assert_eq!(revision(&app), before, "nothing was written");
}

fn soften(
    app: &mut Application,
    session: &mut SessionState,
    angle: f64,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::SoftenEdges, CommandPayload::SoftenEdges(angle)),
    )
}

fn soften_of(app: &Application, id: EntityId) -> Option<f64> {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_render_attributes(id)
        .soften
}

#[test]
fn a_save_captures_explicit_soften_and_restore_reapplies_it() {
    let (mut app, mut session) = super::soften::app_with(vec![super::soften::body(7)]);
    super::soften::select(&mut app, &mut session, &[7]);
    soften(&mut app, &mut session, 30.0).expect("soften 30");
    scene(&mut app, &mut session, SceneCommand::Save("Soft".into())).expect("save");
    let table = scenes(&app);
    let saved = table.resolve("Soft").expect("the row");
    assert_eq!(
        saved.soften,
        Some(vec![cad_domain::scene::SceneSoften {
            entity: EntityId(7),
            angle: 30.0
        }])
    );

    soften(&mut app, &mut session, 45.0).expect("soften 45");
    let before = revision(&app);
    let depth = app.history[&DocumentId(1)].undo_depth();
    scene(&mut app, &mut session, SceneCommand::Restore("Soft".into())).expect("restore");
    assert_eq!(soften_of(&app, EntityId(7)), Some(30.0));
    assert_eq!(revision(&app), Revision(before.0 + 1), "one soften write");
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), depth + 1);
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert_eq!(soften_of(&app, EntityId(7)), Some(45.0));
}

#[test]
fn a_viewer_restore_with_a_different_soften_value_is_refused() {
    let (mut app, mut session) = super::soften::app_with(vec![super::soften::body(7)]);
    super::soften::select(&mut app, &mut session, &[7]);
    soften(&mut app, &mut session, 30.0).expect("soften 30");
    scene(&mut app, &mut session, SceneCommand::Save("Soft".into())).expect("save");
    soften(&mut app, &mut session, 45.0).expect("soften 45");
    let before = revision(&app);
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    assert!(matches!(
        scene(&mut app, &mut session, SceneCommand::Restore("Soft".into())).unwrap_err(),
        CadError::PermissionDenied
    ));
    assert_eq!(soften_of(&app, EntityId(7)), Some(45.0));
    assert_eq!(revision(&app), before);
}

#[test]
fn a_scene_without_an_explicit_soften_clears_the_later_value() {
    let (mut app, mut session) = super::soften::app_with(vec![super::soften::body(7)]);
    super::soften::select(&mut app, &mut session, &[7]);
    scene(&mut app, &mut session, SceneCommand::Save("Plain".into())).expect("save");
    let saved = scenes(&app);
    let row = saved.resolve("Plain").expect("row");
    assert!(row.soften.is_none(), "no explicit value at save");
    assert_eq!(
        row.soften_cleared,
        Some(vec![EntityId(7)]),
        "the body is recorded for clearing"
    );
    soften(&mut app, &mut session, 45.0).expect("soften 45");
    let before = revision(&app);
    scene(
        &mut app,
        &mut session,
        SceneCommand::Restore("Plain".into()),
    )
    .expect("restore");
    assert_eq!(soften_of(&app, EntityId(7)), None, "restore clears it");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert_eq!(soften_of(&app, EntityId(7)), Some(45.0));
}

#[test]
fn a_scene_captures_and_restores_hidden_top_level_entities() {
    let (mut app, mut session) = application_with_bounds(point(0.0, 0.0), point(1.0, 1.0));
    let entity = EntityId(1);
    session.hidden_entities.insert(SelectionRef {
        document: DocumentId(1),
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    });
    scene(&mut app, &mut session, SceneCommand::Save("Hidden".into())).expect("save");
    let saved = scenes(&app);
    assert_eq!(
        saved.resolve("Hidden").expect("row").hidden.as_deref(),
        Some([entity].as_slice())
    );

    // A nested-instance hide has no scene row form: it must survive the
    // top-level replacement rather than be silently cleared.
    let nested = SelectionRef {
        document: DocumentId(1),
        entity: EntityId(2),
        instance: InstancePath(vec![EntityId(1)]),
        sub_element: None,
    };
    session.hidden_entities.clear();
    session.hidden_entities.insert(nested.clone());
    let before = revision(&app);
    scene(
        &mut app,
        &mut session,
        SceneCommand::Restore("Hidden".into()),
    )
    .expect("restore");
    assert_eq!(revision(&app), before, "a hidden restore is session-only");
    assert!(session.hidden_entities.refs().iter().any(|reference| {
        reference.entity == entity
            && reference.instance.0.is_empty()
            && reference.sub_element.is_none()
    }));
    assert!(
        session.hidden_entities.contains(&nested),
        "a nested-instance hide is not representable and must be preserved"
    );
}

#[test]
fn an_empty_hidden_capture_clears_the_session_hidden_set_on_restore() {
    let (mut app, mut session) = application_with_bounds(point(0.0, 0.0), point(1.0, 1.0));
    scene(
        &mut app,
        &mut session,
        SceneCommand::Save("All visible".into()),
    )
    .expect("save");
    let entity = EntityId(1);
    session.hidden_entities.insert(SelectionRef {
        document: DocumentId(1),
        entity,
        instance: InstancePath::default(),
        sub_element: None,
    });
    scene(
        &mut app,
        &mut session,
        SceneCommand::Restore("All visible".into()),
    )
    .expect("restore");
    assert!(
        session.hidden_entities.is_empty(),
        "a saved empty capture is not the same as an old scene that never captured hidden state"
    );
}
