//! Fallible state funnel. A failed query is not replaced with an empty success.
use super::*;
use cad_ui_slint::{
    DiagnosticsPanelState, ImportResourceSummary, LayerPanelState, LayoutPanelState,
    MeasurementUiState, PropertyPanelState, ReferencesResourceSummary, ResourceCompleteness,
    ResourceSections,
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
                standard_view: cad_ui_slint::standard_view_for_camera(&viewport.camera),
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
        // Desktop hosts a document factory, so New is available unless an open
        // is in flight; web/android never call this and keep `can-new` false.
        handle.set_new_available(!self.loading())?;
        // Desktop renders CPU vector plots; web/android leave `can-plot` false.
        handle.set_plot_available(!self.loading())?;
        // Desktop performs a lossy DXF Save As; web/android leave `can-save` false.
        handle.set_save_available(!self.loading())?;
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
        // Raster images that did not resolve or decode are document-level
        // reasons, never a silent drop. `resource.missing` is localized in both
        // catalogs, so the drawer shows the real missing key.
        if let Some(report) = self.last_image_report.borrow().as_ref() {
            for key in &report.unresolved {
                diagnostics.add_document(cad_diagnostics::model::DiagnosticReason::missing(
                    cad_diagnostics::model::codes::RESOURCE_MISSING,
                    vec![cad_diagnostics::model::DiagnosticParameter::Key(
                        key.clone(),
                    )],
                ));
            }
            for failure in &report.failed {
                diagnostics.add_document(cad_diagnostics::model::DiagnosticReason::missing(
                    cad_diagnostics::model::codes::RESOURCE_MISSING,
                    vec![cad_diagnostics::model::DiagnosticParameter::Key(
                        failure.clone(),
                    )],
                ));
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
        // Resources drawer (F10): real sections only. The desktop font loader
        // (`cad_platform::fonts::local::load_engine`) discards its
        // `FontLoadReport`, so `fonts` is not pushed here — the drawer shows its
        // explicit empty state rather than a fabricated summary. Proxy records
        // are not surfaced by the importer in this build either.
        handle.set_resources_sections(&ResourceSections {
            fonts: None,
            import: c
                .last_import_report
                .as_ref()
                .map(|report| ImportResourceSummary {
                    identity: c.document_name_hint.clone(),
                    dwg_version: String::new(),
                    completeness: match &report.completeness {
                        cad_domain::Completeness::Complete => ResourceCompleteness::Complete,
                        cad_domain::Completeness::Partial(items) => {
                            ResourceCompleteness::Partial(items.len())
                        }
                        cad_domain::Completeness::Missing(items) => {
                            ResourceCompleteness::Missing(items.len())
                        }
                        cad_domain::Completeness::Unverified => ResourceCompleteness::Unverified,
                    },
                    diagnostics: Some(report.diagnostics.len()),
                    parse_ms: report.parse_ms.map(|ms| ms.max(0.0) as u64),
                }),
            proxy: None,
            references: Some(ReferencesResourceSummary {
                keys: c
                    .application
                    .workspace
                    .documents
                    .get(&c.document_id)
                    .map(|document| document.resource_keys.clone())
                    .unwrap_or_default(),
            }),
            images_modeled: true,
        })?;
        // The 3D observation drawer is pushed by `sync_view` through
        // `set_view_state` (it also re-derives `set_view3d_state`), so the drawer
        // reflects the real viewport. `set_viewport_scale` is intentionally not
        // called: the layout descriptors carry no scale, so there is no real
        // value to push (the panel shows its explicit "scale unavailable" state).
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
