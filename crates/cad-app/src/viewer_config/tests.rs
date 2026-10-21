use super::*;
use serde_json::json;

/// The complete protocol from `docs/ui-spec/ui-desc.md` (labels are catalog
/// keys in this codebase; the ui-desc example's literal strings are host data).
const FULL_JSON: &str = r#"{
      "schemaVersion": 1,
      "ui": {
        "preset": "full",
        "layout": { "mode": "auto", "breakpoints": { "compactBelow": 1200, "mobileBelow": 720 } },
        "components": {
          "ribbon": {
            "visible": true,
            "tabs": [
              { "id": "review", "label": "ribbon.review", "groups": [
                { "id": "measure", "label": "ribbon.measure", "commands": ["measure.distance", "measure.angle", "measure.area"] }
              ] }
            ]
          },
          "layerPanel": { "visible": true, "placement": "auto", "initiallyOpen": false },
          "propertiesPanel": { "visible": true, "placement": "auto", "initiallyOpen": false },
          "navigationToolbar": { "visible": true, "placement": "right", "commands": ["view.fit", "view.pan", "view.orbit"] },
          "commandBar": { "visible": false },
          "layoutTabs": { "visible": true },
          "statusBar": { "visible": true }
        },
        "commandOverrides": { "view.pan": { "visible": false } },
        "userCustomization": { "allowedPaths": [
          "ui.components.layerPanel.initiallyOpen",
          "ui.components.propertiesPanel.initiallyOpen"
        ] }
      },
      "features": {
        "measure": true
      },
      "view": { "overlays": { "axes": true, "grid": false, "selectionHighlight": true, "snapHints": true } },
      "interaction": { "pointer": true, "touch": true, "keyboardShortcuts": true }
    }"#;

fn store_with_user_paths() -> ViewerConfigStore {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": [
                "ui.components.layerPanel.initiallyOpen",
                "ui.components.propertiesPanel.initiallyOpen",
                "features.measure",
                "ui.components.ribbon.visible",
                "view.overlays.grid"
            ] } }
        }))
        .unwrap();
    store
}

#[test]
fn defaults_roundtrip_and_breakpoints_use_available_surface() {
    let c = ViewerConfig::default();
    assert_eq!(
        serde_json::from_str::<ViewerConfig>(&serde_json::to_string(&c).unwrap()).unwrap(),
        c
    );
    for (size, expected) in [
        ([1280.0, 800.0], LayoutMode::Desktop),
        ([1000.0, 800.0], LayoutMode::Compact),
        ([390.0, 844.0], LayoutMode::Mobile),
        ([1280.0, 360.0], LayoutMode::Compact),
    ] {
        assert_eq!(
            UiPresentationModel::resolve(&c, size, [0.0; 4], false).layout,
            expected
        );
    }
    assert_eq!(
        UiPresentationModel::resolve(&c, [740.0, 800.0], [0.0, 20.0, 0.0, 20.0], true).layout,
        LayoutMode::Mobile
    );
}

#[test]
fn full_protocol_json_roundtrips_and_validates() {
    let config: ViewerConfig = serde_json::from_str(FULL_JSON).unwrap();
    config.validate().unwrap();
    let roundtrip: ViewerConfig =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert_eq!(roundtrip, config);
    assert_eq!(config.ui.components.ribbon.tabs.len(), 1);
    assert_eq!(config.ui.components.ribbon.tabs[0].groups[0].id, "measure");
    assert!(config.features.measure);
    assert_eq!(
        config.ui.user_customization.allowed_paths[0],
        "ui.components.layerPanel.initiallyOpen"
    );
}

#[test]
fn export_plot_format_defaults_to_svg_and_rejects_unknown_values() {
    // Absent inherits the default; the protocol example never names it.
    let config: ViewerConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(
        config.ui.export.plot_format,
        PlotExportFormat::Svg,
        "an absent plotFormat must inherit the svg default"
    );
    assert_eq!(PlotExportFormat::Svg.as_str(), "svg");
    assert_eq!(PlotExportFormat::Pdf.as_str(), "pdf");
    // A named value parses and round-trips.
    let config: ViewerConfig =
        serde_json::from_str(r#"{"ui":{"export":{"plotFormat":"pdf"}}}"#).unwrap();
    assert_eq!(config.ui.export.plot_format, PlotExportFormat::Pdf);
    config.validate().unwrap();
    // An unknown value is rejected (no silent fallback to svg).
    let error = serde_json::from_str::<ViewerConfig>(r#"{"ui":{"export":{"plotFormat":"png"}}}"#)
        .unwrap_err();
    assert!(
        error.to_string().contains("plotFormat") || error.to_string().contains("png"),
        "unknown plotFormat must be rejected with a precise message: {error}"
    );
    // The merge API reaches the existing leaf and rejects an unknown value.
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({"ui":{"export":{"plotFormat":"pdf"}}}))
        .unwrap();
    assert_eq!(
        store.effective().ui.export.plot_format,
        PlotExportFormat::Pdf
    );
    // The merge reaches the existing leaf; an unknown value fails the
    // schema deserialize (no silent fallback to the svg default).
    let rejected = store
        .update_config(json!({"ui":{"export":{"plotFormat":"png"}}}))
        .unwrap_err();
    assert!(
        rejected.reason.contains("unknown variant") || rejected.reason.contains("png"),
        "unknown plotFormat must be rejected: {rejected}"
    );
    assert_eq!(
        store.effective().ui.export.plot_format,
        PlotExportFormat::Pdf
    );
}

#[test]
fn unknown_fields_are_rejected_with_a_precise_path() {
    let error =
        serde_json::from_str::<ViewerConfig>(r#"{"ui":{"script":"alert(1)"}}"#).unwrap_err();
    assert!(error.to_string().contains("script"));
    let mut store = ViewerConfigStore::default();
    assert_eq!(
        store
            .update_config(json!({"ui":{"script":"alert(1)"}}))
            .unwrap_err()
            .path,
        "ui.script"
    );
}

#[test]
fn unknown_command_override_key_is_rejected_by_validation_and_merge() {
    let error = serde_json::from_str::<ViewerConfig>(
        r#"{"ui":{"commandOverrides":{"mystery.command":{"visible":false}}}}"#,
    );
    // `commandOverrides` itself parses; validation is what rejects the key.
    let config = error.unwrap();
    assert_eq!(
        config.validate().unwrap_err().path,
        "ui.commandOverrides.mystery.command"
    );
    // Merge can add a new override entry, then validation rejects it.
    let mut store = ViewerConfigStore::default();
    assert_eq!(
        store
            .update_config(json!({"ui":{"commandOverrides":{"mystery.command":{"visible":false}}}}))
            .unwrap_err()
            .path,
        "ui.commandOverrides.mystery.command"
    );
    // A whitelisted id round-trips through merge.
    store
        .update_config(json!({"ui":{"commandOverrides":{"view.fit":{"visible":false}}}}))
        .unwrap();
    assert!(!store.effective().ui.command_overrides["view.fit"].visible);
}

#[test]
fn unknown_ribbon_command_and_navigation_command_are_rejected() {
    let config: ViewerConfig = serde_json::from_value(json!({
        "ui": { "components": { "ribbon": { "tabs": [
            { "id": "t", "label": "ribbon.review", "groups": [
                { "id": "g", "label": "ribbon.measure", "commands": ["measure.distance", "nope"] }
            ] }
        ] } } }
    }))
    .unwrap();
    assert_eq!(
        config.validate().unwrap_err().path,
        "ui.components.ribbon.tabs[0].groups[0].commands[1]"
    );
    let config: ViewerConfig = serde_json::from_value(json!({
        "ui": { "components": { "navigationToolbar": { "commands": ["view.fit", "bogus"] } } }
    }))
    .unwrap();
    assert_eq!(
        config.validate().unwrap_err().path,
        "ui.components.navigationToolbar.commands[1]"
    );
}

#[test]
fn duplicate_tab_and_group_ids_are_rejected() {
    let config: ViewerConfig = serde_json::from_value(json!({
        "ui": { "components": { "ribbon": { "tabs": [
            { "id": "a", "label": "ribbon.review", "groups": [] },
            { "id": "a", "label": "ribbon.measure", "groups": [] }
        ] } } }
    }))
    .unwrap();
    assert_eq!(
        config.validate().unwrap_err().path,
        "ui.components.ribbon.tabs[1].id"
    );
}

#[test]
fn unknown_allowed_path_is_rejected() {
    let config: ViewerConfig = serde_json::from_value(json!({
        "ui": { "userCustomization": { "allowedPaths": ["ui.components.ribbon.bogus"] } }
    }))
    .unwrap();
    assert_eq!(
        config.validate().unwrap_err().path,
        "ui.userCustomization.allowedPaths[0]"
    );
    // A command-override path is accepted for a whitelisted command id.
    let config: ViewerConfig = serde_json::from_value(json!({
        "ui": {
            "commandOverrides": { "view.fit": { "visible": true } },
            "userCustomization": { "allowedPaths": ["ui.commandOverrides.view.fit.visible"] }
        }
    }))
    .unwrap();
    config.validate().unwrap();
}

/// A user preference may hide a command but must never re-enable one that a
/// preset or feature capability disabled (spec: user preference cannot turn
/// a forbidden capability back on). `ui.commandOverrides.<id>.visible` is a
/// clamped path exactly like `features.*` and `ui.components.*.visible`.
#[test]
fn user_preference_cannot_re_enable_a_disabled_command_override() {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": {
                "preset": "canvasOnly",
                "userCustomization": { "allowedPaths": [
                    "ui.commandOverrides.view.fit.visible"
                ] }
            }
        }))
        .unwrap();
    // A preference asking to show the command is clamped to the host's
    // disabled value, never switched on.
    store
        .apply_user_preference(json!({
            "ui": { "commandOverrides": { "view.fit": { "visible": true } } }
        }))
        .unwrap();
    assert!(
        !store.effective().ui.command_overrides["view.fit"].visible,
        "a user preference must not re-enable a preset-disabled command; got {:?}",
        store.effective().ui.command_overrides
    );
    assert!(
        !UiPresentationModel::resolve(store.effective(), [1280.0, 800.0], [0.0; 4], false)
            .command_visibility["view.fit"],
        "the resolved command visibility must stay disabled"
    );
    // Hiding is still honoured when the capability is otherwise on.
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": [
                "ui.commandOverrides.view.fit.visible"
            ] } }
        }))
        .unwrap();
    store
        .apply_user_preference(json!({
            "ui": { "commandOverrides": { "view.fit": { "visible": false } } }
        }))
        .unwrap();
    assert!(!store.effective().ui.command_overrides["view.fit"].visible);
}

#[test]
fn false_survives_merge_and_failure_is_atomic_with_field_path() {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({"ui":{"components":{"ribbon":{"visible":false}}}}))
        .unwrap();
    assert!(!store.effective().ui.components.ribbon.visible);
    let old = store.effective().clone();
    let rev = store.revision;
    let error = store
        .update_config(json!({"ui":{"layout":{"breakpoints":{"mobileBelow":2000}}}}))
        .unwrap_err();
    assert_eq!(error.path, "ui.layout.breakpoints.compactBelow");
    assert_eq!(store.effective(), &old);
    assert_eq!(store.revision, rev);
    assert_eq!(
        store
            .update_config(json!({"ui":{"script":"alert(1)"}}))
            .unwrap_err()
            .path,
        "ui.script"
    );
    // Null is never a configuration value.
    assert_eq!(
        store
            .update_config(json!({"ui":{"preset": null}}))
            .unwrap_err()
            .path,
        "ui.preset"
    );
}

#[test]
fn arrays_are_replaced_wholesale_by_merge() {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({"ui":{"userCustomization":{"allowedPaths":["view.overlays.grid"]}}}))
        .unwrap();
    assert_eq!(
        store.effective().ui.user_customization.allowed_paths,
        vec!["view.overlays.grid".to_string()]
    );
    store
            .update_config(json!({"ui":{"userCustomization":{"allowedPaths":["view.overlays.axes","view.overlays.snapHints"]}}}))
            .unwrap();
    assert_eq!(
        store.effective().ui.user_customization.allowed_paths.len(),
        2
    );
}

#[test]
fn parse_order_host_then_allowed_user_preference_with_clamping() {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": [
                "features.measure",
                "ui.components.ribbon.visible",
                "ui.components.layerPanel.initiallyOpen",
                "view.overlays.grid"
            ] },
            "components": { "ribbon": { "visible": false } } },
            "features": { "measure": false }
        }))
        .unwrap();
    // The host disabled measure and the ribbon; a user preference may not
    // switch them back on (clamp), but may disable or customize freely.
    store
        .apply_user_preference(json!({
            "features": { "measure": true },
            "ui": { "components": { "ribbon": { "visible": true },
                "layerPanel": { "initiallyOpen": false } } },
            "view": { "overlays": { "grid": false } }
        }))
        .unwrap();
    let effective = store.effective();
    assert!(
        !effective.features.measure,
        "capability must stay clamped off"
    );
    assert!(
        !effective.ui.components.ribbon.visible,
        "component must stay clamped off"
    );
    assert!(!effective.ui.components.layer_panel.initially_open);
    assert!(!effective.view.overlays.grid);
}

#[test]
fn user_preference_outside_allowed_paths_is_rejected_atomically() {
    let mut store = store_with_user_paths();
    let before = store.effective().clone();
    let rev = store.revision;
    let error = store
        .apply_user_preference(json!({"ui": {"preset": "canvasOnly"}}))
        .unwrap_err();
    assert_eq!(error.path, "ui.preset");
    assert_eq!(store.effective(), &before);
    assert_eq!(store.revision, rev);
    assert_eq!(store.user_preference(), &json!({}));
}

#[test]
fn null_or_non_object_user_preference_is_rejected() {
    let mut store = store_with_user_paths();
    assert_eq!(
        store
            .apply_user_preference(json!({"view": {"overlays": {"grid": null}}}))
            .unwrap_err()
            .path,
        "view.overlays.grid"
    );
    assert_eq!(
        store.apply_user_preference(json!([1, 2])).unwrap_err().path,
        "preference"
    );
}

#[test]
fn clear_user_preference_restores_host_values() {
    let mut store = store_with_user_paths();
    store
        .apply_user_preference(
            json!({"ui": {"components": {"layerPanel": {"initiallyOpen": false}}}}),
        )
        .unwrap();
    assert!(!store.effective().ui.components.layer_panel.initially_open);
    store.clear_user_preference();
    assert!(store.effective().ui.components.layer_panel.initially_open);
    assert_eq!(store.user_preference(), &json!({}));
}

#[test]
fn set_config_reapplies_preference_and_drops_revoked_paths() {
    let mut store = store_with_user_paths();
    store
        .apply_user_preference(
            json!({"ui": {"components": {"layerPanel": {"initiallyOpen": false}}}}),
        )
        .unwrap();
    assert!(!store.effective().ui.components.layer_panel.initially_open);
    // New host config no longer allows that path; the stale preference is
    // ignored rather than erroring.
    let mut host = ViewerConfig::default();
    host.ui.user_customization.allowed_paths.clear();
    store.set_config(host).unwrap();
    assert!(store.effective().ui.components.layer_panel.initially_open);
}

#[test]
fn per_command_visibility_combines_features_and_overrides() {
    let mut host = ViewerConfig::default();
    host.features.measure = false;
    host.ui
        .command_overrides
        .insert("view.pan".into(), CommandOverride { visible: false });
    host.validate().unwrap();
    let p = UiPresentationModel::resolve(&host, [1280.0, 800.0], [0.0; 4], false);
    assert!(!p.command_visibility["measure.distance"]);
    assert!(!p.command_visibility["measure.confirm"]);
    assert!(!p.command_visibility["view.pan"]);
    assert!(p.command_visibility["view.fit"]);
    assert!(p.command_visibility["file.open"]);
}

/// The resolved component flags every preset either permits or drops, in the
/// order the truth table in [`PresetComponents`] documents them, followed by
/// one derived aggregate: whether *every* command is visible. The aggregate
/// is `true` only for `Full`; `Minimal` narrows the command surface (see
/// [`MINIMAL_VISIBLE_COMMANDS`]) even though it keeps the panels.
fn component_flags(p: &UiPresentationModel) -> [bool; 8] {
    [
        p.ribbon,
        p.command_bar,
        p.status_bar,
        p.navigation,
        p.layout_tabs,
        p.layer_panel,
        p.properties_panel,
        p.command_visibility.values().all(|visible| *visible),
    ]
}

fn resolve_preset(preset: Preset) -> UiPresentationModel {
    let mut host = ViewerConfig::default();
    host.ui.preset = preset;
    host.validate().unwrap();
    // Full desktop surface so layout mode does not mask preset differences.
    UiPresentationModel::resolve(&host, [1280.0, 800.0], [0.0; 4], false)
}

#[test]
fn minimal_preset_has_explicit_component_semantics() {
    let p = resolve_preset(Preset::Minimal);
    // Minimal keeps the application frame: it is not canvas-only.
    assert!(p.application_ui);
    // Entry points dropped by minimal.
    assert!(!p.ribbon, "minimal hides the ribbon");
    assert!(!p.command_bar, "minimal hides the command bar");
    assert!(!p.status_bar, "minimal hides the status bar");
    // Canvas navigation, layout tabs and both dock panels are kept.
    assert!(p.navigation, "minimal keeps navigation");
    assert!(p.layout_tabs, "minimal keeps the layout tabs");
    assert!(p.layer_panel, "minimal keeps the layer panel");
    assert!(p.properties_panel, "minimal keeps the properties panel");
    // Minimal narrows the command surface explicitly: review/measure stay,
    // authoring/edit commands are hidden. See `MINIMAL_VISIBLE_COMMANDS`.
    assert!(
        p.command_visibility["measure.distance"] && p.command_visibility["view.fit"],
        "minimal keeps the review/measure and navigation surface"
    );
    assert!(
        !p.command_visibility["draw.line"] && !p.command_visibility["edit.undo"],
        "minimal hides authoring/edit commands"
    );
    // Minimal never auto-reserves a dock gutter; only full desktop docks.
    assert_eq!(
        p.dock_width, 0.0,
        "minimal never reserves a dock gutter; only full desktop docks"
    );
}

#[test]
fn minimal_preset_command_set_is_explicit() {
    let p = resolve_preset(Preset::Minimal);
    // Every known command is classified, and the classification matches the
    // documented whitelist/blacklist exactly (no implicit "rest").
    let visible: Vec<&str> = COMMAND_IDS
        .iter()
        .copied()
        .filter(|id| p.command_visibility[*id])
        .collect();
    let hidden: Vec<&str> = COMMAND_IDS
        .iter()
        .copied()
        .filter(|id| !p.command_visibility[*id])
        .collect();
    let mut expected_visible = MINIMAL_VISIBLE_COMMANDS.to_vec();
    expected_visible.sort_unstable();
    let mut actual_visible = visible.clone();
    actual_visible.sort_unstable();
    assert_eq!(
        actual_visible, expected_visible,
        "minimal-visible commands must equal the documented whitelist"
    );
    let mut expected_hidden = MINIMAL_HIDDEN_COMMANDS.to_vec();
    expected_hidden.sort_unstable();
    let mut actual_hidden = hidden.clone();
    actual_hidden.sort_unstable();
    assert_eq!(
        actual_hidden, expected_hidden,
        "minimal-hidden commands must equal the documented blacklist"
    );
    // The two lists partition COMMAND_IDS: no id is unclassified or double
    // classified, so a newly added command forces a policy decision.
    let mut partition: Vec<&str> = MINIMAL_VISIBLE_COMMANDS
        .iter()
        .chain(MINIMAL_HIDDEN_COMMANDS.iter())
        .copied()
        .collect();
    partition.sort_unstable();
    let mut catalog = COMMAND_IDS.to_vec();
    catalog.sort_unstable();
    assert_eq!(
        partition, catalog,
        "the minimal command policy must be total"
    );
    for id in MINIMAL_VISIBLE_COMMANDS {
        assert!(
            preset_command_visible(Preset::Minimal, id),
            "{id} is whitelisted but `preset_command_visible` hides it"
        );
    }
    for id in MINIMAL_HIDDEN_COMMANDS {
        assert!(
            !preset_command_visible(Preset::Minimal, id),
            "{id} is blacklisted but `preset_command_visible` shows it"
        );
    }
    // A feature gate still applies on top of the whitelist: a whitelisted
    // command disappears when its feature is off.
    let mut host = ViewerConfig::default();
    host.ui.preset = Preset::Minimal;
    host.features.measure = false;
    let gated = UiPresentationModel::resolve(&host, [1280.0, 800.0], [0.0; 4], false);
    assert!(
        !gated.command_visibility["measure.distance"],
        "features.measure=false must hide a whitelisted command under minimal"
    );
    assert!(
        gated.command_visibility["view.fit"],
        "an unrelated whitelisted command stays visible under minimal"
    );
    // An override still hides a whitelisted command, and may never re-enable
    // a blacklisted one.
    let mut host = ViewerConfig::default();
    host.ui.preset = Preset::Minimal;
    host.ui
        .command_overrides
        .insert("view.fit".into(), CommandOverride { visible: false });
    host.ui
        .command_overrides
        .insert("draw.line".into(), CommandOverride { visible: true });
    host.validate().unwrap();
    let overridden = UiPresentationModel::resolve(&host, [1280.0, 800.0], [0.0; 4], false);
    assert!(!overridden.command_visibility["view.fit"]);
    assert!(!overridden.command_visibility["draw.line"]);
}

#[test]
fn presets_are_monotonic_canvas_only_le_minimal_le_full() {
    let full = component_flags(&resolve_preset(Preset::Full));
    let minimal = component_flags(&resolve_preset(Preset::Minimal));
    let canvas_only = component_flags(&resolve_preset(Preset::CanvasOnly));
    for index in 0..full.len() {
        let (f, m, co) = (full[index], minimal[index], canvas_only[index]);
        assert!(
            !co || m,
            "canvasOnly shows component flag {index} that minimal hides"
        );
        assert!(
            !m || f,
            "minimal shows component flag {index} that full hides"
        );
    }
    // The presets are distinct, not aliases: each is strictly stronger than
    // the next on at least one field.
    assert!(full.iter().any(|&v| v) && minimal.iter().any(|&v| !v));
    assert!(minimal.iter().any(|&v| v) && canvas_only.iter().all(|&v| !v));
    // Command visibility is monotonic per id: canvasOnly ⊆ minimal ⊆ full.
    // Full is not merely "some commands": it keeps every catalog command.
    let full_vis = resolve_preset(Preset::Full).command_visibility;
    let minimal_vis = resolve_preset(Preset::Minimal).command_visibility;
    let canvas_only_vis = resolve_preset(Preset::CanvasOnly).command_visibility;
    assert!(
        full_vis.values().all(|visible| *visible),
        "the full preset must keep every command visible"
    );
    for id in COMMAND_IDS {
        assert!(
            !canvas_only_vis[*id] || minimal_vis[*id],
            "canvasOnly shows {id} that minimal hides"
        );
        assert!(
            !minimal_vis[*id] || full_vis[*id],
            "minimal shows {id} that full hides"
        );
    }
}

#[test]
fn preset_canvas_only_hides_every_command_and_overlay_state_is_reported() {
    let mut host = ViewerConfig::default();
    host.ui.preset = Preset::CanvasOnly;
    host.features.measure = false;
    host.view.overlays.grid = false;
    host.interaction.touch = false;
    let p = UiPresentationModel::resolve(&host, [1280.0, 800.0], [0.0; 4], false);
    assert!(!p.application_ui);
    assert!(!p.layer_panel && !p.properties_panel && !p.ribbon && !p.navigation);
    assert!(!p.overlays.grid && p.overlays.axes);
    assert!(!p.features.measure);
    assert!(!p.touch && p.pointer);
    assert!(p.command_visibility.values().all(|visible| !visible));
    // CanvasOnly is strictly stronger than Minimal: it hides every chrome
    // component minimal keeps.
    let minimal = resolve_preset(Preset::Minimal);
    assert!(!p.ribbon && !p.command_bar && !p.status_bar && !p.navigation);
    assert!(!p.layout_tabs && !p.layer_panel && !p.properties_panel);
    assert!(minimal.navigation && minimal.layout_tabs && minimal.layer_panel);
    assert_ne!(p.application_ui, minimal.application_ui);
}

#[test]
fn separate_panel_visibility_and_initially_open_are_resolved() {
    let config: ViewerConfig = serde_json::from_value(json!({
        "ui": { "components": {
            "layerPanel": { "visible": false, "initiallyOpen": true },
            "propertiesPanel": { "visible": true, "initiallyOpen": false }
        } }
    }))
    .unwrap();
    let p = UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false);
    assert!(!p.layer_panel && p.properties_panel);
    assert!(!p.layer_panel_initially_open && !p.properties_panel_initially_open);
    assert_eq!(p.dock_width, 240.0);
    assert!(p.panels());
}

#[test]
fn revision_and_observer_fire_once_per_real_change() {
    let mut store = ViewerConfigStore::default();
    let seen: Rc<std::cell::RefCell<Vec<(u64, bool)>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = seen.clone();
    store.subscribe(Rc::new(move |config, revision| {
        sink.borrow_mut()
            .push((revision, config.ui.components.ribbon.visible));
    }));
    assert_eq!(store.revision, 0);
    store
        .update_config(json!({"ui":{"components":{"ribbon":{"visible":false}}}}))
        .unwrap();
    assert_eq!(store.revision, 1);
    // A no-op update does not bump the revision or notify.
    store
        .update_config(json!({"ui":{"components":{"ribbon":{"visible":false}}}}))
        .unwrap();
    assert_eq!(store.revision, 1);
    assert_eq!(&*seen.borrow(), &[(1, false)]);
    // A rejected update does not notify either.
    let _ = store.update_config(json!({"ui":{"script":"x"}}));
    assert_eq!(store.revision, 1);
}

#[test]
fn effective_config_json_query_round_trips() {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({"ui":{"preset":"canvasOnly"}}))
        .unwrap();
    let json = serde_json::to_string(store.effective()).unwrap();
    let parsed: ViewerConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(&parsed, store.effective());
    assert_eq!(parsed.ui.preset, Preset::CanvasOnly);
}

fn ribbon_config_with_hidden_pan() -> (ViewerConfig, BTreeMap<String, bool>) {
    let config: ViewerConfig = serde_json::from_value(json!({
        "ui": { "components": { "ribbon": { "tabs": [
            { "id": "review", "label": "ribbon.review", "groups": [
                { "id": "measure", "label": "ribbon.measure",
                  "commands": ["measure.distance", "measure.area"] },
                { "id": "view", "label": "ribbon.view",
                  "commands": ["view.fit", "view.pan"] }
            ] },
            { "id": "manage", "label": "ribbon.manage", "groups": [] }
        ] } } }
    }))
    .unwrap();
    let visibility =
        UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false).command_visibility;
    (config, visibility)
}

#[test]
fn resolve_ribbon_preserves_order_and_label_keys() {
    let (config, visibility) = ribbon_config_with_hidden_pan();
    let resolved = resolve_ribbon(&config, &visibility);
    assert_eq!(resolved.tabs.len(), 2);
    // Order is the config order; labels stay catalog keys, not literals.
    assert_eq!(resolved.tabs[0].id, "review");
    assert_eq!(resolved.tabs[0].label, "ribbon.review");
    assert_eq!(resolved.tabs[1].id, "manage");
    assert_eq!(
        resolved.tabs[0]
            .groups
            .iter()
            .map(|group| group.id.as_str())
            .collect::<Vec<_>>(),
        vec!["measure", "view"]
    );
    assert_eq!(resolved.tabs[0].groups[0].label, "ribbon.measure");
    assert_eq!(
        resolved.tabs[0].groups[0]
            .commands
            .iter()
            .map(|command| command.id.as_str())
            .collect::<Vec<_>>(),
        vec!["measure.distance", "measure.area"]
    );
    assert!(resolved.tabs[0].groups[0]
        .commands
        .iter()
        .all(|command| command.visible));
    // A tab with no groups is kept, not dropped.
    assert!(resolved.tabs[1].groups.is_empty());
}

#[test]
fn resolve_ribbon_filters_hidden_commands() {
    let (mut config, _) = ribbon_config_with_hidden_pan();
    config
        .ui
        .command_overrides
        .insert("view.pan".into(), CommandOverride { visible: false });
    let visibility =
        UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false).command_visibility;
    let resolved = resolve_ribbon(&config, &visibility);
    let view_group = &resolved.tabs[0].groups[1];
    assert_eq!(
        view_group
            .commands
            .iter()
            .map(|command| command.id.as_str())
            .collect::<Vec<_>>(),
        vec!["view.fit"]
    );
}

#[test]
fn resolve_ribbon_empty_config_and_unknown_visibility_default_to_hidden() {
    let config = ViewerConfig::default();
    let visibility = BTreeMap::new();
    let resolved = resolve_ribbon(&config, &visibility);
    assert!(resolved.tabs.is_empty());
    assert_eq!(resolved, ResolvedRibbon::default());

    // A command absent from the map is treated as hidden, never visible.
    let (config, _) = ribbon_config_with_hidden_pan();
    let resolved = resolve_ribbon(&config, &BTreeMap::new());
    assert!(resolved
        .tabs
        .iter()
        .flat_map(|tab| &tab.groups)
        .all(|group| group.commands.is_empty()));
}

#[test]
fn ribbon_command_display_round_trips_and_reaches_the_resolved_model() {
    // `/display` is camelCase in JSON; the default must be `iconAndLabel`.
    for (json_display, expected) in [
        (serde_json::Value::Null, RibbonCommandDisplay::IconAndLabel),
        (json!("iconAndLabel"), RibbonCommandDisplay::IconAndLabel),
        (json!("iconOnly"), RibbonCommandDisplay::IconOnly),
        (json!("labelOnly"), RibbonCommandDisplay::LabelOnly),
    ] {
        let mut group = json!({
            "id": "measure", "label": "ribbon.measure",
            "commands": ["measure.distance", "measure.area"]
        });
        if !json_display.is_null() {
            group["display"] = json_display.clone();
        }
        let config: ViewerConfig = serde_json::from_value(json!({
            "ui": { "components": { "ribbon": { "tabs": [
                { "id": "review", "label": "ribbon.review", "groups": [group] }
            ] } } }
        }))
        .unwrap();
        config.validate().unwrap();
        // The value survives a serialize/deserialize round trip.
        let roundtrip: ViewerConfig =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(roundtrip, config);
        let resolved = resolve_ribbon(
            &config,
            &BTreeMap::from([
                ("measure.distance".to_string(), true),
                ("measure.area".to_string(), true),
            ]),
        );
        let commands = &resolved.tabs[0].groups[0].commands;
        assert_eq!(commands.len(), 2);
        assert!(
            commands.iter().all(|command| command.display == expected),
            "display {json_display} should resolve to {expected:?}"
        );
    }
    // An unknown display value is rejected by serde, never defaulted.
    assert!(serde_json::from_value::<ViewerConfig>(json!({
        "ui": { "components": { "ribbon": { "tabs": [
            { "id": "review", "label": "ribbon.review", "groups": [
                { "id": "measure", "label": "ribbon.measure",
                  "commands": ["measure.distance"], "display": "big" }
            ] }
        ] } } }
    }))
    .is_err());
}

/// The nullable interaction leaves (`Option<bool>`) serialize as `null` when
/// the host set no value; an allowed user preference may fill that slot. This
/// is what lets `interaction.snapEnabled=false` persist and survive a reload.
#[test]
fn allowed_preference_fills_a_nullable_interaction_leaf() {
    let mut store = ViewerConfigStore::default();
    store
        .update_config(json!({
            "ui": { "userCustomization": { "allowedPaths": [
                "interaction.snapEnabled",
                "interaction.hoverPreview"
            ] } }
        }))
        .unwrap();
    assert_eq!(store.effective().interaction.snap_enabled, None);
    assert_eq!(store.effective().interaction.hover_preview, None);

    store
        .apply_user_preference(json!({
            "interaction": { "snapEnabled": false, "hoverPreview": true }
        }))
        .unwrap();
    assert_eq!(store.effective().interaction.snap_enabled, Some(false));
    assert_eq!(store.effective().interaction.hover_preview, Some(true));

    // The accumulated preference still projects the two leaves for persistence.
    let projected: serde_json::Value =
        serde_json::from_str(&store.projected_user_preference_json()).unwrap();
    assert_eq!(projected["interaction"]["snapEnabled"], json!(false));
    assert_eq!(projected["interaction"]["hoverPreview"], json!(true));
}
