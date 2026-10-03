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

#[test]
fn measurement_save_affordance_requires_a_confirmed_record() {
    // A running preview is not a confirmed result: save stays disabled.
    let tool = cad_app::MeasurementTool::new(MeasurementToolKind::Distance);
    let mut state = MeasurementUiState::from_preview(Some(&tool.preview()), "m");
    assert!(!state.can_save_annotation);
    // A confirmed record enables it explicitly.
    state.set_can_save_annotation(true);
    assert!(state.can_save_annotation);
    // Idle with a retained record still offers the save.
    let mut idle = MeasurementUiState::from_preview(None, "m");
    assert!(!idle.can_save_annotation);
    idle.set_can_save_annotation(true);
    assert!(idle.can_save_annotation);
    // Default is off.
    assert!(!MeasurementUiState::default().can_save_annotation);
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
fn shell_exposes_the_save_and_mode_switch_affordances() {
    // F06/F07: save-as-annotation button + callback, gated by the record flag.
    assert!(UI_DEFINITION.contains("measurement-can-save-annotation"));
    assert!(UI_DEFINITION.contains("save-measurement-requested"));
    assert!(UI_DEFINITION.contains("measure-save-label"));
    // U02: a real mode switch entry showing the catalog mode label.
    assert!(UI_DEFINITION.contains("mode-toggled"));
    assert!(UI_DEFINITION.contains("mode-label"));
    assert!(UI_DEFINITION.contains("work-mode"));
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
        "get_annotation_tool_active",
        "invoke_confirm_annotation_requested",
        "invoke_cancel_annotation_requested",
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
fn draw_overlay_preview_uses_a_circle_for_circle_and_a_band_for_line() {
    use cad_domain::Point3;
    let p = |x: f64, y: f64| Point3 { x, y, z: 0.0 };

    let mut circle = cad_app::DrawTool::new(cad_app::DrawToolKind::Circle, 0);
    circle.push_point(p(1.0, 1.0)).unwrap();
    circle.push_point(p(4.0, 5.0)).unwrap();
    let overlay = draw_overlay_preview(&circle.preview());
    assert_eq!(overlay.kind, cad_app::AnnotationToolKind::Ellipse);
    // The radius is applied to both axes, so the preview is a circle.
    let dx = overlay.points[1].x - overlay.points[0].x;
    let dy = overlay.points[1].y - overlay.points[0].y;
    assert!((dx - dy).abs() < 1e-9);

    // A line preview is a rubber band anchored at the first captured point.
    let mut line = cad_app::DrawTool::new(cad_app::DrawToolKind::Line, 0);
    line.push_point(p(0.0, 0.0)).unwrap();
    line.set_cursor(Some(p(2.0, 2.0)));
    let overlay = draw_overlay_preview(&line.preview());
    assert_eq!(overlay.kind, cad_app::AnnotationToolKind::Freehand);
    assert_eq!(overlay.points, vec![p(0.0, 0.0)]);
    assert_eq!(overlay.cursor, Some(p(2.0, 2.0)));
}

#[test]
fn viewer_presentation_maps_separate_panels_overlays_and_features() {
    use cad_app::viewer_config::{Preset, UiPresentationModel, ViewerConfig};
    let mut config = ViewerConfig::default();
    config.ui.components.layer_panel.visible = false;
    config.ui.components.properties_panel.initially_open = false;
    config.view.overlays.grid = false;
    config.features.annotations.delete = false;
    config.interaction.touch = false;
    let p = UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false);
    assert!(!p.layer_panel && p.properties_panel);
    assert!(!p.layer_panel_initially_open);
    assert!(!p.properties_panel_initially_open);
    assert!(!p.overlays.grid && p.overlays.axes);
    assert!(!p.features.annotation_delete && p.features.measure);
    assert!(!p.touch && p.pointer);
    assert!(!p.command_visibility["annotation.delete"]);
    assert!(p.command_visibility["annotation.text"]);
    // Canvas-only forces every application-UI entry off while overlays stay
    // controlled by `view` independently.
    config.ui.preset = Preset::CanvasOnly;
    let p = UiPresentationModel::resolve(&config, [1280.0, 800.0], [0.0; 4], false);
    assert!(!p.application_ui && !p.ribbon && !p.panels());
    assert!(p.overlays.annotations);
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
