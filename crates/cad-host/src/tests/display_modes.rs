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
