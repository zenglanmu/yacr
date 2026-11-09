//! The display grid has **two** gates, and the render overlay must compose them
//! with AND: the persisted `view.overlays.grid` preference and the session's live
//! `GRID` switch (F7). Both default **off**: reference grid chrome is opt-in, so
//! a fresh session draws no grid until the user opens both gates.
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

    // Preference on + session switch defaulting off: the grid is hidden until
    // the user opens the session gate with `GRID`/F7.
    assert!(core.settings().grid, "the persisted preference is on");
    assert!(
        !core.controller.borrow().grid_enabled(),
        "the session switch defaults off"
    );
    assert!(
        !grid_overlay_visible(&core),
        "the closed session gate keeps the grid hidden even with the preference on"
    );

    // The real `GRID` selector opens the session gate: drawn, without touching
    // the persisted preference.
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID is a session/display command");
    assert!(core.controller.borrow().grid_enabled());
    assert!(
        grid_overlay_visible(&core),
        "both gates are open, so the overlay input is on"
    );
    assert!(
        core.settings().grid,
        "toggling the session switch never rewrites the preference"
    );

    // Closing the session gate hides it again.
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID toggles back");
    assert!(!grid_overlay_visible(&core));
}

#[test]
fn a_disabled_preference_keeps_the_grid_off_even_with_the_session_switch_on() {
    // The host default leaves the overlay preference off.
    let core = core_with_settings(Rc::new(SettingsConfig::default()));
    core.load_startup();
    assert!(!core.settings().grid);

    assert!(
        !core.controller.borrow().grid_enabled(),
        "the session switch also defaults off"
    );
    assert!(!grid_overlay_visible(&core));

    // Opening the session switch is not enough while the preference is off.
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID enables the session switch");
    assert!(core.controller.borrow().grid_enabled());
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
fn an_options_grid_write_drives_the_live_session_switch() {
    let core = core_with_settings(Rc::new(SettingsConfig::default()));
    core.load_startup();
    assert!(!core.controller.borrow().grid_enabled());
    assert!(!grid_overlay_visible(&core));

    // The explicit Options write is a user choice: it persists the preference
    // **and** opens the live session gate, so the grid appears immediately.
    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .expect("the grid preference is writable");
    assert!(core.settings().grid);
    assert!(
        core.controller.borrow().grid_enabled(),
        "the Options write also opens the live session gate"
    );
    assert!(grid_overlay_visible(&core));

    // F7 still closes only the live switch; the persisted preference stays on.
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID disables the session switch");
    assert!(!core.controller.borrow().grid_enabled());
    assert!(
        !grid_overlay_visible(&core),
        "a closed session switch still hides the grid"
    );
    assert!(core.settings().grid, "the preference was not rewritten");

    // Re-opening the live switch restores the grid under the same preference.
    core.command(
        cad_app::CommandId::ToggleGrid,
        cad_app::CommandPayload::None,
    )
    .expect("GRID re-enables the session switch");
    assert!(grid_overlay_visible(&core));
}
