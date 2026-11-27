//! Construction guides (`tables/guides.json`, minor 26): guide-mode
//! measurements and the `GUIDE` management command.

use super::*;

fn measure(
    app: &mut Application,
    session: &mut SessionState,
    points: Vec<Point3>,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Measure, CommandPayload::Points(points)),
    )
}

fn guides(app: &Application) -> cad_domain::guide::GuideTable {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .guides()
        .clone()
}

fn guide_command(
    app: &mut Application,
    session: &mut SessionState,
    option: GuideCommand,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Guide, CommandPayload::Guide(option)),
    )
}

#[test]
fn dist_guide_writes_a_line_guide_and_the_measurement_in_one_command() {
    let (mut app, mut session) = application_with_document();
    session
        .begin_guide_measurement(crate::GuideMode::DistanceLine)
        .expect("guide mode");
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let outcome = measure(
        &mut app,
        &mut session,
        vec![point(0.0, 0.0), point(2.0, 1.0)],
    )
    .expect("the two points complete");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        Revision(before.0 + 1),
        "the guide is one table write"
    );
    let table = guides(&app);
    assert_eq!(table.len(), 1);
    let guide = table.guides()[0];
    assert_eq!(guide.point, point(0.0, 0.0));
    assert!((guide.direction.x - 2.0 / 5.0_f64.sqrt()).abs() < 1e-9);
    assert!((guide.direction.y - 1.0 / 5.0_f64.sqrt()).abs() < 1e-9);
    assert!(
        outcome.measurement.is_some(),
        "the measurement still reports"
    );
    assert!(session.guide_mode().is_none(), "the flag is consumed");

    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert!(guides(&app).is_empty(), "undo removes the guide table");
}

#[test]
fn protractor_guide_follows_the_second_ray() {
    let (mut app, mut session) = application_with_document();
    session
        .begin_guide_measurement(crate::GuideMode::ProtractorLine)
        .expect("guide mode");
    measure(
        &mut app,
        &mut session,
        vec![point(1.0, 1.0), point(3.0, 1.0), point(1.0, 4.0)],
    )
    .expect("the three points complete");
    let guide = guides(&app).guides()[0];
    assert_eq!(guide.point, point(1.0, 1.0));
    assert!((guide.direction.x).abs() < 1e-9, "along +Y only");
    assert!((guide.direction.y - 1.0).abs() < 1e-9);
}

#[test]
fn guide_list_is_read_only_and_clear_is_one_reversible_step() {
    let (mut app, mut session) = application_with_document();
    let empty = guide_command(&mut app, &mut session, GuideCommand::List).expect("list");
    assert!(empty
        .diagnostics
        .iter()
        .any(|row| row.code == "guide.list" && row.message.contains("no construction guides")));

    session
        .begin_guide_measurement(crate::GuideMode::DistanceLine)
        .expect("guide mode");
    measure(
        &mut app,
        &mut session,
        vec![point(0.0, 0.0), point(1.0, 0.0)],
    )
    .expect("guide");
    let before = app.workspace.documents[&DocumentId(1)].drawing.revision();
    let listed = guide_command(&mut app, &mut session, GuideCommand::List).expect("list");
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        before,
        "LIST writes nothing"
    );
    assert!(listed
        .diagnostics
        .iter()
        .any(|row| row.code == "guide.list" && row.message.contains("1 guide")));

    guide_command(&mut app, &mut session, GuideCommand::Clear).expect("clear");
    assert!(guides(&app).is_empty());
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        Revision(before.0 + 1)
    );
    app.execute(&mut session, command(CommandId::Undo, CommandPayload::None))
        .expect("undo");
    assert_eq!(guides(&app).len(), 1, "undo restores the guide");
}

#[test]
fn guide_modes_and_clear_are_refused_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).expect("Viewer");
    assert!(matches!(
        session
            .begin_guide_measurement(crate::GuideMode::DistanceLine)
            .unwrap_err(),
        CadError::PermissionDenied
    ));
    assert!(matches!(
        guide_command(&mut app, &mut session, GuideCommand::Clear).unwrap_err(),
        CadError::PermissionDenied
    ));
}
