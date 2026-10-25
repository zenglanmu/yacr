use super::super::*;
use super::test_support::*;

fn plain_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn session_settings_report_the_effective_defaults() {
    let session = plain_session();
    let settings = session.settings();
    assert!(!settings.grid);
    assert!(settings.axes);
    assert!(settings.selection_highlight);
    assert!(settings.snap_hints);
    assert_eq!(settings.plot_format, "svg");
    assert!(settings.snap_enabled);
    assert!(settings.hover_preview);
    assert!(!settings.grid_snap_enabled, "SNAP starts off");
    assert!(!settings.ortho_enabled, "ORTHO starts off");
    assert_eq!(
        settings.snap_spacing,
        cad_app::DEFAULT_SNAP_SPACING,
        "the declared 10.0 drawing-unit spacing"
    );
    assert_eq!(settings.background_model, "#000000");
    assert_eq!(settings.background_paper, "#000000");
    assert_eq!(settings.crosshair_percent, 100);
    assert_eq!(
        settings.pickbox_size, 3,
        "the AutoCAD PICKBOX initial value"
    );
    assert_eq!(
        settings.aperture_size, 12,
        "this project's existing snap aperture"
    );
    assert_eq!(
        settings.wgpu_backend, "auto",
        "the renderer backend default is `auto`"
    );
    assert_eq!(settings.fonts_dir, None);
}

#[test]
fn session_apply_setting_reports_new_values_and_changes_the_live_state() {
    let session = plain_session();

    let settings = session
        .apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .expect("an allowed overlay write");
    assert!(settings.grid);
    assert!(
        session.render_session().overlays().visibility.grid,
        "the render input the host draws from carries the new value"
    );

    let settings = session
        .apply_setting(CadSettingKey::SnapEnabled, CadSettingValue::Bool(false))
        .expect("an allowed interaction write");
    assert!(!settings.snap_enabled);
    assert!(!session.snapshot(None, None, 0).snap_enabled);

    let settings = session
        .apply_setting(
            CadSettingKey::PlotFormat,
            CadSettingValue::Text("pdf".to_string()),
        )
        .expect("the second supported vector format");
    assert_eq!(settings.plot_format, "pdf");
}

#[test]
fn session_rejects_bad_values_with_stable_codes() {
    let session = plain_session();
    let error = session
        .apply_setting(
            CadSettingKey::PlotFormat,
            CadSettingValue::Text("png".to_string()),
        )
        .unwrap_err();
    assert_eq!(error, SETTINGS_INVALID_PLOT_FORMAT);

    let error = session
        .apply_setting(CadSettingKey::Grid, CadSettingValue::Text("on".to_string()))
        .unwrap_err();
    assert_eq!(error, SETTINGS_INVALID_VALUE);

    // The display options use the same stable refusal code, and an out-of-range
    // percentage is refused rather than clamped.
    for value in ["red", "#abc", ""] {
        let error = session
            .apply_setting(
                CadSettingKey::BackgroundModel,
                CadSettingValue::Text(value.to_string()),
            )
            .unwrap_err();
        assert_eq!(error, SETTINGS_INVALID_VALUE);
    }
    for percent in [0, 101] {
        let error = session
            .apply_setting(
                CadSettingKey::CrosshairSize,
                CadSettingValue::Number(percent),
            )
            .unwrap_err();
        assert_eq!(error, SETTINGS_INVALID_VALUE);
    }
    // AutoCAD's own ranges: pickbox 1..=20, aperture 1..=50.
    for pixels in [0, 21] {
        let error = session
            .apply_setting(CadSettingKey::PickboxSize, CadSettingValue::Number(pixels))
            .unwrap_err();
        assert_eq!(error, SETTINGS_INVALID_VALUE);
    }
    for pixels in [0, 51] {
        let error = session
            .apply_setting(CadSettingKey::ApertureSize, CadSettingValue::Number(pixels))
            .unwrap_err();
        assert_eq!(error, SETTINGS_INVALID_VALUE);
    }

    // Nothing changed: the rejected writes never reached the store.
    let settings = session.settings();
    assert!(!settings.grid);
    assert_eq!(settings.plot_format, "svg");
    assert_eq!(settings.background_model, "#000000");
    assert_eq!(settings.crosshair_percent, 100);
    assert_eq!(settings.pickbox_size, 3);
    assert_eq!(settings.aperture_size, 12);
}

#[test]
fn session_display_settings_reach_the_live_render_inputs() {
    let session = plain_session();

    let settings = session
        .apply_setting(
            CadSettingKey::BackgroundModel,
            CadSettingValue::Text("#102030".to_string()),
        )
        .expect("a valid model background");
    assert_eq!(settings.background_model, "#102030");
    assert_eq!(
        session.render_session().view_snapshot().clear_color,
        cad_app::viewer_config::HexColor::parse("#102030")
            .expect("valid hex")
            .to_linear_rgba(),
        "the frame input the renderer consumes carries the new background"
    );

    let settings = session
        .apply_setting(CadSettingKey::CrosshairSize, CadSettingValue::Number(30))
        .expect("an in-range crosshair size");
    assert_eq!(settings.crosshair_percent, 30);
    assert_eq!(
        session.render_session().overlays().crosshair_percent,
        30,
        "the overlay input the cursor builder consumes carries the new size"
    );

    // The pickbox is the box the same builder draws: its pixel size is the
    // configured value, and the crosshair stays untouched by it.
    let settings = session
        .apply_setting(CadSettingKey::PickboxSize, CadSettingValue::Number(6))
        .expect("an in-range pickbox size");
    assert_eq!(settings.pickbox_size, 6);
    assert_eq!(
        session.render_session().overlays().pickbox_logical_pixels,
        6.0,
        "the cursor overlay sizes its box from the configured pickbox"
    );
    assert_eq!(
        session.render_session().overlays().crosshair_percent,
        30,
        "the pickbox never changes the crosshair"
    );

    let settings = session
        .apply_setting(CadSettingKey::ApertureSize, CadSettingValue::Number(24))
        .expect("an in-range aperture size");
    assert_eq!(settings.aperture_size, 24);
    assert_eq!(
        session.render_session().overlays().pickbox_logical_pixels,
        6.0,
        "the aperture never changes the pickbox"
    );
}

#[test]
fn session_startup_preference_seeds_status_and_interaction() {
    let session = session_with_config(Rc::new(StaticConfig::preference(
        r#"{"interaction":{"snapEnabled":false,"hoverPreview":false}}"#,
    )));
    let status = session.snapshot(None, None, 0);
    assert!(!status.snap_enabled);
    assert!(!status.hover_preview);
    let settings = session.settings();
    assert!(!settings.snap_enabled && !settings.hover_preview);
    // The unset drafting leaves keep the documented session defaults.
    assert!(!status.snap_grid_enabled);
    assert!(!status.ortho_enabled);
    assert_eq!(status.snap_spacing, cad_app::DEFAULT_SNAP_SPACING);
}

/// The persisted drafting values seed the live session at startup through the
/// same seeding call site as `snapEnabled`/`hoverPreview`.
#[test]
fn session_startup_preference_seeds_the_drafting_values() {
    let session = session_with_config(Rc::new(StaticConfig::preference(
        r#"{"interaction":{"gridSnapEnabled":true,"orthoEnabled":true,"snapSpacing":25}}"#,
    )));
    let status = session.snapshot(None, None, 0);
    assert!(status.snap_grid_enabled);
    assert!(status.ortho_enabled);
    assert_eq!(status.snap_spacing, 25.0);
    let settings = session.settings();
    assert!(settings.grid_snap_enabled);
    assert!(settings.ortho_enabled);
    assert_eq!(settings.snap_spacing, 25.0);
}

/// The three persisted drafting settings round-trip through `cad_set_setting` /
/// `cad_settings`: each write reports the new value, changes the live session and
/// never toggles a switch that already matches.
#[test]
fn session_drafting_settings_round_trip_and_are_idempotent() {
    let session = plain_session();
    assert!(
        !session.settings().grid_snap_enabled
            && !session.settings().ortho_enabled
            && session.settings().snap_spacing == cad_app::DEFAULT_SNAP_SPACING
    );

    // Applying the same value twice must leave the switch on: the host dispatches
    // the flip command only when the live value differs.
    for _ in 0..2 {
        let settings = session
            .apply_setting(CadSettingKey::GridSnapEnabled, CadSettingValue::Bool(true))
            .expect("an allowed SNAP write");
        assert!(settings.grid_snap_enabled);
        assert!(session.snapshot(None, None, 0).snap_grid_enabled);
        let settings = session
            .apply_setting(CadSettingKey::OrthoEnabled, CadSettingValue::Bool(true))
            .expect("an allowed ORTHO write");
        assert!(settings.ortho_enabled);
        assert!(session.snapshot(None, None, 0).ortho_enabled);
    }

    let settings = session
        .apply_setting(CadSettingKey::SnapSpacing, CadSettingValue::Real(2.5))
        .expect("a positive finite spacing");
    assert_eq!(settings.snap_spacing, 2.5);
    assert_eq!(session.snapshot(None, None, 0).snap_spacing, 2.5);
    let settings = session
        .apply_setting(CadSettingKey::SnapSpacing, CadSettingValue::Number(25))
        .expect("an integer wire number is the same quantity");
    assert_eq!(settings.snap_spacing, 25.0);

    // Switching back off is reported too.
    let settings = session
        .apply_setting(CadSettingKey::GridSnapEnabled, CadSettingValue::Bool(false))
        .expect("an allowed SNAP write");
    assert!(!settings.grid_snap_enabled);
    assert!(!session.snapshot(None, None, 0).snap_grid_enabled);
}

#[test]
fn session_refuses_invalid_drafting_values_with_stable_codes() {
    let session = plain_session();
    let before = session.settings();

    for value in [
        CadSettingValue::Number(0),
        CadSettingValue::Real(0.0),
        CadSettingValue::Real(-1.0),
        CadSettingValue::Real(f64::NAN),
        CadSettingValue::Real(f64::INFINITY),
    ] {
        let error = session
            .apply_setting(CadSettingKey::SnapSpacing, value.clone())
            .unwrap_err();
        assert_eq!(error, SETTINGS_INVALID_VALUE, "{value:?}");
    }
    for value in [
        CadSettingValue::Text("25".to_string()),
        CadSettingValue::Bool(true),
    ] {
        let error = session
            .apply_setting(CadSettingKey::SnapSpacing, value.clone())
            .unwrap_err();
        assert_eq!(error, SETTINGS_INVALID_VALUE, "{value:?}");
    }
    for value in [
        CadSettingValue::Number(1),
        CadSettingValue::Text("on".to_string()),
        CadSettingValue::Real(1.0),
    ] {
        for key in [CadSettingKey::GridSnapEnabled, CadSettingKey::OrthoEnabled] {
            let error = session.apply_setting(key, value.clone()).unwrap_err();
            assert_eq!(error, SETTINGS_INVALID_VALUE, "{key:?} {value:?}");
        }
    }

    let after = session.settings();
    assert_eq!(after.grid_snap_enabled, before.grid_snap_enabled);
    assert_eq!(after.ortho_enabled, before.ortho_enabled);
    assert_eq!(after.snap_spacing, before.snap_spacing);
}

#[test]
fn session_unknown_keys_are_outside_the_closed_table() {
    assert_eq!(CadSettingKey::parse("bogus"), None);
    assert_eq!(CadSettingKey::parse("view.overlays.grid"), None);
    for key in CadSettingKey::ALL {
        assert_eq!(CadSettingKey::parse(key.key()), Some(key));
    }
    assert_eq!(SETTINGS_UNKNOWN_KEY, "settings.unknown_key");
}

#[test]
fn session_backend_setting_reports_and_accepts_only_the_wire_tokens() {
    let session = plain_session();
    assert_eq!(session.settings().wgpu_backend, "auto");

    // The Options dialog reads `settings.wgpu_backend` and writes the identical
    // `wgpu_backend` key; both are the same closed token set the host persists.
    let settings = session
        .apply_setting(
            CadSettingKey::WgpuBackend,
            CadSettingValue::Text("vulkan".to_string()),
        )
        .expect("a wire token");
    assert_eq!(settings.wgpu_backend, "vulkan");
    assert_eq!(session.settings().wgpu_backend, "vulkan");

    let error = session
        .apply_setting(
            CadSettingKey::WgpuBackend,
            CadSettingValue::Text("gl".to_string()),
        )
        .unwrap_err();
    assert_eq!(error, SETTINGS_INVALID_VALUE);
    assert_eq!(
        session.settings().wgpu_backend,
        "vulkan",
        "a refused token never rewrites the reported value"
    );
    assert_eq!(
        CadSettingKey::parse("wgpu_backend"),
        Some(CadSettingKey::WgpuBackend)
    );
}
