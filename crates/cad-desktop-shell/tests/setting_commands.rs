//! The `cad_set_setting` wire: JSON number shapes and the `cad_settings` mirror.
//!
//! The key table's rules (which key accepts which shape, and every range) are
//! `cad_ui_core`/`cad-host` authority and are pinned there. What is pinned here
//! is the **translation** the desktop IPC performs: a fractional JSON number must
//! reach the owner thread as `CadSettingValue::Real` (never truncated into the
//! integer shape), while the integer path and the refusals stay exactly as they
//! were. Every case drives the same parse-and-admit body the `#[tauri::command]`
//! runs, so the test cannot drift from the IPC path.

use cad_desktop_shell::commands::{cad_set_setting_impl, cad_settings_impl, parse_setting_request};
use cad_desktop_shell::state::{lock, new_shared};
use cad_ui_core::launch::LaunchOptions;
use cad_ui_core::session::{
    CadSettingKey, CadSettingValue, CadSettings, SETTINGS_INVALID_VALUE, SETTINGS_UNKNOWN_KEY,
};
use serde_json::{json, Value};

/// A settings snapshot with every field at a fixed value, so a test can mirror
/// one admitted write into it exactly like the owner tick does.
fn snapshot_with_snap_spacing(snap_spacing: f64) -> CadSettings {
    CadSettings {
        grid: true,
        axes: false,
        selection_highlight: true,
        snap_hints: false,
        plot_format: "svg".to_string(),
        snap_enabled: true,
        hover_preview: true,
        grid_snap_enabled: false,
        ortho_enabled: false,
        snap_spacing,
        background_model: "#000000".to_string(),
        background_paper: "#ffffff".to_string(),
        crosshair_percent: 25,
        pickbox_size: 3,
        aperture_size: 12,
        gpu_budget_percent: 90,
        wgpu_backend: "auto".to_string(),
        fonts_dir: None,
    }
}

fn queued_settings(
    shared: &std::sync::Arc<std::sync::Mutex<cad_desktop_shell::state::Shared>>,
) -> Vec<(CadSettingKey, CadSettingValue)> {
    lock(shared).cad_setting_writes.iter().cloned().collect()
}

#[test]
fn a_fractional_snap_spacing_is_admitted_and_reads_back_untruncated() {
    let shared = new_shared(LaunchOptions::default());
    cad_set_setting_impl(&shared, "snapSpacing", &json!(2.5)).expect("2.5 is a legal SNAPUNIT");

    // The owner thread receives the real, not a truncated integer.
    assert_eq!(
        queued_settings(&shared),
        vec![(CadSettingKey::SnapSpacing, CadSettingValue::Real(2.5))],
        "the fractional spacing must cross the IPC as a real"
    );

    // The tick mirrors the applied snapshot; the read must hand the same real to
    // the Options dialog (2.5 must not surface as 2 on the wire).
    {
        let mut state = lock(&shared);
        state.cad_settings = Some(snapshot_with_snap_spacing(2.5));
    }
    let settings = cad_settings_impl(&shared).expect("the mirrored snapshot");
    assert_eq!(settings.snap_spacing, 2.5);
    let wire = serde_json::to_value(&settings).expect("settings serialize");
    assert_eq!(
        wire.get("snapSpacing"),
        Some(&json!(2.5)),
        "cad_settings must publish the fractional spacing: {wire}"
    );
}

#[test]
fn an_integer_snap_spacing_keeps_the_integer_shape() {
    let shared = new_shared(LaunchOptions::default());
    cad_set_setting_impl(&shared, "snapSpacing", &json!(25)).expect("25 is a legal SNAPUNIT");
    assert_eq!(
        queued_settings(&shared),
        vec![(CadSettingKey::SnapSpacing, CadSettingValue::Number(25))],
        "an integer JSON number keeps the integer shape"
    );
}

#[test]
fn zero_negative_and_text_snap_spacings_are_refused_explicitly() {
    let shared = new_shared(LaunchOptions::default());
    for value in [json!(0), json!(-2), json!("2.5")] {
        let error = cad_set_setting_impl(&shared, "snapSpacing", &value)
            .expect_err("an illegal spacing is refused");
        assert_eq!(
            error.as_str(),
            SETTINGS_INVALID_VALUE,
            "snapSpacing {value} must be refused by the key table"
        );
    }
    assert!(
        queued_settings(&shared).is_empty(),
        "a refused request never reaches the queue"
    );
}

#[test]
fn a_fractional_number_on_a_boolean_key_is_refused() {
    let shared = new_shared(LaunchOptions::default());
    let error = cad_set_setting_impl(&shared, "grid", &json!(2.5))
        .expect_err("a switch only accepts a boolean");
    assert_eq!(error.as_str(), SETTINGS_INVALID_VALUE);
    // The fractional shape is not silently widened into an integer-shaped `1`.
    assert!(queued_settings(&shared).is_empty());

    // Sanity: the same key still accepts its own shape, so the refusal above is
    // about the float, not the key.
    cad_set_setting_impl(&shared, "grid", &json!(false)).expect("a boolean is the switch shape");
    assert_eq!(
        queued_settings(&shared),
        vec![(CadSettingKey::Grid, CadSettingValue::Bool(false))]
    );
}

#[test]
fn a_fractional_number_on_an_integer_key_is_refused() {
    let shared = new_shared(LaunchOptions::default());
    for key in [
        "crosshairSize",
        "gpuBudgetPercent",
        "pickboxSize",
        "apertureSize",
    ] {
        let error = cad_set_setting_impl(&shared, key, &json!(90.5))
            .expect_err("an integer-only key refuses a fractional number");
        assert_eq!(
            error.as_str(),
            SETTINGS_INVALID_VALUE,
            "{key} must not accept a fractional number"
        );
    }
    assert!(queued_settings(&shared).is_empty());
}

#[test]
fn an_unknown_key_is_still_an_unknown_key_for_a_fractional_number() {
    let shared = new_shared(LaunchOptions::default());
    let error = cad_set_setting_impl(&shared, "notAKey", &json!(2.5))
        .expect_err("an unknown key is refused before the value shape");
    assert_eq!(error.as_str(), SETTINGS_UNKNOWN_KEY);
    assert!(queued_settings(&shared).is_empty());
}

#[test]
fn parse_setting_request_maps_each_json_shape_to_its_typed_value() {
    // The parse contract itself, independent of admission state.
    assert_eq!(
        parse_setting_request("snapSpacing", &json!(2.5)).expect("a real"),
        (CadSettingKey::SnapSpacing, CadSettingValue::Real(2.5))
    );
    assert_eq!(
        parse_setting_request("snapSpacing", &json!(25)).expect("an integer"),
        (CadSettingKey::SnapSpacing, CadSettingValue::Number(25))
    );
    assert_eq!(
        parse_setting_request("backgroundModel", &json!("#000000")).expect("text"),
        (
            CadSettingKey::BackgroundModel,
            CadSettingValue::Text("#000000".to_string())
        )
    );
    // A non-numeric JSON shape (null, array, object) is a wrong value shape.
    for value in [Value::Null, json!([2.5]), json!({"spacing": 2.5})] {
        assert_eq!(
            parse_setting_request("snapSpacing", &value).map(|_| ()),
            Err(SETTINGS_INVALID_VALUE.to_string()),
            "{value} is not a setting value"
        );
    }
    assert_eq!(
        parse_setting_request("snapSpacing", &json!(4_294_967_296_u64)).unwrap_err(),
        SETTINGS_INVALID_VALUE,
        "an integer beyond u32 stays refused (it is not a real)"
    );
}
