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
        left_drag_pans: false,
        orbit_sensitivity_percent: 100.0,
        background_model: "#000000".to_string(),
        background_paper: "#ffffff".to_string(),
        crosshair_percent: 25,
        pickbox_size: 3,
        aperture_size: 12,
        gpu_budget_percent: 80,
        gpu_storage_disabled: true,
        msaa_mode: "4x".to_string(),
        wgpu_backend: "auto".to_string(),
        theme: "dark".to_string(),
        navbar_visible: true,
        navvcube_display: 3,
        view_cube_in_2d: true,
        view_cube_in_3d: true,
        format_notice_enabled: true,
        drawing_unit: "drawing".to_string(),
        display_unit: "drawing".to_string(),
        unit_precision: 3,
        recovery_interval: 60,
        fonts_dir: None,
        // The desktop carrier has nothing saved here in this fixture, so both
        // path fields stay **absent** on the wire (never `null`), matching the
        // contract the React lane reads for "not configured".
        fonts_dir_override: None,
        // The row-state flag is always a boolean, never omitted.
        fonts_dir_missing: false,
    }
}

/// The font-directory wire contract the Options row depends on.
///
/// `fontsDir`/`fontsDirOverride` are **omitted** when there is no value (the row
/// reads an absent key as "not configured"), never serialized as `null`;
/// `fontsDirMissing` is always a boolean. A stale saved choice keeps its own
/// value while `fontsDir` reports the directory actually in use.
#[test]
fn the_font_directory_wire_fields_omit_paths_but_always_publish_the_row_state() {
    let wire = serde_json::to_value(snapshot_with_snap_spacing(10.0)).expect("serializes");
    assert!(
        wire.get("fontsDir").is_none(),
        "an unset directory is omitted, not null: {wire}"
    );
    assert!(
        wire.get("fontsDirOverride").is_none(),
        "an unset override is omitted, not null: {wire}"
    );
    assert_eq!(
        wire.get("fontsDirMissing"),
        Some(&json!(false)),
        "the row state is always present: {wire}"
    );

    // A saved, usable directory is published on both path fields.
    let mut saved = snapshot_with_snap_spacing(10.0);
    saved.fonts_dir = Some("/opt/yacr/fonts".to_string());
    saved.fonts_dir_override = Some("/opt/yacr/fonts".to_string());
    let wire = serde_json::to_value(&saved).expect("serializes");
    assert_eq!(wire.get("fontsDir"), Some(&json!("/opt/yacr/fonts")));
    assert_eq!(
        wire.get("fontsDirOverride"),
        Some(&json!("/opt/yacr/fonts"))
    );
    assert_eq!(wire.get("fontsDirMissing"), Some(&json!(false)));

    // A stale saved choice: the override keeps what the user picked, `fontsDir`
    // is the fallback actually in use, and the row state is true.
    let mut stale = snapshot_with_snap_spacing(10.0);
    stale.fonts_dir = Some("/opt/yacr/fonts".to_string());
    stale.fonts_dir_override = Some("/mnt/removed/fonts".to_string());
    stale.fonts_dir_missing = true;
    let wire = serde_json::to_value(&stale).expect("serializes");
    assert_eq!(wire.get("fontsDir"), Some(&json!("/opt/yacr/fonts")));
    assert_eq!(
        wire.get("fontsDirOverride"),
        Some(&json!("/mnt/removed/fonts"))
    );
    assert_eq!(wire.get("fontsDirMissing"), Some(&json!(true)));
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
fn a_fractional_orbit_sensitivity_is_admitted_and_mirrored() {
    let shared = new_shared(LaunchOptions::default());
    cad_set_setting_impl(&shared, "orbitSensitivityPercent", &json!(150.0))
        .expect("150 is a legal orbit sensitivity");
    assert_eq!(
        queued_settings(&shared),
        vec![(
            CadSettingKey::OrbitSensitivityPercent,
            CadSettingValue::Real(150.0)
        )],
        "the fractional percentage must cross the IPC as a real"
    );

    // An integer percentage is the same key in the integer shape.
    let integer = new_shared(LaunchOptions::default());
    cad_set_setting_impl(&integer, "orbitSensitivityPercent", &json!(100))
        .expect("100 is a legal orbit sensitivity");
    assert_eq!(
        queued_settings(&integer),
        vec![(
            CadSettingKey::OrbitSensitivityPercent,
            CadSettingValue::Number(100)
        )]
    );

    // The range is the host table's: 24.9 and 200.1 are refused before the queue.
    for out_of_range in [json!(24.9), json!(200.1), json!(0), json!(201)] {
        let refused = new_shared(LaunchOptions::default());
        let error = cad_set_setting_impl(&refused, "orbitSensitivityPercent", &out_of_range)
            .expect_err("an out-of-range sensitivity is refused");
        assert_eq!(
            error.as_str(),
            SETTINGS_INVALID_VALUE,
            "orbitSensitivityPercent {out_of_range} must be refused"
        );
        assert!(
            queued_settings(&refused).is_empty(),
            "a refused request never reaches the queue"
        );
    }

    // The owner tick mirrors the applied snapshot; the read must publish the
    // number under the same camelCase leaf, never a string or a truncated int.
    {
        let mut state = lock(&shared);
        let mut snapshot = snapshot_with_snap_spacing(10.0);
        snapshot.orbit_sensitivity_percent = 150.0;
        state.cad_settings = Some(snapshot);
    }
    let settings = cad_settings_impl(&shared).expect("the mirrored snapshot");
    assert_eq!(settings.orbit_sensitivity_percent, 150.0);
    let wire = serde_json::to_value(&settings).expect("settings serialize");
    assert_eq!(
        wire.get("orbitSensitivityPercent"),
        Some(&json!(150.0)),
        "cad_settings must publish the orbit sensitivity: {wire}"
    );
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
