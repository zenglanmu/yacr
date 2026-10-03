//! Native on-disk `ViewerConfig` / user-preference persistence contracts.
//!
//! The offscreen Slint platform installs once per process, so every
//! window-backed assertion lives in this single test and its own test binary.
#![cfg(target_os = "linux")]
use app_linux::{LinuxApp, LinuxOptions};
use std::path::PathBuf;

/// Unique scratch directory under the OS temp root, removed before use.
fn temp_dir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("yacr-linux-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn native_config_and_preference_round_trip_through_disk() {
    cad_ui_slint::offscreen::install().unwrap();

    // 1. A present config file replaces the host config.
    let config_directory = temp_dir("config");
    let config = config_directory.join("config.json");
    std::fs::write(&config, r#"{"view":{"overlays":{"grid":false}}}"#).unwrap();
    let configured = LinuxApp::new(LinuxOptions {
        config: Some(config),
        preferences: Some(config_directory.join("preferences.json")),
        ..LinuxOptions::default()
    })
    .unwrap();
    assert!(
        configured.startup_diagnostics().is_empty(),
        "{:?}",
        configured.startup_diagnostics()
    );
    assert!(
        !configured
            .adapter
            .handle()
            .effective_config()
            .view
            .overlays
            .grid,
        "the host config file must be applied"
    );
    let _ = std::fs::remove_dir_all(&config_directory);

    // 2. An allowed preference is persisted and reloaded on the next start.
    let directory = temp_dir("prefs");
    let config = directory.join("config.json");
    let preferences = directory.join("preferences.json");
    let app = LinuxApp::new(LinuxOptions {
        config: Some(config.clone()),
        preferences: Some(preferences.clone()),
        ..LinuxOptions::default()
    })
    .unwrap();
    assert!(
        app.startup_diagnostics().is_empty(),
        "{:?}",
        app.startup_diagnostics()
    );
    // The built-in host policy (mirroring the web host) allows this leaf;
    // re-install it explicitly to exercise the host config patch path.
    app.update_config_json(
        r#"{"ui":{"userCustomization":{"allowedPaths":["ui.components.layerPanel.initiallyOpen"]}}}"#,
    )
    .unwrap();
    app.apply_user_preference_json(
        r#"{"ui":{"components":{"layerPanel":{"initiallyOpen":false}}}}"#,
    )
    .unwrap();
    assert!(preferences.exists(), "allowed projection must be persisted");
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&preferences).unwrap()).unwrap();
    assert_eq!(
        stored["ui"]["components"]["layerPanel"]["initiallyOpen"],
        serde_json::json!(false)
    );

    let reloaded = LinuxApp::new(LinuxOptions {
        config: Some(config),
        preferences: Some(preferences),
        ..LinuxOptions::default()
    })
    .unwrap();
    assert!(
        reloaded.startup_diagnostics().is_empty(),
        "{:?}",
        reloaded.startup_diagnostics()
    );
    assert!(
        !reloaded
            .adapter
            .handle()
            .effective_config()
            .ui
            .components
            .layer_panel
            .initially_open,
        "the reloaded host must reflect the stored preference"
    );
    let _ = std::fs::remove_dir_all(&directory);

    // 3. A malformed preference is reported and never rewritten; defaults hold.
    let directory = temp_dir("malformed");
    let preferences = directory.join("preferences.json");
    std::fs::write(&preferences, "{not valid json").unwrap();
    let app = LinuxApp::new(LinuxOptions {
        config: Some(directory.join("config.json")),
        preferences: Some(preferences.clone()),
        ..LinuxOptions::default()
    })
    .unwrap();
    assert!(
        app.startup_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.contains("preferences")),
        "{:?}",
        app.startup_diagnostics()
    );
    assert!(
        app.adapter
            .handle()
            .effective_config()
            .ui
            .components
            .layer_panel
            .initially_open,
        "a rejected preference must keep the built-in default"
    );
    assert_eq!(
        std::fs::read_to_string(&preferences).unwrap(),
        "{not valid json",
        "a rejected preference must not be rewritten"
    );
    let _ = std::fs::remove_dir_all(&directory);
}
