//! chrome module.

use super::*;

/// Layout/locale configuration for the shell.
#[derive(Debug, Clone)]
pub struct UiConfiguration {
    pub compact: bool,
    pub locale: String,
    pub safe_insets: [f64; 4],
    pub application_title: String,
    pub document: DocumentId,
    pub viewport: ViewportId,
    /// Initial logical window size; hosts keep it in sync with the surface.
    pub logical_size: [f64; 2],
}

impl Default for UiConfiguration {
    fn default() -> Self {
        UiConfiguration {
            compact: false,
            locale: "zh-CN".to_string(),
            safe_insets: [0.0; 4],
            application_title: "yacr CAD".to_string(),
            document: DocumentId(0),
            viewport: ViewportId(0),
            logical_size: [1280.0, 800.0],
        }
    }
}

/// Receives commands emitted by the UI; the application executes them.
pub trait UiCommandSink: 'static {
    fn send(&mut self, command: Command) -> CadResult<()>;
}

/// Push every catalog-driven chrome label/model into the shell (N01).
///
/// This is the single place that maps catalog keys to Slint properties, so a
/// locale switch and the initial construction cannot drift. Combobox models are
/// built from the application's authoritative `ALL` orderings, which is also how
/// a localized label maps back to a kind without relying on Chinese-only
/// `from_label`.
pub(crate) fn apply_chrome(ui: &YacrWindow, messages: &MessageSource) {
    // Toolbar + mode.
    ui.set_open_label(messages.text("file.open", &[]).into());
    ui.set_fit_label(messages.text("toolbar.fit", &[]).into());
    ui.set_undo_label(messages.text("toolbar.undo", &[]).into());
    ui.set_redo_label(messages.text("toolbar.redo", &[]).into());
    ui.set_export_label(messages.text("toolbar.export", &[]).into());
    ui.set_import_label(messages.text("toolbar.import", &[]).into());
    ui.set_diagnostics_label(messages.text("toolbar.diagnostics", &[]).into());
    ui.set_mode_label(messages.text("mode.enhanced", &[]).into());

    // Measurement panel.
    ui.set_measurement_panel_label(messages.text("measure.panel", &[]).into());
    ui.set_measure_confirm_label(messages.text("tool.confirm", &[]).into());
    ui.set_measure_cancel_label(messages.text("tool.cancel", &[]).into());
    ui.set_measurement_kind_labels(string_model(&status::measurement_kind_labels(messages)));

    // Annotation panel.
    ui.set_annotation_panel_label(messages.text("annotation.panel", &[]).into());
    ui.set_annotate_confirm_label(messages.text("tool.confirm", &[]).into());
    ui.set_annotate_cancel_label(messages.text("tool.cancel", &[]).into());
    ui.set_annotation_text_placeholder(messages.text("annotation.text_placeholder", &[]).into());
    ui.set_annotation_delete_label(messages.text("annotation.delete", &[]).into());
    ui.set_annotation_kind_labels(string_model(&status::annotation_kind_labels(messages)));

    // Layer + property panels.
    ui.set_layer_panel_label(messages.text("layers.panel", &[]).into());
    ui.set_layer_restore_label(messages.text("layers.restore", &[]).into());
    ui.set_layer_overridden_marker(messages.text("layers.overridden_marker", &[]).into());
    ui.set_layer_empty_label(messages.text("layers.empty", &[]).into());
    ui.set_property_panel_label(messages.text("properties.panel", &[]).into());
    ui.set_property_clear_label(messages.text("properties.clear_selection", &[]).into());
    ui.set_property_empty_label(messages.text("properties.empty", &[]).into());

    // Backend choice model and diagnostics drawer chrome.
    ui.set_backend_labels(string_model(&status::backend_labels(messages)));
    ui.set_diagnostics_drawer_title(messages.text("diagnostics.title", &[]).into());
    ui.set_diagnostics_close_label(messages.text("diagnostics.close", &[]).into());
    ui.set_diagnostics_empty_label(messages.text("diagnostics.empty", &[]).into());

    // Layout panel chrome (F04/U03).
    ui.set_layout_panel_label(messages.text("layout.panel", &[]).into());
    ui.set_layout_model_space_label(messages.text("layout.model_space", &[]).into());
    ui.set_layout_unsupported_marker(messages.text("layout.unsupported_marker", &[]).into());
    ui.set_layout_empty_label(messages.text("layout.empty", &[]).into());

    // View / projection chrome (F13/F14). The standard-view model is built from
    // the shared `StandardView::ALL` ordering so a row index is authoritative.
    ui.set_view_panel_label(messages.text("view.panel", &[]).into());
    ui.set_view_2d_label(messages.text("view.2d", &[]).into());
    ui.set_view_3d_label(messages.text("view.3d", &[]).into());
    ui.set_view_projection_label(messages.text("view.projection", &[]).into());
    ui.set_view_ortho_label(messages.text("view.projection.ortho", &[]).into());
    ui.set_view_perspective_label(messages.text("view.projection.perspective", &[]).into());
    ui.set_standard_view_labels(string_model(&status::standard_view_labels(messages)));

    // Responsive chrome (U01): the grouped-bar and drawer entry labels.
    ui.set_tools_label(messages.text("shell.tools", &[]).into());
    ui.set_drawer_label(messages.text("shell.drawer", &[]).into());
    ui.set_nav_label(messages.text("shell.nav", &[]).into());
    ui.set_view_mode_label(messages.text("shell.mode_view", &[]).into());
    ui.set_work_mode_label(messages.text("shell.mode_work", &[]).into());
}

/// Push the derived responsive geometry into the shell (audit U01/U07).
///
/// This is the wire that consumes [`UiConfiguration::compact`] and the current
/// logical viewport instead of leaving compact dead. It writes only geometry
/// properties, so pushing it never disturbs pushed panel data.
pub fn apply_responsive(ui: &YacrWindow, logical_size: [f64; 2], compact_config: bool) {
    let metrics = ResponsiveMetrics::derive(logical_size, compact_config);
    ui.set_compact_shell(metrics.compact);
    ui.set_phone_shell(metrics.breakpoint == Breakpoint::Phone);
    ui.set_control_height(metrics.control_height);
    ui.set_touch_target(metrics.touch_target);
    ui.set_side_panel_width(metrics.side_panel_width);
    ui.set_side_panel_collapsed(metrics.side_panel_collapsed);
    ui.set_drawer_height(metrics.drawer_height);
    ui.set_show_floating_nav(metrics.breakpoint.shows_floating_nav());
}

/// Build a Slint string model from owned labels.
pub(crate) fn string_model(labels: &[String]) -> slint::ModelRc<slint::SharedString> {
    let values: Vec<slint::SharedString> =
        labels.iter().map(|label| label.as_str().into()).collect();
    slint::ModelRc::new(slint::VecModel::from(values))
}

/// Localized "temporary override N" text for the layer panel.
pub(crate) fn layer_override_label(messages: &MessageSource, count: usize) -> String {
    messages.text("layers.override_count", &[("count", &count.to_string())])
}

/// Localized "N selected" text for the properties panel.
pub(crate) fn selected_count_label(messages: &MessageSource, count: usize) -> String {
    messages.text(
        "properties.selected_count",
        &[("count", &count.to_string())],
    )
}

/// Localized "N hidden" text for the annotation management panel.
pub(crate) fn annotation_hidden_label(messages: &MessageSource, count: usize) -> String {
    messages.text("annotation.hidden_count", &[("count", &count.to_string())])
}

/// Localized "Renderer backend: X" line for the diagnostics drawer.
pub(crate) fn backend_label(messages: &MessageSource, backend: &str) -> String {
    messages.text("backend.label", &[("backend", backend)])
}
