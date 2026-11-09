//! Wave 8 session-token contracts: `ELEV`, `UCSICON`.
//!
//! `ELEV` is a read-only report of the session elevation token (the setter form
//! is NOT IMPLEMENTED); `UCSICON` flips one session flag and no icon is
//! rendered. Both are payload-free and session-only: they never write the
//! drawing, never bump the revision and never record an undo step.

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
        row.message.contains("0.0"),
        "the default elevation token is reported: {}",
        row.message
    );
    assert!(
        row.message.contains("NOT IMPLEMENTED"),
        "the setter form is declared missing: {}",
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
fn elev_reads_the_stored_token_but_has_no_setter_form() {
    let (mut app, mut session) = application_with_document();
    session.drafting_extras.set_elev(2.5);
    let outcome =
        execute(&mut app, &mut session, ExtendedCommandId::Elev).expect("ELEV is admitted");
    let row = diagnostic(&outcome, "elev.value");
    assert!(
        row.message.contains("2.5"),
        "the stored token is reported: {}",
        row.message
    );
    assert!(
        row.message.contains("creates no object"),
        "no elevation consumer is claimed: {}",
        row.message
    );
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
    assert!(error.to_string().contains("ELEV needs no payload"));
}
