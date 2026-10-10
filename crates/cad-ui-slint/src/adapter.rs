//! adapter module.

use super::*;
use cad_render_wgpu::{wgpu, GpuSelection};

/// Build the command a shell callback emits for the configured document.
fn command_for(
    id: CommandId,
    document: &DocumentId,
    viewport: ViewportId,
    payload: CommandPayload,
) -> Command {
    Command {
        schema_version: 1,
        id,
        document: *document,
        viewport,
        payload,
    }
}

/// The shell action a configured `ui.components.ribbon` command id maps to.
///
/// Kept as a pure value so the mapping is unit-testable without a live window.
/// `Unsupported` is explicit: a command that needs a target the UI does not have
/// (a standard view, a layer row, a backend choice) reports an error rather than
/// silently doing nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RibbonCommandAction {
    Open,
    Undo,
    Redo,
    Fit,
    Pan,
    /// Restore the view to the top-down 2D camera (payload-free `ResetView`).
    ResetView,
    ToggleProjection,
    Switch2d3d,
    /// A measurement algorithm, identified by its machine key.
    MeasureKind(&'static str),
    ConfirmMeasurement,
    CancelMeasurement,
    RestoreLayers,
    /// A draw/edit tool, identified by its machine key.
    BeginDraw(&'static str),
    ToggleMode,
    OpenDiagnostics,
    /// No standalone action exists for this id (still an explicit error).
    Unsupported,
}

/// Catalog key explaining why an id has no standalone ribbon action.
///
/// Every id in `COMMAND_IDS` must resolve: the `_` arm is the honest fallback
/// for an id the build does not know at all. The targeted ids keep
/// [`RibbonCommandAction::Unsupported`] because the ribbon button cannot supply
/// the selection/target (or, for orbit, the drag gesture) the command needs;
/// the message says which kind of input is missing rather than "unsupported".
pub fn ribbon_command_unsupported_reason(id: &str) -> &'static str {
    match id {
        "view.orbit" => "ribbon.command_needs_gesture",
        "view.standard" | "backend.switch" | "layer.toggle" | "layout.switch" => {
            "ribbon.command_needs_target"
        }
        _ => "ribbon.command_unsupported",
    }
}

/// Map a configured command id to its shell action.
///
/// Every id in [`cad_app::viewer_config::COMMAND_IDS`] is considered; ids whose
/// behaviour needs a selection/target the ribbon cannot supply resolve to
/// [`RibbonCommandAction::Unsupported`] so the shell reports rather than fakes.
pub fn ribbon_command_action(id: &str) -> RibbonCommandAction {
    use RibbonCommandAction::*;
    match id {
        "file.open" => Open,
        "edit.undo" => Undo,
        "edit.redo" => Redo,
        "view.fit" => Fit,
        "view.pan" => Pan,
        "view.reset" => ResetView,
        "view.projection" => ToggleProjection,
        "view.switch2d3d" => Switch2d3d,
        "measure.distance" => MeasureKind("distance"),
        "measure.polyline" => MeasureKind("polyline"),
        "measure.angle" => MeasureKind("angle"),
        "measure.area" => MeasureKind("area"),
        "measure.confirm" => ConfirmMeasurement,
        "measure.cancel" => CancelMeasurement,
        "layer.restore" => RestoreLayers,
        "draw.line" => BeginDraw("line"),
        "draw.circle" => BeginDraw("circle"),
        "draw.move" => BeginDraw("move"),
        "draw.trim" => BeginDraw("trim"),
        "mode.toggle" => ToggleMode,
        "diagnostics.open" => OpenDiagnostics,
        // Genuinely ambiguous or unknown: a standard view, an orbit gesture, a
        // layer row, an annotation row, a layout id or a backend choice. The
        // shell reports the concrete missing input via
        // [`ribbon_command_unsupported_reason`] rather than faking a target.
        _ => Unsupported,
    }
}

/// Push the active draw state into the shell and the host preview sink.
///
/// Called after every capture/cursor/start/cancel so the step panel, the
/// confirm affordance and the CAD overlay always agree with the pure state
/// machine. A `None` sink simply means no live overlay; it is never an error.
fn publish_draw(
    report: &Weak<YacrWindow>,
    tool: &Rc<RefCell<Option<cad_app::DrawTool>>>,
    preview_sink: &Rc<RefCell<Option<Rc<dyn DrawPreviewSink>>>>,
) {
    let preview = tool.borrow().as_ref().map(|tool| tool.preview());
    let state = DrawUiState::from_preview(preview.as_ref());
    if let Some(ui) = report.upgrade() {
        ui.set_draw_tool_active(state.active);
        ui.set_draw_can_confirm(state.can_confirm);
        ui.set_draw_step_label(state.step_label.into());
    }
    if let Some(sink) = preview_sink.borrow().as_ref() {
        sink.set_preview(preview);
    }
}

/// Radians of orbit per logical pixel of drag (a UI navigation constant).
pub const ORBIT_RADIANS_PER_PIXEL: f64 = 0.008;

/// Convert a 3D orbit drag delta (logical pixels) into `(yaw, pitch)` radians.
///
/// Horizontal drag yaws about world `+Z`; vertical drag pitches about the view
/// right axis. The sign is chosen so the scene follows the pointer. Non-finite
/// input yields `(0, 0)` rather than poisoning a camera with NaNs.
pub fn orbit_delta(from: [f64; 2], to: [f64; 2]) -> (f64, f64) {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    if !dx.is_finite() || !dy.is_finite() {
        return (0.0, 0.0);
    }
    (-dx * ORBIT_RADIANS_PER_PIXEL, dy * ORBIT_RADIANS_PER_PIXEL)
}

/// Which shell event family an interaction flag gates.
///
/// `Pointer` covers mouse, trackpad and the pointer events Android synthesizes
/// from touch, all of which reach Slint's `pointer-input` callback. `Touch`
/// covers the browser host's native touch router, which enters through its own
/// wasm exports rather than this adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Pointer,
    Touch,
}

/// Whether `kind` input is enabled by the effective interaction config.
///
/// The store is the single source of truth and is read at event time, so a host
/// flip of `interaction.pointer` / `interaction.touch` takes effect on the very
/// next event without reinstalling a callback. This is a pure function so the
/// "disabled means no dispatch" rule is unit-testable without a live window.
pub fn input_enabled(store: &cad_app::viewer_config::ViewerConfigStore, kind: InputKind) -> bool {
    let interaction = &store.effective().interaction;
    match kind {
        InputKind::Pointer => interaction.pointer,
        InputKind::Touch => interaction.touch,
    }
}

/// JSON patch that flips exactly one `view.overlays.*` flag.
///
/// The shell status-bar toggle sends a short key; this maps it to the camelCase
/// config key and builds `{"view":{"overlays":{"<key>":<bool>}}}`. An unknown
/// key returns `None`, so a malformed callback can never write an invalid
/// config or bypass the store with a raw Slint property write. Pure and
/// unit-testable without a live window.
pub fn overlay_toggle_patch(key: &str, value: bool) -> Option<String> {
    let field = match key {
        "axes" => "axes",
        "grid" => "grid",
        "selectionHighlight" => "selectionHighlight",
        "snapHints" => "snapHints",
        _ => return None,
    };
    Some(format!(
        r#"{{"view":{{"overlays":{{"{field}":{value}}}}}}}"#
    ))
}

impl UiAdapter {
    /// Build the shell and connect its callbacks to `sink`.
    pub fn new<S: UiCommandSink>(
        configuration: UiConfiguration,
        sink: S,
        work_mode: bool,
    ) -> CadResult<Self> {
        let ui = YacrWindow::new().map_err(|e| CadError::Invariant(format!("slint: {e}")))?;
        // Kept for closures that need to report state directly (for example the
        // unwired canvas pick), since the command sink has no "set status" verb.
        let ui_weak = ui.as_weak();
        ui.set_application_title(configuration.application_title.clone().into());
        ui.window().set_size(slint::LogicalSize::new(
            configuration.logical_size[0].max(1.0) as f32,
            configuration.logical_size[1].max(1.0) as f32,
        ));
        // The configured locale selects the catalog; no Rust string literals for
        // user-facing text live here (N01). Fallbacks are recorded, never silent.
        let messages = MessageSource::from_request(&configuration.locale);
        if let Some(reason) = messages.resolution().fallback {
            log::info!(
                "locale {:?} resolved to {} ({:?})",
                messages.resolution().requested,
                messages.locale().tag(),
                reason
            );
        }
        apply_chrome(&ui, &messages, work_mode);
        // Consume the compact config and the initial viewport: the responsive
        // geometry is derived once here and on every resize (audit U01).
        apply_responsive(&ui, configuration.logical_size, configuration.compact);
        ui.set_status_label(messages.text("status.scaffold", &[]).into());
        ui.set_work_mode(work_mode);
        ui.set_config_revision(0);
        ui.set_config_preset("full".into());
        ui.set_config_effective_json(
            cad_app::viewer_config::ViewerConfigStore::default()
                .effective_json()
                .into(),
        );

        let work_mode: Rc<Cell<bool>> = Rc::new(Cell::new(work_mode));

        let document = configuration.document;
        let viewport = configuration.viewport;
        let shared: Rc<RefCell<S>> = Rc::new(RefCell::new(sink));
        let view_input: Rc<RefCell<Option<Rc<dyn ViewInput>>>> = Rc::new(RefCell::new(None));
        let pick_mapper: Rc<RefCell<Option<Rc<dyn CanvasPickMapper>>>> =
            Rc::new(RefCell::new(None));
        let layout_switch: Rc<RefCell<Option<Box<dyn LayoutSwitchSink>>>> =
            Rc::new(RefCell::new(None));
        let draw_command_sink: Rc<RefCell<Option<Box<dyn DrawCommandSink>>>> =
            Rc::new(RefCell::new(None));
        let draw_preview_sink: Rc<RefCell<Option<Rc<dyn DrawPreviewSink>>>> =
            Rc::new(RefCell::new(None));
        let draw_tool: Rc<RefCell<Option<cad_app::DrawTool>>> = Rc::new(RefCell::new(None));
        let selected_kind: Rc<Cell<MeasurementToolKind>> =
            Rc::new(Cell::new(MeasurementToolKind::Distance));
        let measurement_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let layer_order: Rc<RefCell<Vec<LayerId>>> = Rc::new(RefCell::new(Vec::new()));
        let layout_order: Rc<RefCell<Vec<cad_domain::LayoutId>>> =
            Rc::new(RefCell::new(Vec::new()));
        let messages_slot: Rc<RefCell<MessageSource>> = Rc::new(RefCell::new(messages.clone()));
        let layer_override_count: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let selection_count: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let view_3d: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let view_state: Rc<Cell<ViewStateUi>> = Rc::new(Cell::new(ViewStateUi::default()));
        let resources_sections: Rc<RefCell<Option<ResourceSections>>> = Rc::new(RefCell::new(None));
        let orbit_last: Rc<Cell<Option<[f64; 2]>>> = Rc::new(Cell::new(None));
        let import_snapshot: Rc<RefCell<Option<cad_app::ImportProgressSnapshot>>> =
            Rc::new(RefCell::new(None));

        // Hoisted above every input closure so each event reads the live
        // effective interaction config instead of a value captured at build
        // time. The command line and the pointer/scroll/pick gates share it.
        let viewer_config: Rc<RefCell<cad_app::viewer_config::ViewerConfigStore>> = Rc::new(
            RefCell::new(cad_app::viewer_config::ViewerConfigStore::default()),
        );

        crate::command_line::connect(&ui, messages_slot.clone(), viewer_config.clone());
        crate::command_completion_ui::connect(&ui);
        crate::layer_search::connect(&ui);

        {
            // Fixed AutoCAD-default keymap: keys whose backing capability does
            // not exist yet report an explicit localized "unsupported" message
            // and expand the command row. Limitation: that feedback renders in
            // the command/status chrome, which clean screen (Ctrl+0) hides, so
            // under clean screen the message is set but not visible; this is
            // documented, not presented as visible feedback.
            let weak = ui_weak.clone();
            let messages = messages_slot.clone();
            ui.on_shortcut_unsupported(move |key| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                ui.set_status_label(
                    messages
                        .borrow()
                        .text("shortcut.unsupported", &[("key", key.as_str())])
                        .into(),
                );
                ui.set_command_expanded(true);
            });
        }

        {
            // Desktop status-bar overlay toggles. The click must be a real config
            // change: it merges one `view.overlays.*` flag into the store, then
            // re-derives and re-pushes the presentation, so a host sees the new
            // `effective_config().view.overlays` on its next state funnel and the
            // canvas overlay is rebuilt rather than shadowed by a Slint write.
            let weak = ui_weak.clone();
            let config = viewer_config.clone();
            let messages = messages_slot.clone();
            ui.on_overlay_toggled(move |key, value| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let Some(patch) = overlay_toggle_patch(key.as_str(), value) else {
                    log::warn!("ignoring unknown overlay toggle key {:?}", key.as_str());
                    return;
                };
                let applied = config.borrow_mut().update_config_json(&patch);
                if applied.is_err() {
                    // A refused patch leaves the store and revision untouched;
                    // the toggle simply stays at the last effective value.
                    return;
                }
                apply_ribbon_config(&ui, config.borrow().effective(), &messages.borrow());
                let size = ui.window().size().to_logical(ui.window().scale_factor());
                apply_viewer_presentation_with(
                    &ui,
                    config.borrow().effective(),
                    [size.width as f64, size.height as f64],
                    Some(&config.borrow()),
                );
            });
        }

        {
            // Configured ribbon buttons dispatch to the exact callbacks the
            // built-in chrome uses; the id mapping is pure and unit-tested.
            // `ResetView` has no shipped button of its own, so it emits the
            // payload-free command directly through the shared sink instead of
            // routing through a redundant shell callback.
            let weak = ui_weak.clone();
            let messages = messages_slot.clone();
            let s = shared.clone();
            let doc = document;
            ui.on_ribbon_command(move |id| {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                let id = id.as_str();
                match ribbon_command_action(id) {
                    RibbonCommandAction::Open => ui.invoke_open_requested(),
                    RibbonCommandAction::Undo => ui.invoke_undo_requested(),
                    RibbonCommandAction::Redo => ui.invoke_redo_requested(),
                    RibbonCommandAction::Fit => ui.invoke_fit_requested(),
                    RibbonCommandAction::Pan => ui.invoke_pan_requested(),
                    RibbonCommandAction::ResetView => {
                        let _ = s.borrow_mut().send(command_for(
                            CommandId::ResetView,
                            &doc,
                            viewport,
                            CommandPayload::None,
                        ));
                    }
                    RibbonCommandAction::ToggleProjection => {
                        ui.invoke_toggle_projection_requested()
                    }
                    RibbonCommandAction::Switch2d3d => ui.invoke_toggle_view_mode_requested(),
                    RibbonCommandAction::MeasureKind(key) => {
                        if let Some(kind) = cad_app::MeasurementToolKind::from_key(key) {
                            let label = messages
                                .borrow()
                                .text(&status::measurement_kind_key(kind.key()), &[]);
                            ui.invoke_measure_kind_selected(label.into());
                        }
                    }
                    RibbonCommandAction::ConfirmMeasurement => {
                        ui.invoke_confirm_measurement_requested()
                    }
                    RibbonCommandAction::CancelMeasurement => {
                        ui.invoke_cancel_measurement_requested()
                    }
                    RibbonCommandAction::RestoreLayers => ui.invoke_restore_layers_requested(),
                    RibbonCommandAction::BeginDraw(key) => ui.invoke_begin_draw_tool(key.into()),
                    RibbonCommandAction::ToggleMode => ui.invoke_mode_toggled(),
                    RibbonCommandAction::OpenDiagnostics => {
                        ui.set_diagnostics_open(true);
                        ui.invoke_diagnostics_requested();
                    }
                    RibbonCommandAction::Unsupported => {
                        ui.set_status_label(
                            messages
                                .borrow()
                                .text(ribbon_command_unsupported_reason(id), &[("command", id)])
                                .into(),
                        );
                    }
                }
            });
        }

        {
            let s = shared.clone();
            let doc = document;
            ui.on_open_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::OpenDrawing,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // New blank drawing is host-owned exactly like open: the shell only
            // emits the request; a host without the capability leaves `can-new`
            // false and the shell reports unsupported before reaching here.
            let s = shared.clone();
            let doc = document;
            ui.on_new_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::NewDrawing,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Cancel a running background open (F01). `CancelLoading` is routed
            // by the host to `HostController::cancel_async_open`; the panel only
            // enables this while a cancellable job is running.
            let s = shared.clone();
            let doc = document;
            ui.on_cancel_open_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::CancelLoading,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_fit_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::FitDrawing,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_zoom_requested(move |factor| {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Zoom,
                    &doc,
                    viewport,
                    CommandPayload::Points(vec![cad_domain::Point3 {
                        x: factor as f64,
                        y: 0.0,
                        z: 0.0,
                    }]),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            let weak = ui_weak.clone();
            let drawing = draw_tool.clone();
            let preview = draw_preview_sink.clone();
            ui.on_pan_requested(move || {
                *drawing.borrow_mut() = None;
                publish_draw(&weak, &drawing, &preview);
                let _ = s.borrow_mut().send(command_for(
                    CommandId::CancelMeasurement,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
                if let Some(ui) = weak.upgrade() {
                    ui.set_pan_active(!ui.get_pan_active());
                }
            });
        }
        {
            // The Measure button starts (or restarts) the selected algorithm
            // instead of sending a payload-free guess (audit U04/F06).
            let s = shared.clone();
            let doc = document;
            let kind = selected_kind.clone();
            let weak = ui_weak.clone();
            ui.on_measure_requested(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_pan_active(false);
                }
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Measure,
                    &doc,
                    viewport,
                    CommandPayload::MeasureTool(kind.get()),
                ));
            });
        }
        {
            // Selecting a kind immediately starts that tool; this is the explicit
            // user choice of algorithm, not a default. The label is localized, so
            // it is mapped back through the same catalog-built ordering rather
            // than the Chinese-only `from_label`.
            let s = shared.clone();
            let doc = document;
            let kind_slot = selected_kind.clone();
            let messages = messages_slot.clone();
            let weak = ui_weak.clone();
            ui.on_measure_kind_selected(move |name| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_pan_active(false);
                }
                let messages = messages.borrow().clone();
                let Some(kind) = status::measurement_kind_from_label(&messages, name.as_str())
                else {
                    return;
                };
                kind_slot.set(kind);
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Measure,
                    &doc,
                    viewport,
                    CommandPayload::MeasureTool(kind),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_confirm_measurement_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ConfirmMeasurement,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_cancel_measurement_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::CancelMeasurement,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Persisted measurements are transient; there is no annotation
            // sidecar anymore, so no save affordance is wired.
        }
        {
            // Viewer/Work switch (audit U02). Emits a real `SetMode` command; the
            // shell reflects the target immediately and the host's authoritative
            // push (`UiHandle::set_mode`) corrects any discrepancy.
            let s = shared.clone();
            let doc = document;
            let work = work_mode.clone();
            let messages = messages_slot.clone();
            let report = ui_weak.clone();
            let draw = draw_tool.clone();
            let draw_preview = draw_preview_sink.clone();
            ui.on_mode_toggled(move || {
                let target = if work.get() {
                    cad_app::AppMode::Viewer
                } else {
                    cad_app::AppMode::Work
                };
                let is_work = target == cad_app::AppMode::Work;
                work.set(is_work);
                // A mode switch cancels an unconfirmed tool, never commits it
                // (mirrors `SessionState::switch_mode`).
                *draw.borrow_mut() = None;
                publish_draw(&report, &draw, &draw_preview);
                if let Some(ui) = report.upgrade() {
                    ui.set_work_mode(is_work);
                    ui.set_mode_label(crate::status::mode_label(&messages.borrow(), target).into());
                }
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SetMode,
                    &doc,
                    viewport,
                    CommandPayload::Mode(target),
                ));
            });
        }
        {
            // Layer visibility toggle: the panel sends a row index; the adapter
            // resolves it to the exact LayerId from the pushed model order.
            let s = shared.clone();
            let doc = document;
            let order = layer_order.clone();
            ui.on_layer_visibility_toggled(move |index, visible| {
                let layer = usize::try_from(index)
                    .ok()
                    .and_then(|i| order.borrow().get(i).copied());
                if let Some(layer) = layer {
                    let _ = s.borrow_mut().send(command_for(
                        CommandId::ToggleLayer,
                        &doc,
                        viewport,
                        CommandPayload::Layer(layer, visible),
                    ));
                }
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_restore_layers_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::RestoreLayers,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // One validated batch covers the full authoritative order, not just
            // the visible search results. Application publishes it atomically.
            let s = shared.clone();
            let doc = document;
            let order = layer_order.clone();
            let weak = ui.as_weak();
            let messages = messages_slot.clone();
            ui.on_layers_visibility_requested(move |visible| {
                let changes = order.borrow().iter().map(|id| (*id, visible)).collect();
                if let Err(error) = s.borrow_mut().send(command_for(
                    CommandId::SetLayerVisibilities,
                    &doc,
                    viewport,
                    CommandPayload::LayerVisibilities(changes),
                )) {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_status_label(
                            messages
                                .borrow()
                                .text("layers.operation_failed", &[("reason", &error.to_string())])
                                .into(),
                        );
                        ui.set_command_expanded(true);
                    }
                }
            });
        }
        {
            // Clearing the selection is a real, read-only Select command with an
            // empty payload; the application records it and mutates nothing.
            let s = shared.clone();
            let doc = document;
            ui.on_clear_selection_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Select,
                    &doc,
                    viewport,
                    CommandPayload::Selection(Vec::new()),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            let weak = ui.as_weak();
            let messages = messages_slot.clone();
            ui.on_select_all_requested(move || {
                if let Err(error) = s.borrow_mut().send(command_for(
                    CommandId::SelectAll,
                    &doc,
                    viewport,
                    CommandPayload::None,
                )) {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_status_label(
                            messages
                                .borrow()
                                .text(
                                    "selection.operation_failed",
                                    &[("reason", &error.to_string())],
                                )
                                .into(),
                        );
                        ui.set_command_expanded(true);
                    }
                }
            });
        }
        {
            // A canvas click becomes one `Measure`/`Points` command when the host
            // installed a world mapper. Without one the click cannot be turned
            // into a real point, so it is reported (not silently swallowed).
            let s = shared.clone();
            let doc = document;
            let mapper = pick_mapper.clone();
            let report = ui_weak.clone();
            let active = measurement_active.clone();
            let drawing = draw_tool.clone();
            let draw_preview = draw_preview_sink.clone();
            let messages = messages_slot.clone();
            let gate = viewer_config.clone();
            ui.on_canvas_pick(move |x, y| {
                // A canvas pick is a pointer gesture; disabling pointer input
                // must silence it even though it bypasses `pointer-input`.
                let enabled = input_enabled(&gate.borrow(), InputKind::Pointer);
                if !enabled {
                    return;
                }
                if report.upgrade().is_some_and(|ui| ui.get_pan_active()) {
                    return;
                }
                // Ordinary navigation clicks must stay silent; only an active
                // capture tool turns a click into a pick.
                let measure = active.get();
                let drafting = drawing.borrow().is_some();
                if !measure && !drafting {
                    return;
                }
                let world = mapper
                    .borrow()
                    .as_ref()
                    .and_then(|mapper| mapper.to_world([x as f64, y as f64]));
                match world {
                    Some(world) => {
                        if drafting {
                            // A drawing capture consumes the click as a point;
                            // the commit is a later explicit confirm, so a
                            // multi-touch gesture can never commit it.
                            let error = {
                                let mut guard = drawing.borrow_mut();
                                match guard.as_mut() {
                                    Some(tool) => tool.push_point(world).err(),
                                    None => None,
                                }
                            };
                            if let Some(error) = error {
                                if let Some(ui) = report.upgrade() {
                                    ui.set_status_label(
                                        draw_error_text(&messages.borrow(), &error).into(),
                                    );
                                }
                                return;
                            }
                            publish_draw(&report, &drawing, &draw_preview);
                            return;
                        }
                        let _ = s.borrow_mut().send(command_for(
                            CommandId::Measure,
                            &doc,
                            viewport,
                            CommandPayload::Points(vec![world]),
                        ));
                    }
                    None => {
                        // Explicit, never a silent no-op: no mapper (or an
                        // unresolved point) means the pick is not wired on this
                        // host yet.
                        if let Some(ui) = report.upgrade() {
                            ui.set_status_label(
                                messages.borrow().text("status.pick_unwired", &[]).into(),
                            );
                        }
                    }
                }
            });
        }
        {
            // Start a draw/edit capture from the ribbon or a command word.
            // Capture itself is session state; the mutation is the confirmed
            // intent (exactly one command). A Viewer attempt is refused here and
            // again by the command layer, never hidden behind a dead button.
            let work = work_mode.clone();
            let tool = draw_tool.clone();
            let sink = draw_command_sink.clone();
            let preview_sink = draw_preview_sink.clone();
            let messages = messages_slot.clone();
            let report = ui_weak.clone();
            let report_state = ui_weak.clone();
            ui.on_begin_draw_tool(move |name| {
                if let Some(ui) = report_state.upgrade() {
                    ui.set_pan_active(false);
                }
                let Some(kind) = draw_kind_from_label(&messages.borrow(), name.as_str())
                    .or_else(|| cad_app::DrawToolKind::from_key(name.as_str()))
                else {
                    if let Some(ui) = report_state.upgrade() {
                        ui.set_status_label(
                            messages
                                .borrow()
                                .text("draw.error.unknown_geometry", &[])
                                .into(),
                        );
                    }
                    return;
                };
                if !work.get() {
                    if let Some(ui) = report_state.upgrade() {
                        ui.set_status_label(
                            messages.borrow().text("draw.error.read_only", &[]).into(),
                        );
                    }
                    return;
                }
                let selection = report_state
                    .upgrade()
                    .map(|ui| ui.get_selection_count().max(0) as usize)
                    .unwrap_or(0);
                if kind.requires_selection() && selection == 0 {
                    if let Some(ui) = report_state.upgrade() {
                        ui.set_status_label(
                            messages
                                .borrow()
                                .text("draw.error.selection_required", &[])
                                .into(),
                        );
                    }
                    return;
                }
                *tool.borrow_mut() = Some(cad_app::DrawTool::new(kind, selection));
                if let Some(sink) = sink.borrow_mut().as_mut() {
                    sink.begin(kind);
                }
                publish_draw(&report, &tool, &preview_sink);
            });
        }
        {
            let tool = draw_tool.clone();
            let sink = draw_command_sink.clone();
            let preview_sink = draw_preview_sink.clone();
            let messages = messages_slot.clone();
            let report = ui_weak.clone();
            ui.on_confirm_draw_requested(move || {
                let intent = {
                    let borrowed = tool.borrow();
                    let Some(active) = borrowed.as_ref() else {
                        return;
                    };
                    match active.intent() {
                        Ok(intent) => intent,
                        Err(error) => {
                            if let Some(ui) = report.upgrade() {
                                ui.set_status_label(
                                    draw_error_text(&messages.borrow(), &error).into(),
                                );
                            }
                            return;
                        }
                    }
                };
                let mut guard = sink.borrow_mut();
                let Some(sink) = guard.as_mut() else {
                    // Explicit, never a fake success: the host has not installed
                    // the draw command sink yet, so there is nowhere real to
                    // commit. The captured parameters are kept for the retry.
                    if let Some(ui) = report.upgrade() {
                        ui.set_status_label(
                            messages.borrow().text("draw.error.unwired", &[]).into(),
                        );
                    }
                    return;
                };
                match sink.commit(intent) {
                    Ok(()) => {
                        drop(guard);
                        // Committed: the tool is done and the preview is cleared.
                        *tool.borrow_mut() = None;
                        publish_draw(&report, &tool, &preview_sink);
                    }
                    Err(error) => {
                        // A refused commit keeps the captured parameters in place
                        // so the user can fix the input; no transaction was made.
                        if let Some(ui) = report.upgrade() {
                            ui.set_status_label(draw_error_text(&messages.borrow(), &error).into());
                        }
                    }
                }
            });
        }
        {
            // Cancel never commits; it clears the capture and the preview.
            let tool = draw_tool.clone();
            let preview_sink = draw_preview_sink.clone();
            let messages = messages_slot.clone();
            let report = ui_weak.clone();
            ui.on_cancel_draw_requested(move || {
                *tool.borrow_mut() = None;
                publish_draw(&report, &tool, &preview_sink);
                if let Some(ui) = report.upgrade() {
                    ui.set_status_label(messages.borrow().text("draw.status.idle", &[]).into());
                }
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_undo_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Undo,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_redo_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Redo,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Layout switch (F04). The panel sends a row index (-1 = model
            // space); the adapter resolves it to the exact LayoutId from the
            // pushed order. The switch is routed through the shared app command
            // path (`SwitchSpace`), which validates the layout against the
            // drawing and records the active space in the session. A host that
            // installed the legacy `LayoutSwitchSink` is still notified directly
            // (source compatibility); in that case the command is not also sent,
            // so the two paths cannot fight.
            let s = shared.clone();
            let doc = document;
            let order = layout_order.clone();
            let report = ui_weak.clone();
            let messages = messages_slot.clone();
            let layout_switch = layout_switch.clone();
            ui.on_layout_selected(move |index| {
                let space = if index < 0 {
                    Some(cad_representation::SpaceSelection::Model)
                } else {
                    usize::try_from(index)
                        .ok()
                        .and_then(|i| order.borrow().get(i).copied())
                        .map(cad_representation::SpaceSelection::Paper)
                };
                let Some(space) = space else {
                    // No such row in the pushed model: report, do not act.
                    if let Some(ui) = report.upgrade() {
                        ui.set_status_label(
                            messages.borrow().text("layout.switch_unwired", &[]).into(),
                        );
                    }
                    return;
                };
                if let Some(sink) = layout_switch.borrow_mut().as_mut() {
                    sink.select(space);
                    return;
                }
                let space_id = match space {
                    cad_representation::SpaceSelection::Model => SpaceId::Model,
                    cad_representation::SpaceSelection::Paper(id) => SpaceId::Paper(id),
                };
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SwitchSpace,
                    &doc,
                    viewport,
                    CommandPayload::Space(space_id),
                ));
            });
        }
        {
            // 2D/3D toggle (F13/F14). A single command so the session keeps the
            // saved camera for a lossless round trip; the shell reflects the
            // result from the host-pushed `view-3d` property.
            let s = shared.clone();
            let doc = document;
            ui.on_toggle_view_mode_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Switch2d3d,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Standard view (F13). The pushed model is built from
            // `StandardView::ALL`, so the localized label maps back to the exact
            // view without a second ordering list; an unknown label is ignored.
            let s = shared.clone();
            let doc = document;
            let messages = messages_slot.clone();
            ui.on_standard_view_selected(move |name| {
                let messages = messages.borrow().clone();
                if let Some(view) = status::standard_view_from_label(&messages, name.as_str()) {
                    let _ = s.borrow_mut().send(command_for(
                        CommandId::StandardView,
                        &doc,
                        viewport,
                        CommandPayload::StandardView(view),
                    ));
                }
            });
        }
        {
            // Explicit orthographic/perspective toggle (F13), distinct from the
            // 2D/3D toggle: it keeps the target and adjusts the projection.
            let s = shared.clone();
            let doc = document;
            ui.on_toggle_projection_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SwitchProjection,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_backend_selected(move |name| {
                let choice = match name.as_str() {
                    "WebGPU" => cad_app::BackendChoice::WebGpu,
                    "WebGL2" => cad_app::BackendChoice::WebGl2,
                    _ => cad_app::BackendChoice::Auto,
                };
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SwitchBackend,
                    &doc,
                    viewport,
                    CommandPayload::Backend(choice),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document;
            ui.on_diagnostics_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Diagnostics,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Opening the resources drawer requests the real `Resources` command
            // (the document's external resource summary); the host then pushes
            // the font/import/proxy details through `set_resources_sections`.
            let s = shared.clone();
            let doc = document;
            ui.on_resources_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::Resources,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Closing a drawer is a pure presentation action; it emits no command.
            let weak = ui.as_weak();
            ui.on_resources_closed(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_resources_open(false);
                }
            });
        }
        {
            // The 3D observation drawer is presentation-only: its controls reuse
            // the existing view callbacks, so opening it emits no command.
            let weak = ui.as_weak();
            ui.on_view3d_requested(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_view3d_open(true);
                }
            });
            let weak = ui.as_weak();
            ui.on_view3d_closed(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_view3d_open(false);
                }
            });
        }
        {
            // Closing the drawer is a pure presentation action; it emits no
            // command (audit U08: the bar stays simple, the drawer is explicit).
            let ui_weak = ui.as_weak();
            ui.on_diagnostics_closed(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_diagnostics_open(false);
                }
            });
        }
        {
            let input = view_input.clone();
            let s = shared.clone();
            let doc = document;
            let is_3d = view_3d.clone();
            let orbit_last = orbit_last.clone();
            let drawing = draw_tool.clone();
            let draw_preview = draw_preview_sink.clone();
            let mapper = pick_mapper.clone();
            let report = ui_weak.clone();
            let gate = viewer_config.clone();
            ui.on_pointer_input(move |kind, button, x, y| {
                // Mouse, trackpad and Android-synthesized touch all enter here;
                // `interaction.pointer` is their shared gate.
                let enabled = input_enabled(&gate.borrow(), InputKind::Pointer);
                if !enabled {
                    return;
                }
                let x = x as f64;
                let y = y as f64;
                if report.upgrade().is_some_and(|ui| ui.get_pan_active()) {
                    if let Some(input) = input.borrow().as_ref() {
                        input.pointer(
                            kind,
                            if button == 1 || button == 0 {
                                3
                            } else {
                                button
                            },
                            x,
                            y,
                        );
                    }
                    return;
                }
                // While a draw/edit capture is active a pointer move updates the
                // rubber-band cursor (no capture, no command); the release picks
                // through `canvas-pick`. Non-primary navigation stays with the host.
                if kind == 2 && button == 0 && drawing.borrow().is_some() {
                    if let Some(world) = mapper
                        .borrow()
                        .as_ref()
                        .and_then(|mapper| mapper.to_world([x, y]))
                    {
                        if let Some(tool) = drawing.borrow_mut().as_mut() {
                            tool.set_cursor(Some(world));
                        }
                        publish_draw(&report, &drawing, &draw_preview);
                    }
                }
                // Primary capture belongs to the draw tool. Forwarding it to
                // navigation would clear MOVE's selection on an empty anchor
                // (and could pan/orbit while choosing drawing points).
                if drawing.borrow().is_some() && (button == 0 || button == 1) {
                    return;
                }
                // In 3D mode a left-button drag orbits the view through the
                // shared command path. Other buttons (middle/right) and scroll
                // still route to the host's `ViewInput`, so a host can keep 3D
                // pan on the middle button.
                if is_3d.get() && button == 1 {
                    match kind {
                        0 => {
                            orbit_last.set(Some([x, y]));
                            return;
                        }
                        2 => {
                            if let Some(from) = orbit_last.get() {
                                let (yaw, pitch) = orbit_delta(from, [x, y]);
                                orbit_last.set(Some([x, y]));
                                if yaw != 0.0 || pitch != 0.0 {
                                    let _ = s.borrow_mut().send(command_for(
                                        CommandId::Orbit,
                                        &doc,
                                        viewport,
                                        CommandPayload::Orbit { yaw, pitch },
                                    ));
                                }
                                return;
                            }
                            // No drag started in 3D: fall through to the host.
                        }
                        1 | 3 => {
                            orbit_last.set(None);
                            return;
                        }
                        _ => {}
                    }
                }
                if let Some(input) = input.borrow().as_ref() {
                    input.pointer(kind, button, x, y);
                }
            });
        }
        {
            let input = view_input.clone();
            let gate = viewer_config.clone();
            ui.on_scroll_input(move |dx, dy| {
                // Scroll is the wheel form of pointer input and shares its gate.
                let enabled = input_enabled(&gate.borrow(), InputKind::Pointer);
                if !enabled {
                    return;
                }
                if let Some(input) = input.borrow().as_ref() {
                    input.scroll(dx as f64, dy as f64);
                }
            });
        }

        Ok(UiAdapter {
            viewer_config,
            configuration,
            ui,
            view_input,
            pick_mapper,
            layout_switch,
            draw_command_sink,
            draw_preview_sink,
            draw_tool,
            selected_kind,
            measurement_active,
            layer_order,
            layout_order,
            messages: messages_slot,
            layer_override_count,
            selection_count,
            view_3d,
            view_state,
            resources_sections,
            work_mode,
            import_snapshot,
        })
    }

    /// Route canvas input to the CAD view; call once the view exists.
    pub fn set_view_input(&self, input: Rc<dyn ViewInput>) {
        *self.view_input.borrow_mut() = Some(input);
    }

    /// Install the logical-pixel → world converter used for measurement picks.
    ///
    /// Until this is installed, a canvas pick reports "unwired" in the status
    /// line instead of dropping the click. Hosts should derive the point from
    /// the authoritative viewport (`Viewport::screen_to_world`).
    pub fn set_canvas_pick_mapper(&self, mapper: Rc<dyn CanvasPickMapper>) {
        *self.pick_mapper.borrow_mut() = Some(mapper);
    }

    /// Install the sink that switches model/paper space when the layout panel
    /// emits `layout-selected` (F04/U03).
    ///
    /// Until this is installed, clicking a layout reports "layout switch not
    /// wired" in the status line instead of silently doing nothing.
    pub fn set_layout_switch_sink(&self, sink: Box<dyn LayoutSwitchSink>) {
        *self.layout_switch.borrow_mut() = Some(sink);
    }

    /// Install the sink that commits a confirmed draw/edit intent as exactly one
    /// drawing command (drawing-edit §2/§4).
    ///
    /// Until this is installed, confirming a draw reports "draw command not
    /// wired" in the status line instead of fabricating a payload-free success.
    pub fn set_draw_command_sink(&self, sink: Box<dyn DrawCommandSink>) {
        *self.draw_command_sink.borrow_mut() = Some(sink);
    }

    /// Install the sink that receives the in-progress draw preview so a host can
    /// push it to `CadView::set_draw_preview` (drawing-edit §4).
    ///
    /// Until this is installed there is simply no live rubber-band overlay; the
    /// step panel still tracks the capture.
    pub fn set_draw_preview_sink(&self, sink: Rc<dyn DrawPreviewSink>) {
        *self.draw_preview_sink.borrow_mut() = Some(sink);
    }

    /// The active draw/edit capture state, if one is running.
    ///
    /// A host that prefers polling over the preview sink can forward this to
    /// [`CadView::set_draw_preview`].
    pub fn draw_preview(&self) -> Option<cad_app::DrawPreview> {
        self.draw_tool.borrow().as_ref().map(|tool| tool.preview())
    }

    /// The current configuration store, cloned for hosts that persist user
    /// preferences or query the effective/host split.
    pub fn config_store(&self) -> Rc<RefCell<cad_app::viewer_config::ViewerConfigStore>> {
        self.viewer_config.clone()
    }

    /// A handle for pushing state from the host.
    pub fn handle(&self) -> UiHandle {
        UiHandle {
            viewer_config: self.viewer_config.clone(),
            ui: self.ui.as_weak(),
            messages: self.messages.clone(),
            selected_kind: self.selected_kind.clone(),
            measurement_active: self.measurement_active.clone(),
            layer_order: self.layer_order.clone(),
            layout_order: self.layout_order.clone(),
            layer_override_count: self.layer_override_count.clone(),
            selection_count: self.selection_count.clone(),
            view_3d: self.view_3d.clone(),
            view_state: self.view_state.clone(),
            resources_sections: self.resources_sections.clone(),
            work_mode: self.work_mode.clone(),
            import_snapshot: self.import_snapshot.clone(),
        }
    }

    pub fn window(&self) -> &slint::Window {
        self.ui.window()
    }

    /// Resize the window so its logical size is `logical_size` at `scale`.
    ///
    /// Mirrors the size back into the recorded configuration and lets a host
    /// that owns the platform surface (Android/web) resize it to the same
    /// physical pixels, so the CAD frame matches the canvas rectangle (U07).
    /// Returns the physical size so the caller can log or forward it.
    pub fn fit_window_to_logical(
        &mut self,
        logical_size: [f64; 2],
        scale: f32,
    ) -> slint::PhysicalSize {
        let physical = self.fitted_physical_size(logical_size, scale);
        self.ui.window().set_size(physical);
        self.configuration.logical_size = logical_size;
        apply_viewer_presentation(
            &self.ui,
            self.viewer_config.borrow().effective(),
            logical_size,
        );
        physical
    }

    /// Physical size the window must have to fit `logical_size` at `scale`.
    fn fitted_physical_size(&self, logical_size: [f64; 2], scale: f32) -> slint::PhysicalSize {
        let width = (logical_size[0].max(1.0) as f32 * scale).round() as u32;
        let height = (logical_size[1].max(1.0) as f32 * scale).round() as u32;
        slint::PhysicalSize::new(width.max(1), height.max(1))
    }

    /// The measurement algorithm currently selected in the shell.
    pub fn selected_measurement_kind(&self) -> MeasurementToolKind {
        self.selected_kind.get()
    }

    /// Show the window and run the platform event loop.
    ///
    /// On Android this is called after `slint::android::init()`; on desktop it
    /// is the winit event loop; on the web it hands over to the browser event
    /// loop (the call may not return — see `apps/app-web`).
    pub fn run(&self) -> CadResult<()> {
        self.ui
            .show()
            .map_err(|e| CadError::Invariant(format!("slint show: {e}")))?;
        slint::run_event_loop().map_err(|e| CadError::Invariant(format!("slint loop: {e}")))?;
        Ok(())
    }

    /// The component, for hosts that need to install a rendering notifier.
    pub fn component(&self) -> &YacrWindow {
        &self.ui
    }
}

/// Ask Slint to render with wgpu so a CAD renderer can share the device.
///
/// Must be called before creating any window. On Android the backend is Skia
/// behind wgpu (`unstable-wgpu-30`), which is what makes a shared texture
/// possible at all; on the web `web::select_backend` chooses WebGPU or WebGL2.
/// Uses the discrete-first default selection. See `docs/render-backends.md`.
pub fn select_wgpu_backend() -> CadResult<()> {
    select_wgpu_backend_with(GpuSelection::Auto)
}

/// Like [`select_wgpu_backend`], with an explicit adapter preference.
///
/// `auto`/`high` prefer a discrete GPU (the dual-GPU default), `low` prefers an
/// integrated one, mapped to wgpu's `PowerPreference`. Can also be overridden
/// with `WGPU_ADAPTER_NAME` / `WGPU_POWER_PREF`. See `docs/render-backends.md`.
pub fn select_wgpu_backend_with(preference: GpuSelection) -> CadResult<()> {
    let mut settings = slint::wgpu_30::WGPUSettings::default();
    settings.power_preference = match preference {
        GpuSelection::Auto | GpuSelection::HighPerformance => {
            wgpu::PowerPreference::HighPerformance
        }
        GpuSelection::LowPower => wgpu::PowerPreference::LowPower,
    };
    slint::BackendSelector::new()
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Automatic(settings))
        .select()
        .map_err(|e| CadError::GpuFailure(format!("slint wgpu backend: {e}")))
}
