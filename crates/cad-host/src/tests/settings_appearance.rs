//! The five host-owned appearance/navigation preferences: `theme`,
//! `navbarVisible`, `navvcubeDisplay`, `viewCubeIn2d`, `viewCubeIn3d`.
//!
//! They are root-level `preference.json` leaves (like `wgpu_backend`), not
//! `ViewerConfig` paths, so these contracts cover the closed key table, the
//! validation, the persisted root leaves, the restart/projection round-trip and
//! the theme's all-or-nothing background writes.

use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

/// A store that reads a durable document but refuses every write, so the
/// all-or-nothing theme path can be observed without a filesystem.
struct ReadOnlyPreferenceStore(&'static str);

impl ConfigStore for ReadOnlyPreferenceStore {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }
    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Text {
            path: "preference.json".to_string(),
            text: self.0.to_string(),
        }
    }
    fn persist_preference(&self, _text: &str) -> CadResult<()> {
        Err(CadError::InvalidInput(
            "preference store is read-only".into(),
        ))
    }
}

fn theme_background(core: &HostCore) -> (String, String) {
    let settings = core.settings();
    (settings.background_model, settings.background_paper)
}

#[test]
fn the_five_appearance_keys_are_in_the_closed_table() {
    assert_eq!(
        CadSettingKey::ALL.len(),
        23,
        "one writable key per contract"
    );
    for (key, wire) in [
        (CadSettingKey::Theme, "theme"),
        (CadSettingKey::NavbarVisible, "navbarVisible"),
        (CadSettingKey::NavvcubeDisplay, "navvcubeDisplay"),
        (CadSettingKey::ViewCubeIn2d, "viewCubeIn2d"),
        (CadSettingKey::ViewCubeIn3d, "viewCubeIn3d"),
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
    // Root leaves, never dotted `ViewerConfig` paths.
    for dotted in [
        "ui.theme",
        "view.theme",
        "navigation.navbarVisible",
        "view.navvcubeDisplay",
    ] {
        assert_eq!(CadSettingKey::parse(dotted), None, "{dotted}");
    }
}

#[test]
fn the_appearance_defaults_are_the_documented_host_values() {
    let core = core();
    let settings = core.settings();
    assert_eq!(settings.theme, "dark");
    assert!(settings.navbar_visible);
    assert_eq!(settings.navvcube_display, 3, "both view classes visible");
    assert!(settings.view_cube_in_2d);
    assert!(settings.view_cube_in_3d);
    // Reading never writes a preference.
    assert!(!HOST_ALLOWED_PATHS_JSON.contains("navbarVisible"));
}

#[test]
fn navvcube_display_accepts_only_zero_to_three() {
    let (core, config) = settings_core();
    for display in 0..=3 {
        core.apply_setting(
            CadSettingKey::NavvcubeDisplay,
            CadSettingValue::Number(display),
        )
        .unwrap();
        assert_eq!(core.settings().navvcube_display, display);
    }
    let before = persisted(&config);
    for display in [4, 5, 100] {
        let error = core
            .apply_setting(
                CadSettingKey::NavvcubeDisplay,
                CadSettingValue::Number(display),
            )
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{display} must be refused: {error:?}"
        );
    }
    for value in [
        CadSettingValue::Bool(true),
        CadSettingValue::Text("2".to_string()),
        CadSettingValue::Real(1.5),
    ] {
        let error = core
            .apply_setting(CadSettingKey::NavvcubeDisplay, value.clone())
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{value:?} must be refused: {error:?}"
        );
    }
    assert_eq!(persisted(&config), before, "a refusal writes nothing");
}

#[test]
fn theme_accepts_only_dark_and_light() {
    let (core, config) = settings_core();
    for token in ["dark", "light"] {
        core.apply_setting(CadSettingKey::Theme, CadSettingValue::Text(token.into()))
            .unwrap();
        assert_eq!(core.settings().theme, token);
    }
    let before = persisted(&config);
    for token in ["", "Dark", "LIGHT", "light theme", "black", "auto"] {
        let error = core
            .apply_setting(CadSettingKey::Theme, CadSettingValue::Text(token.into()))
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{token:?} must be refused: {error:?}"
        );
    }
    for value in [CadSettingValue::Bool(true), CadSettingValue::Number(1)] {
        let error = core
            .apply_setting(CadSettingKey::Theme, value.clone())
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{value:?} must be refused: {error:?}"
        );
    }
    assert_eq!(persisted(&config), before, "a refusal writes nothing");
    assert_eq!(core.settings().theme, "light");
}

#[test]
fn a_theme_write_applies_both_canvas_backgrounds() {
    let core = core();
    core.sync_render().unwrap();
    assert_eq!(
        theme_background(&core),
        ("#000000".to_string(), "#000000".to_string())
    );

    core.apply_setting(CadSettingKey::Theme, CadSettingValue::Text("light".into()))
        .unwrap();
    assert_eq!(core.settings().theme, "light");
    assert_eq!(
        theme_background(&core),
        ("#ffffff".to_string(), "#ffffff".to_string())
    );
    core.sync_render().unwrap();
    assert_eq!(
        core.render.view_snapshot().clear_color,
        [1.0, 1.0, 1.0, 1.0],
        "the active model-space frame clears to the light background"
    );

    core.apply_setting(CadSettingKey::Theme, CadSettingValue::Text("dark".into()))
        .unwrap();
    assert_eq!(
        theme_background(&core),
        ("#000000".to_string(), "#000000".to_string())
    );
    core.sync_render().unwrap();
    assert_eq!(
        core.render.view_snapshot().clear_color,
        [0.0, 0.0, 0.0, 1.0]
    );
}

#[test]
fn a_theme_write_that_cannot_persist_changes_nothing() {
    let core = HostCore::new(
        options(),
        HostPlatform {
            files: Rc::new(FakeFiles::default()),
            config_store: Rc::new(ReadOnlyPreferenceStore(r#"{"theme":"dark"}"#)),
            surface: Rc::new(FakeSurface),
            resources: Rc::new(UnsupportedResources),
            capabilities: capabilities(false),
        },
        RenderSessionHandle::default(),
    )
    .unwrap();
    core.load_startup();
    assert_eq!(core.settings().theme, "dark");

    let error = core
        .apply_setting(CadSettingKey::Theme, CadSettingValue::Text("light".into()))
        .unwrap_err();
    assert!(
        matches!(&error, CadError::Unsupported(reason) if reason.starts_with(SETTINGS_NOT_PERSISTED)),
        "the refusal names the persistence failure: {error:?}"
    );
    // The background write itself was refused by the same store, so neither
    // background nor the theme leaf moved.
    assert_eq!(core.settings().theme, "dark");
    assert_eq!(
        theme_background(&core),
        ("#000000".to_string(), "#000000".to_string())
    );
}

#[test]
fn the_appearance_leaves_persist_at_the_preference_root_and_survive_other_writes() {
    let (core, config) = settings_core();
    core.apply_setting(CadSettingKey::NavbarVisible, CadSettingValue::Bool(false))
        .unwrap();
    core.apply_setting(CadSettingKey::NavvcubeDisplay, CadSettingValue::Number(2))
        .unwrap();
    core.apply_setting(CadSettingKey::ViewCubeIn2d, CadSettingValue::Bool(false))
        .unwrap();
    core.apply_setting(CadSettingKey::ViewCubeIn3d, CadSettingValue::Bool(false))
        .unwrap();
    core.apply_setting(CadSettingKey::Theme, CadSettingValue::Text("light".into()))
        .unwrap();

    let written = persisted(&config).expect("each write persists");
    for leaf in [
        r#""navbarVisible":false"#,
        r#""navvcubeDisplay":2"#,
        r#""viewCubeIn2d":false"#,
        r#""viewCubeIn3d":false"#,
        r#""theme":"light""#,
    ] {
        assert!(written.contains(leaf), "{leaf} missing: {written}");
    }
    // Root leaves, not nested viewer paths.
    let document: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    assert_eq!(document["navbarVisible"], serde_json::json!(false));
    assert!(document
        .get("ui")
        .is_none_or(|ui| ui.get("theme").is_none()));

    // An unrelated preference write keeps every root leaf.
    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .unwrap();
    let written = persisted(&config).expect("the grid write persists");
    for leaf in [
        r#""navbarVisible":false"#,
        r#""navvcubeDisplay":2"#,
        r#""theme":"light""#,
    ] {
        assert!(written.contains(leaf), "{leaf} dropped: {written}");
    }

    // A startup reload reports the same values.
    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    let settings = reloaded.settings();
    assert_eq!(settings.theme, "light");
    assert!(!settings.navbar_visible);
    assert_eq!(settings.navvcube_display, 2);
    assert!(!settings.view_cube_in_2d);
    assert!(!settings.view_cube_in_3d);
    assert_eq!(
        theme_background(&reloaded),
        ("#ffffff".to_string(), "#ffffff".to_string())
    );
}

#[test]
fn malformed_appearance_leaves_fall_back_to_their_defaults() {
    for preference in [
        r#"{"theme":"chartreuse","navbarVisible":"no","navvcubeDisplay":9,"viewCubeIn2d":1}"#,
        r#"{"theme":4,"navbarVisible":null,"navvcubeDisplay":-1}"#,
        "not json at all",
    ] {
        let core = core_with_settings(Rc::new(SettingsConfig::with_preference(preference)));
        // A malformed document is a store diagnostic, not a fatal load.
        core.load_startup();
        let settings = core.settings();
        assert_eq!(settings.theme, "dark", "{preference}");
        assert!(settings.navbar_visible, "{preference}");
        assert_eq!(settings.navvcube_display, 3, "{preference}");
        assert!(settings.view_cube_in_2d, "{preference}");
        assert!(settings.view_cube_in_3d, "{preference}");
    }
}

#[test]
fn a_clear_removes_the_appearance_leaves() {
    let (core, config) = settings_core();
    core.apply_setting(CadSettingKey::NavbarVisible, CadSettingValue::Bool(false))
        .unwrap();
    core.apply_setting(CadSettingKey::Theme, CadSettingValue::Text("light".into()))
        .unwrap();
    core.clear_user_preference().unwrap();
    let written = persisted(&config).expect("the clear persists");
    for leaf in [
        "navbarVisible",
        "navvcubeDisplay",
        "viewCubeIn2d",
        "viewCubeIn3d",
        "theme",
    ] {
        assert!(
            !written.contains(leaf),
            "{leaf} survived a real clear: {written}"
        );
    }
    assert_eq!(core.settings().theme, "dark");
    assert!(core.settings().navbar_visible);
}
