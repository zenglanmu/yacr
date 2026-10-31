//! The `interaction.leftDragPans` input-scheme preference.
//!
//! Unlike the F8/F9 switches this flag has no session state to seed: the 2D
//! gesture arbitration reads the effective config per event, so the contract here
//! is the closed key table, the persisted leaf, the refusal of a non-boolean and
//! the fact that a startup preference is honoured without any seeding call.

use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

/// `ALL` is the one key table: the new key is present exactly once, parses from
/// its camelCase wire spelling and names the leaf it is persisted under. The
/// default is the documented box-select scheme.
#[test]
fn the_left_drag_pans_key_is_in_the_closed_table() {
    let key = CadSettingKey::LeftDragPans;
    assert_eq!(
        CadSettingKey::ALL.len(),
        18,
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
    assert_eq!(key.key(), "leftDragPans");
    assert_eq!(CadSettingKey::parse("leftDragPans"), Some(key));
    assert_eq!(
        CadSettingKey::parse("interaction.leftDragPans"),
        None,
        "the wire key is the leaf, not the dotted path"
    );
    assert_eq!(
        CadSettingKey::parse("left_drag_pans"),
        None,
        "the wire spelling is camelCase, like the sibling interaction keys"
    );
    // The leaf is host-writable: without the allowance the write path would
    // reject the patch instead of persisting it.
    assert!(
        HOST_ALLOWED_PATHS_JSON.contains("interaction.leftDragPans"),
        "the leaf must be in the host's closed allowed-path policy"
    );

    // The default comes from the effective config, never a local constant.
    let core = core();
    assert!(!core.settings().left_drag_pans);
    assert!(!core.left_drag_pans());
    assert!(
        !core.config_snapshot().effective.interaction.left_drag_pans,
        "an absent preference is the schema's `false` default"
    );
}

/// A write persists `interaction.leftDragPans`, the snapshot and the gesture
/// accessor report it, and a fresh startup reads the same value back.
#[test]
fn a_left_drag_pans_write_persists_and_reloads() {
    let (core, config) = settings_core();

    core.apply_setting(CadSettingKey::LeftDragPans, CadSettingValue::Bool(true))
        .unwrap();
    assert!(core.settings().left_drag_pans);
    assert!(
        core.left_drag_pans(),
        "the gesture accessor reads the effective config"
    );
    let written = persisted(&config).expect("the write persists");
    let document: serde_json::Value =
        serde_json::from_str(&written).expect("the persisted preference is JSON");
    assert_eq!(
        document["interaction"]["leftDragPans"],
        serde_json::Value::Bool(true),
        "the flag is persisted under interaction.leftDragPans: {written}"
    );

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert!(
        reloaded.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        reloaded.startup_diagnostics.borrow()
    );
    assert_eq!(reloaded.settings(), core.settings());
    assert!(reloaded.settings().left_drag_pans);
    assert!(reloaded.left_drag_pans());

    // Switching back off is persisted too, and it is the same leaf, not a
    // second entry.
    core.apply_setting(CadSettingKey::LeftDragPans, CadSettingValue::Bool(false))
        .unwrap();
    assert!(!core.settings().left_drag_pans);
    assert!(!core.left_drag_pans());
    let written = persisted(&config).expect("the second write persists");
    assert!(written.contains(r#""leftDragPans":false"#), "{written}");
    assert_eq!(written.matches("\"leftDragPans\"").count(), 1, "{written}");
}

/// A non-boolean value is refused with the stable code and changes nothing.
#[test]
fn non_boolean_left_drag_pans_values_are_refused_without_writing() {
    let (core, config) = settings_core();
    for value in [
        CadSettingValue::Text("on".to_string()),
        CadSettingValue::Text("true".to_string()),
        CadSettingValue::Number(1),
        CadSettingValue::Real(1.0),
    ] {
        let error = core
            .apply_setting(CadSettingKey::LeftDragPans, value.clone())
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{value:?} must be refused with the stable code: {error:?}"
        );
    }
    assert!(
        persisted(&config).is_none(),
        "a refused write must not change the preference file"
    );
    assert!(!core.settings().left_drag_pans);
}

/// A startup preference is honoured as soon as the config is loaded: there is no
/// session seeding step (and therefore nothing to forget after a document
/// replacement), because the gesture reads the effective config.
#[test]
fn a_startup_left_drag_pans_preference_survives_without_seeding() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"interaction":{"leftDragPans":true}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();

    assert!(
        core.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        core.startup_diagnostics.borrow()
    );
    assert!(core.settings().left_drag_pans);
    assert!(core.left_drag_pans());

    // A document replacement rebuilds `SessionState`; a config-derived flag is
    // unaffected by construction.
    core.controller.borrow_mut().new_blank_document().unwrap();
    core.note_document_replaced();
    assert!(
        core.left_drag_pans(),
        "the persisted input scheme survives open/new"
    );

    // An absent preference keeps the documented `false` default.
    let defaulted = core_with_settings(Rc::new(SettingsConfig::default()));
    defaulted.load_startup();
    assert!(!defaulted.left_drag_pans());
}
