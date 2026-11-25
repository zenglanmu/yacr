//! GPU memory budget setting (`view.gpuBudget.percent`).
//!
//! The budget is a percentage of the graphics card's memory, persisted with the
//! rest of the settings through the same allowed-leaf preference path. The host
//! only stores and reports it: the renderer lane that applies it reads the
//! effective config. These contracts therefore cover the closed key table, the
//! refusal rules, and the durable round-trip — never a GPU allocation.

use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

/// The default is the documented 80 % and it is the effective config's value,
/// not a second source of truth.
#[test]
fn the_gpu_budget_defaults_to_80_percent() {
    let core = core();
    let settings = core.settings();
    assert_eq!(settings.gpu_budget_percent, 80);
    assert!(
        settings.gpu_storage_disabled,
        "the shipped default keeps no accelerated GPU-storage copy"
    );
    assert_eq!(
        core.config_snapshot().effective.view.gpu_budget.percent,
        80,
        "the snapshot reports the effective config, not a local default"
    );
    assert!(
        core.config_snapshot().effective.view.gpu_storage.disabled,
        "the storage opt-out is the same effective config value"
    );
}

/// `ALL` is the one key table: the new key is present exactly once, parses from
/// its camelCase wire spelling and is the path the preference is stored under.
#[test]
fn the_gpu_budget_key_is_in_the_closed_table() {
    let key = CadSettingKey::GpuBudgetPercent;
    // Fourteen keys, then the three drafting switches (`interaction.
    // gridSnapEnabled`/`orthoEnabled`/`snapSpacing`), then the left-drag input
    // scheme (`interaction.leftDragPans`) and the 3D orbit sensitivity
    // (`interaction.orbitSensitivityPercent`) joined the table; the count is
    // asserted deliberately so a new key is never added by accident.
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
    assert_eq!(key.key(), "gpuBudgetPercent");
    assert_eq!(
        CadSettingKey::parse("gpuBudgetPercent"),
        Some(CadSettingKey::GpuBudgetPercent)
    );
    assert_eq!(
        CadSettingKey::parse("gpu_budget_percent"),
        None,
        "the wire spelling is camelCase, like the sibling numeric keys"
    );
    assert_eq!(
        cad_app::viewer_config::ViewGpuBudget::default().percent,
        80,
        "the schema default and the snapshot default are the same number"
    );
}

/// Both bounds are accepted as written and reported by the next snapshot.
#[test]
fn the_gpu_budget_accepts_the_whole_10_to_80_range() {
    let (core, config) = settings_core();
    for percent in [10, 50, 80] {
        core.apply_setting(
            CadSettingKey::GpuBudgetPercent,
            CadSettingValue::Number(percent),
        )
        .unwrap();
        assert_eq!(core.settings().gpu_budget_percent, percent);
        assert_eq!(
            core.config_snapshot().effective.view.gpu_budget.percent,
            percent
        );
        let written = persisted(&config).expect("each write persists");
        let document: serde_json::Value =
            serde_json::from_str(&written).expect("the persisted preference is JSON");
        assert_eq!(
            document["view"]["gpuBudget"]["percent"],
            serde_json::Value::from(percent),
            "the budget is persisted under its real config path: {written}"
        );
    }
}

/// Out-of-range values and wrong shapes are refused with the stable code and
/// change nothing — never clamped into the range.
#[test]
fn invalid_gpu_budgets_are_refused_without_writing() {
    let (core, config) = settings_core();
    for percent in [0, 9, 81, 90, 100, 1000] {
        let error = core
            .apply_setting(
                CadSettingKey::GpuBudgetPercent,
                CadSettingValue::Number(percent),
            )
            .expect_err("the range is 10..=80");
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{percent} must be refused with the stable code: {error:?}"
        );
    }
    for value in [
        CadSettingValue::Text("90".to_string()),
        CadSettingValue::Text("90.5".to_string()),
        CadSettingValue::Bool(true),
    ] {
        let error = core
            .apply_setting(CadSettingKey::GpuBudgetPercent, value.clone())
            .expect_err("only an integer percentage is accepted");
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{value:?} must be refused: {error:?}"
        );
    }
    assert!(
        persisted(&config).is_none(),
        "a rejected write must not change the preference file"
    );
    assert_eq!(core.settings().gpu_budget_percent, 80);
}

/// A write survives a restart: the persisted document is re-read by the same
/// startup pass as every other setting.
#[test]
fn the_gpu_budget_roundtrips_through_the_persisted_preference() {
    let (core, config) = settings_core();
    core.apply_setting(CadSettingKey::GpuBudgetPercent, CadSettingValue::Number(25))
        .unwrap();
    let written = persisted(&config).expect("the write persists");

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert!(
        reloaded.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        reloaded.startup_diagnostics.borrow()
    );
    assert_eq!(reloaded.settings().gpu_budget_percent, 25);
    assert_eq!(reloaded.settings(), core.settings());
}

/// Another settings write re-persists the allowed-leaf projection, so the budget
/// leaf is kept rather than dropped.
#[test]
fn unrelated_writes_keep_the_gpu_budget_leaf() {
    let (core, config) = settings_core();
    core.apply_setting(CadSettingKey::GpuBudgetPercent, CadSettingValue::Number(80))
        .unwrap();
    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .unwrap();
    let written = persisted(&config).expect("the grid write persists");
    assert!(
        written.contains("\"gpuBudget\":{\"percent\":80}"),
        "{written}"
    );

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert_eq!(reloaded.settings().gpu_budget_percent, 80);
    assert!(reloaded.settings().grid);
}

/// A persisted budget outside the accepted range is reported by startup as a
/// rejected preference and the default stays in force — never a silent clamp.
#[test]
fn an_out_of_range_persisted_budget_is_rejected_not_clamped() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"view":{"gpuBudget":{"percent":9}}}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();
    assert!(
        !core.startup_diagnostics.borrow().is_empty(),
        "the invalid leaf must be surfaced, not hidden"
    );
    assert_eq!(core.settings().gpu_budget_percent, 80);
}

/// A persisted budget written under the old wider range is migrated down to the
/// current maximum on load, named in a diagnostic and re-persisted — the read
/// path may repair history, a live write may not.
#[test]
fn a_persisted_budget_above_the_current_maximum_is_migrated_on_load() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"view":{"gpuBudget":{"percent":90}}}"#,
    ));
    let core = core_with_settings(config.clone());
    let diagnostics = core.load_startup();
    assert!(
        diagnostics
            .iter()
            .any(|line| line
                .contains("migrated view.gpuBudget.percent 90 -> 80 (the accepted maximum)")),
        "{diagnostics:?}"
    );
    assert_eq!(core.settings().gpu_budget_percent, 80);
    let written = persisted(&config).expect("the migrated preference is re-persisted");
    assert!(
        written.contains("\"gpuBudget\":{\"percent\":80}"),
        "the durable leaf must hold the migrated value: {written}"
    );
}

/// A value above the old `10..=100` range was never valid, so startup must keep
/// refusing it instead of silently repairing it.
#[test]
fn a_persisted_budget_above_the_old_range_is_still_rejected() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"view":{"gpuBudget":{"percent":101}}}"#,
    ));
    let core = core_with_settings(config);
    let diagnostics = core.load_startup();
    assert!(
        diagnostics
            .iter()
            .any(|line| line.contains("preferences preference.json rejected")),
        "{diagnostics:?}"
    );
    assert_eq!(core.settings().gpu_budget_percent, 80);
}

/// The accelerated-GPU-storage opt-out: default **disabled**, closed boolean
/// shape, durable round-trip, and the engine handle records exactly the same
/// choice (the renderer installs it at the next device attach).
#[test]
fn gpu_storage_is_disabled_by_default_and_reaches_the_render_seam() {
    let config = Rc::new(SettingsConfig::default());
    let (core, render) = super::core_with_config_and_render(config.clone());
    assert!(core.settings().gpu_storage_disabled);
    assert!(
        !render.gpu_storage_enabled(),
        "the shipped opt-out must reach the render handle, not only the snapshot"
    );
    assert_eq!(
        CadSettingKey::GpuStorageDisabled.key(),
        "gpuStorageDisabled"
    );
    assert_eq!(
        CadSettingKey::parse("gpuStorageDisabled"),
        Some(CadSettingKey::GpuStorageDisabled)
    );
    assert!(
        HOST_ALLOWED_PATHS_JSON.contains("view.gpuStorage.disabled"),
        "the user-writable leaf must be in the host policy"
    );

    core.apply_setting(
        CadSettingKey::GpuStorageDisabled,
        CadSettingValue::Bool(false),
    )
    .unwrap();
    assert!(!core.settings().gpu_storage_disabled);
    assert!(
        render.gpu_storage_enabled(),
        "the accepted write must reach the renderer policy seam"
    );
    let written = persisted(&config).expect("the write persists");
    assert!(
        written.contains("\"gpuStorage\":{\"disabled\":false}"),
        "the leaf is persisted under its real path: {written}"
    );

    // A wrong shape is refused with the stable code and changes neither the
    // snapshot nor the render handle.
    let config = Rc::new(SettingsConfig::default());
    let (core, render) = super::core_with_config_and_render(config.clone());
    for value in [
        CadSettingValue::Text("false".to_string()),
        CadSettingValue::Number(0),
        CadSettingValue::Real(0.0),
    ] {
        let error = core
            .apply_setting(CadSettingKey::GpuStorageDisabled, value.clone())
            .expect_err("only a boolean is accepted");
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{value:?} must be refused: {error:?}"
        );
    }
    assert!(core.settings().gpu_storage_disabled);
    assert!(!render.gpu_storage_enabled());
    assert!(
        persisted(&config).is_none(),
        "a refused write must not create the preference file"
    );
}

/// A persisted opt-out=false survives a restart and is seeded onto the fresh
/// render handle by the same startup pass as every other setting.
#[test]
fn the_gpu_storage_choice_roundtrips_and_seeds_the_render_handle() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"view":{"gpuStorage":{"disabled":false}}}"#,
    ));
    let (core, render) = super::core_with_config_and_render(config);
    core.load_startup();
    assert!(
        core.startup_diagnostics.borrow().is_empty(),
        "{:?}",
        core.startup_diagnostics.borrow()
    );
    assert!(!core.settings().gpu_storage_disabled);
    assert!(render.gpu_storage_enabled());
}
