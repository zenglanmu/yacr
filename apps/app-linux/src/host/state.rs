//! Fallible state funnel. A failed query is not replaced with an empty success.
use super::*;
use cad_ui_slint::{
    DiagnosticsPanelState, LayerPanelState, LayoutPanelState, MeasurementUiState,
    PropertyPanelState,
};

impl Runtime {
    pub(super) fn sync_view(&self) -> CadResult<()> {
        let c = self.controller.borrow();
        let viewport = c
            .application
            .workspace
            .viewports
            .get(&c.viewport_id)
            .ok_or(CadError::Cancelled)?;
        if let Some(view) = self.view.borrow().as_ref() {
            view.sync_session(&c.session.active_space, viewport);
        }
        if let Some(handle) = self.handle.borrow().as_ref() {
            handle.set_diagnostics_backend(
                &self
                    .view
                    .borrow()
                    .as_ref()
                    .and_then(|view| view.backend_label())
                    .unwrap_or_default(),
            )?;
            handle.set_view_state(cad_ui_slint::ViewStateUi {
                is_3d: matches!(viewport.view_mode, cad_app::ViewMode2d3d::ThreeD { .. }),
                perspective: matches!(
                    viewport.camera.projection,
                    cad_app::Projection::Perspective { .. }
                ),
            })?;
        }
        Ok(())
    }
    pub(super) fn push(&self) -> CadResult<()> {
        self.sync_view()?;
        let c = self.controller.borrow();
        let handles = self.handle.borrow();
        let handle = handles.as_ref().ok_or(CadError::Cancelled)?;
        handle.set_open_available(
            (!self.options.headless || self.options.drawing.is_some()) && !self.loading(),
        )?;
        handle.set_document_name(&c.document_name_hint)?;
        handle.set_history_availability(c.history_availability())?;
        handle.set_mode(c.mode())?;
        let measurement =
            MeasurementUiState::from_preview(c.measurement_preview().as_ref(), c.unit_label());
        handle.set_measurement_state(&measurement)?;
        let layers = c.layer_rows()?;
        handle.set_layer_state(
            &LayerPanelState::from_rows(&layers, self.message("layers.empty", &[])),
            &layers.iter().map(|r| r.id).collect::<Vec<_>>(),
        )?;
        let messages = cad_ui_slint::MessageSource::from_request(&self.options.locale);
        handle.set_property_state(&PropertyPanelState::from_properties(
            &c.selection_properties()?,
            messages.text("properties.empty", &[]),
            |keys| messages.text("properties.mixed", &[("keys", &keys.join(","))]),
        ))?;
        let drawing = c.drawing().ok_or(CadError::Cancelled)?;
        let layouts = cad_ui_slint::layout_descriptors(&drawing);
        let space = CadView::space_selection(&c.session.active_space).ok_or(CadError::Cancelled)?;
        handle.set_layout_state(
            &LayoutPanelState::from_descriptors(
                &layouts,
                space,
                messages.text("layout.empty", &[]),
            ),
            &layouts.iter().map(|r| r.id).collect::<Vec<_>>(),
        )?;
        let mut diagnostics = cad_diagnostics::DiagnosticsModel::new();
        if let Some(report) = &c.last_import_report {
            for diagnostic in &report.diagnostics {
                let reason = cad_diagnostics::model::DiagnosticReason::new(
                    diagnostic.code.clone(),
                    cad_diagnostics::model::Severity::Warning,
                    cad_domain::Completeness::Partial(vec![diagnostic.code.clone()]),
                    Vec::new(),
                );
                if let Some(object) = diagnostic.object {
                    diagnostics.add(object, reason);
                } else {
                    diagnostics.add_document(reason);
                }
            }
        }
        handle.set_diagnostics_state(&DiagnosticsPanelState::from_model(
            &diagnostics,
            &messages,
            self.view
                .borrow()
                .as_ref()
                .and_then(|v| v.backend_label())
                .unwrap_or_default(),
        ))?;
        if let Some(view) = self.view.borrow().as_ref() {
            view.set_overlay_visibility(handle.effective_config().view.overlays.into());
            view.sync_drawing(Some(drawing));
            view.set_layer_overrides(c.session.layer_overrides.clone());
            view.set_selection_highlight(c.selection().clone());
            view.set_measurement_preview(c.measurement_preview());
            view.set_snap_hints(c.snap_hints_near_cursor()?);
            view.request_redraw();
        }
        Ok(())
    }
}
