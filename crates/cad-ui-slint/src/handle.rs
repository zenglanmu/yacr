//! handle module.

use super::*;
use cad_app::viewer_config::ViewerConfigStore;

impl UiHandle {
    fn apply_attached_config(
        &self,
        operation: impl FnOnce(&dyn cad_engine::protocol::EngineClient) -> CadResult<()>,
    ) -> Result<bool, cad_app::viewer_config::ConfigError> {
        let client = self.engine_client.borrow().clone();
        let Some(client) = client else {
            return Ok(false);
        };
        operation(client.as_ref()).map_err(|error| cad_app::viewer_config::ConfigError {
            path: "engine".into(),
            reason: error.to_string(),
        })?;
        self.apply_config_snapshot(client.config_snapshot())
            .map_err(|error| cad_app::viewer_config::ConfigError {
                path: "engine".into(),
                reason: error.to_string(),
            })?;
        Ok(true)
    }

    pub(crate) fn apply_config_snapshot(
        &self,
        snapshot: cad_engine::protocol::ConfigSnapshot,
    ) -> CadResult<()> {
        self.viewer_config
            .borrow_mut()
            .set_config(snapshot.effective.clone())
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        *self.config_snapshot.borrow_mut() = Some(snapshot);
        let ui = self.ui.upgrade().ok_or(CadError::Cancelled)?;
        apply_ribbon_config(
            &ui,
            self.viewer_config.borrow().effective(),
            &self.messages.borrow(),
        );
        push_config_properties(&ui, &self.viewer_config.borrow());
        let size = ui.window().size().to_logical(ui.window().scale_factor());
        apply_viewer_presentation_with(
            &ui,
            self.viewer_config.borrow().effective(),
            [size.width as f64, size.height as f64],
            Some(&self.viewer_config.borrow()),
        );
        Ok(())
    }

    pub(crate) fn apply_host_state(&self, state: cad_engine::protocol::HostState) -> CadResult<()> {
        *self.host_state.borrow_mut() = Some(state.clone());
        self.set_open_available(state.capabilities.open)?;
        self.set_new_available(state.capabilities.new)?;
        self.set_save_available(state.capabilities.save)?;
        self.set_plot_available(state.capabilities.plot)?;
        self.set_document_name(&state.document_name)?;
        self.set_mode(state.mode)?;
        self.set_history_availability(state.history)?;
        // Unknown/drawing units must render their explicit label, never empty.
        let unit = state
            .units
            .as_ref()
            .map(cad_domain::UnitContext::label)
            .unwrap_or_else(|| cad_domain::UnitContext::drawing_units().label());
        let mut measurement =
            MeasurementUiState::from_preview(state.measurement_preview.as_ref(), unit);
        measurement.kind = state.selected_measurement_kind;
        self.set_measurement_state(&measurement)?;
        let mut draw = DrawUiState::from_preview(state.draw_preview.as_ref());
        // `Some` updates the retained selection; `None` (idle) leaves the last
        // reported tool untouched instead of resetting it to a synthetic `Line`.
        if let Some(kind) = state.selected_draw_kind {
            self.selected_draw_kind.set(Some(kind));
            draw.kind = kind;
        }
        self.draw_active.set(draw.active);
        self.set_draw_state(&draw)?;
        *self.import_snapshot.borrow_mut() = state.import_progress.clone();
        self.set_import_state(&ImportProgressUiState::from_snapshot(
            state.import_progress.as_ref(),
            &self.messages.borrow(),
        ))?;
        let empty = self.messages.borrow().text("layers.empty", &[]);
        let layers = LayerPanelState::from_rows(&state.layer_rows, empty);
        let ids: Vec<_> = state.layer_rows.iter().map(|row| row.id).collect();
        self.set_layer_state(&layers, &ids)?;
        let active = CadView::space_selection(&state.active_space).ok_or_else(|| {
            CadError::Unsupported("block space cannot be shown in the layout panel".into())
        })?;
        let layouts = LayoutPanelState::from_descriptors(
            &state.layouts,
            active,
            self.messages.borrow().text("layout.empty", &[]),
        );
        let layout_ids = state
            .layouts
            .iter()
            .map(|layout| layout.id)
            .collect::<Vec<_>>();
        self.set_layout_state(&layouts, &layout_ids)?;
        let props = PropertyPanelState::from_properties(
            &state.selection_properties,
            self.messages.borrow().text("properties.empty", &[]),
            |keys| {
                self.messages
                    .borrow()
                    .text("properties.mixed", &[("keys", &keys.join(","))])
            },
        );
        self.set_property_state(&props)?;
        let mut resources = resource_sections(&state.resources, state.import.as_ref())?;
        // A platform wrapper may have published real font facts the core host
        // does not own; a full update that marks them unavailable must not erase
        // them. Other sections follow the host's full update.
        merge_resource_sections(&mut resources, self.resources_sections.borrow().as_ref());
        self.set_resources_sections(&resources)?;
        let backend = self.backend_label();
        let diagnostics = diagnostics_panel(
            &state.diagnostics,
            state.import.as_ref().map(|value| &value.completeness),
            &self.messages.borrow(),
            &backend,
        );
        self.set_diagnostics_state(&diagnostics)?;
        Ok(())
    }

    /// Apply an independent backend lifecycle delta.
    ///
    /// The renderer's real `api` label and outcome are cached and the diagnostics
    /// drawer is re-derived from the cached host state, so the backend label
    /// updates without a full `HostState` push. A later full update does not
    /// clear this cache, and a locale switch keeps it.
    pub(crate) fn apply_backend_state(
        &self,
        state: cad_engine::protocol::RenderBackendState,
    ) -> CadResult<()> {
        *self.backend_state.borrow_mut() = Some(state);
        let cached = self.host_state.borrow().clone();
        if let Some(host) = cached {
            let backend = self.backend_label();
            let diagnostics = diagnostics_panel(
                &host.diagnostics,
                host.import.as_ref().map(|value| &value.completeness),
                &self.messages.borrow(),
                &backend,
            );
            self.set_diagnostics_state(&diagnostics)?;
        }
        Ok(())
    }

    /// The real renderer api label, or empty when no device has been reported.
    fn backend_label(&self) -> String {
        self.backend_state
            .borrow()
            .as_ref()
            .and_then(|state| state.api.clone())
            .unwrap_or_default()
    }

    pub(crate) fn set_status_message(
        &self,
        status: cad_engine::protocol::StatusMessage,
    ) -> CadResult<()> {
        *self.status_message.borrow_mut() = Some(status.clone());
        let text = match status {
            cad_engine::protocol::StatusMessage::ExternalText(text) => text,
            cad_engine::protocol::StatusMessage::Catalog { key, args } => {
                let args = args
                    .iter()
                    .map(|arg| (arg.key.as_str(), arg.value.as_str()))
                    .collect::<Vec<_>>();
                self.messages.borrow().text(&key, &args)
            }
        };
        self.set_status(text)
    }

    /// Surface a refused request through the shared `command.failed` catalog copy
    /// and expand the command row. Used for both synchronous refusals and terminal
    /// async-completion errors.
    pub(crate) fn report_action_error(&self, error: &CadError) -> CadResult<()> {
        let ui = self.ui.upgrade().ok_or(CadError::Cancelled)?;
        ui.set_status_label(
            self.messages
                .borrow()
                .text("command.failed", &[("error", &error.to_string())])
                .into(),
        );
        ui.set_command_expanded(true);
        Ok(())
    }

    /// Host capability and unsaved-state gating, independent of entry visibility.
    pub fn set_open_available(&self, available: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_open(available))
    }
    /// Host capability for starting a new blank drawing (desktop only).
    ///
    /// Hosts without a document factory leave `can-new` false, so the shell and
    /// the command line report an explicit unsupported instead of a no-op.
    pub fn set_new_available(&self, available: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_new(available))
    }
    /// Host capability for vector plot/export (desktop only).
    ///
    /// Hosts without a CPU vector export leave `can-plot` false, so the shell and
    /// the command line report an explicit unsupported instead of a no-op.
    pub fn set_plot_available(&self, available: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_plot(available))
    }
    /// Host capability for lossy Save As (desktop only).
    ///
    /// Hosts without a document writer leave `can-save` false, so the shell and
    /// the command line report an explicit unsupported instead of a no-op. The
    /// desktop host also passes `false` while a document is loading, so Save As
    /// is unavailable during an open (the request itself would be refused busy).
    pub fn set_save_available(&self, available: bool) -> CadResult<()> {
        self.with(|ui| ui.set_can_save(available))
    }
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
                && (20.0..212.0).contains(&y)
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
        let json = serde_json::to_string(&config).map_err(|error| {
            cad_app::viewer_config::ConfigError {
                path: "config".into(),
                reason: error.to_string(),
            }
        })?;
        if self.apply_attached_config(|client| client.set_config_json(&json))? {
            return Ok(());
        }
        let ui = self
            .ui
            .upgrade()
            .ok_or_else(|| cad_app::viewer_config::ConfigError {
                path: "window".into(),
                reason: "window was dropped".into(),
            })?;
        self.viewer_config.borrow_mut().set_config(config)?;
        apply_ribbon_config(
            &ui,
            self.viewer_config.borrow().effective(),
            &self.messages.borrow(),
        );
        let size = ui.window().size().to_logical(ui.window().scale_factor());
        apply_viewer_presentation_with(
            &ui,
            self.viewer_config.borrow().effective(),
            [size.width as f64, size.height as f64],
            Some(&self.viewer_config.borrow()),
        );
        Ok(())
    }
    /// Structured object merge into the host config from JSON (arrays replace,
    /// explicit `false` survives). Atomic: a rejected patch leaves the old value
    /// and revision.
    pub fn update_config_json(
        &self,
        patch_json: &str,
    ) -> Result<(), cad_app::viewer_config::ConfigError> {
        if self.apply_attached_config(|client| client.update_config_json(patch_json))? {
            return Ok(());
        }
        let ui = self
            .ui
            .upgrade()
            .ok_or_else(|| cad_app::viewer_config::ConfigError {
                path: "window".into(),
                reason: "window was dropped".into(),
            })?;
        self.viewer_config
            .borrow_mut()
            .update_config_json(patch_json)?;
        apply_ribbon_config(
            &ui,
            self.viewer_config.borrow().effective(),
            &self.messages.borrow(),
        );
        let size = ui.window().size().to_logical(ui.window().scale_factor());
        apply_viewer_presentation_with(
            &ui,
            self.viewer_config.borrow().effective(),
            [size.width as f64, size.height as f64],
            Some(&self.viewer_config.borrow()),
        );
        Ok(())
    }
    /// Merge a host-allowed user preference JSON (localStorage for the web
    /// host). Paths outside `ui.userCustomization.allowedPaths` are rejected;
    /// capability and component visibility can only be turned off, never on.
    pub fn apply_user_preference_json(
        &self,
        patch_json: &str,
    ) -> Result<(), cad_app::viewer_config::ConfigError> {
        if self.apply_attached_config(|client| client.apply_user_preference_json(patch_json))? {
            return Ok(());
        }
        let ui = self
            .ui
            .upgrade()
            .ok_or_else(|| cad_app::viewer_config::ConfigError {
                path: "window".into(),
                reason: "window was dropped".into(),
            })?;
        self.viewer_config
            .borrow_mut()
            .apply_user_preference_json(patch_json)?;
        apply_ribbon_config(
            &ui,
            self.viewer_config.borrow().effective(),
            &self.messages.borrow(),
        );
        let size = ui.window().size().to_logical(ui.window().scale_factor());
        apply_viewer_presentation_with(
            &ui,
            self.viewer_config.borrow().effective(),
            [size.width as f64, size.height as f64],
            Some(&self.viewer_config.borrow()),
        );
        Ok(())
    }
    /// Replace the whole host config from JSON.
    pub fn set_config_json(
        &self,
        config_json: &str,
    ) -> Result<(), cad_app::viewer_config::ConfigError> {
        if self.apply_attached_config(|client| client.set_config_json(config_json))? {
            return Ok(());
        }
        let ui = self
            .ui
            .upgrade()
            .ok_or_else(|| cad_app::viewer_config::ConfigError {
                path: "window".into(),
                reason: "window was dropped".into(),
            })?;
        self.viewer_config
            .borrow_mut()
            .set_config_json(config_json)?;
        apply_ribbon_config(
            &ui,
            self.viewer_config.borrow().effective(),
            &self.messages.borrow(),
        );
        let size = ui.window().size().to_logical(ui.window().scale_factor());
        apply_viewer_presentation_with(
            &ui,
            self.viewer_config.borrow().effective(),
            [size.width as f64, size.height as f64],
            Some(&self.viewer_config.borrow()),
        );
        Ok(())
    }
    /// Drop every stored user preference and recompute.
    ///
    /// Attached: delegates to the engine client's own store and mirrors the
    /// returned snapshot (the client stays the only writable authority).
    /// Standalone: clears the local store exactly as before.
    pub fn clear_user_preference(&self) -> Result<(), cad_app::viewer_config::ConfigError> {
        if self.apply_attached_config(|client| client.clear_user_preference())? {
            return Ok(());
        }
        let ui = self
            .ui
            .upgrade()
            .ok_or_else(|| cad_app::viewer_config::ConfigError {
                path: "window".into(),
                reason: "window was dropped".into(),
            })?;
        self.viewer_config.borrow_mut().clear_user_preference();
        apply_ribbon_config(
            &ui,
            self.viewer_config.borrow().effective(),
            &self.messages.borrow(),
        );
        let size = ui.window().size().to_logical(ui.window().scale_factor());
        apply_viewer_presentation_with(
            &ui,
            self.viewer_config.borrow().effective(),
            [size.width as f64, size.height as f64],
            Some(&self.viewer_config.borrow()),
        );
        Ok(())
    }
    /// Subscribe to effective-config changes. The observer is a plain Rust
    /// callback; the configuration JSON itself never carries script.
    pub fn on_config_changed(
        &self,
        observer: cad_app::viewer_config::ConfigObserver,
    ) -> Result<(), cad_app::viewer_config::ConfigError> {
        self.viewer_config.borrow_mut().subscribe(observer);
        Ok(())
    }
    /// Current effective revision (bumped only on a real effective change).
    pub fn config_revision(&self) -> u64 {
        self.viewer_config.borrow().revision
    }
    pub fn effective_config(&self) -> cad_app::viewer_config::ViewerConfig {
        self.viewer_config.borrow().effective().clone()
    }
    /// The effective config as JSON, for hosts that expose a query API.
    pub fn effective_config_json(&self) -> String {
        self.viewer_config.borrow().effective_json()
    }
    /// The persisted projection of the user preference as JSON: only leaves still
    /// under the host's `allowedPaths`. Never includes a revoked path.
    pub fn projected_user_preference_json(&self) -> String {
        self.viewer_config.borrow().projected_user_preference_json()
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
        self.view_state.set(state);
        let messages = self.messages.borrow().clone();
        let panel = View3dPanelState::from_view_state(&state, &messages);
        self.with(|ui| {
            ui.set_view_3d(state.is_3d);
            ui.set_view_perspective(state.perspective);
        })?;
        self.set_view3d_state(&panel)
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
        self.with(|ui| {
            ui.set_measurement_active(state.active);
            ui.set_measurement_can_confirm(state.can_confirm);
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
            crate::layer_search::refresh(ui);
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
                scale: row.viewport_scale.clone().unwrap_or_default().into(),
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let active = state.active_index.unwrap_or(-1);
        let empty = state.empty_label.clone();
        let messages = self.messages.borrow().clone();
        let scale_unavailable = crate::status::layout_scale_unavailable_label(&messages);
        let scale_reason = crate::status::layout_scale_control_reason(&messages);
        let scale_available = state.scale_control_available;
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
            ui.set_layout_scale_unavailable_label(scale_unavailable.into());
            ui.set_layout_scale_control_reason(scale_reason.into());
            ui.set_layout_scale_control_available(scale_available);
        })
    }

    /// Push the resources drawer sections (F10) from the host's real reports.
    ///
    /// The raw sections are retained so a live locale switch can re-derive the
    /// localized rows without the host re-collecting them.
    pub fn set_resources_sections(&self, sections: &ResourceSections) -> CadResult<()> {
        *self.resources_sections.borrow_mut() = Some(sections.clone());
        self.push_resources_state()
    }

    /// Push a pre-built resources drawer state (F10).
    pub fn set_resources_state(&self, state: &ResourcesPanelState) -> CadResult<()> {
        let rows: Vec<ResourceRow> = state
            .rows
            .iter()
            .map(|row| ResourceRow {
                section: row.section.clone().into(),
                label: row.label.clone().into(),
                value: row.value.clone().into(),
            })
            .collect();
        let model = slint::ModelRc::new(slint::VecModel::from(rows));
        let empty = if state.empty_label.is_empty() {
            self.messages.borrow().text("resources.empty", &[])
        } else {
            state.empty_label.clone()
        };
        self.with(|ui| {
            ui.set_resources_rows(model);
            ui.set_resources_empty_label(empty.into());
        })
    }

    /// Re-derive and re-push the resources rows for the active catalog.
    fn push_resources_state(&self) -> CadResult<()> {
        let sections = self.resources_sections.borrow().clone();
        let messages = self.messages.borrow().clone();
        let state = match sections.as_ref() {
            Some(sections) => ResourcesPanelState::from_sections(sections, &messages),
            None => ResourcesPanelState::default(),
        };
        self.set_resources_state(&state)
    }

    /// Open or close the resources drawer without emitting a command.
    pub fn set_resources_open(&self, open: bool) -> CadResult<()> {
        self.with(|ui| ui.set_resources_open(open))
    }

    /// Push the 3D observation drawer state (F13).
    ///
    /// Derived from [`ViewStateUi`] by [`View3dPanelState::from_view_state`]; the
    /// drawer never invents a standard view or an orbit state the camera is not
    /// actually in.
    pub fn set_view3d_state(&self, state: &View3dPanelState) -> CadResult<()> {
        let standard_index = state.standard_view_index.unwrap_or(-1);
        let labels = string_model(&state.standard_view_labels);
        let orbit = state.orbit_status.clone();
        self.with(|ui| {
            ui.set_view_standard_index(standard_index);
            ui.set_standard_view_labels(labels);
            ui.set_view_orbit_status(orbit.into());
        })
    }

    /// Open or close the 3D observation drawer without emitting a command.
    pub fn set_view3d_open(&self, open: bool) -> CadResult<()> {
        self.with(|ui| ui.set_view3d_open(open))
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
        let override_label = layer_override_label(&messages, override_count);
        let selected_label = selected_count_label(&messages, selection_count);
        let work = self.work_mode.get();
        self.with(|ui| {
            apply_chrome(ui, &messages, work);
            apply_ribbon_config(ui, self.viewer_config.borrow().effective(), &messages);
            ui.set_layer_override_label(override_label.into());
            ui.set_property_selected_label(selected_label.into());
        })?;
        self.refresh_import_labels(&messages)?;
        // Re-derive the drawers whose rows are localized from the raw source
        // snapshots, so a live locale switch never leaves stale-language rows.
        self.push_resources_state()?;
        let view_state = self.view_state.get();
        let panel = View3dPanelState::from_view_state(&view_state, &messages);
        self.set_view3d_state(&panel)?;
        // Clone out of the RefCells before re-entering their setters: holding a
        // `Ref` across `set_status_message` (which borrows the same cell) panics.
        let cached_status = self.status_message.borrow().clone();
        if let Some(status) = cached_status {
            self.set_status_message(status)?;
        }
        let cached_host_state = self.host_state.borrow().clone();
        if let Some(state) = cached_host_state {
            let backend = self.backend_label();
            let diagnostics = diagnostics_panel(
                &state.diagnostics,
                state.import.as_ref().map(|value| &value.completeness),
                &messages,
                &backend,
            );
            self.set_diagnostics_state(&diagnostics)?;
        }
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

    pub fn set_diagnostics_backend(&self, backend: &str) -> CadResult<()> {
        let label = backend_label(&self.messages.borrow(), backend);
        self.with(|ui| ui.set_diagnostics_backend_label(label.into()))
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

    /// Reclassify native window layout without setting its size or changing the CAD camera.
    pub fn refresh_window_layout(&self) -> CadResult<()> {
        self.with(|ui| {
            let size = ui.window().size().to_logical(ui.window().scale_factor());
            apply_viewer_presentation_with(
                ui,
                self.viewer_config.borrow().effective(),
                [size.width as f64, size.height as f64],
                Some(&self.viewer_config.borrow()),
            );
        })
    }

    /// Fit the shared shell to the browser CSS viewport, not its preferred size.
    pub fn resize_browser_surface(&self, size: [f64; 2], scale: f64) -> CadResult<()> {
        self.with(|ui| {
            apply_viewer_presentation_with(
                ui,
                self.viewer_config.borrow().effective(),
                size,
                Some(&self.viewer_config.borrow()),
            );
            ui.window().set_size(slint::PhysicalSize::new(
                (size[0] * scale).round().max(1.0) as u32,
                (size[1] * scale).round().max(1.0) as u32,
            ));
            ui.window().request_redraw();
        })
    }
}

fn resource_sections(
    resources: &cad_engine::protocol::ResourceState,
    import: Option<&cad_engine::protocol::ImportState>,
) -> CadResult<ResourceSections> {
    let fonts = resources.fonts.as_ref().map(|font| FontResourceSummary {
        catalog_entries: font.catalog_entries,
        requested: font.requested.len(),
        planned: font.planned.len(),
        registered: font.registered.len(),
        failed: font.failed.clone(),
        unresolved: font.unresolved.clone(),
        default_face: font.default_face.clone(),
    });
    let import = import
        .map(|report| {
            let count = report.diagnostics.len();
            let completeness = match &report.completeness {
                cad_domain::Completeness::Complete => ResourceCompleteness::Complete,
                cad_domain::Completeness::Partial(_) => ResourceCompleteness::Partial(count),
                cad_domain::Completeness::Missing(_) => ResourceCompleteness::Missing(count),
                cad_domain::Completeness::Unverified => ResourceCompleteness::Unverified,
            };
            let parse_ms = report
                .parse_ms
                .map(|value| {
                    if value.is_finite() && value >= 0.0 {
                        Ok(value.min(u64::MAX as f64) as u64)
                    } else {
                        Err(CadError::InvalidInput(
                            "import parse time must be finite and non-negative".into(),
                        ))
                    }
                })
                .transpose()?;
            Ok::<_, CadError>(ImportResourceSummary {
                identity: report.identity.short(),
                dwg_version: String::new(),
                completeness,
                diagnostics: Some(count),
                parse_ms,
            })
        })
        .transpose()?;
    let proxy = resources.proxy.as_ref().map(|proxy| ProxyResourceSummary {
        decoded_records: proxy.decoded_records,
        unsupported: proxy
            .unsupported
            .iter()
            .map(|record| ProxyUnsupportedRow {
                record_type: record.record_type,
                reason: record.reason.clone(),
                bytes: record.bytes,
            })
            .collect(),
    });
    Ok(ResourceSections {
        fonts,
        import,
        proxy,
        references: resources
            .references
            .as_ref()
            .map(|keys| ReferencesResourceSummary { keys: keys.clone() }),
        images_modeled: resources.images_modeled,
    })
}

/// Localized rendering of one raw diagnostic parameter, mirroring
/// [`status::parameter_text`] for the protocol's raw parameter shape.
fn raw_parameter_text(
    messages: &MessageSource,
    parameter: &cad_engine::protocol::RawDiagnosticParameter,
) -> String {
    use cad_engine::protocol::RawDiagnosticParameter as P;
    let (key, value) = match parameter {
        P::Object(id) => ("diagnostic.param.object", id.0.to_string()),
        P::Key(value) => ("diagnostic.param.key", value.clone()),
        P::Identifier(value) => ("diagnostic.param.identifier", value.clone()),
        P::Count(value) => ("diagnostic.param.count", value.to_string()),
        P::Bytes(value) => ("diagnostic.param.bytes", value.to_string()),
        P::Limit(value) => ("diagnostic.param.limit", value.to_string()),
        P::Depth(value) => ("diagnostic.param.depth", value.to_string()),
    };
    messages.text(key, &[("value", &value)])
}

/// Build the diagnostics drawer from the raw protocol rows.
///
/// Reuses the existing catalog helpers (`reason_text`/`severity_text`) so an
/// unknown code still shows the explicit `diagnostics.unknown_code` fallback and
/// a locale switch re-localizes without a host re-push. The summary is aggregated
/// through the shared [`cad_diagnostics::DiagnosticsModel`] over the real
/// per-reason completeness, then combined with the document import verdict, so a
/// `Complete` import that still carries a `Missing` image/decode reason is never
/// reported as complete.
fn diagnostics_panel(
    diagnostics: &[cad_engine::protocol::RawDiagnostic],
    import_completeness: Option<&cad_domain::Completeness>,
    messages: &MessageSource,
    backend: &str,
) -> DiagnosticsPanelState {
    use cad_engine::protocol::DiagnosticSeverity;
    let separator = messages.text("diagnostics.detail.separator", &[]);
    let rows = diagnostics
        .iter()
        .map(|diagnostic| {
            let description = match status::reason_text(messages, &diagnostic.code) {
                status::ReasonText::Found(text) | status::ReasonText::Unknown(text) => text,
            };
            // Import diagnostics default to Warning when the source did not
            // classify severity, matching the pre-decoupling host projection.
            let severity = status::severity_text(
                messages,
                match diagnostic.severity.unwrap_or(DiagnosticSeverity::Warning) {
                    DiagnosticSeverity::Info => cad_diagnostics::model::Severity::Info,
                    DiagnosticSeverity::Warning => cad_diagnostics::model::Severity::Warning,
                    DiagnosticSeverity::Error => cad_diagnostics::model::Severity::Error,
                },
            );
            let object = diagnostic
                .object
                .map(|id| messages.text("diagnostic.param.object", &[("value", &id.0.to_string())]))
                .unwrap_or_default();
            let details = diagnostic
                .parameters
                .iter()
                .map(|parameter| raw_parameter_text(messages, parameter))
                .collect::<Vec<_>>()
                .join(&separator);
            DiagnosticRowUi {
                code: diagnostic.code.clone(),
                severity,
                description,
                object,
                details,
            }
        })
        .collect();
    // Aggregate every real reason's completeness with the existing model rules
    // (never a hand-rolled ordering). An unclassified row keeps the
    // pre-decoupling `Partial`/`Warning` fallback and is never promoted to
    // `Missing`.
    let mut model = cad_diagnostics::DiagnosticsModel::new();
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity.unwrap_or(DiagnosticSeverity::Warning) {
            DiagnosticSeverity::Info => cad_diagnostics::model::Severity::Info,
            DiagnosticSeverity::Warning => cad_diagnostics::model::Severity::Warning,
            DiagnosticSeverity::Error => cad_diagnostics::model::Severity::Error,
        };
        let completeness = diagnostic
            .completeness
            .clone()
            .unwrap_or(cad_domain::Completeness::Partial(Vec::new()));
        let reason = cad_diagnostics::DiagnosticReason::new(
            diagnostic.code.clone(),
            severity,
            completeness,
            Vec::new(),
        );
        match diagnostic.object {
            Some(object) => model.add(object, reason),
            None => model.add_document(reason),
        }
    }
    let mut summary = model.summary();
    // Fold the document import verdict into the aggregated verdict.
    if let Some(import) = import_completeness {
        let combined = summary.completeness.clone().combine(import.clone());
        summary.completeness = combined;
        summary.complete = matches!(summary.completeness, cad_domain::Completeness::Complete);
    }
    // `summary_text` expresses a verdict through object counts. The model's
    // counts are real object counts; a document-level verdict (the import
    // record) has no object entry, so it only contributes a minimal `max(1)`
    // placeholder. Never treat `Completeness::reasons().len()` as an object
    // count: several reasons on one object (or two import keys) are still one
    // verdict, not N objects.
    match &summary.completeness {
        cad_domain::Completeness::Partial(_) => {
            summary.partial_objects = summary.partial_objects.max(1);
        }
        cad_domain::Completeness::Missing(_) => {
            summary.missing_objects = summary.missing_objects.max(1);
        }
        cad_domain::Completeness::Unverified => {
            summary.unverified_objects = summary.unverified_objects.max(1);
        }
        cad_domain::Completeness::Complete => {}
    }
    let summary = DiagnosticsPanelState::summary_text(messages, &summary);
    DiagnosticsPanelState {
        rows,
        summary,
        // The real renderer api label (from the backend delta), never the
        // requested preference; empty only when no device has been reported.
        backend: backend.to_string(),
        empty_label: messages.text("diagnostics.empty", &[]),
    }
}

/// Preserve a platform-published font section when a full update reports it
/// unavailable, without fabricating any data the host did not publish.
///
/// Fonts are installed once by the platform wrapper and are document-independent
/// (Android keeps them across updates; the Web overlay must do the same). Other
/// sections are document-scoped and follow the host's full update.
fn merge_resource_sections(incoming: &mut ResourceSections, cached: Option<&ResourceSections>) {
    let Some(cached) = cached else {
        return;
    };
    if incoming.fonts.is_none() {
        incoming.fonts = cached.fonts.clone();
    }
}

/// Mirror the resolved config into the shell's data-only readout properties.
///
/// These are the exact values the pure [`cad_app::viewer_config::UiPresentationModel`]
/// derives (not a second interpretation) plus a JSON snapshot for hosts/tests.
pub(crate) fn push_config_properties(ui: &YacrWindow, store: &ViewerConfigStore) {
    let effective = store.effective();
    let p = cad_app::viewer_config::UiPresentationModel::resolve(
        effective,
        [1280.0, 800.0],
        [0.0; 4],
        false,
    );
    ui.set_config_revision(store.revision as i32);
    ui.set_config_preset(preset_key(effective.ui.preset).into());
    ui.set_overlay_axes(p.overlays.axes);
    ui.set_overlay_grid(p.overlays.grid);
    ui.set_overlay_selection_highlight(p.overlays.selection_highlight);
    ui.set_overlay_snap_hints(p.overlays.snap_hints);
    ui.set_feature_measure(p.features.measure);
    ui.set_config_effective_json(store.effective_json().into());
}

fn preset_key(preset: cad_app::viewer_config::Preset) -> &'static str {
    match preset {
        cad_app::viewer_config::Preset::Full => "full",
        cad_app::viewer_config::Preset::Minimal => "minimal",
        cad_app::viewer_config::Preset::CanvasOnly => "canvasOnly",
    }
}

#[cfg(test)]
mod diagnostic_summary_contracts {
    use super::*;
    use cad_domain::{Completeness, ObjectId};
    use cad_engine::protocol::RawDiagnostic;

    fn raw(code: &str, completeness: Option<Completeness>) -> RawDiagnostic {
        RawDiagnostic {
            object: None,
            code: code.into(),
            message: String::new(),
            severity: None,
            completeness,
            parameters: Vec::new(),
        }
    }

    fn raw_object(object: u128, code: &str, completeness: Option<Completeness>) -> RawDiagnostic {
        RawDiagnostic {
            object: Some(ObjectId(object)),
            ..raw(code, completeness)
        }
    }

    #[test]
    fn complete_import_with_a_missing_image_summarizes_missing() {
        let messages = MessageSource::for_locale(Locale::ZhCn);
        let diagnostics = [raw(
            "resource.missing",
            Some(Completeness::Missing(vec!["missing-image".into()])),
        )];
        let panel = diagnostics_panel(
            &diagnostics,
            Some(&Completeness::Complete),
            &messages,
            "vulkan",
        );
        assert_eq!(
            panel.summary,
            messages.text("diagnostics.summary.missing", &[("count", "1")]),
            "a Complete import with a missing image must not summarize as complete"
        );
        assert_ne!(
            panel.summary,
            messages.text("diagnostics.summary.complete", &[])
        );
        // The raw machine key stays visible on the row.
        assert_eq!(panel.rows[0].code, "resource.missing");
    }

    #[test]
    fn decode_failure_missing_summarizes_missing() {
        let messages = MessageSource::for_locale(Locale::En);
        let diagnostics = [raw(
            "proxy.undecoded",
            Some(Completeness::Missing(vec!["record".into()])),
        )];
        let panel = diagnostics_panel(&diagnostics, Some(&Completeness::Complete), &messages, "");
        assert_eq!(
            panel.summary,
            messages.text("diagnostics.summary.missing", &[("count", "1")])
        );
    }

    #[test]
    fn unclassified_reason_is_partial_never_fake_missing() {
        // A raw reason with no verdict must fall back to the pre-decoupling
        // Partial/Warning projection, never a fabricated Missing.
        let messages = MessageSource::for_locale(Locale::En);
        let diagnostics = [raw("import.unknown_entity", None)];
        let panel = diagnostics_panel(&diagnostics, Some(&Completeness::Complete), &messages, "");
        assert_eq!(
            panel.summary,
            messages.text("diagnostics.summary.partial", &[("count", "1")])
        );
    }

    #[test]
    fn missing_outranks_partial_when_both_are_present() {
        let messages = MessageSource::for_locale(Locale::En);
        let diagnostics = [
            raw(
                "import.unknown_entity",
                Some(Completeness::Partial(Vec::new())),
            ),
            raw("resource.missing", Some(Completeness::Missing(Vec::new()))),
        ];
        let panel = diagnostics_panel(&diagnostics, Some(&Completeness::Complete), &messages, "");
        assert_eq!(
            panel.summary,
            messages.text("diagnostics.summary.missing", &[("count", "1")]),
            "the weakest (Missing) verdict must win"
        );
    }

    #[test]
    fn locale_switch_keeps_raw_code_and_localizes_summary() {
        let diagnostics = [raw(
            "resource.missing",
            Some(Completeness::Missing(vec!["missing-image".into()])),
        )];
        let zh = diagnostics_panel(
            &diagnostics,
            Some(&Completeness::Complete),
            &MessageSource::for_locale(Locale::ZhCn),
            "",
        );
        let en = diagnostics_panel(
            &diagnostics,
            Some(&Completeness::Complete),
            &MessageSource::for_locale(Locale::En),
            "",
        );
        assert_eq!(
            zh.rows[0].code, en.rows[0].code,
            "raw key is locale-independent"
        );
        assert_eq!(zh.rows[0].code, "resource.missing");
        assert_ne!(
            zh.summary, en.summary,
            "the summary is localized but the raw code is not"
        );
    }

    #[test]
    fn two_reasons_on_one_object_count_as_one_object() {
        // Same object, two Partial reasons: the summary must report one object,
        // never `reasons.len()` (which would inflate it to two).
        let messages = MessageSource::for_locale(Locale::En);
        let diagnostics = [
            raw_object(
                7,
                "representation.approximate",
                Some(Completeness::Partial(Vec::new())),
            ),
            raw_object(
                7,
                "render.lineweight_not_drawn",
                Some(Completeness::Partial(Vec::new())),
            ),
        ];
        let panel = diagnostics_panel(&diagnostics, Some(&Completeness::Complete), &messages, "");
        assert_eq!(
            panel.summary,
            messages.text("diagnostics.summary.partial", &[("count", "1")]),
            "two reasons on one object are still one object"
        );
    }

    #[test]
    fn multiple_real_objects_keep_their_object_count() {
        let messages = MessageSource::for_locale(Locale::En);
        let diagnostics = [
            raw_object(
                1,
                "resource.missing",
                Some(Completeness::Missing(Vec::new())),
            ),
            raw_object(
                2,
                "resource.missing",
                Some(Completeness::Missing(Vec::new())),
            ),
            raw_object(
                3,
                "resource.missing",
                Some(Completeness::Missing(Vec::new())),
            ),
        ];
        let panel = diagnostics_panel(&diagnostics, Some(&Completeness::Complete), &messages, "");
        assert_eq!(
            panel.summary,
            messages.text("diagnostics.summary.missing", &[("count", "3")]),
            "three distinct objects must report three objects"
        );
    }
}
