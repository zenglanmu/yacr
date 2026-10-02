//! adapter module.

use super::*;

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
        document: document.clone(),
        viewport,
        payload,
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

        let work_mode: Rc<Cell<bool>> = Rc::new(Cell::new(work_mode));

        let document = configuration.document.clone();
        let viewport = configuration.viewport;
        let shared: Rc<RefCell<S>> = Rc::new(RefCell::new(sink));
        let view_input: Rc<RefCell<Option<Rc<dyn ViewInput>>>> = Rc::new(RefCell::new(None));
        let pick_mapper: Rc<RefCell<Option<Rc<dyn CanvasPickMapper>>>> =
            Rc::new(RefCell::new(None));
        let layout_switch: Rc<RefCell<Option<Box<dyn LayoutSwitchSink>>>> =
            Rc::new(RefCell::new(None));
        let selected_kind: Rc<Cell<MeasurementToolKind>> =
            Rc::new(Cell::new(MeasurementToolKind::Distance));
        let measurement_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let selected_annotation_kind: Rc<Cell<AnnotationToolKind>> =
            Rc::new(Cell::new(AnnotationToolKind::Text));
        let annotation_active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let annotation_order: Rc<RefCell<Vec<AnnotationId>>> = Rc::new(RefCell::new(Vec::new()));
        let layer_order: Rc<RefCell<Vec<LayerId>>> = Rc::new(RefCell::new(Vec::new()));
        let layout_order: Rc<RefCell<Vec<cad_domain::LayoutId>>> =
            Rc::new(RefCell::new(Vec::new()));
        let messages_slot: Rc<RefCell<MessageSource>> = Rc::new(RefCell::new(messages.clone()));
        let layer_override_count: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let selection_count: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let annotation_hidden_count: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let view_3d: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let orbit_last: Rc<Cell<Option<[f64; 2]>>> = Rc::new(Cell::new(None));
        let import_snapshot: Rc<RefCell<Option<cad_app::ImportProgressSnapshot>>> =
            Rc::new(RefCell::new(None));

        crate::command_line::connect(&ui, messages_slot.clone());

        {
            let s = shared.clone();
            let doc = document.clone();
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
            // Cancel a running background open (F01). `CancelLoading` is routed
            // by the host to `HostController::cancel_async_open`; the panel only
            // enables this while a cancellable job is running.
            let s = shared.clone();
            let doc = document.clone();
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
            let doc = document.clone();
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
            // The Measure button starts (or restarts) the selected algorithm
            // instead of sending a payload-free guess (audit U04/F06).
            let s = shared.clone();
            let doc = document.clone();
            let kind = selected_kind.clone();
            ui.on_measure_requested(move || {
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
            let doc = document.clone();
            let kind_slot = selected_kind.clone();
            let messages = messages_slot.clone();
            ui.on_measure_kind_selected(move |name| {
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
            let doc = document.clone();
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
            let doc = document.clone();
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
            // Persist the last confirmed measurement as an annotation (F06/F07).
            // The command layer refuses with InvalidInput when no record exists;
            // the shell only enables the button when the host pushed a record.
            let s = shared.clone();
            let doc = document.clone();
            ui.on_save_measurement_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SaveMeasurementAsAnnotation,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Viewer/Work switch (audit U02). Emits a real `SetMode` command; the
            // shell reflects the target immediately and the host's authoritative
            // push (`UiHandle::set_mode`) corrects any discrepancy.
            let s = shared.clone();
            let doc = document.clone();
            let work = work_mode.clone();
            let messages = messages_slot.clone();
            let report = ui_weak.clone();
            ui.on_mode_toggled(move || {
                let target = if work.get() {
                    cad_app::AppMode::Viewer
                } else {
                    cad_app::AppMode::Work
                };
                let is_work = target == cad_app::AppMode::Work;
                work.set(is_work);
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
            let doc = document.clone();
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
            let doc = document.clone();
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
            // Clearing the selection is a real, read-only Select command with an
            // empty payload; the application records it and mutates nothing.
            let s = shared.clone();
            let doc = document.clone();
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
            // A canvas click becomes one `Measure`/`Points` command when the host
            // installed a world mapper. Without one the click cannot be turned
            // into a real point, so it is reported (not silently swallowed).
            let s = shared.clone();
            let doc = document.clone();
            let mapper = pick_mapper.clone();
            let report = ui_weak.clone();
            let active = measurement_active.clone();
            let annotating = annotation_active.clone();
            let messages = messages_slot.clone();
            ui.on_canvas_pick(move |x, y| {
                // Ordinary navigation clicks must stay silent; only an active
                // capture tool turns a click into a pick.
                let measure = active.get();
                let annotate = annotating.get();
                if !measure && !annotate {
                    return;
                }
                let world = mapper
                    .borrow()
                    .as_ref()
                    .and_then(|mapper| mapper.to_world([x as f64, y as f64]));
                match world {
                    Some(world) => {
                        let (id, payload) = if annotate {
                            (
                                CommandId::AppendAnnotationPoints,
                                CommandPayload::AppendAnnotationPoints(vec![world]),
                            )
                        } else {
                            (CommandId::Measure, CommandPayload::Points(vec![world]))
                        };
                        let _ = s
                            .borrow_mut()
                            .send(command_for(id, &doc, viewport, payload));
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
            // Annotate starts (or restarts) the selected annotation kind instead
            // of sending a payload-free command (audit U04/F07).
            let s = shared.clone();
            let doc = document.clone();
            let kind = selected_annotation_kind.clone();
            ui.on_annotate_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::BeginAnnotationTool,
                    &doc,
                    viewport,
                    CommandPayload::AnnotationTool(kind.get()),
                ));
            });
        }
        {
            // Selecting a kind immediately starts that tool; this is the explicit
            // user choice, not a default. Localized labels map back through the
            // catalog-built ordering.
            let s = shared.clone();
            let doc = document.clone();
            let kind_slot = selected_annotation_kind.clone();
            let messages = messages_slot.clone();
            ui.on_annotation_kind_selected(move |name| {
                let messages = messages.borrow().clone();
                let Some(kind) = status::annotation_kind_from_label(&messages, name.as_str())
                else {
                    return;
                };
                kind_slot.set(kind);
                let _ = s.borrow_mut().send(command_for(
                    CommandId::BeginAnnotationTool,
                    &doc,
                    viewport,
                    CommandPayload::AnnotationTool(kind),
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_confirm_annotation_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ConfirmAnnotationTool,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_cancel_annotation_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::CancelAnnotationTool,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            // Text payload for text/leader annotate tools. The shell only sends
            // it when the active kind requires text; the application validates
            // that again.
            let s = shared.clone();
            let doc = document.clone();
            ui.on_annotation_text_edited(move |text| {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::AnnotationText,
                    &doc,
                    viewport,
                    CommandPayload::AnnotationText(text.to_string()),
                ));
            });
        }
        {
            // Row click selects the annotation for edit/delete. The adapter
            // resolves the row index to the exact id from the pushed order.
            let s = shared.clone();
            let doc = document.clone();
            let order = annotation_order.clone();
            ui.on_annotation_selected(move |index| {
                let id = usize::try_from(index)
                    .ok()
                    .and_then(|i| order.borrow().get(i).copied());
                let _ = s.borrow_mut().send(command_for(
                    CommandId::SelectAnnotation,
                    &doc,
                    viewport,
                    CommandPayload::SelectAnnotation(id),
                ));
            });
        }
        {
            // Delete the selected annotation row (resolved through the order).
            let s = shared.clone();
            let doc = document.clone();
            let order = annotation_order.clone();
            ui.on_annotation_delete_requested(move |index| {
                let id = usize::try_from(index)
                    .ok()
                    .and_then(|i| order.borrow().get(i).copied());
                if let Some(id) = id {
                    let _ = s.borrow_mut().send(command_for(
                        CommandId::DeleteAnnotationById,
                        &doc,
                        viewport,
                        CommandPayload::DeleteAnnotation(id),
                    ));
                }
            });
        }
        {
            // Annotation visibility toggle: session state only, no transaction.
            let s = shared.clone();
            let doc = document.clone();
            let order = annotation_order.clone();
            ui.on_annotation_visibility_toggled(move |index, visible| {
                let id = usize::try_from(index)
                    .ok()
                    .and_then(|i| order.borrow().get(i).copied());
                if let Some(id) = id {
                    let _ = s.borrow_mut().send(command_for(
                        CommandId::SetAnnotationVisibility,
                        &doc,
                        viewport,
                        CommandPayload::AnnotationVisibility(id, visible),
                    ));
                }
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
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
            let doc = document.clone();
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
            let doc = document.clone();
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
            let doc = document.clone();
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
            let doc = document.clone();
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
            let doc = document.clone();
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
            let doc = document.clone();
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
            let doc = document.clone();
            ui.on_export_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ExportAnnotations,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
            ui.on_import_requested(move || {
                let _ = s.borrow_mut().send(command_for(
                    CommandId::ImportAnnotations,
                    &doc,
                    viewport,
                    CommandPayload::None,
                ));
            });
        }
        {
            let s = shared.clone();
            let doc = document.clone();
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
            let doc = document.clone();
            let is_3d = view_3d.clone();
            let orbit_last = orbit_last.clone();
            ui.on_pointer_input(move |kind, button, x, y| {
                let x = x as f64;
                let y = y as f64;
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
            ui.on_scroll_input(move |dx, dy| {
                if let Some(input) = input.borrow().as_ref() {
                    input.scroll(dx as f64, dy as f64);
                }
            });
        }

        Ok(UiAdapter {
            configuration,
            ui,
            view_input,
            pick_mapper,
            layout_switch,
            selected_kind,
            measurement_active,
            selected_annotation_kind,
            annotation_active,
            annotation_order,
            layer_order,
            layout_order,
            messages: messages_slot,
            layer_override_count,
            selection_count,
            annotation_hidden_count,
            view_3d,
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

    /// A handle for pushing state from the host.
    pub fn handle(&self) -> UiHandle {
        UiHandle {
            ui: self.ui.as_weak(),
            messages: self.messages.clone(),
            selected_kind: self.selected_kind.clone(),
            measurement_active: self.measurement_active.clone(),
            selected_annotation_kind: self.selected_annotation_kind.clone(),
            annotation_active: self.annotation_active.clone(),
            annotation_order: self.annotation_order.clone(),
            layer_order: self.layer_order.clone(),
            layout_order: self.layout_order.clone(),
            layer_override_count: self.layer_override_count.clone(),
            selection_count: self.selection_count.clone(),
            annotation_hidden_count: self.annotation_hidden_count.clone(),
            view_3d: self.view_3d.clone(),
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

    /// The annotation kind currently selected in the shell (Annotate button and
    /// canvas picks agree with the selector).
    pub fn selected_annotation_kind(&self) -> AnnotationToolKind {
        self.selected_annotation_kind.get()
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
/// See `docs/render-backends.md`.
pub fn select_wgpu_backend() -> CadResult<()> {
    slint::BackendSelector::new()
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::default())
        .select()
        .map_err(|e| CadError::GpuFailure(format!("slint wgpu backend: {e}")))
}
