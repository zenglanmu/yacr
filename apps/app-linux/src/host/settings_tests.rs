//! The `cad_settings` wire shape (the typed admission rules themselves live in
//! `cad_ui_core::session` and are pinned by that crate's tests).

use cad_ui_core::session::CadSettings;

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
        wgpu_backend: "dx12".to_string(),
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
            "wgpu_backend": "dx12",
            "fontsDir": "/tmp/yacr-fonts",
        })
    );
    // Exactly the fourteen documented fields; no unrelated Options fields.
    assert_eq!(wire.as_object().expect("object").len(), 14);
    // The backend choice keeps the shared snake_case key on the snapshot wire
    // (`settings.wgpu_backend` in the UI), never camelCased into `wgpuBackend`.
    assert!(
        wire.get("wgpuBackend").is_none(),
        "the backend field must not camelCase on the wire: {wire}"
    );
}
