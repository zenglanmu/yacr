//! `FILL` / `QTEXT` / `VIEWRES` at the host boundary: the session token a
//! command stores is the value the render session's base build receives.
//!
//! This is the last hop of the promotion contract — the command flips
//! `cad_app::SessionState`, `HostCore::sync_render` resolves it into
//! `cad_app::render_scene::DisplaySettings`, and the engine's settings value (and
//! therefore the scene base) changes. It is a CPU/session contract: no GPU, pixel
//! or performance claim.

use super::*;

fn set_fill(core: &HostCore, enabled: bool) {
    core.controller
        .borrow_mut()
        .session
        .drafting_extras
        .set_fill(enabled);
}

fn set_qtext(core: &HostCore, enabled: bool) {
    core.controller
        .borrow_mut()
        .session
        .drafting_extras
        .set_qtext(enabled);
}

/// `FILL` and `QTEXT` default to AutoCAD's initial states, reach the render
/// session on the first sync, and a flip advances the input exactly once.
#[test]
fn fill_and_qtext_tokens_reach_the_render_session() {
    let core = core();
    core.sync_render().expect("the baseline sync publishes");
    // FILL starts ON and QTEXT OFF; both are already in the engine's settings,
    // so the baseline sync carries them and a repeat is a no-op.
    let baseline = core.render.input_revision();
    core.sync_render().expect("the repeat sync publishes");
    assert_eq!(
        core.render.input_revision(),
        baseline,
        "an unchanged FILL/QTEXT must not churn the base"
    );

    set_fill(&core, false);
    core.sync_render().expect("the FILL OFF sync publishes");
    let after_fill = core.render.input_revision();
    assert!(
        after_fill > baseline,
        "FILL OFF drops fill geometry, so the display base must be rebuilt"
    );

    set_qtext(&core, true);
    core.sync_render().expect("the QTEXT ON sync publishes");
    let after_qtext = core.render.input_revision();
    assert!(
        after_qtext > after_fill,
        "QTEXT ON replaces glyph batches, so the display base must be rebuilt"
    );

    // Back to the initial states: the input advances again, and the token really
    // is the value the engine holds (not a one-way latch).
    set_fill(&core, true);
    set_qtext(&core, false);
    core.sync_render().expect("the restore sync publishes");
    assert!(core.render.input_revision() > after_qtext);
}

/// `VSCURRENT` maps to the one body-display flag: `Wireframe` sets
/// `body_wireframe`, `Shaded` (the session default) clears it.
#[test]
fn vscurrent_maps_to_the_body_wireframe_display_flag() {
    let mut session = cad_app::SessionState::new(cad_domain::DocumentId(1), cad_app::AppMode::Work);
    assert_eq!(
        session.view_extras.visual_style,
        cad_app::VisualStyleToken::Shaded,
        "the session default must be SHADED"
    );
    assert!(
        !crate::state::display_settings(&session).body_wireframe,
        "SHADED keeps the filled body display"
    );

    session
        .view_extras
        .set_visual_style("WIREFRAME")
        .expect("WIREFRAME is admitted");
    assert!(
        crate::state::display_settings(&session).body_wireframe,
        "WIREFRAME requests edges-only bodies"
    );

    session
        .view_extras
        .set_visual_style("2D")
        .expect("2D is an accepted wireframe spelling");
    assert!(
        crate::state::display_settings(&session).body_wireframe,
        "2D is the same wireframe token"
    );

    session
        .view_extras
        .set_visual_style("SHADEDWITHEDGES")
        .expect("SHADEDWITHEDGES is admitted");
    assert!(
        !crate::state::display_settings(&session).body_wireframe,
        "SHADEDWITHEDGES maps back to the shaded body display"
    );
}

/// `VIEWRES` is a session percentage with a display consumer: a value below the
/// analytic-conic default schedules a rebuild, and an unchanged value does not.
#[test]
fn viewres_token_reaches_the_render_session() {
    let core = core();
    core.sync_render().expect("the baseline sync publishes");
    let baseline = core.render.input_revision();

    core.controller
        .borrow_mut()
        .session
        .view_extras
        .set_viewres(50)
        .expect("50 is inside 1..=1000");
    core.sync_render().expect("the VIEWRES sync publishes");
    let coarsened = core.render.input_revision();
    assert!(
        coarsened > baseline,
        "VIEWRES changes the chord budget, so the display base must be rebuilt"
    );

    core.sync_render().expect("the repeat sync publishes");
    assert_eq!(
        core.render.input_revision(),
        coarsened,
        "an unchanged VIEWRES must not churn the base"
    );
}

/// `IBL` maps to the engine `ViewSnapshot.ibl_enabled` the host publishes.
///
/// The narrowest observable seam is the render session's own published snapshot
/// (`HostCore::render.view_snapshot()`); the session token has no
/// display-settings or scene-base consumer, so it is not visible through
/// [`display_settings`]. Image-based lighting is the session default,
/// `IBL OFF` clears it, and the bare `IBL` selector toggles it back. The switch
/// is session-only: it produces no ChangeSet and records no undo entry, so the
/// document revision must not move.
#[test]
fn the_published_view_snapshot_carries_the_session_ibl_switch() {
    let core = core();
    let document = core.controller.borrow().document_id;
    let revision_before = core.controller.borrow().drawing().unwrap().revision();

    core.sync_render().expect("the baseline sync publishes");
    assert!(
        core.render.view_snapshot().ibl_enabled,
        "IBL defaults to ImageBased, so the published view enables image-based lighting"
    );

    core.command(CommandId::Ibl, CommandPayload::Ibl(false))
        .expect("IBL OFF is a session-only switch");
    core.sync_render().expect("the IBL OFF sync publishes");
    assert!(
        !core.render.view_snapshot().ibl_enabled,
        "IBL OFF must reach the engine snapshot as the legacy lighting path"
    );

    core.command(CommandId::Ibl, CommandPayload::None)
        .expect("bare IBL toggles");
    core.sync_render().expect("the bare-IBL sync publishes");
    assert!(
        core.render.view_snapshot().ibl_enabled,
        "the bare IBL toggle returns to image-based lighting"
    );

    let revision_after = core.controller.borrow().drawing().unwrap().revision();
    assert_eq!(
        revision_after, revision_before,
        "IBL is session state: no ChangeSet is produced, so the drawing revision stays put"
    );
    assert!(
        !core.controller.borrow().application.can_undo(&document),
        "IBL records no undo entry"
    );
}

/// `SHADOWS`/`FOG` reach the published view snapshot from the document record.
///
/// The document's `tables/display.json` record is the single source: the host
/// resolves the sun to a unit vector and the fog band into the frame input, and
/// a write repaints through the ordinary snapshot path (no scene rebuild). This
/// is a CPU/session contract: no GPU or pixel claim, and cast shadows remain
/// unimplemented.
#[test]
fn the_published_view_snapshot_carries_the_document_sun_and_fog() {
    use cad_app::DisplayCommand;
    let core = core();
    core.sync_render().expect("the baseline sync publishes");
    assert_eq!(core.render.view_snapshot().sun, None);
    assert_eq!(core.render.view_snapshot().fog, None);

    core.command(
        CommandId::Shadows,
        CommandPayload::Display(DisplayCommand::Shadows(true)),
    )
    .expect("shadows on");
    core.command(
        CommandId::Shadows,
        CommandPayload::Display(DisplayCommand::Sun {
            azimuth: 0.0,
            elevation: 90.0,
        }),
    )
    .expect("sun");
    core.command(
        CommandId::Fog,
        CommandPayload::Display(DisplayCommand::FogRange {
            start: 2.0,
            end: 20.0,
        }),
    )
    .expect("fog range");
    core.command(
        CommandId::Fog,
        CommandPayload::Display(DisplayCommand::Fog(true)),
    )
    .expect("fog on");
    core.sync_render()
        .expect("the display write sync publishes");

    let view = core.render.view_snapshot();
    let sun = view.sun.expect("an enabled sun is a published direction");
    assert!((sun[2] - 1.0).abs() < 1e-9, "zenith points up: {sun:?}");
    assert!(sun[0].abs() < 1e-9 && sun[1].abs() < 1e-9, "{sun:?}");
    let fog = view.fog.expect("an enabled fog is a published band");
    assert_eq!((fog.start, fog.end), (2.0, 20.0));

    // Turning shadows/fog off clears the frame inputs again.
    core.command(
        CommandId::Shadows,
        CommandPayload::Display(DisplayCommand::Shadows(false)),
    )
    .expect("shadows off");
    core.command(
        CommandId::Fog,
        CommandPayload::Display(DisplayCommand::Fog(false)),
    )
    .expect("fog off");
    core.sync_render().expect("the off sync publishes");
    let view = core.render.view_snapshot();
    assert_eq!(view.sun, None);
    assert_eq!(view.fog, None);
}
