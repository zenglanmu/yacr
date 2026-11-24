//! Extended batch session-toggle contracts: `QTEXT`, `FILL`, `TFRAMES`,
//! `REGENAUTO`, `ISODRAFT`.
//!
//! Each is a payload-free session flag: it flips in place, emits one
//! `*.toggled` diagnostic, never writes the drawing, never bumps the revision and
//! never records an undo step.
//!
//! Three of them are **consumed** by the display build (`FILL`, `QTEXT`,
//! `TFRAMES`): their report names what the derived display now does. The other
//! two (`REGENAUTO`, `ISODRAFT`) have no consumer in this build and still say
//! their effect is NOT IMPLEMENTED.
//!
//! The display half itself is pinned by `render_scene::display_mode_tests` (the
//! rebuilt batches) and `cad_host::tests::display_modes` (the session token
//! reaching the render session); this file pins the command boundary.

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
    // FILL and TFRAMES start ON (AutoCAD's defaults); the rest start OFF.
    assert!(session.drafting_extras.fill());
    assert!(session.drafting_extras.tframes());
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
            diagnostic.message.contains("ON") || diagnostic.message.contains("OFF"),
            "the message states the new state: {}",
            diagnostic.message
        );
        // The two tokens without a consumer must still say so; the three
        // display tokens must not claim "not implemented" any more.
        let consumed = matches!(
            ext,
            ExtendedCommandId::QText | ExtendedCommandId::Fill | ExtendedCommandId::TFrames
        );
        assert_eq!(
            diagnostic.message.contains("NOT IMPLEMENTED"),
            !consumed,
            "{} names its real consumer exactly when it has one: {}",
            ext.selector(),
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

/// `TFRAMES OFF` names the two frames it hides and the xref frame it cannot.
#[test]
fn tframes_off_names_the_frames_it_hides_and_the_xref_boundary() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::TFrames),
        CommandPayload::None,
    )
    .expect("TFRAMES is admitted");
    assert!(
        !session.drafting_extras.tframes(),
        "TFRAMES started ON and flipped"
    );
    assert!(outcome.changes.is_none(), "TFRAMES writes nothing");
    let message = &outcome.diagnostics[0].message;
    assert!(message.contains("image"), "{message}");
    assert!(message.contains("WIPEOUT"), "{message}");
    assert!(message.contains("External-reference frames"), "{message}");
}

/// `FILL OFF` reports the outline-only rule it triggers — and the primitives it
/// deliberately does not switch.
#[test]
fn fill_off_names_the_outline_rule_and_its_declared_boundary() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::Fill),
        CommandPayload::None,
    )
    .expect("FILL is admitted");
    assert!(
        !session.drafting_extras.fill(),
        "FILL started ON and flipped"
    );
    let message = &outcome.diagnostics[0].message;
    assert!(message.contains("outline only"), "{message}");
    assert!(
        message.contains("WIPEOUT") && message.contains("image"),
        "the declared non-switched primitives are named: {message}"
    );
}

/// `QTEXT ON` reports the placeholder rule and the display-only boundary.
#[test]
fn qtext_names_the_placeholder_and_the_missing_extent_case() {
    let (mut app, mut session) = application_with_document();
    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::Extended(ExtendedCommandId::QText),
        CommandPayload::None,
    )
    .expect("QTEXT is admitted");
    assert!(
        session.drafting_extras.qtext(),
        "QTEXT started OFF and flipped"
    );
    let message = &outcome.diagnostics[0].message;
    assert!(message.contains("placeholder rectangle"), "{message}");
    assert!(
        message.contains("qtext.extent_unknown"),
        "the case with no derivable extent is named: {message}"
    );
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
