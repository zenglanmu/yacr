//! `VSCURRENT` session-only 3D body visual style.
//!
//! The token is session state: it never writes the drawing and reaches the
//! display as `cad_representation::RepresentationContext::body_wireframe`.
//! These are contract tests — no GPU, pixel or performance claim.

use super::*;

fn execute(
    app: &mut Application,
    session: &mut SessionState,
    id: CommandId,
    payload: CommandPayload,
) -> CadResult<CommandOutcome> {
    app.execute(session, command(id, payload))
}

fn revision(app: &Application) -> Revision {
    app.drawing(DocumentId(1))
        .expect("the fixture document is open")
        .revision()
}

#[test]
fn vscurrent_defaults_to_shaded_and_maps_the_accepted_spellings() {
    // This build diverges from AutoCAD's 2D-wireframe initial value on purpose:
    // `Shaded` is exactly the pre-VSCURRENT filled body display.
    let (mut app, mut session) = application_with_document();
    assert_eq!(
        session.view_extras.visual_style,
        VisualStyleToken::Shaded,
        "the session default is SHADED"
    );
    assert_eq!(VisualStyleToken::Shaded.name(), "SHADED");
    assert_eq!(VisualStyleToken::Wireframe.name(), "WIREFRAME");

    for spelling in ["WIREFRAME", "wireframe", "2DWIREFRAME", "2d"] {
        execute(
            &mut app,
            &mut session,
            CommandId::VisualStyle,
            CommandPayload::VisualStyle(spelling.into()),
        )
        .unwrap_or_else(|error| panic!("VSCURRENT {spelling} is admitted: {error}"));
        assert_eq!(
            session.view_extras.visual_style,
            VisualStyleToken::Wireframe,
            "`{spelling}` maps to WIREFRAME"
        );
    }
    for spelling in ["SHADED", "shaded", "SHADEDWITHEDGES", "ShadedWithEdges"] {
        execute(
            &mut app,
            &mut session,
            CommandId::VisualStyle,
            CommandPayload::VisualStyle(spelling.into()),
        )
        .unwrap_or_else(|error| panic!("VSCURRENT {spelling} is admitted: {error}"));
        assert_eq!(
            session.view_extras.visual_style,
            VisualStyleToken::Shaded,
            "`{spelling}` maps to SHADED"
        );
    }
}

#[test]
fn vscurrent_sets_the_session_token_and_refuses_other_styles_by_name() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);

    let outcome = execute(
        &mut app,
        &mut session,
        CommandId::VisualStyle,
        CommandPayload::VisualStyle("WIREFRAME".into()),
    )
    .expect("VSCURRENT WIREFRAME is admitted");
    assert_eq!(
        session.view_extras.visual_style,
        VisualStyleToken::Wireframe
    );
    assert!(
        outcome.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "vscurrent"
                && diagnostic.message.contains("WIREFRAME")
                && diagnostic.message.contains("session state only")
                && diagnostic.message.contains("3D bodies")
        }),
        "the outcome names the active style and its session-only body scope: {:?}",
        outcome.diagnostics
    );
    assert_eq!(revision(&app), before, "VSCURRENT never writes the drawing");
    assert!(
        !app.can_undo(&DocumentId(1)),
        "VSCURRENT records no undo step"
    );

    // Every unimplemented style is a named refusal, and the stored token
    // survives it (no silent mapping onto the two implemented tokens).
    for refused_style in [
        "CONCEPTUAL",
        "REALISTIC",
        "HIDDEN",
        "SKETCHY",
        "XRAY",
        "VISUALSTYLES",
    ] {
        let refused = execute(
            &mut app,
            &mut session,
            CommandId::VisualStyle,
            CommandPayload::VisualStyle(refused_style.into()),
        );
        let error = format!(
            "{}",
            refused.expect_err("an unimplemented style must be refused")
        );
        assert!(
            error.contains("vscurrent.style_not_implemented"),
            "the refusal must be named: {error}"
        );
        assert_eq!(
            session.view_extras.visual_style,
            VisualStyleToken::Wireframe,
            "`{refused_style}` must not change the stored token"
        );
    }
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}
