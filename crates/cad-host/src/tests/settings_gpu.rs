//! GPU memory budget setting (`view.gpuBudget.percent`).
//!
//! The budget is a percentage of the graphics card's memory, persisted with the
//! rest of the settings through the same allowed-leaf preference path. The host
//! only stores and reports it: the renderer lane that applies it reads the
//! effective config. These contracts therefore cover the closed key table, the
//! refusal rules, and the durable round-trip — never a GPU allocation.

use super::settings::{core_with_settings, persisted, settings_core, SettingsConfig};
use super::*;

/// The default is the documented 90 % and it is the effective config's value,
/// not a second source of truth.
#[test]
fn the_gpu_budget_defaults_to_90_percent() {
    let core = core();
    let settings = core.settings();
    assert_eq!(settings.gpu_budget_percent, 90);
    assert_eq!(
        core.config_snapshot().effective.view.gpu_budget.percent,
        90,
        "the snapshot reports the effective config, not a local default"
    );
}

/// `ALL` is the one key table: the new key is present exactly once, parses from
/// its camelCase wire spelling and is the path the preference is stored under.
#[test]
fn the_gpu_budget_key_is_in_the_closed_table() {
    let key = CadSettingKey::GpuBudgetPercent;
    assert_eq!(
        CadSettingKey::ALL.len(),
        14,
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
        90,
        "the schema default and the snapshot default are the same number"
    );
}

/// Both bounds are accepted as written and reported by the next snapshot.
#[test]
fn the_gpu_budget_accepts_the_whole_10_to_100_range() {
    let (core, config) = settings_core();
    for percent in [10, 90, 100] {
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
    for percent in [0, 9, 101, 1000] {
        let error = core
            .apply_setting(
                CadSettingKey::GpuBudgetPercent,
                CadSettingValue::Number(percent),
            )
            .expect_err("the range is 10..=100");
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
    assert_eq!(core.settings().gpu_budget_percent, 90);
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
    core.apply_setting(
        CadSettingKey::GpuBudgetPercent,
        CadSettingValue::Number(100),
    )
    .unwrap();
    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .unwrap();
    let written = persisted(&config).expect("the grid write persists");
    assert!(
        written.contains("\"gpuBudget\":{\"percent\":100}"),
        "{written}"
    );

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&written)));
    reloaded.load_startup();
    assert_eq!(reloaded.settings().gpu_budget_percent, 100);
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
    assert_eq!(core.settings().gpu_budget_percent, 90);
}
