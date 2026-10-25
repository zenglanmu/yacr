//! The display grid has **two** gates, and the render overlay must compose them
//! with AND: the persisted `view.overlays.grid` preference and the session's live
//! `GRID` switch (F7).
//!
//! These contracts read the overlay input the renderer actually draws from
//! (`HostCore::sync_render`), not a second copy of the decision.

use super::settings::{core_with_settings, SettingsConfig};
use super::*;

/// A preference document that turns the display grid on.
const GRID_ON: &str = r#"{"view":{"overlays":{"grid":true}}}"#;

fn grid_overlay_visible(core: &HostCore) -> bool {
    core.sync_render().expect("the synthetic session syncs");
    core.render.overlays().visibility.grid
}

#[test]
fn the_grid_overlay_draws_only_when_the_preference_and_the_session_switch_agree() {
    let core = core_with_settings(Rc::new(SettingsConfig::with_preference(GRID_ON)));
    core.load_startup();

    // Preference on + session default on: the renderer is told to draw the grid.
    assert!(core.settings().grid, "the persisted preference is on");
    assert!(
        core.controller.borrow().grid_enabled(),
        "the session switch defaults on"
    );
    assert!(
        grid_overlay_visible(&core),
        "both gates are open, so the overlay input is on"
    );

    // The real `GRID` selector closes the session gate: off, without touching
    // the persisted preference.
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID is a session/display command");
    assert!(!core.controller.borrow().grid_enabled());
    assert!(
        !grid_overlay_visible(&core),
        "the session switch is off, so the grid is not drawn"
    );
    assert!(
        core.settings().grid,
        "toggling the session switch never rewrites the preference"
    );

    // Switching it back on restores the overlay, because the preference is still
    // on.
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID toggles back");
    assert!(grid_overlay_visible(&core));
}

#[test]
fn a_disabled_preference_keeps_the_grid_off_even_with_the_session_switch_on() {
    // The host default leaves the overlay preference off.
    let core = core_with_settings(Rc::new(SettingsConfig::default()));
    core.load_startup();
    assert!(!core.settings().grid);

    assert!(
        core.controller.borrow().grid_enabled(),
        "the session switch still defaults on"
    );
    assert!(
        !grid_overlay_visible(&core),
        "the preference gate is closed, so the session switch cannot enable the grid"
    );

    // Toggle off and back on: the AND still holds, so the overlay stays off. This
    // is the explicit AND contract — never "the last toggle wins".
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID disables the session switch");
    assert!(!grid_overlay_visible(&core));
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID re-enables the session switch");
    assert!(core.controller.borrow().grid_enabled());
    assert!(
        !grid_overlay_visible(&core),
        "with the preference off the overlay stays off after toggling back on"
    );
}

#[test]
fn enabling_the_preference_at_runtime_respects_the_live_session_switch() {
    let core = core_with_settings(Rc::new(SettingsConfig::default()));
    core.load_startup();
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID disables the session switch");
    assert!(!grid_overlay_visible(&core));

    // The Options write opens the preference gate, but the session switch is off,
    // so the overlay stays off — the other half of the AND.
    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .expect("the grid preference is writable");
    assert!(core.settings().grid);
    assert!(
        !grid_overlay_visible(&core),
        "a closed session switch still wins over an open preference"
    );

    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID re-enables the session switch");
    assert!(grid_overlay_visible(&core));
}
