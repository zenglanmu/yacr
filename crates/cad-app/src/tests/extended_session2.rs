//! Wave 2 session-token contracts: `DRAGMODE`, `APERTURE`, `ISOPLANE`,
//! `HIGHLIGHTNEW`, `TRAYSETTINGS`, `TEXTSCR`, `GRAPHSCR`, `FINDINCLUDE`,
//! `FINDMATCH`, `REVCLOUDPROPERTIES`.
//!
//! Each flips/reads one token in `SessionState::drafting_extras`, emits one
//! honest diagnostic, never writes the drawing, never bumps the revision and
//! never records an undo step. `APERTURE <n>` and `REVCLOUDPROPERTIES <length>`
//! are the two validated **set** forms (payload carried as `LayerWalk` tokens);
//! their consumers are `SessionState::interaction_tolerance` and `REVCLOUD`
//! respectively.

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
fn aperture_and_revcloud_properties_report_their_session_tokens_read_only() {
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
        row.message.contains("has not been set explicitly"),
        "the configured value is named until the token is set: {}",
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
        row.message.contains("REVCLOUD reads it"),
        "the consumer is named: {}",
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
fn aperture_set_writes_only_the_session_token() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::Aperture),
                CommandPayload::LayerWalk(vec!["24".into()]),
            ),
        )
        .expect("an in-range APERTURE value is admitted");
    assert!(
        outcome.changes.is_none(),
        "no ChangeSet for a session token"
    );
    assert!(diagnostic(&outcome, "aperture.set")
        .message
        .contains("24 logical pixels"));
    assert_eq!(session.drafting_extras.aperture(), 24);
    assert!(session.drafting_extras.aperture_explicit());
    // The token is the session object-snap aperture, not a pickbox write.
    let policy = session.interaction_tolerance(cad_domain::TolerancePolicy::default());
    assert_eq!(policy.interaction_logical_pixels, 24.0);
    assert_eq!(
        policy.pickbox_logical_pixels,
        cad_domain::TolerancePolicy::default().pickbox_logical_pixels,
        "APERTURE never changes PICKBOX"
    );
    assert_eq!(revision(&app), before_revision, "no drawing write");
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry");
}

#[test]
fn aperture_out_of_range_and_malformed_tokens_are_refused_and_change_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = session.drafting_extras.aperture();
    for (tokens, needle) in [
        (vec!["0".to_string()], "aperture.out_of_range"),
        (vec!["51".to_string()], "aperture.out_of_range"),
        (vec!["abc".to_string()], "aperture.invalid_argument"),
        (vec!["1.5".to_string()], "aperture.invalid_argument"),
        (
            vec!["10".to_string(), "20".to_string()],
            "aperture.invalid_argument",
        ),
    ] {
        let error = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::Aperture),
                    CommandPayload::LayerWalk(tokens),
                ),
            )
            .expect_err("an invalid aperture is refused");
        match &error {
            CadError::InvalidInput(reason) => assert!(reason.contains(needle), "{reason}"),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
        assert_eq!(
            session.drafting_extras.aperture(),
            before,
            "a refusal stores nothing"
        );
        assert!(!session.drafting_extras.aperture_explicit());
    }
}

#[test]
fn revcloud_properties_set_writes_the_session_arc_length_token() {
    let (mut app, mut session) = application_with_document();
    let before_revision = revision(&app);
    let outcome = app
        .execute(
            &mut session,
            command(
                CommandId::Extended(ExtendedCommandId::RevCloudProperties),
                CommandPayload::LayerWalk(vec!["2.5".into()]),
            ),
        )
        .expect("a positive arc length is admitted");
    assert!(outcome.changes.is_none());
    assert!(diagnostic(&outcome, "revcloud.properties.set")
        .message
        .contains("2.5"));
    assert_eq!(session.drafting_extras.revcloud_arc_length(), 2.5);
    assert_eq!(revision(&app), before_revision, "no drawing write");
    assert!(!app.can_undo(&DocumentId(1)), "no undo entry");

    for (token, needle) in [
        ("0", "revcloudproperties.invalid_argument"),
        ("-1", "revcloudproperties.invalid_argument"),
        ("NaN", "revcloudproperties.invalid_argument"),
        ("inf", "revcloudproperties.invalid_argument"),
        ("abc", "revcloudproperties.invalid_argument"),
    ] {
        let error = app
            .execute(
                &mut session,
                command(
                    CommandId::Extended(ExtendedCommandId::RevCloudProperties),
                    CommandPayload::LayerWalk(vec![token.into()]),
                ),
            )
            .expect_err("an unusable arc length is refused");
        match &error {
            CadError::InvalidInput(reason) => assert!(reason.contains(needle), "{reason}"),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
        assert_eq!(
            session.drafting_extras.revcloud_arc_length(),
            2.5,
            "a refusal stores nothing"
        );
    }
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
