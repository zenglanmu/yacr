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
    assert!(UI_DEFINITION.contains("root.phone-shell && root.tools-open"));
    assert!(UI_DEFINITION.contains("show-floating-nav"));
}

#[test]
fn shell_reaches_every_merged_panel_and_layout_list() {
    // U03: every already-merged panel is reachable from the shell.
    for marker in [
        "layer-rows",
        "property-rows",
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
fn phone_drawer_reaches_the_layout_list() {
    // The desktop tabs are hidden on a phone, so the drawer opened by the
    // bottom "more" entry must carry the same real layout list. Locate that
    // drawer block by its gate and assert the layout controls live inside it,
    // not merely somewhere else in the component.
    let gate = "root.phone-shell && root.tools-open && root.layouts-visible";
    let start = UI_DEFINITION
        .find(gate)
        .expect("phone drawer must gate the layout strip on its open state");
    // The drawer block ends at the next top-level branch; the phone bottom bar
    // is the following `root.phone-shell : Rectangle` arm.
    let tail = &UI_DEFINITION[start..];
    let end = tail
        .find("if root.application-ui && root.phone-shell : Rectangle")
        .expect("phone bottom bar must follow the drawer layout strip");
    let drawer = &tail[..end];

    for marker in [
        "layout-rows",
        "layout-selected",
        "layout-empty-label",
        "layout-model-space-label",
        "row.supported",
        "root.layout-active-index == index",
    ] {
        assert!(
            drawer.contains(marker),
            "phone drawer layout strip must bind {marker}"
        );
    }
    // Model space (index -1) is offered exactly like the desktop tabs.
    assert!(
        drawer.contains("root.layout-selected(-1)"),
        "phone drawer must keep model space reachable"
    );
    // The desktop tabs are untouched and still not shown on a phone.
    assert!(
        UI_DEFINITION.contains("root.layouts-visible && !root.phone-shell"),
        "desktop layout tabs must remain the wide/desktop branch"
    );
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
        viewport_scale: None,
    });
    state.rows.push(LayoutRowUi {
        id: 8,
        name: "Sheet B".into(),
        supported: false,
        reason: "unsupported viewport".into(),
        viewport_count: 1,
        viewport_scale: None,
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
        "diagnostics-label",
        "measurement-panel-label",
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

#[test]
fn mode_ui_state_reflects_the_authoritative_mode() {
    let zh = MessageSource::for_locale(Locale::ZhCn);
    let work = ModeUiState::from_mode(cad_app::AppMode::Work, &zh);
    assert!(work.work);
    assert_eq!(work.label, "增强");
    let viewer = ModeUiState::from_mode(cad_app::AppMode::Viewer, &zh);
    assert!(!viewer.work);
    assert_eq!(viewer.label, "查看");

    let en = MessageSource::for_locale(Locale::En);
    assert_eq!(
        ModeUiState::from_mode(cad_app::AppMode::Viewer, &en).label,
        "Viewer"
    );
    assert_eq!(
        ModeUiState::from_mode(cad_app::AppMode::Work, &en).label,
        "Enhanced"
    );
}

#[test]
fn mode_labels_resolve_in_both_catalogs() {
    for messages in [
        MessageSource::for_locale(Locale::ZhCn),
        MessageSource::for_locale(Locale::En),
    ] {
        for mode in [cad_app::AppMode::Work, cad_app::AppMode::Viewer] {
            assert!(!status::mode_label(&messages, mode).is_empty());
        }
    }
}

#[test]
fn shell_exposes_the_async_open_progress_panel_and_cancel() {
    // F01: a progress panel with a real/indeterminate bar and a cancel action.
    for marker in [
        "import-active",
        "import-phase-label",
        "import-progress-text",
        "import-percent",
        "import-indeterminate",
        "import-cancellable",
        "import-panel-label",
        "import-cancel-label",
        "cancel-open-requested",
    ] {
        assert!(
            UI_DEFINITION.contains(marker),
            "shell must expose async-open marker {marker}"
        );
    }
}

#[test]
fn import_ui_state_is_hidden_when_idle_or_opened() {
    let zh = MessageSource::for_locale(Locale::ZhCn);
    // Idle: nothing pushed, panel hidden, no fabricated labels.
    let idle = ImportProgressUiState::from_snapshot(None, &zh);
    assert!(!idle.visible);
    assert_eq!(idle.percent, None);
    assert_eq!(idle.phase_label, "");
    assert_eq!(idle.progress_text, "");
    assert!(!idle.cancellable);
    assert!(!idle.terminal);
    assert_eq!(idle.source, None);

    // A successful open is a terminal but the panel is hidden (the document is
    // the feedback); it is not an error state.
    let opened =
        cad_app::ImportProgressSnapshot::terminal(cad_app::ImportTerminal::Opened { entities: 4 });
    let state = ImportProgressUiState::from_snapshot(Some(&opened), &zh);
    assert!(!state.visible);
    assert!(!state.terminal);
    assert_eq!(state.percent, None);
}

#[test]
fn import_ui_state_never_fabricates_a_percent_or_a_byte_count() {
    let zh = MessageSource::for_locale(Locale::ZhCn);

    // A running job with no observed total: indeterminate, no percent, and the
    // text must not claim a total.
    let indeterminate = cad_app::ImportProgressSnapshot {
        running: true,
        phase: None,
        entities_done: 3,
        entities_total: None,
        bytes: None,
        cancellable: true,
        terminal: None,
    };
    let state = ImportProgressUiState::from_snapshot(Some(&indeterminate), &zh);
    assert!(state.visible);
    assert_eq!(state.percent, None);
    assert!(state.cancellable);
    assert!(state.progress_text.contains('3'));
    assert!(
        !state.progress_text.contains("0 字节") && !state.progress_text.contains("bytes"),
        "an unknown byte count must not render as bytes: {}",
        state.progress_text
    );

    // A real total yields a real percent and an exact count.
    let determinate = cad_app::ImportProgressSnapshot {
        running: true,
        phase: None,
        entities_done: 2,
        entities_total: Some(4),
        bytes: Some(2048),
        cancellable: true,
        terminal: None,
    };
    let state = ImportProgressUiState::from_snapshot(Some(&determinate), &zh);
    assert_eq!(state.percent, Some(0.5));
    assert!(state.progress_text.contains('2'));
    assert!(state.progress_text.contains('4'));
    assert!(state.progress_text.contains("2048"));

    // A known zero total is indeterminate rather than a division by zero.
    let zero = cad_app::ImportProgressSnapshot {
        entities_total: Some(0),
        ..determinate
    };
    let state = ImportProgressUiState::from_snapshot(Some(&zero), &zh);
    assert_eq!(state.percent, None);
}

#[test]
fn import_ui_state_reports_cancelled_and_failed_terminals_explicitly() {
    let zh = MessageSource::for_locale(Locale::ZhCn);
    let en = MessageSource::for_locale(Locale::En);

    let cancelled = cad_app::ImportProgressSnapshot::terminal(cad_app::ImportTerminal::Cancelled);
    for messages in [&zh, &en] {
        let state = ImportProgressUiState::from_snapshot(Some(&cancelled), messages);
        assert!(state.visible && state.terminal);
        assert!(!state.cancellable, "a terminal cannot be cancelled again");
        assert_eq!(state.percent, None);
        assert!(!state.phase_label.is_empty());
    }

    let failed = cad_app::ImportProgressSnapshot::terminal(cad_app::ImportTerminal::Failed {
        error: cad_domain::CadError::CorruptData("bad".into()),
    });
    let state = ImportProgressUiState::from_snapshot(Some(&failed), &zh);
    assert!(state.visible && state.terminal);
    assert!(!state.cancellable);
    assert!(state.phase_label.contains("bad"));
}

#[test]
fn import_phase_keys_all_resolve_in_both_catalogs() {
    // Every machine phase key cad-app can emit must have a label; the mapper
    // itself is unit-tested in cad-app, so here we only assert coverage.
    for messages in [
        MessageSource::for_locale(Locale::ZhCn),
        MessageSource::for_locale(Locale::En),
    ] {
        for key in [
            "reading",
            "parsing",
            "tables",
            "entities",
            "resolving",
            "finishing",
            "unknown",
        ] {
            let label = messages.text(&format!("import.phase.{key}"), &[]);
            assert!(
                !label.contains("import.phase."),
                "phase key {key} has no catalog label: {label}"
            );
        }
    }
}

#[test]
fn multi_touch_cancels_a_draw_and_never_commits() {
    // drawing-edit §4: the input policy is shared; a second contact cancels the
    // in-progress tool rather than committing it. The draw capture itself only
    // commits on an explicit confirm, so a gesture can never write a drawing.
    use cad_app::{InputOutcome, InputPolicy, PointerUpdate};
    let mut policy = InputPolicy::new();
    policy.handle(PointerUpdate {
        logical_position: [10.0, 10.0],
        contacts: 1,
    });
    let outcome = policy.handle(PointerUpdate {
        logical_position: [12.0, 12.0],
        contacts: 2,
    });
    assert!(matches!(
        outcome,
        InputOutcome::ToolCancelled {
            reason: cad_app::CancelReason::MultiTouch
        }
    ));
    // Lifting both contacts emits no commit.
    assert_eq!(
        policy.handle(PointerUpdate {
            logical_position: [12.0, 12.0],
            contacts: 0,
        }),
        InputOutcome::Ignored
    );
}

#[test]
fn shell_exposes_real_draw_edit_tools() {
    // drawing-edit §4: the ribbon draw group is real and mode-gated, and the
    // shell has a capture row with confirm/cancel and a step label.
    for marker in [
        "begin-draw-tool",
        "confirm-draw-requested",
        "cancel-draw-requested",
        "draw-tool-active",
        "draw-can-confirm",
        "draw-step-label",
        "draw-confirm-label",
        "draw-cancel-label",
        "draw-status",
    ] {
        assert!(
            UI_DEFINITION.contains(marker),
            "shell must expose draw marker {marker}"
        );
    }
}

#[test]
fn ribbon_draw_buttons_are_enabled_by_work_mode_not_hardcoded_disabled() {
    let ribbon = include_str!("../ui/ribbon.slint");
    // The buttons start a tool and are gated by the real mode.
    assert!(ribbon.contains("root.draw-tool(label)"));
    assert!(ribbon.contains("enabled: root.work-mode"));
    assert!(ribbon.contains("callback draw-tool(string)"));
    // The old read-only placeholder comment is gone.
    assert!(!ribbon.contains("ui.ribbon.draw_modify"));
    assert!(!ribbon.contains("drawing database is"));
}

#[test]
fn command_line_has_no_editing_placeholder_and_knows_draw_words() {
    // The command vocabulary recognizes the draw/edit words and the unknown
    // fallback is a real "unknown command" text, not a pending placeholder.
    let source = include_str!("command_line.rs");
    for word in ["\"LINE\"", "\"CIRCLE\"", "\"MOVE\"", "\"TRIM\""] {
        assert!(source.contains(word), "command line must know {word}");
    }
    assert!(!source.contains("ui.command.editing"));
    assert!(!source.contains("command.pending"));
    for messages in [
        MessageSource::for_locale(Locale::ZhCn),
        MessageSource::for_locale(Locale::En),
    ] {
        assert!(
            messages
                .message("command.unknown", &[("command", "X")])
                .is_found(),
            "unknown input must have explicit text"
        );
        // Build the removed keys dynamically so the i18n scanner (which flags
        // literal `a.b` lookups with no catalog entry) does not read this
        // negative assertion as a real reference.
        let pending_key = ["command", "pending"].join(".");
        let ribbon_pending_key = ["ribbon", "pending"].join(".");
        assert!(
            !messages.message(&pending_key, &[]).is_found(),
            "the pending placeholder must be gone"
        );
        assert!(
            !messages.message(&ribbon_pending_key, &[]).is_found(),
            "the ribbon pending placeholder must be gone"
        );
    }
}

#[test]
fn command_line_confirms_or_cancels_whatever_tool_is_active() {
    // AutoCAD convention: ESC/CONFIRM must act on the current command, not only
    // a draw/edit capture. Guard the wiring against a regression to draw-only.
    let source = include_str!("command_line.rs");
    for marker in [
        "get_measurement_active",
        "invoke_confirm_measurement_requested",
        "invoke_cancel_measurement_requested",
        "get_draw_tool_active",
        "set_pan_active",
    ] {
        assert!(
            source.contains(marker),
            "command line must handle {marker} for the active tool"
        );
    }
}

#[test]
fn draw_kind_labels_and_errors_resolve_in_both_locales() {
    for messages in [
        MessageSource::for_locale(Locale::ZhCn),
        MessageSource::for_locale(Locale::En),
    ] {
        let labels = draw_kind_labels(&messages);
        assert_eq!(labels.len(), cad_app::DrawToolKind::ALL.len());
        for (index, kind) in cad_app::DrawToolKind::ALL.iter().copied().enumerate() {
            assert!(
                !labels[index].contains("draw.kind."),
                "kind {} has no catalog label: {}",
                kind.key(),
                labels[index]
            );
            assert_eq!(draw_kind_from_label(&messages, &labels[index]), Some(kind));
        }
        // A machine key never resolves to a translated label.
        assert_eq!(draw_kind_from_label(&messages, "line"), None);
        // The three explicit error cases and the unwired case resolve.
        for error in [
            cad_domain::CadError::PermissionDenied,
            cad_domain::CadError::InvalidInput("no intersection with boundary".into()),
            cad_domain::CadError::Unsupported("SPLINE".into()),
        ] {
            let text = draw_error_text(&messages, &error);
            assert!(!text.is_empty());
            assert!(!text.contains("draw.error."), "missing error label: {text}");
        }
        assert!(!messages
            .text("draw.error.unwired", &[])
            .contains("draw.error."));
    }
}

#[test]
fn viewer_presentation_maps_separate_panels_overlays_and_features() {
    use cad_app::viewer_config::{Preset, UiPresentationModel, ViewerConfig};
    let mut config = ViewerConfig::default();
    config.ui.components.layer_panel.visible = false;
    config.ui.components.properties_panel.initially_open = false;
    config.view.overlays.grid = false;
    config.features.measure = false;
    config.interaction.touch = false;
    let p = UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false);
    assert!(!p.layer_panel && p.properties_panel);
    assert!(!p.layer_panel_initially_open);
    assert!(!p.properties_panel_initially_open);
    assert!(!p.overlays.grid && p.overlays.axes);
    assert!(!p.features.measure);
    assert!(!p.touch && p.pointer);
    assert!(!p.command_visibility["measure.distance"]);
    assert!(p.command_visibility["view.fit"]);
    // Canvas-only forces every application-UI entry off while overlays stay
    // controlled by `view` independently.
    config.ui.preset = Preset::CanvasOnly;
    let p = UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false);
    assert!(!p.application_ui && !p.ribbon && !p.panels());
    assert!(p.overlays.axes);
}

#[test]
fn config_chrome_properties_are_pushed_from_the_store() {
    // The shell exposes data-only config readouts the host can query/persist.
    for property in [
        "config-revision",
        "config-preset",
        "config-effective-json",
        "overlay-grid",
        "feature-measure",
        "cmd-measure-visible",
        "layer-panel-visible",
        "properties-panel-visible",
        "layer-panel-initially-open",
    ] {
        assert!(
            UI_DEFINITION.contains(property),
            "shell must expose config property {property}"
        );
    }
}

#[test]
fn shell_exposes_status_bar_overlay_toggles() {
    // The desktop status bar carries real interactive overlay toggles wired to
    // the config funnel, not just the read-only `overlay-*` readouts.
    assert!(
        UI_DEFINITION.contains("callback overlay-toggled(string, bool)"),
        "shell must expose the overlay-toggled callback"
    );
    for marker in [
        "overlay-toggled(\"axes\"",
        "overlay-toggled(\"grid\"",
        "overlay-toggled(\"snapHints\"",
        // The toggles bind their checked state to the effective overlay flags.
        "checked: root.overlay-axes",
        "checked: root.overlay-grid",
        "checked: root.overlay-snap-hints",
    ] {
        assert!(
            UI_DEFINITION.contains(marker),
            "shell must expose status-bar overlay marker {marker}"
        );
    }
}

#[test]
fn shell_has_no_dead_export_import_controls() {
    // Annotation export/import was removed from the product (docs/handoff.md),
    // but its ribbon/top-bar buttons lingered as enabled, clickable no-ops. A
    // host must never render a control whose callback has no handler, so the
    // affordances are removed outright; this guards against their return.
    for marker in [
        "export-requested",
        "import-requested",
        "cmd-export-visible",
        "cmd-import-visible",
        "can-export",
        "can-import",
    ] {
        assert!(
            !UI_DEFINITION.contains(marker),
            "removed export/import affordance `{marker}` must not return"
        );
    }
}

#[test]
fn status_overlay_toggle_patch_updates_store_and_presentation() {
    use cad_app::viewer_config::{UiPresentationModel, ViewerConfigStore};

    // Grid starts off (AutoCAD convention); the toggle turns it on through the
    // exact JSON patch the adapter merges into the store.
    let mut store = ViewerConfigStore::default();
    assert!(!store.effective().view.overlays.grid);

    let patch = overlay_toggle_patch("grid", true).expect("grid is a known overlay");
    store.update_config_json(&patch).unwrap();
    assert!(store.effective().view.overlays.grid);
    let p = UiPresentationModel::resolve(store.effective(), [1280.0, 800.0], [0.0; 4], false);
    assert!(p.overlays.grid);

    // Axes and snap hints flow through the same path and stay independent.
    let patch = overlay_toggle_patch("axes", false).expect("axes is a known overlay");
    store.update_config_json(&patch).unwrap();
    let patch = overlay_toggle_patch("snapHints", false).expect("snapHints is a known overlay");
    store.update_config_json(&patch).unwrap();
    let p = UiPresentationModel::resolve(store.effective(), [1280.0, 800.0], [0.0; 4], false);
    assert!(!p.overlays.axes && p.overlays.grid && !p.overlays.snap_hints);

    // An unknown key is refused, never an invalid config write or a bypass.
    assert!(overlay_toggle_patch("bogus", true).is_none());
}

#[test]
fn shell_exposes_configured_ribbon_markers() {
    for marker in [
        "ribbon-config-tabs",
        "ribbon-config-driven",
        "callback ribbon-command(string)",
        "RibbonTabModel",
    ] {
        assert!(
            UI_DEFINITION.contains(marker),
            "shell must expose configured-ribbon marker {marker}"
        );
    }
}

#[test]
fn configured_ribbon_model_is_localized_and_visibility_filtered() {
    use cad_app::viewer_config::{CommandOverride, RibbonGroup, RibbonTab, ViewerConfig};
    use slint::Model;

    let messages = MessageSource::for_locale(Locale::En);
    let mut config = ViewerConfig::default();
    config.ui.components.ribbon.tabs = vec![RibbonTab {
        id: "review".into(),
        label: "ribbon.review".into(),
        groups: vec![RibbonGroup {
            id: "measure".into(),
            label: "ribbon.measure".into(),
            commands: vec![
                "measure.distance".into(),
                "measure.area".into(),
                "view.fit".into(),
            ],
            display: cad_app::viewer_config::RibbonCommandDisplay::IconAndLabel,
        }],
    }];
    config
        .ui
        .command_overrides
        .insert("measure.area".into(), CommandOverride { visible: false });

    let chrome = build_ribbon_config(&config, &messages);
    assert!(chrome.config_driven);
    assert_eq!(chrome.tab_labels, vec![messages.text("ribbon.review", &[])]);

    let tab = &chrome.tabs[0];
    assert_eq!(tab.id.as_str(), "review");
    assert_eq!(tab.label.as_str(), messages.text("ribbon.review", &[]));
    assert_eq!(tab.groups.row_count(), 1);
    let group = tab.groups.row_data(0).unwrap();
    assert_eq!(group.label.as_str(), messages.text("ribbon.measure", &[]));
    // The hidden command is absent; the rest keep config order and resolve
    // their catalog labels (never a literal).
    assert_eq!(group.commands.row_count(), 2);
    let first = group.commands.row_data(0).unwrap();
    assert_eq!(first.id.as_str(), "measure.distance");
    assert_eq!(
        first.label.as_str(),
        messages.text("measure.kind.distance", &[])
    );
    assert!(!first.icon.is_empty());
    assert!(first.visible);
    // The default display mode reaches the model as the icon+label code.
    assert_eq!(first.display, 0);
    assert_eq!(group.commands.row_data(1).unwrap().id.as_str(), "view.fit");
}

#[test]
fn empty_ribbon_config_keeps_the_builtin_tabs() {
    use cad_app::viewer_config::ViewerConfig;

    let messages = MessageSource::for_locale(Locale::ZhCn);
    let config = ViewerConfig::default();
    let chrome = build_ribbon_config(&config, &messages);
    assert!(!chrome.config_driven);
    assert!(chrome.tabs.is_empty());
    let expected: Vec<String> = ["ribbon.file", "ribbon.view", "ribbon.measure", "shell.more"]
        .iter()
        .map(|key| messages.text(key, &[]))
        .collect();
    assert_eq!(chrome.tab_labels, expected);
}

/// Build a one-tab/one-group config whose commands are all visible, so the
/// assertions are about display/overflow rather than visibility filtering.
fn ribbon_config_with_commands(
    display: &str,
    commands: &[&str],
) -> cad_app::viewer_config::ViewerConfig {
    use cad_app::viewer_config::{RibbonCommandDisplay, RibbonGroup, RibbonTab, ViewerConfig};
    let display = match display {
        "iconOnly" => RibbonCommandDisplay::IconOnly,
        "labelOnly" => RibbonCommandDisplay::LabelOnly,
        _ => RibbonCommandDisplay::IconAndLabel,
    };
    let mut config = ViewerConfig::default();
    config.ui.components.ribbon.tabs = vec![RibbonTab {
        id: "review".into(),
        label: "ribbon.review".into(),
        groups: vec![RibbonGroup {
            id: "measure".into(),
            label: "ribbon.measure".into(),
            commands: commands.iter().map(|id| (*id).to_string()).collect(),
            display,
        }],
    }];
    config
}

#[test]
fn configured_ribbon_command_display_reaches_the_model() {
    use cad_app::viewer_config::RibbonCommandDisplay;
    use slint::Model;

    let messages = MessageSource::for_locale(Locale::En);
    // The enum code mapping is explicit: 0 = icon+label, 1 = icon only,
    // 2 = label only.
    assert_eq!(
        ribbon_command_display_code(RibbonCommandDisplay::IconAndLabel),
        0
    );
    assert_eq!(
        ribbon_command_display_code(RibbonCommandDisplay::IconOnly),
        1
    );
    assert_eq!(
        ribbon_command_display_code(RibbonCommandDisplay::LabelOnly),
        2
    );

    for (display_json, expected_code) in [("iconAndLabel", 0), ("iconOnly", 1), ("labelOnly", 2)] {
        let config = ribbon_config_with_commands(display_json, &["measure.distance"]);
        let chrome = build_ribbon_config(&config, &messages);
        let command = chrome.tabs[0].groups.row_data(0).unwrap().commands;
        let first = command.row_data(0).unwrap();
        assert_eq!(
            first.display, expected_code,
            "display {display_json} should map to {expected_code}"
        );
    }
}

#[test]
fn configured_ribbon_groups_over_the_cap_keep_every_command() {
    use slint::Model;

    let messages = MessageSource::for_locale(Locale::En);
    // 8 visible commands > the Slint overflow cap of 6: all must stay in the
    // resolved model so the overflow path can reach them (nothing dropped).
    let commands = [
        "view.fit",
        "view.pan",
        "view.orbit",
        "view.reset",
        "view.standard",
        "view.projection",
        "view.switch2d3d",
        "draw.line",
    ];
    let config = ribbon_config_with_commands("iconAndLabel", &commands);
    let chrome = build_ribbon_config(&config, &messages);
    let group = chrome.tabs[0].groups.row_data(0).unwrap();
    assert_eq!(group.commands.row_count(), commands.len());
    let ids: Vec<String> = (0..group.commands.row_count())
        .map(|index| group.commands.row_data(index).unwrap().id.to_string())
        .collect();
    assert_eq!(
        ids,
        commands
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn ribbon_overflow_is_a_floating_anchored_flyout_not_inline() {
    let ribbon = include_str!("../ui/ribbon.slint");
    // The overflow opens as a PopupWindow, i.e. its own surface anchored below
    // the ▾ button, so the ribbon row height does not grow.
    assert!(
        ribbon.contains("PopupWindow"),
        "overflow must use a floating PopupWindow"
    );
    for marker in [
        "overflow-flyout := PopupWindow",
        "overflow-flyout.show()",
        "overflow-flyout.close()",
        // Anchoring: x/y track the open button's recorded origin.
        "x: root.overflow-anchor-x",
        "y: root.overflow-anchor-y",
        // Outside-click close resets the ribbon's open-group index.
        "overflow-flyout-open",
        "close-on-click-outside",
    ] {
        assert!(
            ribbon.contains(marker),
            "floating flyout must expose marker {marker}"
        );
    }
    // The remainder must dispatch the real command id and close the flyout.
    assert!(ribbon.contains("root.ribbon-command(command.id)"));
    // No inline remainder list remains: the only occurrence of the past-cap
    // bound is inside the popup, and no inline group ever grows with it.
    let past_cap = "command-index >= root.max-commands";
    assert_eq!(
        ribbon.matches(past_cap).count(),
        1,
        "exactly one past-cap renderer (the flyout) should remain"
    );
    assert!(
        !ribbon.contains("Expanded overflow list"),
        "the inline overflow list must be gone"
    );
}

#[test]
fn ribbon_group_overflow_markers_are_present() {
    let ribbon = include_str!("../ui/ribbon.slint");
    for marker in [
        "overflow-open-group",
        "max-commands",
        "root.overflow-label",
        "group.commands.length > root.max-commands",
    ] {
        assert!(
            ribbon.contains(marker),
            "ribbon must expose overflow marker {marker}"
        );
    }
    // The model carries the display code the renderer branches on.
    let model = include_str!("../ui/ribbon-model.slint");
    assert!(model.contains("display: int"));
}

#[test]
fn ribbon_command_actions_dispatch_or_report_explicitly() {
    use RibbonCommandAction::*;

    assert_eq!(ribbon_command_action("file.open"), Open);
    assert_eq!(ribbon_command_action("edit.redo"), Redo);
    assert_eq!(ribbon_command_action("view.fit"), Fit);
    assert_eq!(ribbon_command_action("view.pan"), Pan);
    assert_eq!(ribbon_command_action("view.reset"), ResetView);
    assert_eq!(ribbon_command_action("view.projection"), ToggleProjection);
    assert_eq!(ribbon_command_action("view.switch2d3d"), Switch2d3d);
    assert_eq!(
        ribbon_command_action("measure.distance"),
        MeasureKind("distance")
    );
    assert_eq!(ribbon_command_action("layer.restore"), RestoreLayers);
    assert_eq!(ribbon_command_action("draw.trim"), BeginDraw("trim"));
    assert_eq!(ribbon_command_action("mode.toggle"), ToggleMode);
    assert_eq!(ribbon_command_action("diagnostics.open"), OpenDiagnostics);
    // Every whitelisted id is classified, never silently dropped.
    for id in cad_app::viewer_config::command_ids() {
        let _ = ribbon_command_action(id);
    }
    // Ids that need a target the ribbon cannot supply are explicit, not faked.
    for id in [
        "view.orbit",
        "view.standard",
        "layer.toggle",
        "layout.switch",
        "backend.switch",
        "totally.unknown",
    ] {
        assert_eq!(ribbon_command_action(id), Unsupported, "id {id}");
    }
}

#[test]
fn every_whitelisted_ribbon_command_has_a_classified_action_and_reason() {
    use RibbonCommandAction::*;

    // The regression net for the whole data-driven ribbon: every id must map to
    // a concrete action or be an explicit `Unsupported`, and every `Unsupported`
    // id must carry a concrete reason (never the generic "not available" for a
    // command the build actually knows).
    let expected: &[(&str, RibbonCommandAction)] = &[
        ("file.open", Open),
        ("edit.undo", Undo),
        ("edit.redo", Redo),
        ("view.fit", Fit),
        ("view.pan", Pan),
        ("view.reset", ResetView),
        ("view.projection", ToggleProjection),
        ("view.switch2d3d", Switch2d3d),
        ("measure.distance", MeasureKind("distance")),
        ("measure.polyline", MeasureKind("polyline")),
        ("measure.angle", MeasureKind("angle")),
        ("measure.area", MeasureKind("area")),
        ("measure.confirm", ConfirmMeasurement),
        ("measure.cancel", CancelMeasurement),
        ("layer.restore", RestoreLayers),
        ("draw.line", BeginDraw("line")),
        ("draw.circle", BeginDraw("circle")),
        ("draw.move", BeginDraw("move")),
        ("draw.trim", BeginDraw("trim")),
        ("mode.toggle", ToggleMode),
        ("diagnostics.open", OpenDiagnostics),
    ];
    // Every id with a concrete action is asserted above.
    for (id, action) in expected {
        assert_eq!(ribbon_command_action(id), *action, "id {id}");
    }
    // The whitelist is exactly the wired ids plus the deliberately ambiguous
    // ones, so adding an id to `COMMAND_IDS` without an action fails here.
    let unsupported: &[&str] = &[
        "view.orbit",
        "view.standard",
        "layer.toggle",
        "layout.switch",
        "backend.switch",
    ];
    let mut classified: Vec<&str> = expected.iter().map(|(id, _)| *id).collect();
    classified.extend_from_slice(unsupported);
    classified.sort_unstable();
    let mut whitelisted = cad_app::viewer_config::command_ids().to_vec();
    whitelisted.sort_unstable();
    assert_eq!(classified, whitelisted, "COMMAND_IDS coverage drifted");

    // An ambiguous command explains which input is missing; orbit needs a
    // gesture, the rest need a selection/target, unknown ids stay generic.
    assert_eq!(
        ribbon_command_unsupported_reason("view.orbit"),
        "ribbon.command_needs_gesture"
    );
    for id in [
        "view.standard",
        "layer.toggle",
        "layout.switch",
        "backend.switch",
    ] {
        assert_eq!(
            ribbon_command_unsupported_reason(id),
            "ribbon.command_needs_target",
            "id {id}"
        );
    }
    assert_eq!(
        ribbon_command_unsupported_reason("totally.unknown"),
        "ribbon.command_unsupported"
    );
    // The reasons resolve to real, localized text in both catalogs (never a
    // bracketed missing-key fallback).
    for messages in [
        MessageSource::for_locale(Locale::ZhCn),
        MessageSource::for_locale(Locale::En),
    ] {
        for reason in [
            "ribbon.command_unsupported",
            "ribbon.command_needs_target",
            "ribbon.command_needs_gesture",
        ] {
            let text = messages.text(reason, &[("command", "view.orbit")]);
            assert!(!text.contains(reason), "missing catalog text: {reason}");
            assert!(text.contains("view.orbit"), "placeholder not substituted");
        }
    }
}

#[test]
fn ribbon_reset_and_unsupported_actions_are_wired_in_the_adapter() {
    // `src/tests.rs` is deliberately GPU-free (the offscreen platform installs
    // once per test binary), so the live dispatch test lives in
    // `tests/interaction_gating.rs` style. Here we guard the wiring the
    // dispatch depends on: `ResetView` must emit the same payload-free
    // `CommandId::ResetView` the command layer handles, and the unsupported arm
    // must consult the reason helper instead of hardcoding one message.
    let source = include_str!("adapter.rs");
    assert!(source.contains("RibbonCommandAction::ResetView =>"));
    assert!(source.contains("CommandId::ResetView,"));
    assert!(source.contains("ribbon_command_unsupported_reason(id)"));
    // The bare id must never fall back to a generic "unsupported" message: the
    // reason helper is what selects the concrete text.
    assert!(!source.contains(".text(\"ribbon.command_unsupported\", &[(\"command\", id)])"));
}

#[test]
fn interaction_config_gates_pointer_and_touch_input() {
    use cad_app::viewer_config::{ViewerConfig, ViewerConfigStore};

    // Defaults: both input families are enabled.
    let mut store = ViewerConfigStore::default();
    assert!(input_enabled(&store, InputKind::Pointer));
    assert!(input_enabled(&store, InputKind::Touch));

    // Pointer and touch are independent flags: disabling one never disables the
    // other, so a keyboard/mouse host keeps working on a touchless surface and
    // vice versa.
    let mut config = ViewerConfig::default();
    config.interaction.pointer = false;
    store.set_config(config).unwrap();
    assert!(!input_enabled(&store, InputKind::Pointer));
    assert!(input_enabled(&store, InputKind::Touch));

    let mut config = ViewerConfig::default();
    config.interaction.touch = false;
    store.set_config(config).unwrap();
    assert!(input_enabled(&store, InputKind::Pointer));
    assert!(!input_enabled(&store, InputKind::Touch));

    // The reader observes the live store, so re-enabling is immediate.
    store.set_config(ViewerConfig::default()).unwrap();
    assert!(input_enabled(&store, InputKind::Pointer));
    assert!(input_enabled(&store, InputKind::Touch));
}

#[test]
fn keyboard_aliases_expand_only_when_shortcuts_are_enabled() {
    use crate::command_line::canonical_command;

    // Enabled: aliases expand to the exact command names.
    for (alias, exact) in [
        ("L", "LINE"),
        ("C", "CIRCLE"),
        ("M", "MOVE"),
        ("TR", "TRIM"),
        ("ESC", "CANCEL"),
    ] {
        assert_eq!(canonical_command(alias, true), exact);
    }

    // Disabled: a bare alias stays itself and is reported unknown, never a
    // silent CAD edit.
    for alias in ["L", "C", "M", "TR", "ESC"] {
        assert_eq!(canonical_command(alias, false), alias);
    }

    // Exact names keep working regardless of the shortcut setting.
    for exact in ["LINE", "CIRCLE", "MOVE", "TRIM", "CANCEL", "ZOOM EXTENTS"] {
        assert_eq!(canonical_command(exact, false), exact);
        assert_eq!(canonical_command(exact, true), exact);
    }
}

#[test]
fn layout_panel_state_surfaces_scale_only_when_the_host_pushes_it() {
    use cad_domain::LayoutId;
    use cad_representation::{LayoutDescriptor, SpaceSelection};

    let descriptors = vec![LayoutDescriptor {
        id: LayoutId(1),
        name: "Sheet A".into(),
        supported: true,
        reason: String::new(),
        viewport_count: 1,
    }];
    let mut state =
        LayoutPanelState::from_descriptors(&descriptors, SpaceSelection::Model, "无布局");
    // The descriptors carry no scale and no scale command exists, so the panel
    // starts in an explicit read-only state.
    assert_eq!(state.rows[0].viewport_scale, None);
    assert!(!state.scale_control_available);

    // A real scale is attached by index; an out-of-range index inserts nothing.
    assert!(state.set_viewport_scale(0, "1:100"));
    assert_eq!(state.rows[0].viewport_scale.as_deref(), Some("1:100"));
    assert!(!state.set_viewport_scale(9, "1:50"));
    assert_eq!(state.rows.len(), 1);
}

#[test]
fn standard_view_for_camera_matches_named_views_and_refuses_free_orbit() {
    fn camera_for(view: cad_app::StandardView) -> cad_app::Camera {
        let offset = view.eye_offset();
        let projection = if view.is_plan() {
            cad_app::Projection::Orthographic { scale: 1.0 }
        } else {
            cad_app::Projection::Perspective {
                vertical_fov_radians: std::f64::consts::FRAC_PI_4,
            }
        };
        cad_app::Camera {
            eye: Point3 {
                x: offset.x * 10.0,
                y: offset.y * 10.0,
                z: offset.z * 10.0,
            },
            target: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            up: view.up_hint(),
            projection,
        }
    }

    // The default 2D plan is the Top standard view.
    assert_eq!(
        standard_view_for_camera(&cad_app::Camera::top_view_2d()),
        Some(cad_app::StandardView::Top)
    );
    for view in cad_app::StandardView::ALL {
        assert_eq!(
            standard_view_for_camera(&camera_for(view)),
            Some(view),
            "view {view:?} must round-trip"
        );
    }

    // An arbitrary orbit orientation is not a named standard view.
    let orbit = cad_app::Camera {
        eye: Point3 {
            x: 3.0,
            y: 4.0,
            z: 5.0,
        },
        target: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        up: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        projection: cad_app::Projection::Perspective {
            vertical_fov_radians: std::f64::consts::FRAC_PI_4,
        },
    };
    assert_eq!(standard_view_for_camera(&orbit), None);
}

#[test]
fn view3d_panel_state_derives_mode_projection_standard_view_and_orbit() {
    let zh = MessageSource::for_locale(Locale::ZhCn);

    let three_d = ViewStateUi {
        is_3d: true,
        perspective: true,
        standard_view: Some(cad_app::StandardView::Isometric),
    };
    let panel = View3dPanelState::from_view_state(&three_d, &zh);
    assert!(panel.is_3d && panel.perspective);
    assert_eq!(
        panel.standard_view_index,
        Some(cad_app::StandardView::Isometric.index() as i32)
    );
    assert!(panel.orbit_available);
    assert_eq!(panel.orbit_status, zh.text("view.orbit.drag", &[]));
    assert_eq!(
        panel.standard_view_labels.len(),
        cad_app::StandardView::ALL.len()
    );

    // The default 2D view has no standard-view row and states why orbit is off.
    let two_d = ViewStateUi::default();
    let panel = View3dPanelState::from_view_state(&two_d, &zh);
    assert!(!panel.is_3d && !panel.orbit_available);
    assert_eq!(panel.standard_view_index, None);
    assert_eq!(panel.orbit_status, zh.text("view.orbit.needs_3d", &[]));
    // 3D zoom-to-fit is not implemented; the reason is explicit, never blank.
    assert_eq!(panel.fit_reason, zh.text("view.fit3d_unavailable", &[]));
}

#[test]
fn resources_panel_state_is_empty_until_a_host_pushes_a_section() {
    let zh = MessageSource::for_locale(Locale::ZhCn);
    let empty = ResourcesPanelState::from_sections(&ResourceSections::default(), &zh);
    assert!(empty.is_empty());
    assert_eq!(empty.source_count, 0);
    assert!(!empty.empty_label.is_empty());
}

#[test]
fn resources_panel_state_projects_real_sources_and_names_the_image_limit() {
    let zh = MessageSource::for_locale(Locale::ZhCn);
    let sections = ResourceSections {
        fonts: Some(FontResourceSummary {
            catalog_entries: 12,
            requested: 3,
            planned: 2,
            registered: 2,
            failed: vec!["a.ttf: bad".into()],
            unresolved: vec!["SHXNAME".into()],
            default_face: Some("osifont".into()),
        }),
        import: Some(ImportResourceSummary {
            identity: "plan.dwg".into(),
            dwg_version: "AC1027".into(),
            completeness: ResourceCompleteness::Partial(2),
            diagnostics: Some(4),
            parse_ms: Some(37),
        }),
        proxy: Some(ProxyResourceSummary {
            decoded_records: 5,
            unsupported: vec![ProxyUnsupportedRow {
                record_type: 99,
                reason: "unknown opcode".into(),
                bytes: 8,
            }],
        }),
        references: Some(ReferencesResourceSummary {
            keys: vec!["xref-a".into()],
        }),
        images_modeled: false,
    };
    let state = ResourcesPanelState::from_sections(&sections, &zh);
    assert_eq!(state.source_count, 4);
    assert!(!state.is_empty());
    // The first row carries the section header; later rows do not repeat it.
    assert_eq!(state.rows[0].section, zh.text("resources.fonts", &[]));
    // The real proxy record is listed with its own reason and byte size.
    assert!(state.rows.iter().any(|row| {
        row.label == zh.text("resources.proxy.record_type", &[("type", "99")])
            && row.value.contains("unknown opcode")
    }));
    // Images are not modelled: stated explicitly, not left as silent absence.
    assert!(state.rows.iter().any(|row| {
        row.section == zh.text("resources.images", &[])
            && row.value == zh.text("resources.images_unsupported", &[])
    }));
}

#[test]
fn resources_panel_state_reports_clean_sources_and_skips_the_image_note_when_modelled() {
    let en = MessageSource::for_locale(Locale::En);
    let clean = ResourceSections {
        proxy: Some(ProxyResourceSummary {
            decoded_records: 0,
            unsupported: Vec::new(),
        }),
        references: Some(ReferencesResourceSummary { keys: Vec::new() }),
        // Images modelled: no explicit limitation row is added.
        images_modeled: true,
        ..ResourceSections::default()
    };
    let state = ResourcesPanelState::from_sections(&clean, &en);
    assert!(state
        .rows
        .iter()
        .any(|row| row.value == en.text("resources.proxy.none", &[])));
    assert!(state
        .rows
        .iter()
        .any(|row| row.value == en.text("resources.references.none", &[])));
    assert!(!state
        .rows
        .iter()
        .any(|row| row.section == en.text("resources.images", &[])));
}

#[test]
fn shell_exposes_the_layout_scale_and_resource_and_3d_drawers() {
    for marker in [
        "layout-scale-label",
        "layout-scale-unavailable-label",
        "layout-scale-control-reason",
        "layout-scale-control-available",
        "resources-open",
        "resources-rows",
        "resources-empty-label",
        "view3d-open",
        "view-standard-index",
        "view-orbit-status",
        "view-fit-reason",
        "CadResourcesDrawer",
        "CadView3dDrawer",
    ] {
        assert!(
            UI_DEFINITION.contains(marker),
            "shell must expose drawer marker {marker}"
        );
    }
    let ribbon = include_str!("../ui/ribbon.slint");
    assert!(ribbon.contains("root.action(13)"));
    assert!(ribbon.contains("root.action(14)"));
    assert!(ribbon.contains("resources-label"));
    assert!(ribbon.contains("view3d-label"));
}

#[test]
fn new_drawer_keys_resolve_in_both_catalogs() {
    for messages in [
        MessageSource::for_locale(Locale::ZhCn),
        MessageSource::for_locale(Locale::En),
    ] {
        for key in [
            "layout.scale_label",
            "layout.scale_unavailable",
            "layout.scale_control_label",
            "layout.scale_control_unavailable",
            "view.standard_label",
            "view.orbit_label",
            "view.orbit.drag",
            "view.orbit.needs_3d",
            "view.fit_label",
            "view.fit3d_unavailable",
            "view3d.title",
            "resources.title",
            "resources.close",
            "resources.empty",
            "resources.fonts",
            "resources.fonts.default_face",
            "resources.import",
            "resources.completeness.partial",
            "resources.proxy",
            "resources.proxy.record",
            "resources.proxy.none",
            "resources.references",
            "resources.images",
            "resources.images_unsupported",
        ] {
            let text = messages.text(
                key,
                &[
                    ("type", "99"),
                    ("reason", "x"),
                    ("bytes", "8"),
                    ("count", "1"),
                ],
            );
            assert!(
                !text.contains(key),
                "missing catalog text for {key}: {text}"
            );
        }
    }
}
