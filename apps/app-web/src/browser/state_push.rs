//! Single funnel that pushes derived application state into the Slint shell.
//!
//! Every command, document open, annotation import/export and startup restore
//! calls [`push_panel_state`] so no host path can leave a panel stale. The
//! projection from the authoritative `cad-app` getters into `cad-ui-slint` panel
//! snapshots is factored into pure functions ([`derive_layer_state`], …) so the
//! mapping is unit-tested without a Slint host.
//!
//! Doctrine: panels are *derived*, never authored here. A getter that errors or
//! has no data leaves the panel at its explicit empty state rather than showing
//! fabricated rows. All user-facing labels come from the shared catalog
//! (`super::messages`), never from Rust literals.

use std::cell::RefCell;
use std::rc::Rc;

use cad_app::host::HostController;
use cad_app::layers::LayerRow;
use cad_app::{AnnotationPreview, AnnotationRow, MeasurementPreview, SelectionProperties};
use cad_diagnostics::model::{DiagnosticReason, DiagnosticsModel, Severity};
use cad_domain::{AnnotationId, Completeness, Diagnostic, LayerId, LayoutId};
use cad_ui_slint::{
    AnnotationPanelState, CadView, DiagnosticsPanelState, LayerPanelState, LayoutPanelState,
    MeasurementUiState, MessageSource, PropertyPanelState, UiHandle,
};

use super::async_open as async_open_state;
use super::messages::current_messages;

/// Layer panel state plus the ordered ids the adapter maps row indices through.
pub(super) fn derive_layer_state(
    rows: &[LayerRow],
    messages: &MessageSource,
) -> (LayerPanelState, Vec<LayerId>) {
    let order = rows.iter().map(|row| row.id).collect();
    (
        LayerPanelState::from_rows(rows, messages.text("layers.empty", &[])),
        order,
    )
}

/// Read-only property panel state for the current selection.
pub(super) fn derive_property_state(
    properties: &SelectionProperties,
    messages: &MessageSource,
) -> PropertyPanelState {
    PropertyPanelState::from_properties(
        properties,
        messages.text("properties.empty", &[]),
        |keys| messages.text("properties.mixed", &[("keys", &keys.join(","))]),
    )
}

/// Annotation panel state plus the ordered ids the adapter maps row indices
/// through. The `preview` drives the active-tool step text, if any.
pub(super) fn derive_annotation_state(
    rows: &[AnnotationRow],
    preview: Option<&AnnotationPreview>,
    messages: &MessageSource,
) -> (AnnotationPanelState, Vec<AnnotationId>) {
    let order = rows.iter().map(|row| row.id).collect();
    (
        AnnotationPanelState::from_rows(rows, preview, messages.text("annotation.empty", &[])),
        order,
    )
}

/// Layout (paper-space) panel state plus the ordered ids for switching.
pub(super) fn derive_layout_state(
    descriptors: &[cad_representation::LayoutDescriptor],
    active: cad_representation::SpaceSelection,
    messages: &MessageSource,
) -> (LayoutPanelState, Vec<LayoutId>) {
    let order = descriptors.iter().map(|d| d.id).collect();
    (
        LayoutPanelState::from_descriptors(descriptors, active, messages.text("layout.empty", &[])),
        order,
    )
}

/// Map an import report's plain [`Diagnostic`]s into the structured drawer model.
///
/// The import report carries a stable machine `code` for every reason but no
/// severity or typed parameters, so this mapping is deliberately conservative:
/// the code is preserved verbatim (the drawer localizes it from the catalog),
/// parameters are left empty rather than invented, and the severity word is a
/// display-only heuristic from the code family (`unavailable`/`missing`/
/// `not_implemented`/`undecoded` are errors, everything else a warning). It
/// never adds a row the report did not contain.
pub(super) fn diagnostics_model_from_import(diagnostics: &[Diagnostic]) -> DiagnosticsModel {
    let mut model = DiagnosticsModel::new();
    for diagnostic in diagnostics {
        let severity = severity_for_code(&diagnostic.code);
        let reason = DiagnosticReason::new(
            diagnostic.code.clone(),
            severity,
            completeness_for_severity(severity),
            Vec::new(),
        );
        match diagnostic.object {
            Some(object) => model.add(object, reason),
            None => model.add_document(reason),
        }
    }
    model
}

/// Display-only severity word for a stable diagnostic code.
fn severity_for_code(code: &str) -> Severity {
    const ERROR_SUFFIXES: [&str; 4] =
        [".unavailable", ".missing", ".not_implemented", ".undecoded"];
    if ERROR_SUFFIXES.iter().any(|suffix| code.ends_with(suffix)) {
        Severity::Error
    } else {
        Severity::Warning
    }
}

/// Completeness implied by the heuristic severity, so the drawer summary is not
/// "complete" while a reason is shown.
fn completeness_for_severity(severity: Severity) -> Completeness {
    match severity {
        Severity::Error => Completeness::Missing(Vec::new()),
        Severity::Warning => Completeness::Partial(Vec::new()),
        Severity::Info => Completeness::Unverified,
    }
}

/// A one-slot `Rc<RefCell<Option<CadView>>>` around a borrowed view.
///
/// Host code frequently holds a `&CadView` while the shared signature wants the
/// slot the sink/view-input use; the clone is cheap (the view is `Rc`-backed).
pub(super) fn view_slot(view: &CadView) -> Rc<RefCell<Option<CadView>>> {
    Rc::new(RefCell::new(Some(view.clone())))
}

/// The triple of transient overlay inputs the funnel pushes into `CadView`.
///
/// Factored out of the funnel so the mapping is unit-testable without a Slint
/// host: an empty selection and an idle tool must produce the explicit empty
/// value (`SelectionSet::new()` / `None`), which clears the overlay rather than
/// leaving the previous one drawn.
pub(super) struct OverlayPush {
    pub selection: cad_app::SelectionSet,
    pub measurement: Option<MeasurementPreview>,
    pub annotation: Option<AnnotationPreview>,
}

/// Read the authoritative overlay inputs from the controller.
pub(super) fn derive_overlay_push(controller: &HostController) -> OverlayPush {
    OverlayPush {
        selection: controller.selection().clone(),
        measurement: controller.measurement_preview(),
        annotation: controller.annotation_preview(),
    }
}

/// Push every derived panel state into the shell in one place.
///
/// Called after every command execute, document open and annotation mutation.
/// The history push replaces the old `set_can_undo`-only calls: it writes both
/// undo and redo so undoing to empty leaves `can_redo` stale-free (audit U11).
///
/// It is also the **only** path that feeds the transient render overlay: the
/// selection highlight and the active tool previews are pushed here so a
/// command, a selection pick and a cancel all reach the GPU through the same
/// funnel. A cleared selection or a stopped tool pushes the explicit empty
/// value (`SelectionSet::new()` / `None`), which removes the overlay rather than
/// leaving the previous one on screen.
pub(super) fn push_panel_state(
    controller: &Rc<RefCell<HostController>>,
    handle: &UiHandle,
    view: &Rc<RefCell<Option<CadView>>>,
) {
    let messages = current_messages();
    let backend = backend_display(view);

    // Asynchronous open progress panel (F01): derived from the retained
    // controller snapshot (worker host) or the synchronous fallback terminal.
    // Pushed here, in the single funnel, so every command/open re-renders it —
    // including a `CancelLoading` cancel. Uses the shared `Rc` before the
    // long-lived borrow below shadows it.
    async_open_state::push_state(controller, handle);

    let controller = controller.borrow();

    // Transient render overlay: selection highlight + tool previews. Kept in the
    // funnel (not at call sites) so no command path can forget it; an empty
    // selection or idle tool clears the overlay with the explicit empty value.
    if let Some(view) = view.borrow().as_ref() {
        view.sync_drawing(controller.drawing());
        let overlay = derive_overlay_push(&controller);
        view.set_selection_highlight(overlay.selection);
        view.set_measurement_preview(overlay.measurement);
        view.set_annotation_preview(overlay.annotation);
    }

    // Undo/redo: one snapshot drives both flags.
    let _ = handle.set_history_availability(controller.history_availability());

    // Measurement panel: active preview + unit context. The save-as-annotation
    // affordance is enabled only when a confirmed record exists (F06/F07).
    let measurement = controller.measurement_preview();
    let has_record = controller.has_last_measurement();
    let mut measurement_state =
        MeasurementUiState::from_preview(measurement.as_ref(), controller.unit_label());
    measurement_state.set_can_save_annotation(has_record);
    let _ = handle.set_measurement_state(&measurement_state);

    // Session mode (audit U02): one call writes both the work flag and the
    // catalog label from the authoritative session, so the shell can never show
    // a mode the command layer does not enforce.
    let _ = handle.set_mode(controller.mode());

    // Layer panel: real database rows + session overrides.
    if let Ok(rows) = controller.layer_rows() {
        let (state, order) = derive_layer_state(&rows, &messages);
        let _ = handle.set_layer_state(&state, &order);
    }

    // Properties panel: read-only projection of the current selection.
    if let Ok(properties) = controller.selection_properties() {
        let _ = handle.set_property_state(&derive_property_state(&properties, &messages));
    }

    // Annotation management + active tool.
    if let Ok(rows) = controller.annotation_rows() {
        let preview = controller.annotation_preview();
        let (state, order) = derive_annotation_state(&rows, preview.as_ref(), &messages);
        let _ = handle.set_annotation_state(&state, &order);
    }

    // Layout panel: the database's real layout table, with the active space.
    if let Some(drawing) = controller.drawing() {
        let descriptors = cad_app::render_scene::layout_descriptors(drawing.as_ref());
        let active = CadView::space_selection(&controller.session.active_space)
            .unwrap_or(cad_representation::SpaceSelection::Model);
        let (state, order) = derive_layout_state(&descriptors, active, &messages);
        let _ = handle.set_layout_state(&state, &order);
    }

    // Diagnostics drawer: an explicit empty model before any import report.
    let model = controller
        .last_import_report
        .as_ref()
        .map(|report| diagnostics_model_from_import(&report.diagnostics))
        .unwrap_or_default();
    let _ = handle.set_diagnostics_state(&DiagnosticsPanelState::from_model(
        &model, &messages, backend,
    ));

    if let Some(view) = view.borrow().as_ref() {
        view.request_redraw();
    }
}

/// Convenience wrapper for callers that hold a `&CadView` instead of the slot.
pub(super) fn push_panel_state_for_view(
    controller: &Rc<RefCell<HostController>>,
    handle: &UiHandle,
    view: &CadView,
) {
    push_panel_state(controller, handle, &view_slot(view));
}

/// Display name of the renderer backend, for the diagnostics drawer header.
fn backend_display(view: &Rc<RefCell<Option<CadView>>>) -> String {
    // Bind the `Ref` so the `&CadView` borrow outlives this expression.
    let borrowed = view.borrow();
    let Some(view) = borrowed.as_ref() else {
        return String::new();
    };
    // `BackendPreference` is a product name; only `Auto` is worded.
    match format!("{:?}", view.preference()).as_str() {
        "Auto" => current_messages().text("backend.auto", &[]),
        "WebGpu" => "WebGPU".to_string(),
        "WebGl2" => "WebGL2".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::layers::LayerRow;
    use cad_app::{AnnotationRow, PropertyRow, SelectionProperties};
    use cad_diagnostics::model::codes;
    use cad_domain::{EntityId, InstancePath, LayerId, ObjectId};
    use cad_ui_slint::Locale;

    fn zh() -> MessageSource {
        MessageSource::for_locale(Locale::ZhCn)
    }

    fn layer_row(id: u128, name: &str, visible: bool, override_visible: Option<bool>) -> LayerRow {
        let effective = override_visible.unwrap_or(visible);
        LayerRow {
            id: LayerId(id),
            name: name.to_string(),
            database_visible: visible,
            override_visible,
            effective_visible: effective,
        }
    }

    #[test]
    fn layer_state_keeps_row_order_and_ids_exact() {
        let rows = [
            layer_row(2, "WALLS", true, None),
            layer_row(9, "0", true, Some(false)),
        ];
        let (state, order) = derive_layer_state(&rows, &zh());
        assert_eq!(state.rows.len(), 2);
        assert_eq!(order, vec![LayerId(2), LayerId(9)]);
        assert_eq!(state.override_count, 1);
        assert!(state.rows[1].overridden);
        assert!(!state.empty_label.is_empty());
    }

    #[test]
    fn empty_layer_state_is_explicit_and_uses_the_catalog() {
        let (state, order) = derive_layer_state(&[], &zh());
        assert!(state.rows.is_empty());
        assert!(order.is_empty());
        assert_eq!(state.empty_label, "无图层");
    }

    #[test]
    fn empty_property_state_has_no_rows_and_the_catalog_label() {
        let properties = SelectionProperties {
            empty: true,
            ..SelectionProperties::default()
        };
        let state = derive_property_state(&properties, &zh());
        assert!(state.rows.is_empty());
        assert_eq!(state.count, 0);
        assert_eq!(state.empty_label, "未选择");
    }

    #[test]
    fn multi_select_property_state_uses_the_mixed_catalog_template() {
        let properties = SelectionProperties {
            empty: false,
            count: 2,
            rows: vec![PropertyRow::new("type", "AcDbLine")],
            mixed_keys: vec!["layer", "length"],
        };
        let state = derive_property_state(&properties, &zh());
        assert_eq!(state.count, 2);
        assert_eq!(state.mixed_label, "多值: layer,length");
        assert_eq!(state.empty_label, "未选择");
    }

    #[test]
    fn annotation_state_keeps_ids_and_hidden_count() {
        let rows = [
            AnnotationRow {
                id: AnnotationId(5),
                kind: "text",
                kind_label: "文字",
                text: "note".to_string(),
                visible: false,
                overridden: true,
                selected: true,
            },
            AnnotationRow {
                id: AnnotationId(6),
                kind: "text",
                kind_label: "文字",
                text: "keep".to_string(),
                visible: true,
                overridden: false,
                selected: false,
            },
        ];
        let (state, order) = derive_annotation_state(&rows, None, &zh());
        assert_eq!(order, vec![AnnotationId(5), AnnotationId(6)]);
        assert_eq!(state.rows.len(), 2);
        assert_eq!(state.hidden_count, 1);
        assert!(!state.tool_active);
        assert_eq!(state.empty_label, "无注解");
    }

    #[test]
    fn diagnostics_model_preserves_every_reason_and_object() {
        let diagnostics = vec![
            Diagnostic {
                object: Some(ObjectId(3)),
                code: codes::REPRESENTATION_UNAVAILABLE.to_string(),
                message: "unavailable".to_string(),
            },
            Diagnostic {
                object: Some(ObjectId(3)),
                code: codes::REPRESENTATION_APPROXIMATE.to_string(),
                message: "approx".to_string(),
            },
            Diagnostic {
                object: None,
                code: codes::FONT_UNRESOLVED.to_string(),
                message: "font".to_string(),
            },
        ];
        let model = diagnostics_model_from_import(&diagnostics);
        // Document reason + both object reasons are kept.
        assert_eq!(model.document.len(), 1);
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.objects[0].reasons.len(), 2);
        assert!(!model.summary().complete);
    }

    #[test]
    fn no_import_diagnostics_means_an_explicit_empty_drawer() {
        let model = diagnostics_model_from_import(&[]);
        assert!(model.document.is_empty());
        assert!(model.objects.is_empty());
        let state = DiagnosticsPanelState::from_model(&model, &zh(), "WebGL2");
        assert!(state.is_empty());
        assert!(!state.empty_label.is_empty());
    }

    #[test]
    fn severity_mapping_is_conservative_and_documented() {
        assert_eq!(
            severity_for_code(codes::REPRESENTATION_UNAVAILABLE),
            Severity::Error
        );
        assert_eq!(
            severity_for_code(codes::REPRESENTATION_APPROXIMATE),
            Severity::Warning
        );
        // An unknown code is never invented as an error.
        assert_eq!(severity_for_code("something.else"), Severity::Warning);
    }

    // Documents that the entity identity helper is available for property tests.
    #[test]
    fn selection_ref_identity_is_stable() {
        use cad_app::SelectionSet;
        use cad_domain::{DocumentId, SelectionRef};
        let a = SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        };
        let mut set = SelectionSet::new();
        assert!(set.insert(a.clone()));
        assert!(!set.insert(a));
    }

    #[test]
    fn overlay_push_is_explicitly_empty_before_any_selection_or_tool() {
        let controller = HostController::with_demo_document([800.0, 600.0]).unwrap();
        let overlay = derive_overlay_push(&controller);
        // The highlight is the explicit empty set, and both previews are None:
        // the funnel clears the overlay rather than leaving a stale one.
        assert!(overlay.selection.is_empty());
        assert_eq!(overlay.selection, cad_app::SelectionSet::new());
        assert!(overlay.measurement.is_none());
        assert!(overlay.annotation.is_none());
    }

    #[test]
    fn overlay_push_carries_the_drawn_selection_and_active_preview() {
        use cad_app::SelectionSet;
        use cad_domain::{DocumentId, SelectionRef};
        use std::rc::Rc;

        let controller = Rc::new(RefCell::new(
            HostController::with_demo_document([800.0, 600.0]).unwrap(),
        ));
        let reference = SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        };
        controller
            .borrow_mut()
            .set_selection(vec![reference.clone()])
            .unwrap();

        let overlay = derive_overlay_push(&controller.borrow());
        let mut expected = SelectionSet::new();
        assert!(expected.insert(reference));
        assert_eq!(overlay.selection, expected);
        assert!(!overlay.selection.is_empty());

        // Starting the distance tool makes the measurement preview non-None.
        controller
            .borrow_mut()
            .execute(cad_app::Command {
                schema_version: 1,
                id: cad_app::CommandId::Measure,
                document: controller.borrow().document_id,
                viewport: controller.borrow().viewport_id,
                payload: cad_app::CommandPayload::None,
            })
            .unwrap();
        let overlay = derive_overlay_push(&controller.borrow());
        assert!(overlay.measurement.is_some());
        assert!(overlay.annotation.is_none());
    }
}
