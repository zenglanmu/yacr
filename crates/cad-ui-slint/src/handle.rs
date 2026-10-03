//! handle module.

use super::*;

impl UiHandle {
    /// Shell-local hit query: floating controls must not be consumed by host touch navigation.
    pub fn canvas_hit_test(&self, point: [f64; 2]) -> CadResult<bool> {
        let ui = self.ui.upgrade().ok_or(CadError::Cancelled)?;
        let (rect, _) = self.shell_geometry()?;
        let x = point[0] - rect[0];
        let y = point[1] - rect[1];
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x >= rect[2] || y >= rect[3] {
            return Ok(false);
        }
        if ui.get_application_ui() {
            if ui.get_navigation_visible()
                && x >= rect[2] - 64.0
                && x < rect[2] - 16.0
                && (20.0..68.0).contains(&y)
            {
                return Ok(false);
            }
            if ui.get_phone_shell()
                && ui.get_layouts_visible()
                && (12.0..152.0).contains(&x)
                && (16.0..64.0).contains(&y)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// Atomic layout-only configuration. Unsupported protocol fields are refused in cad-app.
    pub fn set_config(
        &self,
        config: cad_app::viewer_config::ViewerConfig,
    ) -> Result<(), cad_app::viewer_config::ConfigError> {
        let ui = self
            .ui
            .upgrade()
            .ok_or_else(|| cad_app::viewer_config::ConfigError {
                path: "window".into(),
                reason: "window was dropped".into(),
            })?;
        self.viewer_config.borrow_mut().set_config(config)?;
        let size = ui.window().size().to_logical(ui.window().scale_factor());
        apply_viewer_presentation(
            &ui,
            self.viewer_config.borrow().effective(),
            [size.width as f64, size.height as f64],
        );
        Ok(())
    }
    pub fn effective_config(&self) -> cad_app::viewer_config::ViewerConfig {
        self.viewer_config.borrow().effective().clone()
    }
    /// Logical CAD hit rectangle and shell expansion state for browser touch routing.
    pub fn shell_geometry(&self) -> CadResult<([f64; 4], [bool; 4])> {
        let ui = self.ui.upgrade().ok_or(CadError::Cancelled)?;
        Ok((
            [
                ui.get_cad_left() as f64,
                ui.get_cad_top() as f64,
                ui.get_cad_width() as f64,
                ui.get_cad_height() as f64,
            ],
            [
                ui.get_phone_shell(),
                ui.get_tools_open(),
                ui.get_ribbon_expanded(),
                ui.get_command_expanded(),
            ],
        ))
    }
    fn with(&self, f: impl FnOnce(&YacrWindow)) -> CadResult<()> {
        let ui = self.ui.upgrade().ok_or(CadError::Cancelled)?;
        f(&ui);
        Ok(())
    }

    /// Replace the composited CAD frame with a new texture-backed image.
    pub fn set_cad_frame(&self, image: Image) -> CadResult<()> {
        self.with(|ui| ui.set_cad_frame(image))
    }

    pub fn touch_pick(&self, x: f64, y: f64) -> CadResult<()> {
        self.with(|ui| ui.invoke_canvas_pick(x as f32, y as f32))
    }

    /// Abandon shell-owned drawing capture without submitting a transaction.
    /// Hosts call this on touch cancellation and after replacing a document.
    pub fn cancel_draw_capture(&self) -> CadResult<()> {
        self.with(|ui| {
            if ui.get_draw_tool_active() {
                ui.invoke_cancel_draw_requested();
            }
        })
    }

    /// Push the derived view/observation state into the shell (F13/F14).
    ///
    /// The values come from the authoritative application viewport (see
    /// [`bridge::CadView::sync_from_viewport`]); this only mirrors them so the
    /// 2D/3D and projection affordances show the real state. It also updates the
    /// shared flag the adapter uses to route a 3D drag to `Orbit`.
    pub fn set_view_state(&self, state: ViewStateUi) -> CadResult<()> {
        self.view_3d.set(state.is_3d);
        self.with(|ui| {
            ui.set_view_3d(state.is_3d);
            ui.set_view_perspective(state.perspective);
        })
    }

    pub fn set_status(&self, status: impl Into<slint::SharedString>) -> CadResult<()> {
        let s = status.into();
        self.with(|ui| ui.set_status_label(s))
    }

    /// Document title is host data, never the concept illustration's sample filename.
    pub fn set_document_name(&self, name: &str) -> CadResult<()> {
        self.with(|ui| ui.set_application_title(name.into()))
    }

    pub fn set_work_mode(&self, work: bool) -> CadResult<()> {
        self.work_mode.set(work);
        let label = crate::status::mode_label(
            &self.messages.borrow(),
            if work {
                cad_app::AppMode::Work
            } else {
                cad_app::AppMode::Viewer
            },
        );
        self.with(|ui| {
            ui.set_work_mode(work);
            ui.set_mode_label(label.into());
        })
    }

    /// Push the authoritative session mode into the shell (audit U02).
    ///
    /// Sets both the `work-mode` flag and the catalog `mode-label`, so the shell
    /// always shows the mode the command layer actually enforces. Hosts call this
    /// with `HostController::mode()` after every command.
    pub fn set_mode(&self, mode: cad_app::AppMode) -> CadResult<()> {
        self.set_work_mode(mode == cad_app::AppMode::Work)
    }

    pub fn set_can_undo(&self, can_undo: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_undo(can_undo))
    }

    /// Independent redo availability (audit U11). Never derived from
    /// `set_can_undo`.
    pub fn set_can_redo(&self, can_redo: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_redo(can_redo))
    }

    /// Push an application [`cad_app::HistoryAvailability`] snapshot verbatim.
    ///
    /// Hosts call this with `HostController::history_availability()` after every
    /// command, so the two flags always reflect the two history stacks — undoing
    /// to empty disables undo while keeping redo enabled.
    pub fn set_history_availability(
        &self,
        availability: cad_app::HistoryAvailability,
    ) -> CadResult<()> {
        self.with(|ui| {
            ui.set_can_undo(availability.can_undo);
            ui.set_can_redo(availability.can_redo);
        })
    }

    /// Push the whole measurement panel state in one call (audit U03/U04).
    pub fn set_measurement_state(&self, state: &MeasurementUiState) -> CadResult<()> {
        self.selected_kind.set(state.kind);
        self.measurement_active.set(state.active);
        let step = state.step_label.clone();
        let unit = state.unit_label.clone();
        let can_save = state.can_save_annotation;
        self.with(|ui| {
            ui.set_measurement_active(state.active);
            ui.set_measurement_can_confirm(state.can_confirm);
            ui.set_measurement_can_save_annotation(can_save);
            ui.set_measurement_kind_index(state.kind_index());
            ui.set_measurement_step_label(step.into());
            ui.set_unit_label(unit.into());
        })
    }

    /// Push the draw/edit panel state (drawing-edit §4).
    ///
    /// A host calls this with the preview from `cad_app::DrawTool::preview()` so
    /// the step text and confirm/cancel affordance track the capture state
    /// machine. Idle (`active == false`) hides the row; nothing is fabricated.
    pub fn set_draw_state(&self, state: &DrawUiState) -> CadResult<()> {
        let step = state.step_label.clone();
        self.with(|ui| {
            ui.set_draw_tool_active(state.active);
            ui.set_draw_can_confirm(state.can_confirm);
            ui.set_draw_step_label(step.into());
        })
    }

    /// Push the layer panel state (audit F03/U03).
    ///
    /// Also records the ordered `LayerId`s so a later toggle callback can map
    /// its row index back to the real layer without a lossy cast.
    pub fn set_layer_state(&self, state: &LayerPanelState, order: &[LayerId]) -> CadResult<()> {
        *self.layer_order.borrow_mut() = order.to_vec();
        let rows: Vec<LayerRow> = state
            .rows
            .iter()
            .map(|row| LayerRow {
                id: row.id,
                name: row.name.clone().into(),
                visible: row.visible,
                overridden: row.overridden,
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let override_count = state.override_count as i32;
        let empty = state.empty_label.clone();
        self.layer_override_count.set(override_count);
        let messages = self.messages.borrow().clone();
        let override_label = layer_override_label(&messages, state.override_count);
        self.with(|ui| {
            ui.set_layer_rows(model);
            ui.set_layer_override_count(override_count);
            ui.set_layer_override_label(override_label.into());
            ui.set_layer_empty_label(empty.into());
        })
    }

    /// Push the layout (paper-space) list state (audit F04/U03).
    ///
    /// Rows come from the database's real layout table via
    /// [`bridge::layout_descriptors`]; until a host pushes them the panel shows
    /// its explicit empty state, never a fabricated layout. Also records the
    /// ordered `LayoutId`s so a later switch callback can map a row index back to
    /// the exact layout without a lossy cast.
    pub fn set_layout_state(
        &self,
        state: &LayoutPanelState,
        order: &[cad_domain::LayoutId],
    ) -> CadResult<()> {
        *self.layout_order.borrow_mut() = order.to_vec();
        let rows: Vec<LayoutRow> = state
            .rows
            .iter()
            .map(|row| LayoutRow {
                id: row.id,
                name: row.name.clone().into(),
                supported: row.supported,
                reason: row.reason.clone().into(),
                viewport_count: row.viewport_count,
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let active = state.active_index.unwrap_or(-1);
        let empty = state.empty_label.clone();
        self.with(|ui| {
            ui.set_layout_rows(model);
            let mut labels = vec![ui.get_layout_model_space_label().to_string()];
            labels.extend(state.rows.iter().map(|row| {
                if row.supported {
                    row.name.clone()
                } else {
                    format!("{} — {}", row.name, row.reason)
                }
            }));
            ui.set_layout_labels(string_model(&labels));
            ui.set_layout_active_index(active);
            ui.set_layout_empty_label(empty.into());
        })
    }

    /// Push the read-only properties panel state (audit F05/U03).
    pub fn set_property_state(&self, state: &PropertyPanelState) -> CadResult<()> {
        let rows: Vec<PropertyRow> = state
            .rows
            .iter()
            .map(|row| PropertyRow {
                key: row.key.clone().into(),
                value: row.value.clone().into(),
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let count = state.count as i32;
        let empty = state.empty_label.clone();
        let mixed = state.mixed_label.clone();
        self.selection_count.set(count);
        let messages = self.messages.borrow().clone();
        let selected_label = selected_count_label(&messages, state.count);
        self.with(|ui| {
            ui.set_property_rows(model);
            ui.set_selection_count(count);
            ui.set_property_selected_label(selected_label.into());
            ui.set_property_empty_label(empty.into());
            ui.set_property_mixed_label(mixed.into());
        })
    }

    /// Push the annotation management + tool panel state (audit F07/F09/U03).
    ///
    /// Also records the ordered `AnnotationId`s so a later visibility/delete
    /// callback can map its row index back to the real annotation.
    pub fn set_annotation_state(
        &self,
        state: &AnnotationPanelState,
        order: &[AnnotationId],
    ) -> CadResult<()> {
        *self.annotation_order.borrow_mut() = order.to_vec();
        if let Some(kind) = AnnotationToolKind::from_index(state.tool_kind_index) {
            self.selected_annotation_kind.set(kind);
        }
        self.annotation_active.set(state.tool_active);
        let rows: Vec<AnnotationRow> = state
            .rows
            .iter()
            .map(|row| AnnotationRow {
                id: row.id,
                kind: row.kind.clone().into(),
                text: row.text.clone().into(),
                visible: row.visible,
                overridden: row.overridden,
                selected: row.selected,
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let hidden = state.hidden_count as i32;
        self.annotation_hidden_count.set(hidden);
        let empty = state.empty_label.clone();
        let step = state.tool_step_label.clone();
        let messages = self.messages.borrow().clone();
        let hidden_label = annotation_hidden_label(&messages, state.hidden_count);
        self.with(|ui| {
            ui.set_annotation_rows(model);
            ui.set_annotation_hidden_count(hidden);
            ui.set_annotation_hidden_label(hidden_label.into());
            ui.set_annotation_empty_label(empty.into());
            ui.set_annotation_tool_active(state.tool_active);
            ui.set_annotation_tool_can_confirm(state.tool_can_confirm);
            ui.set_annotation_kind_index(state.tool_kind_index);
            ui.set_annotation_step_label(step.into());
            ui.set_annotation_requires_text(state.requires_text);
            ui.set_annotation_text_supplied(state.text_supplied);
        })
    }

    pub fn set_backend_index(&self, index: i32) -> CadResult<()> {
        self.with(|ui| ui.set_backend_index(index))
    }

    /// Push the asynchronous-open progress panel state (F01).
    ///
    /// Stores the raw snapshot so a live locale switch can re-derive the
    /// phase/progress labels, then writes the shell's progress properties. The
    /// panel is hidden while idle and after a successful open; a cancelled or
    /// failed terminal is explicit.
    pub fn set_import_state(&self, state: &ImportProgressUiState) -> CadResult<()> {
        *self.import_snapshot.borrow_mut() = state.source.clone();
        let active = state.visible;
        let phase = state.phase_label.clone();
        let progress = state.progress_text.clone();
        // The shell shows an indeterminate bar whenever no real total exists.
        let indeterminate = state.percent.is_none();
        let percent = state.percent.unwrap_or(0.0);
        let cancellable = state.cancellable;
        self.with(|ui| {
            ui.set_import_active(active);
            ui.set_import_phase_label(phase.into());
            ui.set_import_progress_text(progress.into());
            ui.set_import_indeterminate(indeterminate);
            ui.set_import_percent(percent);
            ui.set_import_cancellable(cancellable);
        })
    }

    /// Re-derive and re-push the import panel labels for the active catalog.
    fn refresh_import_labels(&self, messages: &MessageSource) -> CadResult<()> {
        let snapshot = self.import_snapshot.borrow().clone();
        let state = ImportProgressUiState::from_snapshot(snapshot.as_ref(), messages);
        self.set_import_state(&state)
    }

    /// Re-apply the catalog for `locale` and update **all** chrome labels.
    ///
    /// Returns the resolution actually applied; callers can log a fallback. The
    /// adapter reformats the currently displayed override/selection counts too,
    /// so a live locale switch does not leave a stale Chinese count label. Hosts
    /// that supply their own `empty_label`/`mixed_label` strings should re-push
    /// the panel state with catalog-derived text after switching.
    pub fn set_locale(&self, locale: &str) -> CadResult<LocaleResolution> {
        let messages = MessageSource::from_request(locale);
        let resolution = messages.resolution().clone();
        *self.messages.borrow_mut() = messages.clone();
        let override_count = self.layer_override_count.get().max(0) as usize;
        let selection_count = self.selection_count.get().max(0) as usize;
        let hidden_count = self.annotation_hidden_count.get().max(0) as usize;
        let override_label = layer_override_label(&messages, override_count);
        let selected_label = selected_count_label(&messages, selection_count);
        let hidden_label = annotation_hidden_label(&messages, hidden_count);
        let work = self.work_mode.get();
        self.with(|ui| {
            apply_chrome(ui, &messages, work);
            ui.set_layer_override_label(override_label.into());
            ui.set_property_selected_label(selected_label.into());
            ui.set_annotation_hidden_label(hidden_label.into());
        })?;
        self.refresh_import_labels(&messages)?;
        Ok(resolution)
    }

    /// Push the diagnostics drawer state (audit U08).
    ///
    /// The rows come from [`DiagnosticsPanelState`], which a host builds from the
    /// real `cad_diagnostics::DiagnosticsModel`. Until a host pushes one, the
    /// drawer shows its explicit empty state rather than fabricated rows.
    pub fn set_diagnostics_state(&self, state: &DiagnosticsPanelState) -> CadResult<()> {
        let rows: Vec<DiagnosticRow> = state
            .rows
            .iter()
            .map(|row| DiagnosticRow {
                code: row.code.clone().into(),
                severity: row.severity.clone().into(),
                description: row.description.clone().into(),
                object: row.object.clone().into(),
                details: row.details.clone().into(),
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let backend = backend_label(&self.messages.borrow().clone(), &state.backend);
        let summary = state.summary.clone();
        let empty = state.empty_label.clone();
        self.with(|ui| {
            ui.set_diagnostics_rows(model);
            ui.set_diagnostics_backend_label(backend.into());
            ui.set_diagnostics_summary_label(summary.into());
            ui.set_diagnostics_empty_label(empty.into());
        })
    }

    /// Open or close the diagnostics drawer without emitting a command.
    pub fn set_diagnostics_open(&self, open: bool) -> CadResult<()> {
        self.with(|ui| ui.set_diagnostics_open(open))
    }

    /// Trigger a redraw without restarting the event loop.
    pub fn request_redraw(&self) -> CadResult<()> {
        self.with(|ui| ui.window().request_redraw())
    }

    /// Current physical size of the window, if it still exists.
    pub fn physical_size(&self) -> Option<slint::PhysicalSize> {
        Some(self.ui.upgrade()?.window().size())
    }

    /// Logical CAD region (excluding chrome) and the window's actual scale.
    pub fn cad_surface_size(&self) -> Option<([f64; 2], f64)> {
        let ui = self.ui.upgrade()?;
        Some((
            [ui.get_cad_width() as f64, ui.get_cad_height() as f64],
            ui.window().scale_factor() as f64,
        ))
    }

    /// Fit the shared shell to the browser CSS viewport, not its preferred size.
    pub fn resize_browser_surface(&self, size: [f64; 2], scale: f64) -> CadResult<()> {
        self.with(|ui| {
            apply_viewer_presentation(ui, self.viewer_config.borrow().effective(), size);
            ui.window().set_size(slint::PhysicalSize::new(
                (size[0] * scale).round().max(1.0) as u32,
                (size[1] * scale).round().max(1.0) as u32,
            ));
            ui.window().request_redraw();
        })
    }
}
