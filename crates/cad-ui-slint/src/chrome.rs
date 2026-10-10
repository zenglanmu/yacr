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
    ui.set_ribbon_tabs(string_model(&builtin_ribbon_tab_labels(messages)));
    ui.set_command_title(messages.text("command.title", &[]).into());
    ui.set_command_prompt(messages.text("command.prompt", &[]).into());
    ui.set_command_completion_label(messages.text("command.completions", &[]).into());
    ui.set_command_history_label(messages.text("command.history", &[]).into());
    ui.set_command_history_clear_label(messages.text("command.clear_history", &[]).into());
    ui.set_command_history_limit_label(messages.text("command.history_limit", &[]).into());
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
    ui.set_undo_label(messages.text("toolbar.undo", &[]).into());
    ui.set_redo_label(messages.text("toolbar.redo", &[]).into());
    ui.set_diagnostics_label(messages.text("toolbar.diagnostics", &[]).into());
    ui.set_mode_label(messages.text(mode_key(work_mode), &[]).into());

    // Measurement panel.
    ui.set_measurement_panel_label(messages.text("measure.panel", &[]).into());
    ui.set_measure_confirm_label(messages.text("tool.confirm", &[]).into());
    ui.set_measure_cancel_label(messages.text("tool.cancel", &[]).into());
    ui.set_measurement_kind_labels(string_model(&status::measurement_kind_labels(messages)));

    // Layer + property panels.
    ui.set_layer_panel_label(messages.text("layers.panel", &[]).into());
    ui.set_layer_restore_label(messages.text("layers.restore", &[]).into());
    ui.set_layers_show_all_label(messages.text("layers.show_all", &[]).into());
    ui.set_layers_hide_all_label(messages.text("layers.hide_all", &[]).into());
    ui.set_layer_overridden_marker(messages.text("layers.overridden_marker", &[]).into());
    ui.set_layer_empty_label(messages.text("layers.empty", &[]).into());
    ui.set_layer_search_placeholder(messages.text("layers.search_placeholder", &[]).into());
    ui.set_layer_search_clear_label(messages.text("layers.clear_search", &[]).into());
    ui.set_layer_search_empty_label(messages.text("layers.no_matches", &[]).into());
    ui.set_property_panel_label(messages.text("properties.panel", &[]).into());
    ui.set_property_clear_label(messages.text("properties.clear_selection", &[]).into());
    ui.set_selection_all_label(messages.text("selection.all", &[]).into());
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
    // Viewport-scale surface (F04). There is no scale command yet, so the
    // control stays disabled and its reason is always shown.
    ui.set_layout_scale_label(messages.text("layout.scale_label", &[]).into());
    ui.set_layout_scale_unavailable_label(status::layout_scale_unavailable_label(messages).into());
    ui.set_layout_scale_control_label(messages.text("layout.scale_control_label", &[]).into());
    ui.set_layout_scale_control_reason(status::layout_scale_control_reason(messages).into());
    ui.set_layout_scale_control_available(LAYOUT_SCALE_CONTROL_WIRED);

    // View / projection chrome (F13/F14). The standard-view model is built from
    // the shared `StandardView::ALL` ordering so a row index is authoritative.
    ui.set_view_panel_label(messages.text("view.panel", &[]).into());
    ui.set_view_2d_label(messages.text("view.2d", &[]).into());
    ui.set_view_3d_label(messages.text("view.3d", &[]).into());
    ui.set_view_projection_label(messages.text("view.projection", &[]).into());
    ui.set_view_ortho_label(messages.text("view.projection.ortho", &[]).into());
    ui.set_view_perspective_label(messages.text("view.projection.perspective", &[]).into());
    ui.set_standard_view_labels(string_model(&status::standard_view_labels(messages)));

    // 3D observation drawer chrome (F13). The controls reuse the view labels
    // above; these are the drawer-only labels. The initial orbit status matches
    // the default 2D view; the authoritative push overrides it.
    ui.set_view_standard_label(messages.text("view.standard_label", &[]).into());
    ui.set_view_orbit_label(messages.text("view.orbit_label", &[]).into());
    ui.set_view_orbit_status(messages.text("view.orbit.needs_3d", &[]).into());
    ui.set_view_fit_label(messages.text("view.fit_label", &[]).into());
    ui.set_view_fit_reason(messages.text("view.fit3d_unavailable", &[]).into());
    ui.set_view3d_title(messages.text("view3d.title", &[]).into());
    ui.set_view3d_label(messages.text("view3d.title", &[]).into());
    ui.set_view3d_close_label(messages.text("diagnostics.close", &[]).into());

    // Resources drawer chrome (F10).
    ui.set_resources_label(messages.text("resources.title", &[]).into());
    ui.set_resources_title(messages.text("resources.title", &[]).into());
    ui.set_resources_close_label(messages.text("resources.close", &[]).into());
    ui.set_resources_empty_label(messages.text("resources.empty", &[]).into());

    // Responsive chrome (U01): the grouped-bar and drawer entry labels.
    ui.set_tools_label(messages.text("shell.tools", &[]).into());
    ui.set_more_label(messages.text("shell.more", &[]).into());
    ui.set_drawer_label(messages.text("shell.drawer", &[]).into());
    ui.set_nav_label(messages.text("shell.nav", &[]).into());
    ui.set_view_mode_label(messages.text("shell.mode_view", &[]).into());
    ui.set_work_mode_label(messages.text("shell.mode_work", &[]).into());
    ui.set_overlay_axes_label(messages.text("overlay.axes", &[]).into());
    ui.set_overlay_grid_label(messages.text("overlay.grid", &[]).into());
    ui.set_overlay_snap_hints_label(messages.text("overlay.snap_hints", &[]).into());
}

/// The shipped catalog ribbon tab labels, in shell order.
///
/// Shared by [`apply_chrome`] and [`apply_ribbon_config`] so a host that clears
/// its configured tabs always falls back to the exact built-in model.
pub(crate) fn builtin_ribbon_tab_labels(messages: &MessageSource) -> Vec<String> {
    vec![
        messages.text("ribbon.file", &[]),
        messages.text("ribbon.view", &[]),
        messages.text("ribbon.measure", &[]),
        messages.text("shell.more", &[]),
    ]
}

/// Resolve a config label catalog key to display text.
///
/// An empty key falls back to the catalog's "more" label; a key the catalog
/// lacks renders as its bracketed name (never a hardcoded literal).
fn ribbon_text(key: &str, messages: &MessageSource) -> String {
    if key.is_empty() {
        messages.text("shell.more", &[])
    } else {
        messages.text(key, &[])
    }
}

/// Catalog key for a command's button label.
///
/// Reuses the label a command already shows elsewhere in the shell and the
/// `ribbon.command.*` family for ids that only appear here.
pub(crate) fn ribbon_command_label_key(id: &str) -> &'static str {
    match id {
        "file.open" => "file.open",
        "edit.undo" => "toolbar.undo",
        "edit.redo" => "toolbar.redo",
        "view.fit" => "toolbar.fit",
        "view.pan" => "toolbar.pan",
        "view.orbit" => "ribbon.command.view_orbit",
        "view.reset" => "ribbon.command.view_reset",
        "view.standard" => "ribbon.command.view_standard",
        "view.projection" => "view.projection",
        "view.switch2d3d" => "ribbon.command.view_switch2d3d",
        "measure.distance" => "measure.kind.distance",
        "measure.polyline" => "measure.kind.polyline",
        "measure.angle" => "measure.kind.angle",
        "measure.area" => "measure.kind.area",
        "measure.confirm" => "tool.confirm",
        "measure.cancel" => "tool.cancel",
        "layer.toggle" => "ribbon.command.layer_toggle",
        "layer.restore" => "layers.restore",
        "layout.switch" => "layout.panel",
        "draw.line" => "draw.kind.line",
        "draw.circle" => "draw.kind.circle",
        "draw.move" => "draw.kind.move",
        "draw.trim" => "draw.kind.trim",
        "backend.switch" => "ribbon.command.backend_switch",
        "diagnostics.open" => "toolbar.diagnostics",
        "mode.toggle" => "ribbon.command.mode_toggle",
        // Unknown ids resolve to the empty key and the "more" fallback; config
        // validation rejects unknown ids before they reach the shell.
        _ => "",
    }
}

/// Map a resolved command display mode onto the documented Slint int constants:
/// `0` = icon + label, `1` = icon only, `2` = label only.
pub(crate) fn ribbon_command_display_code(
    display: cad_app::viewer_config::RibbonCommandDisplay,
) -> i32 {
    use cad_app::viewer_config::RibbonCommandDisplay::*;
    match display {
        IconAndLabel => 0,
        IconOnly => 1,
        LabelOnly => 2,
    }
}

/// Short icon token for a command id, matched against the shared `Button`
/// symbol table. Unknown ids have no icon.
pub(crate) fn ribbon_command_icon(id: &str) -> &'static str {
    match id {
        "file.open" => "▱",
        "edit.undo" => "↶",
        "edit.redo" => "↷",
        "view.fit" => "⊡",
        "view.pan" => "pan",
        "view.orbit" => "orbit",
        "view.reset" => "fit",
        "view.standard" => "◇",
        "view.projection" => "◇",
        "view.switch2d3d" => "3d",
        "measure.distance" => "↔",
        "measure.polyline" => "∿",
        "measure.angle" => "∠",
        "measure.area" => "▨",
        "measure.confirm" => "✓",
        "measure.cancel" => "×",
        "layer.toggle" => "layers",
        "layer.restore" => "layers",
        "layout.switch" => "layout",
        "draw.line" => "line",
        "draw.circle" => "circle",
        "draw.move" => "pan",
        "draw.trim" => "trim",
        "backend.switch" => "backend",
        "diagnostics.open" => "info",
        "mode.toggle" => "mode",
        _ => "",
    }
}

/// The config-driven ribbon chrome the shell renders.
///
/// Holds Slint row structs, so it lives in this crate while the pure
/// tab/group/command resolution lives in `cad-app::viewer_config`.
#[derive(Debug, Clone)]
pub(crate) struct RibbonConfigChrome {
    /// Whether the host declared custom tabs (the built-in layout is used otherwise).
    pub config_driven: bool,
    /// Tab-strip labels; also mirrored into the legacy `ribbon-tabs` property.
    pub tab_labels: Vec<String>,
    /// Resolved tab/group/command model.
    pub tabs: Vec<RibbonTabModel>,
}

/// Build the localized, visibility-filtered ribbon chrome from a config.
///
/// Pure (no live window) so it is unit-testable; display text comes from
/// `messages` while ids stay machine values for dispatch.
pub(crate) fn build_ribbon_config(
    config: &ViewerConfig,
    messages: &MessageSource,
) -> RibbonConfigChrome {
    let tabs = &config.ui.components.ribbon.tabs;
    if tabs.is_empty() {
        return RibbonConfigChrome {
            config_driven: false,
            tab_labels: builtin_ribbon_tab_labels(messages),
            tabs: Vec::new(),
        };
    }
    // Resolve through the shared presentation model so a configured button
    // disappears exactly when the corresponding command entry would.
    let command_visibility = cad_app::viewer_config::UiPresentationModel::resolve(
        config,
        [1280.0, 800.0],
        [0.0; 4],
        false,
    )
    .command_visibility;
    let resolved = cad_app::viewer_config::resolve_ribbon(config, &command_visibility);
    let tab_labels = resolved
        .tabs
        .iter()
        .map(|tab| ribbon_text(&tab.label, messages))
        .collect();
    let tabs = resolved
        .tabs
        .iter()
        .map(|tab| {
            let groups: Vec<RibbonGroupModel> = tab
                .groups
                .iter()
                .map(|group| {
                    let commands: Vec<RibbonCommandModel> = group
                        .commands
                        .iter()
                        .map(|command| RibbonCommandModel {
                            id: command.id.as_str().into(),
                            label: messages
                                .text(ribbon_command_label_key(&command.id), &[])
                                .into(),
                            icon: ribbon_command_icon(&command.id).into(),
                            visible: command.visible,
                            display: ribbon_command_display_code(command.display),
                        })
                        .collect();
                    RibbonGroupModel {
                        id: group.id.as_str().into(),
                        label: ribbon_text(&group.label, messages).into(),
                        commands: slint::ModelRc::new(slint::VecModel::from(commands)),
                    }
                })
                .collect();
            RibbonTabModel {
                id: tab.id.as_str().into(),
                label: ribbon_text(&tab.label, messages).into(),
                groups: slint::ModelRc::new(slint::VecModel::from(groups)),
            }
        })
        .collect();
    RibbonConfigChrome {
        config_driven: true,
        tab_labels,
        tabs,
    }
}

/// Push the configured ribbon chrome into the shell.
///
/// With no host tabs the built-in catalog model stays exactly as `apply_chrome`
/// wrote it (any previously pushed model is cleared). With host tabs the tab
/// strip, groups and command buttons all come from the config.
pub(crate) fn apply_ribbon_config(
    ui: &YacrWindow,
    config: &ViewerConfig,
    messages: &MessageSource,
) {
    let chrome = build_ribbon_config(config, messages);
    ui.set_ribbon_tabs(string_model(&chrome.tab_labels));
    ui.set_ribbon_config_driven(chrome.config_driven);
    if chrome.config_driven {
        ui.set_ribbon_config_tabs(slint::ModelRc::new(slint::VecModel::from(chrome.tabs)));
        // The configured list replaces the five built-in tabs; select the first.
        ui.set_ribbon_tab(0);
    } else {
        ui.set_ribbon_config_tabs(slint::ModelRc::new(slint::VecModel::from(Vec::<
            RibbonTabModel,
        >::new())));
    }
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
    ui.set_keyboard_shortcuts_enabled(config.interaction.keyboard_shortcuts);
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

/// Localized "Renderer backend: X" line for the diagnostics drawer.
pub(crate) fn backend_label(messages: &MessageSource, backend: &str) -> String {
    messages.text("backend.label", &[("backend", backend)])
}
