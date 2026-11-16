//! Wave 8 session-token contracts: `ELEV`, `UCSICON` and the `FSMODE` token.
//!
//! `ELEV <height>` sets the session elevation the shared create funnel creates new
//! entities at (the bare selector reports it); `FSMODE ON|OFF` sets the `FS` chain
//! mode; `UCSICON` flips one session flag and no icon is rendered. All three are
//! session-only: they never write the drawing, never bump the revision and never
//! record an undo step.

use super::*;

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
) -> CadResult<CommandOutcome> {
    app.execute(
        session,
        command(CommandId::Extended(ext), CommandPayload::None),
    )
}

fn execute_payload(
    app: &mut Application,
    session: &mut SessionState,
    ext: ExtendedCommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(CommandId::Extended(ext), payload))
}

fn revision(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}

fn diagnostic<'a>(outcome: &'a CommandOutcome, code: &str) -> &'a Diagnostic {
    outcome
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("missing {code}"))
}

#[test]
fn elev_reports_the_session_token_without_drawing_writes() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    assert_eq!(session.drafting_extras.elev(), 0.0);

    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::Elev).expect("ELEV is admitted");
    assert!(outcome.changes.is_none(), "ELEV writes nothing");
    let row = diagnostic(&outcome, "elev.value");
    assert!(
        row.message.contains("0"),
        "the default elevation token is reported: {}",
        row.message
    );
    assert!(
        row.message.contains("`ELEV <height>` sets it"),
        "the setter form is declared: {}",
        row.message
    );
    assert!(
        row.message.contains("session state only"),
        "ELEV says it is session state only: {}",
        row.message
    );
    assert_eq!(
        session.drafting_extras.elev(),
        0.0,
        "the report does not change the token"
    );
    assert_eq!(
        revision(&app),
        before_revision,
        "ELEV never writes the drawing"
    );
    assert!(!app.can_undo(&DocumentId(1)), "ELEV records no undo step");
}

#[test]
fn elev_sets_the_token_and_a_non_finite_height_is_refused_without_change() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    session
        .drafting_extras
        .set_elev(2.5)
        .expect("2.5 is finite");
    let outcome = execute_payload(
        &mut app,
        &mut session,
        ExtendedCommandId::Elev,
        CommandPayload::Elevation(3.25),
    )
    .expect("ELEV <height> is admitted");
    assert_eq!(session.drafting_extras.elev(), 3.25);
    assert!(outcome.changes.is_none());
    assert!(diagnostic(&outcome, "elev.set")
        .message
        .contains("session state only"));
    assert_eq!(revision(&app), before_revision);

    // The token can never become non-finite: the setter refuses by name.
    let error = session
        .drafting_extras
        .set_elev(f64::INFINITY)
        .expect_err("infinity is refused");
    assert!(error.to_string().contains("elev.invalid_value"), "{error}");
    assert_eq!(session.drafting_extras.elev(), 3.25, "the token is kept");

    // A payload that is neither `None` nor `Elevation` is a named refusal.
    let error = execute_payload(
        &mut app,
        &mut session,
        ExtendedCommandId::Elev,
        CommandPayload::Mode(AppMode::Work),
    )
    .expect_err("a foreign payload is refused");
    assert!(
        error.to_string().contains("elev.invalid_payload"),
        "{error}"
    );
    assert_eq!(session.drafting_extras.elev(), 3.25);
    assert_eq!(revision(&app), before_revision);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn fsmode_is_the_setter_fs_reads_and_refuses_a_foreign_payload() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    assert!(!session.drafting_extras.fs_mode(), "OFF is the default");

    // The bare selector is the report and never changes the token.
    let report = execute(&mut app, &mut session, ExtendedCommandId::FsMode)
        .expect("the bare FSMODE reports");
    assert!(report.changes.is_none());
    assert!(diagnostic(&report, "fsmode.value").message.contains("OFF"));
    assert!(!session.drafting_extras.fs_mode());

    for (payload, expected) in [
        (CommandPayload::FsMode(true), true),
        (CommandPayload::FsMode(true), true),
        (CommandPayload::FsMode(false), false),
    ] {
        execute_payload(&mut app, &mut session, ExtendedCommandId::FsMode, payload)
            .expect("FSMODE <ON|OFF> is admitted");
        assert_eq!(session.drafting_extras.fs_mode(), expected);
    }

    let error = execute_payload(
        &mut app,
        &mut session,
        ExtendedCommandId::FsMode,
        CommandPayload::Mode(AppMode::Work),
    )
    .expect_err("a foreign payload is refused");
    assert!(
        error.to_string().contains("fsmode.invalid_payload"),
        "{error}"
    );
    assert!(
        !session.drafting_extras.fs_mode(),
        "the refusal changed nothing"
    );
    assert_eq!(
        revision(&app),
        before_revision,
        "FSMODE never writes the drawing"
    );
    assert!(!app.can_undo(&DocumentId(1)), "FSMODE records no undo step");
}

#[test]
fn elev_places_new_objects_at_the_stored_elevation() {
    // The create funnel is the one consumer: `LINE` (and every other create
    // command) is inserted with its z offset by the token.
    let (mut app, mut session) = application_with_document();
    session.drafting_extras.set_elev(5.0).expect("5 is finite");
    let before = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity_count();

    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::CreateLine,
                CommandPayload::Points(vec![point(0.0, 0.0), point(10.0, 0.0)]),
            ),
        )
        .expect("LINE is admitted");
    let id = outcome.objects[0];
    let geometry = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(id.0))
        .expect("the line exists")
        .geometry
        .clone();
    let SemanticGeometry::Line { start, end } = geometry else {
        panic!("expected the created LINE");
    };
    assert_eq!((start.x, start.y, start.z), (0.0, 0.0, 5.0));
    assert_eq!((end.x, end.y, end.z), (10.0, 0.0, 5.0));
    assert_eq!(
        app.workspace.documents[&DocumentId(1)]
            .drawing
            .entity_count(),
        before + 1
    );

    // A second kind proves the rule is the funnel's, not a LINE special case.
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::CreateCircle,
                CommandPayload::Points(vec![point(2.0, 2.0), point(4.0, 2.0)]),
            ),
        )
        .expect("CIRCLE is admitted");
    let geometry = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("the circle exists")
        .geometry
        .clone();
    let SemanticGeometry::Circle { center, .. } = geometry else {
        panic!("expected the created CIRCLE");
    };
    assert_eq!(center.z, 5.0, "the circle's centre is elevated");

    // ELEV 0 is identity: the pre-`ELEV` coordinates come back bit-identically.
    session.drafting_extras.set_elev(0.0).expect("0 is finite");
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::CreateLine,
                CommandPayload::Points(vec![point(1.0, 1.0), point(2.0, 1.0)]),
            ),
        )
        .expect("LINE is admitted");
    let geometry = app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(outcome.objects[0].0))
        .expect("the line exists")
        .geometry
        .clone();
    let SemanticGeometry::Line { start, .. } = geometry else {
        panic!("expected the created LINE");
    };
    assert_eq!(start.z, 0.0, "ELEV 0 writes z = 0 exactly");
}

#[test]
fn ucsicon_toggles_the_session_flag_without_drawing_writes() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    // AutoCAD's default is OFF, and this build renders no icon either way.
    assert!(!session.drafting_extras.ucsicon());

    for expected in [true, false] {
        let outcome = execute(&mut app, &mut session, ExtendedCommandId::UcsIcon)
            .expect("UCSICON is admitted");
        assert_eq!(session.drafting_extras.ucsicon(), expected);
        assert!(outcome.changes.is_none(), "UCSICON writes nothing");
        let row = diagnostic(&outcome, "ucsicon.toggled");
        assert!(
            row.message.contains(if expected { "ON" } else { "OFF" }),
            "the new flag state is named: {}",
            row.message
        );
        assert!(
            row.message.contains("renders no UCS icon"),
            "the missing effect is declared: {}",
            row.message
        );
        assert!(
            row.message.contains("session state only"),
            "UCSICON says it is session state only: {}",
            row.message
        );
    }

    assert_eq!(
        revision(&app),
        before_revision,
        "UCSICON never writes the drawing"
    );
    assert!(
        !app.can_undo(&DocumentId(1)),
        "UCSICON records no undo step"
    );
}

#[test]
fn wave8_session_tokens_stay_available_in_viewer_mode() {
    let (mut app, mut session) = application_with_document();
    session.switch_mode(AppMode::Viewer).unwrap();
    execute(&mut app, &mut session, ExtendedCommandId::Elev).expect("ELEV is Viewer-legal");
    execute(&mut app, &mut session, ExtendedCommandId::FsMode).expect("FSMODE is Viewer-legal");
    execute(&mut app, &mut session, ExtendedCommandId::UcsIcon).expect("UCSICON is Viewer-legal");
    assert!(session.drafting_extras.ucsicon(), "the token still flips");
}

#[test]
fn a_caller_payload_is_refused_before_the_session_token() {
    let (mut app, mut session) = application_with_document();
    let error = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::Elev),
                CommandPayload::Mode(AppMode::Work),
            ),
        )
        .expect_err("a payload is refused");
    assert!(matches!(error, CadError::InvalidInput(_)));
    assert!(error.to_string().contains("ELEV"));
}
