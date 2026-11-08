//! Extended batch session-toggle contracts: `QTEXT`, `FILL`, `TFRAMES`,
//! `REGENAUTO`, `ISODRAFT`.
//!
//! Each is a payload-free session flag: it flips in place, emits one honest
//! `*.toggled` diagnostic (the effect is NOT IMPLEMENTED), never writes the
//! drawing, never bumps the revision and never records an undo step.

use super::*;

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn flag(session: &SessionState, ext: ExtendedCommandId) -> bool {
    match ext {
        ExtendedCommandId::QText => session.drafting_extras.qtext(),
        ExtendedCommandId::Fill => session.drafting_extras.fill(),
        ExtendedCommandId::TFrames => session.drafting_extras.tframes(),
        ExtendedCommandId::RegenAuto => session.drafting_extras.regenauto(),
        ExtendedCommandId::IsoDraft => session.drafting_extras.isodraft(),
        other => panic!("not a drafting toggle: {other:?}"),
    }
}

#[test]
fn drafting_toggles_flip_session_flags_without_drawing_writes() {
    let (mut app, mut session) = application_with_document();
    let revision = app.workspace.documents[&DocumentId(1)].drawing.revision();
    // FILL starts ON (AutoCAD's default); the rest start OFF.
    assert!(session.drafting_extras.fill());
    assert!(!session.drafting_extras.qtext());
    for ext in [
        ExtendedCommandId::QText,
        ExtendedCommandId::Fill,
        ExtendedCommandId::TFrames,
        ExtendedCommandId::RegenAuto,
        ExtendedCommandId::IsoDraft,
    ] {
        let before = flag(&session, ext);
        let outcome = execute(
            &mut app,
            &mut session,
            CommandId::Extended(ext),
            CommandPayload::None,
        )
        .expect("a drafting toggle is admitted");
        assert_eq!(flag(&session, ext), !before, "{} toggles", ext.selector());
        assert!(
            outcome.changes.is_none(),
            "{} writes nothing",
            ext.selector()
        );
        assert_eq!(outcome.diagnostics.len(), 1, "one diagnostic per toggle");
        let diagnostic = &outcome.diagnostics[0];
        assert_eq!(
            diagnostic.code,
            format!("{}.toggled", ext.selector().to_ascii_lowercase())
        );
        assert!(
            diagnostic.message.contains("NOT IMPLEMENTED"),
            "the toggle states its honest partial: {}",
            diagnostic.message
        );
        assert!(
            diagnostic.message.contains("ON") || diagnostic.message.contains("OFF"),
            "the message states the new state: {}",
            diagnostic.message
        );
    }
    assert_eq!(
        app.workspace.documents[&DocumentId(1)].drawing.revision(),
        revision,
        "session toggles never write the drawing"
    );
    assert!(!app.can_undo(&DocumentId(1)), "no undo step is recorded");
}

#[test]
fn drafting_toggle_refuses_a_payload_and_changes_nothing() {
    let (mut app, mut session) = application_with_document();
    let before = session.drafting_extras.qtext();
    let refused = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::QText),
        CommandPayload::Points(vec![point(1.0, 2.0)]),
    );
    assert!(refused.is_err(), "QTEXT takes no payload");
    assert_eq!(
        session.drafting_extras.qtext(),
        before,
        "a refused toggle does not flip the flag"
    );
    assert!(
        !app.can_undo(&DocumentId(1)),
        "a refusal records no history"
    );
}
