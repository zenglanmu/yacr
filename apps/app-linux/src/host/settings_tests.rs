//! Tests for the typed settings IPC surface (wire shape + admission).

use std::collections::VecDeque;

use cad_ui_core::session::{
    CadSettingKey, CadSettingValue, CadSettings, SETTINGS_INVALID_PLOT_FORMAT,
    SETTINGS_INVALID_VALUE, SETTINGS_UNKNOWN_KEY,
};

use crate::host::commands::{admit_setting, parse_setting_request, CAD_SETTING_CAP};

#[test]
fn cad_settings_wire_shape_is_a_flat_camel_case_object() {
    let settings = CadSettings {
        grid: true,
        axes: false,
        selection_highlight: true,
        snap_hints: false,
        plot_format: "pdf".to_string(),
        snap_enabled: false,
        hover_preview: true,
        background_model: "#000000".to_string(),
        background_paper: "#ffffff".to_string(),
        crosshair_percent: 25,
        pickbox_size: 3,
        aperture_size: 12,
        fonts_dir: Some("/tmp/yacr-fonts".to_string()),
    };
    let wire = serde_json::to_value(&settings).expect("settings serialize");
    assert_eq!(
        wire,
        serde_json::json!({
            "grid": true,
            "axes": false,
            "selectionHighlight": true,
            "snapHints": false,
            "plotFormat": "pdf",
            "snapEnabled": false,
            "hoverPreview": true,
            "backgroundModel": "#000000",
            "backgroundPaper": "#ffffff",
            "crosshairPercent": 25,
            "pickboxSize": 3,
            "apertureSize": 12,
            "fontsDir": "/tmp/yacr-fonts",
        })
    );
    // Exactly the thirteen documented fields; no unrelated Options fields.
    assert_eq!(wire.as_object().expect("object").len(), 13);
}

#[test]
fn parse_setting_request_refuses_unknown_keys_formats_and_types() {
    assert_eq!(
        parse_setting_request("bogus", &serde_json::json!(true)).unwrap_err(),
        SETTINGS_UNKNOWN_KEY
    );
    assert_eq!(
        parse_setting_request("settings.bogus", &serde_json::json!(true)).unwrap_err(),
        SETTINGS_UNKNOWN_KEY
    );
    assert_eq!(
        parse_setting_request("grid", &serde_json::json!("on")).unwrap_err(),
        SETTINGS_INVALID_VALUE
    );
    assert_eq!(
        parse_setting_request("grid", &serde_json::json!(1)).unwrap_err(),
        SETTINGS_INVALID_VALUE
    );
    assert_eq!(
        parse_setting_request("plotFormat", &serde_json::json!("png")).unwrap_err(),
        SETTINGS_INVALID_PLOT_FORMAT
    );
    assert_eq!(
        parse_setting_request("plotFormat", &serde_json::json!(true)).unwrap_err(),
        SETTINGS_INVALID_VALUE
    );

    assert_eq!(
        parse_setting_request("selectionHighlight", &serde_json::json!(false)).unwrap(),
        (
            CadSettingKey::SelectionHighlight,
            CadSettingValue::Bool(false)
        )
    );
    assert_eq!(
        parse_setting_request("plotFormat", &serde_json::json!("pdf")).unwrap(),
        (
            CadSettingKey::PlotFormat,
            CadSettingValue::Text("pdf".to_string())
        )
    );
    // The display options: a hex colour is a string, the crosshair is an integer
    // percentage. A number where a string is expected (and vice versa) is refused.
    assert_eq!(
        parse_setting_request("backgroundModel", &serde_json::json!("#1A2B3C")).unwrap(),
        (
            CadSettingKey::BackgroundModel,
            CadSettingValue::Text("#1A2B3C".to_string())
        )
    );
    assert_eq!(
        parse_setting_request("crosshairSize", &serde_json::json!(40)).unwrap(),
        (CadSettingKey::CrosshairSize, CadSettingValue::Number(40))
    );
    // The pickbox/aperture sizes are integer pixel counts in AutoCAD's own
    // option ranges.
    assert_eq!(
        parse_setting_request("pickboxSize", &serde_json::json!(6)).unwrap(),
        (CadSettingKey::PickboxSize, CadSettingValue::Number(6))
    );
    assert_eq!(
        parse_setting_request("apertureSize", &serde_json::json!(24)).unwrap(),
        (CadSettingKey::ApertureSize, CadSettingValue::Number(24))
    );
    for (key, value) in [
        ("backgroundPaper", serde_json::json!("#abc")),
        ("backgroundPaper", serde_json::json!("red")),
        ("backgroundPaper", serde_json::json!(1)),
        ("crosshairSize", serde_json::json!("40")),
        ("crosshairSize", serde_json::json!(0)),
        ("crosshairSize", serde_json::json!(101)),
        ("crosshairSize", serde_json::json!(-1)),
        ("crosshairSize", serde_json::json!(40.5)),
        ("crosshairSize", serde_json::json!(true)),
        ("pickboxSize", serde_json::json!(0)),
        ("pickboxSize", serde_json::json!(21)),
        ("pickboxSize", serde_json::json!(3.5)),
        ("pickboxSize", serde_json::json!("3")),
        ("pickboxSize", serde_json::json!(true)),
        ("apertureSize", serde_json::json!(0)),
        ("apertureSize", serde_json::json!(51)),
        ("apertureSize", serde_json::json!(12.5)),
        ("apertureSize", serde_json::json!("12")),
        ("apertureSize", serde_json::json!(true)),
    ] {
        assert_eq!(
            parse_setting_request(key, &value).unwrap_err(),
            SETTINGS_INVALID_VALUE,
            "{key} = {value} must be refused"
        );
    }
}

#[test]
fn settings_admission_is_typed_bounded_and_has_no_modal_gate() {
    // The admission signature intentionally has no `modal_open` parameter: an
    // Options write from the popup window is not CAD input and must stay
    // allowed while the modal gate refuses commands.
    let mut queue = VecDeque::new();
    admit_setting(
        &mut queue,
        true,
        CadSettingKey::Grid,
        CadSettingValue::Bool(true),
    )
    .unwrap();
    assert_eq!(queue.len(), 1);

    // Validation happens before the queue.
    assert_eq!(
        admit_setting(
            &mut queue,
            true,
            CadSettingKey::PlotFormat,
            CadSettingValue::Text("png".to_string()),
        )
        .unwrap_err(),
        SETTINGS_INVALID_PLOT_FORMAT
    );
    assert_eq!(queue.len(), 1);

    // Probe mode is explicit.
    assert!(admit_setting(
        &mut VecDeque::new(),
        false,
        CadSettingKey::Grid,
        CadSettingValue::Bool(true),
    )
    .is_err());

    // The queue is bounded.
    let mut full = VecDeque::new();
    for _ in 0..CAD_SETTING_CAP {
        admit_setting(
            &mut full,
            true,
            CadSettingKey::Grid,
            CadSettingValue::Bool(true),
        )
        .unwrap();
    }
    assert!(admit_setting(
        &mut full,
        true,
        CadSettingKey::Grid,
        CadSettingValue::Bool(true),
    )
    .is_err());
}
