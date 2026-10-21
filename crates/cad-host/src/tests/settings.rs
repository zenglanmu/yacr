use super::*;

// --- typed settings surface -------------------------------------------------

/// A config store with a fixed config/preference text and a record of every
/// persisted projection, so a test can assert both the startup input and the
/// exact JSON leaves a write produced (no `serde_json` dependency in this crate;
/// the persisted projection is also reloaded through the real parser below).
#[derive(Default)]
struct SettingsConfig {
    config: Option<String>,
    preference: Option<String>,
    persisted: RefCell<Vec<String>>,
}

impl SettingsConfig {
    fn with_preference(text: &str) -> Self {
        Self {
            preference: Some(text.to_string()),
            ..Self::default()
        }
    }
}

impl ConfigStore for SettingsConfig {
    fn load_config(&self) -> ConfigLoad {
        match &self.config {
            Some(text) => ConfigLoad::Text {
                path: "config.json".to_string(),
                text: text.clone(),
            },
            None => ConfigLoad::Absent,
        }
    }

    fn load_preference(&self) -> ConfigLoad {
        match &self.preference {
            Some(text) => ConfigLoad::Text {
                path: "preference.json".to_string(),
                text: text.clone(),
            },
            None => ConfigLoad::Absent,
        }
    }

    fn persist_preference(&self, text: &str) -> CadResult<()> {
        self.persisted.borrow_mut().push(text.to_string());
        Ok(())
    }
}

fn persisted(config: &SettingsConfig) -> Option<String> {
    config.persisted.borrow().last().cloned()
}

/// A platform that records the suggested save name and refuses the picker, so
/// the suggested extension is observable without a dialog or a real export.
#[derive(Default)]
struct NameRecordingFiles {
    suggested: RefCell<Vec<String>>,
}

impl PlatformFiles for NameRecordingFiles {
    fn begin_open(&self) -> CadResult<()> {
        Ok(())
    }
    fn begin_open_target(&self, _target: &FileTarget) -> CadResult<()> {
        Ok(())
    }
    fn begin_save(&self, _purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        self.suggested.borrow_mut().push(default_name.to_string());
        Err(CadError::Cancelled)
    }
    fn write_atomic(&self, _target: &FileTarget, _bytes: &[u8]) -> CadResult<()> {
        Ok(())
    }
    fn refuses_source_overwrite(&self, _target: &FileTarget) -> bool {
        false
    }
    fn set_source(&self, _target: Option<&FileTarget>) {}
    fn poll(&self) -> Vec<FileEvent> {
        Vec::new()
    }
}

fn core_with_settings(config: Rc<SettingsConfig>) -> HostCore {
    core_with_config(config)
}

fn settings_core() -> (HostCore, Rc<SettingsConfig>) {
    let config = Rc::new(SettingsConfig::default());
    (core_with_settings(config.clone()), config)
}

#[test]
fn settings_default_to_the_effective_host_config() {
    let core = core();
    let settings = core.settings();
    assert!(!settings.grid, "the host default keeps the grid off");
    assert!(settings.axes);
    assert!(settings.selection_highlight);
    assert!(settings.snap_hints);
    assert_eq!(settings.plot_format, "svg");
    assert!(settings.snap_enabled, "the session default enables snap");
    assert!(settings.hover_preview);
    assert_eq!(settings.fonts_dir, None);

    // The config-derived fields are the effective config, not a second source.
    let effective = core.config_snapshot().effective;
    assert_eq!(settings.grid, effective.view.overlays.grid);
    assert_eq!(settings.axes, effective.view.overlays.axes);
    assert_eq!(
        settings.selection_highlight,
        effective.view.overlays.selection_highlight
    );
    assert_eq!(settings.snap_hints, effective.view.overlays.snap_hints);
    assert_eq!(
        settings.plot_format,
        effective.ui.export.plot_format.as_str()
    );
}

#[test]
fn each_writable_key_is_persisted_and_reported() {
    let (core, config) = settings_core();

    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .unwrap();
    assert!(core.settings().grid);
    assert!(persisted(&config).unwrap().contains(r#""grid":true"#));

    core.apply_setting(CadSettingKey::Axes, CadSettingValue::Bool(false))
        .unwrap();
    assert!(!core.settings().axes);
    assert!(persisted(&config).unwrap().contains(r#""axes":false"#));

    core.apply_setting(
        CadSettingKey::SelectionHighlight,
        CadSettingValue::Bool(false),
    )
    .unwrap();
    assert!(!core.settings().selection_highlight);
    assert!(persisted(&config)
        .unwrap()
        .contains(r#""selectionHighlight":false"#));

    core.apply_setting(CadSettingKey::SnapHints, CadSettingValue::Bool(false))
        .unwrap();
    assert!(!core.settings().snap_hints);
    assert!(persisted(&config).unwrap().contains(r#""snapHints":false"#));

    core.apply_setting(
        CadSettingKey::PlotFormat,
        CadSettingValue::Text("pdf".to_string()),
    )
    .unwrap();
    assert_eq!(core.settings().plot_format, "pdf");
    assert!(persisted(&config)
        .unwrap()
        .contains(r#""plotFormat":"pdf""#));

    core.apply_setting(CadSettingKey::SnapEnabled, CadSettingValue::Bool(false))
        .unwrap();
    assert!(!core.settings().snap_enabled);
    assert!(!core.controller.borrow().snap_enabled());
    assert!(persisted(&config)
        .unwrap()
        .contains(r#""snapEnabled":false"#));

    core.apply_setting(CadSettingKey::HoverPreview, CadSettingValue::Bool(false))
        .unwrap();
    assert!(!core.settings().hover_preview);
    assert!(!core.controller.borrow().hover_preview_enabled());
    assert!(persisted(&config)
        .unwrap()
        .contains(r#""hoverPreview":false"#));

    // The persisted projection is the allowed leaves only: no capability and no
    // unrelated configuration field leaked into durable storage.
    let persisted = persisted(&config).unwrap();
    for forbidden in ["features", "components", "preset", "commandOverrides"] {
        assert!(
            !persisted.contains(forbidden),
            "the preference projection must not carry {forbidden}: {persisted}"
        );
    }
}

#[test]
fn overlay_settings_change_the_values_the_render_path_reads() {
    let core = core();
    // Publish the effective config once: `HostCore::new` installs the host
    // policy in the store, and the render inputs follow on the first sync.
    core.sync_render().unwrap();
    assert!(!core.render.overlays().visibility.grid);

    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .unwrap();
    assert!(core.render.overlays().visibility.grid);

    core.apply_setting(CadSettingKey::Axes, CadSettingValue::Bool(false))
        .unwrap();
    assert!(!core.render.overlays().visibility.axes);

    core.apply_setting(
        CadSettingKey::SelectionHighlight,
        CadSettingValue::Bool(false),
    )
    .unwrap();
    assert!(!core.render.overlays().visibility.selection_highlight);

    core.apply_setting(CadSettingKey::SnapHints, CadSettingValue::Bool(false))
        .unwrap();
    assert!(!core.render.overlays().visibility.snap_hints);
}

#[test]
fn plot_format_pdf_changes_the_suggested_save_extension() {
    let files = Rc::new(NameRecordingFiles::default());
    let core = core_with_caps(files.clone(), capabilities(false));
    core.apply_setting(
        CadSettingKey::PlotFormat,
        CadSettingValue::Text("pdf".to_string()),
    )
    .unwrap();
    let _ = core.request_plot();
    let suggested = files.suggested.borrow();
    assert_eq!(suggested.len(), 1);
    assert!(
        suggested[0].ends_with(".pdf"),
        "the suggested plot name is .pdf: {}",
        suggested[0]
    );
}

#[test]
fn startup_preference_seeds_the_live_session_switches() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"interaction":{"snapEnabled":false,"hoverPreview":false}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();

    let settings = core.settings();
    assert!(!settings.snap_enabled);
    assert!(!settings.hover_preview);
    let controller = core.controller.borrow();
    assert!(!controller.snap_enabled());
    assert!(!controller.hover_preview_enabled());
}

#[test]
fn startup_seeding_survives_a_document_replacement() {
    let config = Rc::new(SettingsConfig::with_preference(
        r#"{"interaction":{"snapEnabled":false}}"#,
    ));
    let core = core_with_settings(config);
    core.load_startup();
    assert!(!core.settings().snap_enabled);

    // A replacement rebuilds SessionState (switches back to their defaults);
    // the host re-applies the persisted interaction choice.
    core.controller.borrow_mut().new_blank_document().unwrap();
    core.note_document_replaced();
    assert!(
        !core.settings().snap_enabled,
        "the persisted snap choice survives open/new"
    );
    assert!(
        core.settings().hover_preview,
        "an unset interaction field keeps the session default"
    );
}

#[test]
fn write_then_reload_returns_the_same_effective_settings() {
    let (core, config) = settings_core();
    core.apply_setting(CadSettingKey::Grid, CadSettingValue::Bool(true))
        .unwrap();
    core.apply_setting(
        CadSettingKey::PlotFormat,
        CadSettingValue::Text("pdf".to_string()),
    )
    .unwrap();
    core.apply_setting(CadSettingKey::SnapEnabled, CadSettingValue::Bool(false))
        .unwrap();
    let persisted = persisted(&config).expect("each write persists");

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&persisted)));
    reloaded.load_startup();
    assert_eq!(reloaded.settings(), core.settings());
    assert!(reloaded.settings().grid);
    assert_eq!(reloaded.settings().plot_format, "pdf");
    assert!(!reloaded.settings().snap_enabled);
}

#[test]
fn write_rejections_are_explicit_and_leave_the_preference_untouched() {
    let (core, config) = settings_core();

    // Unknown keys are outside the closed table; the wire parse maps them to a
    // stable code instead of ignoring them.
    for key in ["bogus", "view.overlays.grid", "Grid", ""] {
        assert_eq!(CadSettingKey::parse(key), None, "{key:?} is not writable");
    }
    for key in CadSettingKey::ALL {
        assert_eq!(CadSettingKey::parse(key.key()), Some(key));
    }
    assert_eq!(SETTINGS_UNKNOWN_KEY, "settings.unknown_key");

    // A format the CPU vector path cannot produce is refused by name.
    let error = core
        .apply_setting(
            CadSettingKey::PlotFormat,
            CadSettingValue::Text("png".to_string()),
        )
        .unwrap_err();
    assert!(matches!(
        &error,
        CadError::InvalidInput(code) if code == SETTINGS_INVALID_PLOT_FORMAT
    ));

    // A wrong value type is refused, not coerced.
    let error = core
        .apply_setting(
            CadSettingKey::Grid,
            CadSettingValue::Text("true".to_string()),
        )
        .unwrap_err();
    assert!(matches!(
        &error,
        CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE
    ));
    let error = core
        .apply_setting(CadSettingKey::PlotFormat, CadSettingValue::Bool(true))
        .unwrap_err();
    assert!(matches!(
        &error,
        CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE
    ));

    // Nothing was written and the effective values are unchanged.
    assert!(
        config.persisted.borrow().is_empty(),
        "a rejected write must not change the preference file"
    );
    assert!(!core.settings().grid);
    assert_eq!(core.settings().plot_format, "svg");
}

#[test]
fn settings_report_the_configured_fonts_dir_read_only() {
    let mut options = options();
    options.fonts_dir = Some(std::path::PathBuf::from("/tmp/yacr-fonts"));
    let platform = HostPlatform {
        files: Rc::new(FakeFiles::default()),
        config_store: Rc::new(SettingsConfig::default()),
        surface: Rc::new(FakeSurface),
        resources: Rc::new(UnsupportedResources),
        capabilities: capabilities(false),
    };
    let core = HostCore::new(options, platform, RenderSessionHandle::default()).unwrap();
    assert_eq!(
        core.settings().fonts_dir.as_deref(),
        Some("/tmp/yacr-fonts")
    );
}

// --- display options: backgrounds + crosshair --------------------------------

/// Give the demo document one empty paper layout so a space switch is real.
fn install_paper_layout(core: &HostCore) {
    let mut builder = cad_db::DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1));
    builder
        .insert_layer(cad_db::Layer {
            id: cad_domain::LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_layout(cad_db::Layout {
            id: cad_domain::LayoutId(1),
            name: "Sheet1".into(),
            viewports: Vec::new(),
        })
        .unwrap();
    let drawing = std::sync::Arc::new(builder.finish().unwrap());
    let mut controller = core.controller.borrow_mut();
    let document = controller.document_id;
    controller
        .application
        .workspace
        .documents
        .get_mut(&document)
        .expect("the host always has a document")
        .drawing = drawing;
}

#[test]
fn display_settings_default_to_black_backgrounds_and_a_full_crosshair() {
    let core = core();
    let settings = core.settings();
    assert_eq!(settings.background_model, "#000000");
    assert_eq!(settings.background_paper, "#000000");
    assert_eq!(settings.crosshair_percent, 100);
    // The defaults are the effective config's, not a second source.
    let effective = core.config_snapshot().effective;
    assert_eq!(
        settings.background_model,
        effective.view.background.model.to_hex()
    );
    assert_eq!(
        settings.background_paper,
        effective.view.background.paper.to_hex()
    );
    assert_eq!(
        settings.crosshair_percent,
        effective.view.crosshair.size_percent
    );
    // And the renderer's documented default (opaque black) is what the frame
    // input carries when no preference was ever written.
    core.sync_render().unwrap();
    assert_eq!(
        core.render.view_snapshot().clear_color,
        [0.0, 0.0, 0.0, 1.0]
    );
}

#[test]
fn the_display_keys_are_persisted_and_reported() {
    let (core, config) = settings_core();

    core.apply_setting(
        CadSettingKey::BackgroundModel,
        CadSettingValue::Text("#FF0000".to_string()),
    )
    .unwrap();
    assert_eq!(core.settings().background_model, "#ff0000");
    assert!(persisted(&config)
        .unwrap()
        .contains("\"model\":\"#ff0000\""));

    core.apply_setting(
        CadSettingKey::BackgroundPaper,
        CadSettingValue::Text("#00ff00".to_string()),
    )
    .unwrap();
    assert_eq!(core.settings().background_paper, "#00ff00");
    assert!(persisted(&config)
        .unwrap()
        .contains("\"paper\":\"#00ff00\""));

    core.apply_setting(CadSettingKey::CrosshairSize, CadSettingValue::Number(25))
        .unwrap();
    assert_eq!(core.settings().crosshair_percent, 25);
    assert!(persisted(&config).unwrap().contains(r#""sizePercent":25"#));

    // Only the allowed display leaves reach durable storage, under their real
    // config paths (never a flattened or invented key).
    let persisted = persisted(&config).unwrap();
    assert!(
        persisted
            .contains("{\"view\":{\"background\":{\"model\":\"#ff0000\",\"paper\":\"#00ff00\"}"),
        "the background leaves are persisted under view.background: {persisted}"
    );
    assert!(
        persisted.contains("\"crosshair\":{\"sizePercent\":25}"),
        "the crosshair leaf is persisted under view.crosshair: {persisted}"
    );
    for forbidden in ["features", "components", "preset", "commandOverrides"] {
        assert!(
            !persisted.contains(forbidden),
            "the preference projection must not carry {forbidden}: {persisted}"
        );
    }
}

#[test]
fn the_display_settings_apply_to_the_next_frame_live() {
    let core = core();
    // The demo document must have a paper layout for the space switch to be a
    // real switch (the same fixture shape the other host contracts use).
    install_paper_layout(&core);
    core.sync_render().unwrap();

    // Model space: the clear colour the renderer will consume is the new one.
    core.apply_setting(
        CadSettingKey::BackgroundModel,
        CadSettingValue::Text("#ff0000".to_string()),
    )
    .unwrap();
    assert_eq!(
        core.render.view_snapshot().clear_color,
        [1.0, 0.0, 0.0, 1.0],
        "the model-space frame clears to the configured colour"
    );

    // Paper space really is a different background: switching space changes the
    // clear colour without reopening anything.
    core.apply_setting(
        CadSettingKey::BackgroundPaper,
        CadSettingValue::Text("#0000ff".to_string()),
    )
    .unwrap();
    core.command(
        cad_app::CommandId::SwitchSpace,
        CommandPayload::Space(SpaceId::Paper(cad_domain::LayoutId(1))),
    )
    .expect("the fixture has the paper layout");
    core.sync_render().unwrap();
    assert_eq!(
        core.render.view_snapshot().clear_color,
        [0.0, 0.0, 1.0, 1.0],
        "the paper-space frame clears to the paper colour"
    );

    // Back to model space: the model colour is still the one configured.
    core.command(
        cad_app::CommandId::SwitchSpace,
        CommandPayload::Space(SpaceId::Model),
    )
    .expect("model space is always switchable");
    core.sync_render().unwrap();
    assert_eq!(
        core.render.view_snapshot().clear_color,
        [1.0, 0.0, 0.0, 1.0]
    );

    // The crosshair size is the value the overlay builder consumes.
    assert_eq!(core.render.overlays().crosshair_percent, 100);
    core.apply_setting(CadSettingKey::CrosshairSize, CadSettingValue::Number(40))
        .unwrap();
    assert_eq!(core.render.overlays().crosshair_percent, 40);
}

#[test]
fn invalid_display_values_are_refused_without_writing() {
    let (core, config) = settings_core();

    for value in ["red", "#abc", "", "#12345", "#gggggg"] {
        let error = core
            .apply_setting(
                CadSettingKey::BackgroundModel,
                CadSettingValue::Text(value.to_string()),
            )
            .unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{value:?} must be refused with the stable code: {error:?}"
        );
    }
    // A wrong shape (a number for a colour, a string or an out-of-range number
    // for the crosshair) is refused too, never coerced.
    for (key, value) in [
        (CadSettingKey::BackgroundPaper, CadSettingValue::Number(1)),
        (CadSettingKey::BackgroundPaper, CadSettingValue::Bool(true)),
        (
            CadSettingKey::CrosshairSize,
            CadSettingValue::Text("50".to_string()),
        ),
        (CadSettingKey::CrosshairSize, CadSettingValue::Number(0)),
        (CadSettingKey::CrosshairSize, CadSettingValue::Number(101)),
    ] {
        let error = core.apply_setting(key, value).unwrap_err();
        assert!(
            matches!(&error, CadError::InvalidInput(code) if code == SETTINGS_INVALID_VALUE),
            "{key:?} must be refused: {error:?}"
        );
    }

    assert!(
        config.persisted.borrow().is_empty(),
        "a rejected write must not change the preference file"
    );
    let settings = core.settings();
    assert_eq!(settings.background_model, "#000000");
    assert_eq!(settings.background_paper, "#000000");
    assert_eq!(settings.crosshair_percent, 100);
}

#[test]
fn the_display_settings_survive_a_restart() {
    let (core, config) = settings_core();
    core.apply_setting(
        CadSettingKey::BackgroundModel,
        CadSettingValue::Text("#102030".to_string()),
    )
    .unwrap();
    core.apply_setting(
        CadSettingKey::BackgroundPaper,
        CadSettingValue::Text("#405060".to_string()),
    )
    .unwrap();
    core.apply_setting(CadSettingKey::CrosshairSize, CadSettingValue::Number(75))
        .unwrap();
    let persisted = persisted(&config).expect("each write persists");

    let reloaded = core_with_settings(Rc::new(SettingsConfig::with_preference(&persisted)));
    reloaded.load_startup();
    reloaded.sync_render().unwrap();
    assert_eq!(reloaded.settings(), core.settings());
    assert_eq!(reloaded.settings().background_model, "#102030");
    assert_eq!(reloaded.settings().background_paper, "#405060");
    assert_eq!(reloaded.settings().crosshair_percent, 75);
    // The reloaded values reach the live render inputs on the first sync.
    assert_eq!(
        reloaded.render.view_snapshot().clear_color,
        cad_app::viewer_config::HexColor::parse("#102030")
            .unwrap()
            .to_linear_rgba()
    );
    assert_eq!(reloaded.render.overlays().crosshair_percent, 75);
}
