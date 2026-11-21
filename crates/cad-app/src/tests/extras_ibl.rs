//! `IBL` session-only PBR environment-lighting switch.
//!
//! The token is session state: it never writes the drawing, bumps the revision
//! or records history. A bare `IBL` toggles; `IBL ON|OFF` (and the `1`/`0`
//! spellings of the command line) set it explicitly. These are contract tests —
//! no GPU, pixel or performance claim.

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
fn ibl_defaults_to_image_based_and_maps_the_accepted_spellings() {
    let (_app, mut session) = application_with_document();
    // Image-based lighting is exactly the procedural environment the PBR shader
    // already renders, so it is the default (the toggle stays backwards-safe).
    assert_eq!(
        session.view_extras.pbr_environment,
        PbrEnvironment::ImageBased
    );
    assert!(session.view_extras.pbr_environment.is_image_based());
    assert_eq!(PbrEnvironment::ImageBased.name(), "IBL");
    assert_eq!(PbrEnvironment::Legacy.name(), "LEGACY");
    assert!(!PbrEnvironment::Legacy.is_image_based());

    for spelling in ["IBL", "ibl", "ON", "on", "1", "IMAGEBASED", " imagebased "] {
        session
            .view_extras
            .set_pbr_environment(spelling)
            .unwrap_or_else(|error| panic!("IBL {spelling:?} is admitted: {error}"));
        assert_eq!(
            session.view_extras.pbr_environment,
            PbrEnvironment::ImageBased,
            "`{spelling}` maps to IBL"
        );
    }
    for spelling in ["LEGACY", "legacy", "OFF", "off", "0"] {
        session
            .view_extras
            .set_pbr_environment(spelling)
            .unwrap_or_else(|error| panic!("IBL {spelling:?} is admitted: {error}"));
        assert_eq!(
            session.view_extras.pbr_environment,
            PbrEnvironment::Legacy,
            "`{spelling}` maps to LEGACY"
        );
    }

    // Every other value is a named refusal, and the stored token survives it.
    for refused in ["maybe", "2", "HDR", ""] {
        let error = session
            .view_extras
            .set_pbr_environment(refused)
            .expect_err("an unknown IBL value must be refused");
        assert!(
            error.to_string().contains("ibl.invalid_value"),
            "the refusal must be named: {error}"
        );
        assert_eq!(
            session.view_extras.pbr_environment,
            PbrEnvironment::Legacy,
            "`{refused}` must not change the stored token"
        );
    }
}

#[test]
fn bare_ibl_toggles_the_session_token_without_touching_the_drawing() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);

    // A bare `IBL` is the toggle form: ImageBased (the default) → Legacy.
    let outcome = execute(&mut app, &mut session, CommandId::Ibl, CommandPayload::None)
        .expect("bare IBL toggles");
    assert_eq!(session.view_extras.pbr_environment, PbrEnvironment::Legacy);
    assert!(
        outcome.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "ibl.value"
                && diagnostic.message.contains("IBL")
                && diagnostic.message.contains("OFF")
                && diagnostic.message.contains("session state only")
        }),
        "the outcome names the active switch and its session-only scope: {:?}",
        outcome.diagnostics
    );
    assert_eq!(revision(&app), before, "IBL never writes the drawing");
    assert!(!app.can_undo(&DocumentId(1)), "IBL records no undo step");

    // …and toggles straight back to ImageBased.
    execute(&mut app, &mut session, CommandId::Ibl, CommandPayload::None)
        .expect("bare IBL toggles back");
    assert_eq!(
        session.view_extras.pbr_environment,
        PbrEnvironment::ImageBased
    );
    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}

#[test]
fn ibl_sets_explicit_values_and_refuses_a_foreign_payload() {
    let (mut app, mut session) = application_with_document();
    let before = revision(&app);

    execute(
        &mut app,
        &mut session,
        CommandId::Ibl,
        CommandPayload::Ibl(false),
    )
    .expect("IBL OFF is admitted");
    assert_eq!(session.view_extras.pbr_environment, PbrEnvironment::Legacy);

    execute(
        &mut app,
        &mut session,
        CommandId::Ibl,
        CommandPayload::Ibl(true),
    )
    .expect("IBL ON is admitted");
    assert_eq!(
        session.view_extras.pbr_environment,
        PbrEnvironment::ImageBased
    );

    // A payload that is not this command's own is refused, never treated as a
    // toggle.
    let error = execute(
        &mut app,
        &mut session,
        CommandId::Ibl,
        CommandPayload::ViewRes(100),
    )
    .expect_err("a foreign payload must be refused");
    assert!(
        error.to_string().contains("ibl.invalid_payload"),
        "the refusal must be named: {error}"
    );
    assert_eq!(
        session.view_extras.pbr_environment,
        PbrEnvironment::ImageBased,
        "the foreign payload must not change the stored token"
    );

    assert_eq!(revision(&app), before);
    assert!(!app.can_undo(&DocumentId(1)));
}
