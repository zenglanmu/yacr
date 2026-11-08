//! Wave 2 session-token contracts: `DRAGMODE`, `APERTURE`, `ISOPLANE`,
//! `HIGHLIGHTNEW`, `TRAYSETTINGS`, `TEXTSCR`, `GRAPHSCR`, `FINDINCLUDE`,
//! `FINDMATCH`, `REVCLOUDPROPERTIES`.
//!
//! Each is payload-free and session-only: it flips or reads one token in
//! `SessionState::drafting_extras`, emits one honest diagnostic (the effect is
//! NOT IMPLEMENTED or the form is read-only), never writes the drawing, never
//! bumps the revision and never records an undo step.

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
fn flag_tokens_flip_without_drawing_writes() {
    type Case = (
        ExtendedCommandId,
        &'static str,
        &'static str,
        fn(&SessionState) -> bool,
    );

    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    // AutoCAD-documented defaults: DRAGMODE/HIGHLIGHTNEW/TRAYSETTINGS/
    // FINDINCLUDE start ON, the others OFF.
    assert!(session.drafting_extras.dragmode());
    assert!(session.drafting_extras.highlightnew());
    assert!(session.drafting_extras.traysettings());
    assert!(session.drafting_extras.find_include());
    assert!(!session.drafting_extras.find_match_case());

    let cases: [Case; 5] = [
        (
            ExtendedCommandId::DragMode,
            "dragmode.toggled",
            "drag-preview behaviour is NOT IMPLEMENTED",
            |session| session.drafting_extras.dragmode(),
        ),
        (
            ExtendedCommandId::HighlightNew,
            "highlightnew.toggled",
            "no new-feature highlight",
            |session| session.drafting_extras.highlightnew(),
        ),
        (
            ExtendedCommandId::TraySettings,
            "traysettings.toggled",
            "no status-bar tray UI",
            |session| session.drafting_extras.traysettings(),
        ),
        (
            ExtendedCommandId::FindInclude,
            "findinclude.toggled",
            "cross-block/xref text search is NOT IMPLEMENTED",
            |session| session.drafting_extras.find_include(),
        ),
        (
            ExtendedCommandId::FindMatch,
            "findmatch.toggled",
            "case-insensitively",
            |session| session.drafting_extras.find_match_case(),
        ),
    ];

    for (ext, code, boundary, read) in cases {
        let before = read(&session);
        let outcome = execute(&mut app, &mut session, ext).expect("a session token is admitted");
        assert_eq!(
            read(&session),
            !before,
            "{} flips its token",
            ext.selector()
        );
        assert!(
            outcome.changes.is_none(),
            "{} writes nothing",
            ext.selector()
        );
        let row = diagnostic(&outcome, code);
        assert!(
            row.message.contains(boundary),
            "{} states its honest boundary: {}",
            ext.selector(),
            row.message
        );
        assert!(
            row.message.contains("session state only"),
            "{} says it is session state only: {}",
            ext.selector(),
            row.message
        );
    }

    assert_eq!(
        revision(&app),
        before_revision,
        "tokens never write the drawing"
    );
    assert!(!app.can_undo(&DocumentId(1)), "no undo step is recorded");
}

#[test]
fn isoplane_cycles_left_top_right_without_drawing_writes() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    assert_eq!(session.drafting_extras.isoplane(), IsoPlane::Left);
    for expected in [IsoPlane::Top, IsoPlane::Right, IsoPlane::Left] {
        let outcome = execute(&mut app, &mut session, ExtendedCommandId::IsoPlane)
            .expect("ISOPLANE is admitted");
        assert_eq!(session.drafting_extras.isoplane(), expected);
        let row = diagnostic(&outcome, "isoplane.toggled");
        assert!(
            row.message.contains(expected.label()),
            "the new plane is named: {}",
            row.message
        );
        assert!(
            row.message.contains("no isometric cursor or snap change"),
            "the missing effect is declared: {}",
            row.message
        );
    }
    assert_eq!(revision(&app), before_revision);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn text_screen_tokens_set_the_session_flag_polarity() {
    let (mut app, mut session) = application_with_document();
    assert!(!session.drafting_extras.text_screen());

    execute(&mut app, &mut session, ExtendedCommandId::TextScreen).expect("TEXTSCR is admitted");
    assert!(session.drafting_extras.text_screen(), "TEXTSCR sets it ON");

    execute(&mut app, &mut session, ExtendedCommandId::GraphScreen).expect("GRAPHSCR is admitted");
    assert!(
        !session.drafting_extras.text_screen(),
        "GRAPHSCR sets it OFF"
    );

    let outcome = execute(&mut app, &mut session, ExtendedCommandId::TextScreen)
        .expect("TEXTSCR is admitted twice");
    assert!(
        diagnostic(&outcome, "textscr.toggled")
            .message
            .contains("no text-screen window"),
        "the missing window is declared"
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn read_only_token_reports_state_that_their_setter_is_not_implemented() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);

    let aperture =
        execute(&mut app, &mut session, ExtendedCommandId::Aperture).expect("APERTURE is admitted");
    let row = diagnostic(&aperture, "aperture.value");
    assert!(
        row.message.contains("session aperture token is 10"),
        "{}",
        row.message
    );
    assert!(
        row.message.contains("setter form") && row.message.contains("NOT IMPLEMENTED"),
        "the setter form is named as missing: {}",
        row.message
    );

    let revcloud = execute(
        &mut app,
        &mut session,
        ExtendedCommandId::RevCloudProperties,
    )
    .expect("REVCLOUDPROPERTIES is admitted");
    let row = diagnostic(&revcloud, "revcloud.properties");
    assert!(
        row.message.contains("arc-length token is 15"),
        "{}",
        row.message
    );
    assert!(
        row.message.contains("NOT IMPLEMENTED"),
        "the setter form is named as missing: {}",
        row.message
    );

    assert_eq!(
        revision(&app),
        before_revision,
        "read-only token reports write nothing"
    );
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn a_payload_is_refused_and_changes_no_token() {
    let (mut app, mut session) = application_with_document();
    let before = session.drafting_extras.dragmode();
    let refused = app.execute(
        &mut session,
        command(
            CommandId::Extended(ExtendedCommandId::DragMode),
            CommandPayload::Points(vec![point(1.0, 2.0)]),
        ),
    );
    assert!(refused.is_err(), "DRAGMODE takes no payload");
    assert_eq!(session.drafting_extras.dragmode(), before);
    assert!(
        !app.can_undo(&DocumentId(1)),
        "a refusal records no history"
    );
}
