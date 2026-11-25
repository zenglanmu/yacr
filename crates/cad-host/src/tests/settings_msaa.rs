//! Canvas anti-aliasing (`view.msaa.mode`).
//!
//! The choice is a closed three-token setting persisted through the same
//! allowed-leaf preference path as the rest of the Options surface. The live path
//! is the next `ViewSnapshot`: the host resolves the token to the renderer's
//! sample count (`1`/`2`/`4`). These contracts cover the closed table, the
//! refusal rules, the durable round-trip and that frame input — never a GPU
//! allocation or a rendered pixel.

use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

/// The documented default is `4x`, and it is the effective config's value, not a
/// second source of truth.
#[test]
fn msaa_defaults_to_4x() {
    let core = core();
    let settings = core.settings();
    assert_eq!(settings.msaa_mode, "4x");
    assert_eq!(
        core.config_snapshot().effective.view.msaa.mode,
        cad_app::viewer_config::MsaaMode::X4,
        "the snapshot reports the effective config, not a local default"
    );
    // The token is what a direct frame caller gets: 4x is four samples.
    assert_eq!(
        cad_app::viewer_config::MsaaMode::X4.samples(),
        4,
        "4x is four samples"
    );
}

/// The key is in the one closed table, its wire spelling is `msaaMode`, and the
/// schema token set is the only accepted value set.
#[test]
fn the_msaa_key_is_in_the_closed_table() {
    let key = CadSettingKey::MsaaMode;
    assert_eq!(
        CadSettingKey::ALL.len(),
        34,
        "one writable key per contract"
    );
    assert_eq!(
        CadSettingKey::ALL
            .into_iter()
            .filter(|candidate| *candidate == key)
            .count(),
        1,
        "the key is declared exactly once"
    );
    for candidate in CadSettingKey::ALL {
        assert_eq!(CadSettingKey::parse(candidate.key()), Some(candidate));
    }
    assert_eq!(key.key(), "msaaMode");
    assert_eq!(CadSettingKey::parse("msaaMode"), Some(key));
    assert_eq!(
        CadSettingKey::parse("mode"),
        None,
        "the wire key names the option, not the schema leaf"
    );
    assert_eq!(
        CadSettingKey::parse("view.msaa.mode"),
        None,
        "the wire key is never the dotted path"
    );
    assert!(
        HOST_ALLOWED_PATHS_JSON.contains("view.msaa.mode"),
        "the leaf must be host-writable"
    );
}

/// All three tokens are accepted, reported and persisted under their real leaf,
/// and each reaches the next frame input as its sample count.
#[test]
fn every_msaa_token_is_accepted_persisted_and_reaches_the_frame() {
    let (core, config) = settings_core();
    for (token, samples) in [("off", 1), ("2x", 2), ("4x", 4)] {
        core.apply_setting(
            CadSettingKey::MsaaMode,
            CadSettingValue::Text(token.to_string()),
        )
        .unwrap();
        assert_eq!(core.settings().msaa_mode, token);
        assert_eq!(
            core.render.view_snapshot().msaa_samples,
            samples,
            "{token} must publish its sample count on the next frame input"
        );
        let written = persisted(&config).expect("each write persists");
        let document: serde_json::Value =
            serde_json::from_str(&written).expect("the persisted preference is JSON");
        assert_eq!(
            document["view"]["msaa"]["mode"],
            serde_json::Value::from(token),
            "the choice is persisted under view.msaa.mode: {written}"
        );
    }
}

/// Everything outside the three tokens is refused with the stable code and
/// changes nothing — never clamped to the nearest sample count.
#[test]
fn invalid_msaa_values_are_refused_without_writing() {
    let (core, config) = settings_core();
    for value in [
        // A token-shaped but unsupported level.
        CadSettingValue::Text("8x".to_string()),
        CadSettingValue::Text("1x".to_string()),
        CadSettingValue::Text("off ".to_string()),
        CadSettingValue::Text(String::new()),
        // The renderer's numeric sample count is not the setting's shape.
        CadSettingValue::Text("4".to_string()),
        CadSettingValue::Number(4),
        CadSettingValue::Bool(true),
        CadSettingValue::Real(4.0),
    ] {
        let error = core
            .apply_setting(CadSettingKey::MsaaMode, value.clone())
            .expect_err("only the three tokens are accepted");
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{value:?} must be refused with the stable code: {error:?}"
        );
    }
    assert!(
        persisted(&config).is_none(),
        "a rejected write must not change the preference file"
    );
    assert_eq!(core.settings().msaa_mode, "4x");
}

/// The choice survives a restart: the persisted document is re-read by the same
/// startup pass as every other setting, and the first frame input carries it.
#[test]
fn the_msaa_choice_roundtrips_through_the_persisted_preference() {
    let (core, config) = settings_core();
    core.apply_setting(
        CadSettingKey::MsaaMode,
        CadSettingValue::Text("2x".to_string()),
    )
    .unwrap();
    let written = persisted(&config).expect("the write persists");

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert!(
        reloaded.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        reloaded.startup_diagnostics.borrow()
    );
    assert_eq!(reloaded.settings().msaa_mode, "2x");
    assert_eq!(reloaded.settings(), core.settings());
    reloaded.sync_render().unwrap();
    assert_eq!(reloaded.render.view_snapshot().msaa_samples, 2);
}
