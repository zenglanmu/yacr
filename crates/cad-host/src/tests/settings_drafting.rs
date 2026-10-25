//! Drafting session switches as persisted settings: `SNAP` (grid snap, F9),
//! `ORTHO` (F8) and the `SNAPUNIT` spacing.
//!
//! These three are session/display state (no database write, no history), but the
//! user's choice is persisted through exactly the same allowed-leaf preference
//! machinery as `interaction.snapEnabled`/`hoverPreview`, and re-applied at
//! startup and after every authoritative document replacement. The contracts here
//! cover the closed key table, the live application through the existing typed
//! command path, the refusal rules and the durable round trip.

use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

fn undo_depth(core: &HostCore) -> usize {
    let controller = core.controller.borrow();
    controller
        .application
        .history
        .get(&controller.document_id)
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

/// The documented session defaults: `SNAP` off, `ORTHO` off, spacing 10 (the
/// declared `DEFAULT_SNAP_SPACING`). No preference was ever written.
#[test]
fn the_drafting_settings_default_to_the_session_defaults() {
    let core = core();
    let settings = core.settings();
    assert!(!settings.grid_snap_enabled, "SNAP starts off");
    assert!(!settings.ortho_enabled, "ORTHO starts off");
    assert_eq!(
        settings.snap_spacing,
        cad_app::DEFAULT_SNAP_SPACING,
        "the declared 10.0 drawing-unit spacing"
    );
    let controller = core.controller.borrow();
    assert!(!controller.snap_grid_enabled());
    assert!(!controller.ortho_enabled());
    assert_eq!(controller.snap_spacing(), cad_app::DEFAULT_SNAP_SPACING);
    assert_eq!(
        core.config_snapshot().effective.interaction.snap_spacing,
        None,
        "an absent preference stays `None`, not a fabricated number"
    );
}

/// `ALL` is the one key table: each new key is present exactly once, parses from
/// its camelCase wire spelling and names the leaf it is persisted under.
#[test]
fn the_three_drafting_keys_are_in_the_closed_table() {
    assert_eq!(
        CadSettingKey::ALL.len(),
        17,
        "three more writable keys on top of the fourteen documented ones"
    );
    for (key, wire) in [
        (CadSettingKey::GridSnapEnabled, "gridSnapEnabled"),
        (CadSettingKey::OrthoEnabled, "orthoEnabled"),
        (CadSettingKey::SnapSpacing, "snapSpacing"),
    ] {
        assert_eq!(
            CadSettingKey::ALL
                .into_iter()
                .filter(|candidate| *candidate == key)
                .count(),
            1,
            "{key:?} is declared exactly once"
        );
        assert_eq!(key.key(), wire);
        assert_eq!(CadSettingKey::parse(wire), Some(key));
    }
    assert_eq!(
        CadSettingKey::parse("interaction.snapSpacing"),
        None,
        "the wire key is the leaf, not the dotted path"
    );
}

/// Each key persists its own leaf and the snapshot reports the live value.
#[test]
fn the_drafting_keys_are_persisted_and_reported() {
    let (core, config) = settings_core();

    core.apply_setting(CadSettingKey::GridSnapEnabled, CadSettingValue::Bool(true))
        .unwrap();
    assert!(core.settings().grid_snap_enabled);
    assert!(core.controller.borrow().snap_grid_enabled());
    let written = persisted(&config).expect("the SNAP write persists");
    assert!(
        written.contains(r#""gridSnapEnabled":true"#),
        "the SNAP switch is persisted under interaction.gridSnapEnabled: {written}"
    );

    core.apply_setting(CadSettingKey::OrthoEnabled, CadSettingValue::Bool(true))
        .unwrap();
    assert!(core.settings().ortho_enabled);
    assert!(core.controller.borrow().ortho_enabled());
    let written = persisted(&config).expect("the ORTHO write persists");
    assert!(
        written.contains(r#""orthoEnabled":true"#),
        "the ORTHO switch is persisted under interaction.orthoEnabled: {written}"
    );
    assert!(
        written.contains(r#""gridSnapEnabled":true"#),
        "the earlier SNAP leaf is kept: {written}"
    );

    // The spacing travels as a real quantity; an integer wire number is accepted
    // too and lands as the same drawing-unit value.
    core.apply_setting(CadSettingKey::SnapSpacing, CadSettingValue::Real(2.5))
        .unwrap();
    assert_eq!(core.settings().snap_spacing, 2.5);
    assert_eq!(core.controller.borrow().snap_spacing(), 2.5);
    let written = persisted(&config).expect("the SNAPUNIT write persists");
    assert!(
        written.contains(r#""snapSpacing":2.5"#),
        "the spacing is persisted under interaction.snapSpacing: {written}"
    );

    core.apply_setting(CadSettingKey::SnapSpacing, CadSettingValue::Number(25))
        .unwrap();
    assert_eq!(core.settings().snap_spacing, 25.0);
    assert_eq!(core.controller.borrow().snap_spacing(), 25.0);
    let written = persisted(&config).expect("the integer spacing write persists");
    assert!(written.contains(r#""snapSpacing":25"#), "{written}");

    // Only the interaction leaves reach durable storage.
    for forbidden in ["features", "components", "preset", "commandOverrides"] {
        assert!(
            !written.contains(forbidden),
            "the preference projection must not carry {forbidden}: {written}"
        );
    }
}

/// Writing the value the session already holds must not dispatch the toggle: the
/// toggle flips, so an unconditional dispatch would leave the switch wrong, and a
/// no-op write must not stage anything either.
#[test]
fn a_matching_drafting_write_is_a_no_op() {
    let (core, config) = settings_core();
    let before = undo_depth(&core);

    for _ in 0..2 {
        core.apply_setting(CadSettingKey::GridSnapEnabled, CadSettingValue::Bool(true))
            .unwrap();
        core.apply_setting(CadSettingKey::OrthoEnabled, CadSettingValue::Bool(true))
            .unwrap();
        core.apply_setting(CadSettingKey::SnapSpacing, CadSettingValue::Real(2.5))
            .unwrap();
    }

    assert!(
        core.settings().grid_snap_enabled,
        "a matching SNAP write never flips the switch back"
    );
    assert!(
        core.settings().ortho_enabled,
        "a matching ORTHO write never flips the switch back"
    );
    assert_eq!(
        core.settings().snap_spacing,
        2.5,
        "a matching SNAPUNIT write keeps the spacing"
    );
    assert_eq!(undo_depth(&core), before, "no history entry was created");
    // The last write still refreshed the same two leaves: the preference is a
    // single document, never an append log of duplicate values.
    let written = persisted(&config).expect("each write persists");
    assert_eq!(
        written.matches("\"gridSnapEnabled\"").count(),
        1,
        "one leaf, not a repeated key: {written}"
    );
    assert_eq!(written.matches("\"snapSpacing\"").count(), 1, "{written}");
}

/// A non-finite or non-positive spacing is an explicit refusal, not a default.
#[test]
fn invalid_snap_spacing_is_refused_explicitly() {
    let (core, config) = settings_core();
    core.apply_setting(CadSettingKey::SnapSpacing, CadSettingValue::Real(12.0))
        .unwrap();
    let written_before = persisted(&config).expect("the valid spacing persists");

    for value in [
        CadSettingValue::Number(0),
        CadSettingValue::Real(0.0),
        CadSettingValue::Real(-1.0),
        CadSettingValue::Real(f64::NAN),
        CadSettingValue::Real(f64::INFINITY),
        CadSettingValue::Real(f64::NEG_INFINITY),
        CadSettingValue::Bool(true),
        CadSettingValue::Text("25".to_string()),
    ] {
        let error = core
            .apply_setting(CadSettingKey::SnapSpacing, value.clone())
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{value:?} must be refused with the stable code: {error:?}"
        );
        assert_eq!(
            core.settings().snap_spacing,
            12.0,
            "a refused spacing never rewrites the live value"
        );
    }
    assert_eq!(
        persisted(&config).as_deref(),
        Some(written_before.as_str()),
        "a refused write must not change the preference file"
    );
    // A wrong shape for the two boolean switches is refused the same way.
    for value in [
        CadSettingValue::Number(1),
        CadSettingValue::Text("on".to_string()),
        CadSettingValue::Real(1.0),
    ] {
        for key in [CadSettingKey::GridSnapEnabled, CadSettingKey::OrthoEnabled] {
            let error = core.apply_setting(key, value.clone()).unwrap_err();
            assert!(
                matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE)
            );
        }
    }
    assert!(!core.settings().grid_snap_enabled);
    assert!(!core.settings().ortho_enabled);
}

/// All five persisted interaction values seed the live session at startup.
#[test]
fn startup_preference_seeds_all_five_interaction_values() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"interaction":{"snapEnabled":false,"hoverPreview":false,
             "gridSnapEnabled":true,"orthoEnabled":true,"snapSpacing":25}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();

    let settings = core.settings();
    assert!(!settings.snap_enabled);
    assert!(!settings.hover_preview);
    assert!(settings.grid_snap_enabled);
    assert!(settings.ortho_enabled);
    assert_eq!(settings.snap_spacing, 25.0);
    let controller = core.controller.borrow();
    assert!(controller.snap_grid_enabled());
    assert!(controller.ortho_enabled());
    assert_eq!(controller.snap_spacing(), 25.0);
}

/// The persisted drafting values are re-applied after a document replacement
/// (which rebuilds `SessionState`), through the one seeding call site.
#[test]
fn the_seeded_drafting_values_survive_a_document_replacement() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"interaction":{"gridSnapEnabled":true,"orthoEnabled":true,"snapSpacing":25}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();
    assert!(core.settings().grid_snap_enabled);

    core.controller.borrow_mut().new_blank_document().unwrap();
    core.note_document_replaced();

    let settings = core.settings();
    assert!(
        settings.grid_snap_enabled,
        "the persisted SNAP choice survives open/new"
    );
    assert!(
        settings.ortho_enabled,
        "the persisted ORTHO choice survives open/new"
    );
    assert_eq!(
        settings.snap_spacing, 25.0,
        "the persisted SNAPUNIT spacing survives open/new"
    );
}

/// An absent drafting preference keeps the documented session default: seeding
/// must not invent `false`/`0` for a leaf the user never chose.
#[test]
fn absent_drafting_preferences_keep_the_session_defaults() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"interaction":{"snapEnabled":false}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();
    let settings = core.settings();
    assert!(
        !settings.grid_snap_enabled,
        "SNAP keeps its session default"
    );
    assert!(!settings.ortho_enabled, "ORTHO keeps its session default");
    assert_eq!(
        settings.snap_spacing,
        cad_app::DEFAULT_SNAP_SPACING,
        "the spacing keeps the declared default"
    );
    // A replacement does not invent them either.
    core.controller.borrow_mut().new_blank_document().unwrap();
    core.note_document_replaced();
    assert!(!core.settings().grid_snap_enabled);
    assert!(!core.settings().ortho_enabled);
    assert_eq!(core.settings().snap_spacing, cad_app::DEFAULT_SNAP_SPACING);
}

/// The whole set round-trips through the durable projection and a fresh startup.
#[test]
fn the_drafting_settings_survive_a_restart() {
    let (core, config) = settings_core();
    core.apply_setting(CadSettingKey::GridSnapEnabled, CadSettingValue::Bool(true))
        .unwrap();
    core.apply_setting(CadSettingKey::OrthoEnabled, CadSettingValue::Bool(true))
        .unwrap();
    core.apply_setting(CadSettingKey::SnapSpacing, CadSettingValue::Real(2.5))
        .unwrap();
    let persisted = persisted(&config).expect("each write persists");

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&persisted)));
    reloaded.load_startup();
    assert_eq!(reloaded.settings(), core.settings());
    assert!(reloaded.settings().grid_snap_enabled);
    assert!(reloaded.settings().ortho_enabled);
    assert_eq!(reloaded.settings().snap_spacing, 2.5);
}
