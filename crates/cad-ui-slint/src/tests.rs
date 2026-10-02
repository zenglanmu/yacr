//! Unit tests.

use super::*;

#[test]
fn shell_definition_mentions_a_cad_frame_property() {
    assert!(UI_DEFINITION.contains("cad-frame"));
    assert!(ZH_CN_MESSAGES.contains('{'));
}

#[test]
fn shell_exposes_independent_redo_and_measurement_affordances() {
    // U11: redo must not share the undo flag.
    assert!(UI_DEFINITION.contains("can-redo"));
    assert!(UI_DEFINITION.contains("can-undo"));
    // U03/U04: kind selector + confirm/cancel + step/unit status.
    assert!(UI_DEFINITION.contains("measure-kind-selected"));
    assert!(UI_DEFINITION.contains("confirm-measurement-requested"));
    assert!(UI_DEFINITION.contains("cancel-measurement-requested"));
    assert!(UI_DEFINITION.contains("canvas-pick"));
}

#[test]
fn shell_is_responsive_and_consumes_compact_config() {
    // U01: the arrangement is width-driven, not a single hardcoded row.
    for property in [
        "compact-shell",
        "phone-shell",
        "control-height",
        "touch-target",
        "side-panel-width",
        "side-panel-collapsed",
        "drawer-height",
        "show-floating-nav",
    ] {
        assert!(
            UI_DEFINITION.contains(property),
            "shell must expose responsive {property}"
        );
    }
    // The three arrangements are all present in one component.
    assert!(UI_DEFINITION.contains("side-panel-open"));
    assert!(UI_DEFINITION.contains("tools-open"));
    // A phone drawer entry and a wide collapsible panel entry exist; the
    // desktop bar and phone bar are separate branches, not a squash.
    assert!(UI_DEFINITION.contains("phone-shell && root.tools-open"));
    assert!(UI_DEFINITION.contains("show-floating-nav"));
}

#[test]
fn shell_reaches_every_merged_panel_and_layout_list() {
    // U03: every already-merged panel is reachable from the shell.
    for marker in [
        "layer-rows",
        "property-rows",
        "annotation-rows",
        "diagnostics-rows",
        "layout-rows",
        "layout-selected",
        "layout-empty-label",
    ] {
        assert!(
            UI_DEFINITION.contains(marker),
            "missing panel marker {marker}"
        );
    }
}

#[test]
fn layout_rows_and_active_index_are_pushed_faithfully() {
    let mut state = LayoutPanelState::default();
    state.rows.push(LayoutRowUi {
        id: 7,
        name: "Sheet A".into(),
        supported: true,
        reason: String::new(),
        viewport_count: 2,
    });
    state.rows.push(LayoutRowUi {
        id: 8,
        name: "Sheet B".into(),
        supported: false,
        reason: "unsupported viewport".into(),
        viewport_count: 1,
    });
    state.active_index = Some(1);
    assert_eq!(state.rows[1].name, "Sheet B");
    assert!(!state.rows[1].supported);
    assert_eq!(state.active_index, Some(1));

    // Model space has no active layout row.
    state.active_index = None;
    assert_eq!(state.active_index, None);
}

#[test]
fn shell_exposes_layer_and_property_panels() {
    // F03: layer list with visibility toggles + restore affordance.
    assert!(UI_DEFINITION.contains("layer-rows"));
    assert!(UI_DEFINITION.contains("layer-visibility-toggled"));
    assert!(UI_DEFINITION.contains("restore-layers-requested"));
    // F05: read-only properties + explicit empty/mixed states.
    assert!(UI_DEFINITION.contains("property-rows"));
    assert!(UI_DEFINITION.contains("property-empty-label"));
    assert!(UI_DEFINITION.contains("property-mixed-label"));
    assert!(UI_DEFINITION.contains("clear-selection-requested"));
}

#[test]
fn layout_panel_state_mirrors_descriptors_and_marks_the_active_row() {
    use cad_domain::LayoutId;
    use cad_representation::{LayoutDescriptor, SpaceSelection};

    let descriptors = vec![
        LayoutDescriptor {
            id: LayoutId(1),
            name: "Layout1".into(),
            supported: true,
            reason: String::new(),
            viewport_count: 1,
        },
        LayoutDescriptor {
            id: LayoutId(2),
            name: "Layout2".into(),
            supported: false,
            reason: "viewport scale must be positive".into(),
            viewport_count: 1,
        },
    ];
    let state = LayoutPanelState::from_descriptors(
        &descriptors,
        SpaceSelection::Paper(LayoutId(2)),
        "无布局",
    );
    assert_eq!(state.rows.len(), 2);
    assert!(state.rows[0].supported);
    assert!(!state.rows[1].supported);
    assert_eq!(state.active_index, Some(1));

    // Model space selects no row.
    let model = LayoutPanelState::from_descriptors(&descriptors, SpaceSelection::Model, "无布局");
    assert_eq!(model.active_index, None);

    // Empty input yields an empty model, not a fabricated row.
    let empty = LayoutPanelState::from_descriptors(&[], SpaceSelection::Model, "无布局");
    assert!(empty.rows.is_empty());
    assert_eq!(empty.empty_label, "无布局");
}

#[test]
fn layer_panel_state_mirrors_real_rows() {
    use cad_app::layers::{LayerOverrideSet, LayerRow};
    use cad_domain::LayerId;

    let mut overrides = LayerOverrideSet::new();
    overrides.set(LayerId(1), false);
    let rows = vec![
        LayerRow {
            id: LayerId(0),
            name: "0".into(),
            database_visible: true,
            override_visible: None,
            effective_visible: true,
        },
        LayerRow {
            id: LayerId(1),
            name: "WALLS".into(),
            database_visible: true,
            override_visible: Some(false),
            effective_visible: false,
        },
    ];
    let state = LayerPanelState::from_rows(&rows, "无图层");
    assert_eq!(state.rows.len(), 2);
    assert_eq!(state.override_count, 1);
    assert!(!state.rows[1].visible);
    assert!(state.rows[1].overridden);
    assert_eq!(state.empty_label, "无图层");

    // Empty input yields an empty model, not a fabricated row.
    let empty = LayerPanelState::from_rows(&[], "无图层");
    assert!(empty.rows.is_empty());
    assert_eq!(empty.override_count, 0);
}

#[test]
fn property_panel_state_tracks_empty_and_mixed() {
    use cad_app::SelectionProperties;

    let empty = SelectionProperties::default();
    let state = PropertyPanelState::from_properties(&empty, "未选择", |_| String::new());
    assert_eq!(state.count, 0);
    assert!(state.rows.is_empty());
    assert_eq!(state.empty_label, "未选择");

    let mixed = SelectionProperties {
        count: 2,
        rows: Vec::new(),
        mixed_keys: vec!["id", "length"],
        empty: false,
    };
    let state = PropertyPanelState::from_properties(&mixed, "未选择", |keys| {
        format!("多值: {}", keys.join(","))
    });
    assert_eq!(state.count, 2);
    assert_eq!(state.mixed_label, "多值: id,length");
}

#[test]
fn measurement_ui_state_tracks_the_preview() {
    use cad_app::MeasurementTool;
    let mut tool = MeasurementTool::new(MeasurementToolKind::Area);

    // Idle: nothing active, no confirm.
    let idle = MeasurementUiState::from_preview(None, "drawing units");
    assert!(!idle.active);
    assert!(!idle.can_confirm);
    assert_eq!(idle.step_label, "");
    assert_eq!(idle.unit_label, "drawing units");

    // Capturing: active but not confirmable yet.
    tool.push_point(Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    let capturing = MeasurementUiState::from_preview(Some(&tool.preview()), "m");
    assert!(capturing.active);
    assert!(!capturing.can_confirm);
    assert_eq!(capturing.kind, MeasurementToolKind::Area);
    assert_eq!(
        capturing.kind_index(),
        MeasurementToolKind::Area.index() as i32
    );

    // Ready: confirm enabled.
    tool.push_point(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    tool.push_point(Point3 {
        x: 1.0,
        y: 1.0,
        z: 0.0,
    });
    let ready = MeasurementUiState::from_preview(Some(&tool.preview()), "m");
    assert!(ready.can_confirm);
}

#[test]
fn kind_index_round_trips_through_ui_state() {
    for (index, kind) in MeasurementToolKind::ALL.iter().copied().enumerate() {
        let state = MeasurementUiState {
            kind,
            ..MeasurementUiState::default()
        };
        assert_eq!(state.kind_index() as usize, index);
        assert_eq!(
            MeasurementToolKind::from_index(state.kind_index()),
            Some(kind)
        );
    }
}

#[test]
fn catalog_drives_ui_defaults() {
    // The shell no longer hardcodes its initial labels: they come from the
    // embedded catalog, so both languages stay reachable through config.
    assert_eq!(
        MessageSource::for_locale(Locale::ZhCn).text("file.open", &[]),
        "打开图纸"
    );
    assert_eq!(
        MessageSource::for_locale(Locale::En).text("file.open", &[]),
        "Open drawing"
    );
}

#[test]
fn chrome_is_catalog_driven_not_hardcoded() {
    // Every chrome control binds to a pushed property; the shell holds no
    // literal label. This mirrors `scripts/check-i18n.py` in-crate.
    for property in [
        "fit-label",
        "undo-label",
        "redo-label",
        "export-label",
        "import-label",
        "diagnostics-label",
        "measurement-panel-label",
        "annotation-panel-label",
        "layer-panel-label",
        "property-panel-label",
        "diagnostics-drawer-title",
        "layout-panel-label",
        "view-panel-label",
        "view-2d-label",
        "view-3d-label",
        "view-projection-label",
        "view-ortho-label",
        "view-perspective-label",
        "standard-view-labels",
        "tools-label",
        "drawer-label",
        "nav-label",
    ] {
        assert!(
            UI_DEFINITION.contains(property),
            "shell must expose {property}"
        );
    }
    // A hardcoded CJK literal in the shell is a missing translation.
    assert!(
        !UI_DEFINITION.chars().any(is_cjk),
        "ui/app.slint still contains a hardcoded CJK literal"
    );
}

/// True for CJK ideographs and the CJK punctuation used in the chrome.
fn is_cjk(ch: char) -> bool {
    let code = ch as u32;
    (0x4E00..=0x9FFF).contains(&code) || (0x3000..=0x303F).contains(&code)
}

#[test]
fn diagnostics_panel_state_is_pushable_and_empty_by_default() {
    let state = DiagnosticsPanelState::default();
    assert!(state.is_empty());
    // The drawer needs an explicit empty label; the adapter supplies the
    // catalog one, never a fabricated row.
    assert!(state.rows.is_empty());
}

#[test]
fn shell_reaches_the_view_and_space_controls() {
    // F04/F13/F14: the 2D/3D toggle, standard views and the layout selector
    // are all reachable from the shell and bound to catalog-built models.
    for marker in [
        "view-3d",
        "view-perspective",
        "toggle-view-mode-requested",
        "toggle-projection-requested",
        "standard-view-selected",
        "standard-view-labels",
        "layout-selected",
    ] {
        assert!(
            UI_DEFINITION.contains(marker),
            "shell must expose view/space control {marker}"
        );
    }
}

#[test]
fn orbit_delta_is_proportional_finite_and_signed() {
    // A rightward drag yaws negative; a downward drag pitches positive. The
    // exact sign is a UI choice; the magnitude and finiteness are the
    // contract.
    let (yaw, pitch) = orbit_delta([10.0, 10.0], [30.0, 40.0]);
    assert!(yaw < 0.0);
    assert!(pitch > 0.0);
    assert_eq!(yaw, -20.0 * ORBIT_RADIANS_PER_PIXEL);
    assert_eq!(pitch, 30.0 * ORBIT_RADIANS_PER_PIXEL);
    // No movement is exactly zero, so no command is emitted.
    assert_eq!(orbit_delta([5.0, 5.0], [5.0, 5.0]), (0.0, 0.0));
    // Non-finite input is refused rather than poisoning the camera.
    assert_eq!(orbit_delta([0.0, 0.0], [f64::NAN, 2.0]), (0.0, 0.0));
    assert_eq!(orbit_delta([0.0, 0.0], [1.0, f64::INFINITY]), (0.0, 0.0));
}

#[test]
fn view_state_ui_defaults_are_2d_not_3d() {
    let state = ViewStateUi::default();
    assert!(!state.is_3d);
    assert!(!state.perspective);
}
