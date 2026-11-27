//! `SECTIONPLANE` command and scene capture/restore contracts.

use super::*;
use crate::{SceneCommand, SectionCommand};

fn plane_command(
    app: &mut Application,
    session: &mut SessionState,
    option: SectionCommand,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::SectionPlane, CommandPayload::Section(option)),
    )
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn refusal_text(error: CadError) -> String {
    match error {
        CadError::InvalidInput(text) | CadError::Unsupported(text) => text,
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

fn set_xy_plane() -> SectionCommand {
    SectionCommand::Set {
        origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 2.0,
        },
    }
}

#[test]
fn a_set_writes_one_revision_and_undo_restores_no_plane() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);
    plane_command(&mut app, &mut session, set_xy_plane()).expect("set");
    assert_eq!(revision(&app), Revision(before.0 + 1));
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 1);
    let plane = app.workspace.documents[&DocumentId(1)]
        .drawing
        .section_plane()
        .expect("stored");
    assert!(plane.active);
    assert!(
        (plane.normal.z - 1.0).abs() < 1.0e-9,
        "the normal is normalized"
    );

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .section_plane()
        .is_none());
}

#[test]
fn toggle_off_on_and_clear_are_distinct_writes() {
    let (mut app, mut session) = application_with_document();
    plane_command(&mut app, &mut session, set_xy_plane()).expect("set");
    plane_command(&mut app, &mut session, SectionCommand::Off).expect("off");
    assert!(
        !app.workspace.documents[&DocumentId(1)]
            .drawing
            .section_plane()
            .expect("stored")
            .active
    );
    plane_command(&mut app, &mut session, SectionCommand::Toggle).expect("toggle");
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .section_plane()
            .expect("stored")
            .active
    );
    plane_command(&mut app, &mut session, SectionCommand::Clear).expect("clear");
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .section_plane()
        .is_none());
    assert_eq!(app.history[&DocumentId(1)].undo_depth(), 4);
}

#[test]
fn on_off_without_a_stored_plane_is_a_named_refusal() {
    let (mut app, mut session) = application_with_document();
    for option in [
        SectionCommand::On,
        SectionCommand::Off,
        SectionCommand::Toggle,
    ] {
        let error = plane_command(&mut app, &mut session, option).unwrap_err();
        let text = refusal_text(error);
        assert!(text.contains("section.plane_missing"), "{text}");
    }
    assert_eq!(revision(&app), Revision(0), "no refusal wrote the document");
}

#[test]
fn viewer_lists_but_cannot_write_the_plane() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).expect("viewer");
    plane_command(&mut app, &mut session, SectionCommand::List).expect("list");
    assert!(matches!(
        plane_command(&mut app, &mut session, set_xy_plane()).unwrap_err(),
        CadError::PermissionDenied
    ));
}

#[test]
fn a_scene_captures_and_restores_the_section_plane() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::Scene,
            CommandPayload::Scene(SceneCommand::Save("cut".into())),
        ),
    )
    .expect("save without a plane");
    plane_command(&mut app, &mut session, set_xy_plane()).expect("set");
    let before = revision(&app);
    app.execute(
        &mut session,
        command(
            CommandId::Scene,
            CommandPayload::Scene(SceneCommand::Restore("cut".into())),
        ),
    )
    .expect("restore");
    assert_eq!(
        revision(&app),
        Revision(before.0 + 1),
        "the plane write is one revision"
    );
    assert!(app.workspace.documents[&DocumentId(1)]
        .drawing
        .section_plane()
        .is_none());
}

#[test]
fn a_viewer_restore_that_would_change_the_section_is_refused_before_any_write() {
    let (mut app, mut session) = application_with_document();
    app.execute(
        &mut session,
        command(
            CommandId::Scene,
            CommandPayload::Scene(SceneCommand::Save("cut".into())),
        ),
    )
    .expect("save without a plane");
    plane_command(&mut app, &mut session, set_xy_plane()).expect("set");
    session.switch_mode(AppMode::Viewer).expect("viewer");
    let before = revision(&app);
    assert!(matches!(
        app.execute(
            &mut session,
            command(
                CommandId::Scene,
                CommandPayload::Scene(SceneCommand::Restore("cut".into()))
            ),
        )
        .unwrap_err(),
        CadError::PermissionDenied
    ));
    assert_eq!(revision(&app), before);
    assert!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .section_plane()
            .expect("the plane stays")
            .active
    );
}
