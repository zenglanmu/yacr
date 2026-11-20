//! The persisted CAD font directory (`ui.paths.fontsDir`).
//!
//! What is pinned here is the **host wiring**: the closed key, the persisted
//! `ui.paths.fontsDir` leaf, the precedence `--fonts-dir` > persisted > discovery,
//! the refusals that leave the preference untouched, and the deliberate
//! difference between a stale persisted choice (diagnosed, tolerated) and a
//! missing `--fonts-dir` (a hard load error).

use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;
use std::path::PathBuf;

/// A fresh absolute directory for one test.
fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yacr-fonts-dir-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a writable temp directory");
    dir
}

/// The directory the discovery order would pick for this executable, as the
/// snapshot reports it (precedence step 3).
fn discovered() -> Option<String> {
    let exe = std::env::current_exe().unwrap_or_default();
    cad_platform::fonts::local::font_dir_candidates(&exe)
        .into_iter()
        .find(|dir| dir.is_dir())
        .map(|dir| dir.display().to_string())
}

/// `ui.paths.fontsDir` preference text for a path.
fn preference_with(dir: &std::path::Path) -> String {
    format!(r#"{{"ui":{{"paths":{{"fontsDir":"{}"}}}}}}"#, dir.display())
}

#[test]
fn the_fonts_dir_key_is_in_the_closed_table_and_the_allowed_policy() {
    let key = CadSettingKey::FontsDir;
    assert_eq!(key.key(), "fontsDir");
    assert_eq!(CadSettingKey::parse("fontsDir"), Some(key));
    assert_eq!(CadSettingKey::parse("fonts_dir"), None);
    assert!(
        HOST_ALLOWED_PATHS_JSON.contains(r#""ui.paths.fontsDir""#),
        "the font directory must be an individually allowed path leaf"
    );
    // `""` is the documented clear; it never needs a directory on disk.
    assert_eq!(key.validate(&CadSettingValue::Text(String::new())), Ok(()));
}

/// A write persists the absolute leaf, the snapshot reports it as both the
/// editable choice and the effective directory, and a reload keeps it.
#[test]
fn a_fonts_dir_write_round_trips_through_the_preference_store() {
    let dir = temp_dir("roundtrip");
    let path = dir.display().to_string();
    let (core, config) = settings_core();
    assert_eq!(core.settings().fonts_dir_override, None);

    core.apply_setting(CadSettingKey::FontsDir, CadSettingValue::Text(path.clone()))
        .expect("an absolute existing directory");

    let settings = core.settings();
    assert_eq!(settings.fonts_dir_override.as_deref(), Some(path.as_str()));
    assert!(
        !settings.fonts_dir_missing,
        "the saved directory still exists"
    );
    assert_eq!(
        settings.fonts_dir.as_deref(),
        Some(path.as_str()),
        "the persisted directory is what font loading uses"
    );
    let written = persisted(&config).expect("the write persists");
    assert!(
        written.contains(r#""paths":{"fontsDir":"#),
        "the leaf lives under ui.paths: {written}"
    );
    assert!(written.contains(&path), "{written}");

    // The durable document is what the next start reads: same choice, no
    // diagnostic, and the same effective directory.
    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert_eq!(
        reloaded.settings().fonts_dir_override,
        settings.fonts_dir_override
    );
    assert_eq!(reloaded.settings().fonts_dir, settings.fonts_dir);
    assert!(
        reloaded.startup_diagnostics().is_empty(),
        "a usable directory is not a diagnostic"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty value clears the override (the leaf is written as `""`), and the
/// effective directory falls back to the discovery order.
#[test]
fn an_empty_value_clears_the_override_and_falls_back_to_discovery() {
    let dir = temp_dir("clear");
    let (core, config) = settings_core();
    core.apply_setting(
        CadSettingKey::FontsDir,
        CadSettingValue::Text(dir.display().to_string()),
    )
    .expect("an absolute existing directory");

    core.apply_setting(
        CadSettingKey::FontsDir,
        CadSettingValue::Text(String::new()),
    )
    .expect("the documented clear");

    let settings = core.settings();
    assert_eq!(
        settings.fonts_dir_override, None,
        "cleared: nothing to edit"
    );
    assert!(
        !settings.fonts_dir_missing,
        "nothing saved is nothing missing"
    );
    assert_eq!(
        settings.fonts_dir,
        discovered(),
        "the effective directory is the discovery order again"
    );
    let written = persisted(&config).expect("the clear persists");
    assert!(
        written.contains(r#""fontsDir":"""#),
        "a clear is written as an empty string, not a deleted leaf: {written}"
    );
    assert!(!written.contains(&dir.display().to_string()), "{written}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A relative path, a missing directory and a file are all refused with the
/// stable code, before any write.
#[test]
fn a_relative_or_missing_font_dir_is_refused_without_a_write() {
    let dir = temp_dir("refusals");
    let file = dir.join("not-a-directory.json");
    std::fs::write(&file, b"{}").expect("a writable file");
    let missing = dir.join("removed");
    let (core, config) = settings_core();

    for path in [
        "fonts".to_string(),
        "relative/fonts".to_string(),
        missing.display().to_string(),
        file.display().to_string(),
    ] {
        let error = core
            .apply_setting(CadSettingKey::FontsDir, CadSettingValue::Text(path.clone()))
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{path:?} must be refused with the stable code, got {error:?}"
        );
    }
    // A non-string shape is the same refusal.
    let error = core
        .apply_setting(CadSettingKey::FontsDir, CadSettingValue::Number(1))
        .unwrap_err();
    assert!(matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE));

    assert!(persisted(&config).is_none(), "a refusal writes nothing");
    assert_eq!(core.settings().fonts_dir_override, None);
    assert!(!core.settings().fonts_dir_missing);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A directory that was persisted and then removed must not fail startup: the
/// choice is kept for the Options row, the effective directory falls through to
/// discovery, and the fact is recorded as a diagnostic.
#[test]
fn a_persisted_directory_that_disappeared_is_diagnosed_and_tolerated() {
    let dir = temp_dir("stale");
    let path = dir.display().to_string();
    let preference = preference_with(&dir);
    // Persisted, then gone: the next start sees a stale absolute path.
    std::fs::remove_dir_all(&dir).expect("removable");

    let core = core_with_settings(Rc::new(SettingsConfig::with_preference(&preference)));
    let diagnostics = core.load_startup();

    let settings = core.settings();
    assert_eq!(
        settings.fonts_dir_override.as_deref(),
        Some(path.as_str()),
        "the row still shows what the user chose"
    );
    assert!(
        settings.fonts_dir_missing,
        "the saved choice is exactly what the row reports as missing"
    );
    assert_eq!(
        settings.fonts_dir,
        discovered(),
        "the directory actually used is the discovered default"
    );
    assert!(
        diagnostics.iter().any(|line| line.contains(&path)),
        "the startup diagnostic names the missing directory: {diagnostics:?}"
    );
    assert!(
        core.startup_diagnostics()
            .iter()
            .any(|line| line.contains(&path)),
        "the diagnostic is published to the carrier too"
    );
    // The font funnel resolves without error (the stale choice is only reported).
    let exe = std::env::current_exe().unwrap_or_default();
    assert_eq!(core.explicit_font_dir(), None);
    let resolved =
        cad_platform::fonts::local::resolve_font_dir(core.explicit_font_dir().as_deref(), &exe)
            .expect("a stale preference is not a load error");
    assert_eq!(resolved, discovered().map(PathBuf::from));
}

/// `--fonts-dir` still wins over the preference and keeps its hard-error
/// semantics when the directory is missing.
#[test]
fn the_cli_fonts_dir_still_wins_and_keeps_its_hard_error() {
    let persisted_dir = temp_dir("cli-persisted");
    let cli = std::env::temp_dir().join(format!("yacr-fonts-cli-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cli);

    let mut options = options();
    options.fonts_dir = Some(cli.clone());
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(SettingsConfig::with_preference(&preference_with(
            &persisted_dir,
        ))),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let core = HostCore::new(options, platform, RenderSessionHandle::default()).unwrap();
    core.load_startup();

    assert_eq!(
        core.settings().fonts_dir.as_deref(),
        Some(cli.display().to_string().as_str()),
        "the CLI directory is the effective one"
    );
    assert_eq!(
        core.settings().fonts_dir_override.as_deref(),
        Some(persisted_dir.display().to_string().as_str()),
        "the preference is still reported as the editable value"
    );
    assert!(
        !core.settings().fonts_dir_missing,
        "a usable saved directory is not reported missing just because --fonts-dir wins"
    );

    let exe = std::env::current_exe().unwrap_or_default();
    assert_eq!(core.explicit_font_dir(), Some(cli.clone()));
    let resolved =
        cad_platform::fonts::local::resolve_font_dir(core.explicit_font_dir().as_deref(), &exe);
    assert!(
        matches!(resolved, Err(CadError::InvalidInput(_))),
        "a missing --fonts-dir is still a hard error: {resolved:?}"
    );

    let _ = std::fs::remove_dir_all(&persisted_dir);
}
