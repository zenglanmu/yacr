//! chrome module.

use super::*;
use slint::Model;

use cad_app::viewer_config::ViewerConfig;

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
pub(crate) fn apply_chrome(ui: &YacrWindow, messages: &MessageSource, work_mode: bool) {
    ui.set_ribbon_tabs(string_model(&[
        messages.text("ribbon.file", &[]),
        messages.text("ribbon.view", &[]),
        messages.text("ribbon.measure", &[]),
        messages.text("ribbon.annotate", &[]),
        messages.text("shell.more", &[]),
    ]));
    ui.set_command_title(messages.text("command.title", &[]).into());
    ui.set_command_prompt(messages.text("command.prompt", &[]).into());
    ui.set_draw_status(messages.text("draw.status.idle", &[]).into());
    // The ribbon draw buttons present the shared `DrawToolKind::ALL` order; the
    // adapter maps a clicked label straight back to the exact kind.
    ui.set_editing_labels(string_model(&draw::draw_kind_labels(messages)));
    ui.set_draw_confirm_label(messages.text("tool.confirm", &[]).into());
    ui.set_draw_cancel_label(messages.text("tool.cancel", &[]).into());
    // Toolbar + mode. The mode label reflects the real session mode, not a
    // hardcoded "enhanced" (audit U02).
    ui.set_open_label(messages.text("file.open", &[]).into());
    ui.set_fit_label(messages.text("toolbar.fit", &[]).into());
    ui.set_pan_label(messages.text("toolbar.pan", &[]).into());
    ui.set_zoom_in_label(messages.text("toolbar.zoom_in", &[]).into());
    ui.set_zoom_out_label(messages.text("toolbar.zoom_out", &[]).into());
    ui.set_measure_label(messages.text("measure.panel", &[]).into());
    ui.set_annotate_label(messages.text("annotation.panel", &[]).into());
    ui.set_undo_label(messages.text("toolbar.undo", &[]).into());
    ui.set_redo_label(messages.text("toolbar.redo", &[]).into());
    ui.set_export_label(messages.text("toolbar.export", &[]).into());
    ui.set_import_label(messages.text("toolbar.import", &[]).into());
    ui.set_diagnostics_label(messages.text("toolbar.diagnostics", &[]).into());
    ui.set_mode_label(messages.text(mode_key(work_mode), &[]).into());

    // Measurement panel.
    ui.set_measurement_panel_label(messages.text("measure.panel", &[]).into());
    ui.set_measure_confirm_label(messages.text("tool.confirm", &[]).into());
    ui.set_measure_cancel_label(messages.text("tool.cancel", &[]).into());
    ui.set_measure_save_label(messages.text("measure.save_annotation", &[]).into());
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

    // Asynchronous open progress panel chrome (F01).
    ui.set_import_panel_label(messages.text("import.panel", &[]).into());
    ui.set_import_cancel_label(messages.text("import.cancel", &[]).into());

    // Layout panel chrome (F04/U03).
    ui.set_layout_panel_label(messages.text("layout.panel", &[]).into());
    ui.set_layout_model_space_label(messages.text("layout.model_space", &[]).into());
    if ui.get_layout_rows().row_count() == 0 {
        ui.set_layout_labels(string_model(&[messages.text("layout.model_space", &[])]));
    }
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
    ui.set_more_label(messages.text("shell.more", &[]).into());
    ui.set_drawer_label(messages.text("shell.drawer", &[]).into());
    ui.set_nav_label(messages.text("shell.nav", &[]).into());
    ui.set_view_mode_label(messages.text("shell.mode_view", &[]).into());
    ui.set_work_mode_label(messages.text("shell.mode_work", &[]).into());
}

/// Catalog keys for a configured ribbon tab, if it declares one.
///
/// The shipped default is the catalog's own five tabs; a config tab carries a
/// catalog key (`ribbon.review`) that the UI resolves through `MessageSource`.
/// An empty/missing label falls back to the catalog default, never a literal.
fn ribbon_tab_labels(tab: &cad_app::viewer_config::RibbonTab, messages: &MessageSource) -> String {
    if tab.label.is_empty() {
        messages.text("shell.more", &[])
    } else {
        messages.text(&tab.label, &[])
    }
}

/// Push the configured ribbon tab model (if any) alongside the catalog chrome.
///
/// Called after [`apply_chrome`] so a config-declared ribbon replaces the default
/// tab model. An empty `tabs` list leaves the catalog model in place.
pub(crate) fn apply_ribbon_config(
    ui: &YacrWindow,
    config: &ViewerConfig,
    messages: &MessageSource,
) {
    let tabs = &config.ui.components.ribbon.tabs;
    if tabs.is_empty() {
        return;
    }
    let labels: Vec<String> = tabs
        .iter()
        .map(|tab| ribbon_tab_labels(tab, messages))
        .collect();
    ui.set_ribbon_tabs(string_model(&labels));
}

/// Push the derived responsive geometry into the shell (audit U01/U07).
///
/// This is the wire that consumes [`UiConfiguration::compact`] and the current
/// logical viewport instead of leaving compact dead. It writes only geometry
/// properties, so pushing it never disturbs pushed panel data.
pub fn apply_responsive(ui: &YacrWindow, logical_size: [f64; 2], compact_config: bool) {
    let mut config = cad_app::viewer_config::ViewerConfig::default();
    if compact_config {
        config.ui.layout.mode = cad_app::viewer_config::LayoutMode::Compact;
    }
    apply_viewer_presentation_with(ui, &config, logical_size, None);
}

/// Derive the presentation from `config` and apply it to the shell.
///
/// When `store` is supplied the resolved config readout properties are pushed
/// too, so a host always sees the exact effective config it was applied from.
pub(crate) fn apply_viewer_presentation(
    ui: &YacrWindow,
    config: &cad_app::viewer_config::ViewerConfig,
    size: [f64; 2],
) {
    apply_viewer_presentation_with(ui, config, size, None);
}

pub(crate) fn apply_viewer_presentation_with(
    ui: &YacrWindow,
    config: &cad_app::viewer_config::ViewerConfig,
    size: [f64; 2],
    store: Option<&cad_app::viewer_config::ViewerConfigStore>,
) {
    use cad_app::viewer_config::{LayoutMode, UiPresentationModel};
    let p = UiPresentationModel::resolve(config, size, [0.0; 4], false);
    let phone = p.layout == LayoutMode::Mobile;
    let compact = p.layout == LayoutMode::Compact;
    let changed_layout = ui.get_phone_shell() != phone || ui.get_compact_shell() != compact;
    ui.set_phone_shell(phone);
    ui.set_compact_shell(compact);
    ui.set_control_height(if phone { 56.0 } else { 40.0 });
    ui.set_touch_target(p.touch_target);
    ui.set_side_panel_width(p.dock_width);
    ui.set_drawer_height(if phone { 280.0 } else { 0.0 });
    ui.set_application_ui(p.application_ui);
    ui.set_ribbon_visible(p.ribbon);
    ui.set_panels_visible(p.panels());
    ui.set_layer_panel_visible(p.layer_panel);
    ui.set_properties_panel_visible(p.properties_panel);
    ui.set_layer_panel_initially_open(p.layer_panel_initially_open);
    ui.set_properties_panel_initially_open(p.properties_panel_initially_open);
    ui.set_navigation_visible(p.navigation);
    ui.set_command_visible(p.command_bar);
    ui.set_layouts_visible(p.layout_tabs);
    ui.set_status_visible(p.status_bar);
    ui.set_show_floating_nav(p.navigation);
    let visible = |id: &str| p.command_visibility.get(id).copied().unwrap_or(false);
    ui.set_cmd_measure_visible(visible("measure.distance"));
    ui.set_cmd_annotate_visible(visible("annotation.text"));
    ui.set_cmd_annotation_delete_visible(visible("annotation.delete"));
    ui.set_cmd_export_visible(visible("file.exportAnnotations"));
    ui.set_cmd_import_visible(visible("file.importAnnotations"));
    if changed_layout {
        // A user's later explicit open/close toggle must not be clobbered by a
        // mere re-apply at the same breakpoint; only a real layout change resets it.
        ui.set_side_panel_open(p.layer_panel_initially_open);
        ui.set_tools_open(false);
        ui.set_ribbon_expanded(!compact);
    }
    if !p.application_ui {
        ui.set_side_panel_open(false);
        ui.set_diagnostics_open(false);
        ui.set_tools_open(false);
        ui.set_command_expanded(false);
    }
    if let Some(store) = store {
        crate::handle::push_config_properties(ui, store);
    }
}

/// Catalog key for the current mode label (audit U02). Work keeps the existing
/// `mode.enhanced` key; Viewer uses `mode.viewer`.
pub(crate) fn mode_key(work: bool) -> &'static str {
    if work {
        "mode.enhanced"
    } else {
        "mode.viewer"
    }
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
