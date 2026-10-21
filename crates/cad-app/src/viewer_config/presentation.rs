use super::*;

/// Overlay visibility resolved for the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayVisibility {
    pub axes: bool,
    pub grid: bool,
    pub selection_highlight: bool,
    pub snap_hints: bool,
}

impl Default for OverlayVisibility {
    /// Every overlay is visible. `OverlayInputs::default()` therefore keeps the
    /// historical "nothing is hidden" behavior; only an explicit host push
    /// turns an overlay off.
    fn default() -> Self {
        Self {
            axes: true,
            grid: true,
            selection_highlight: true,
            snap_hints: true,
        }
    }
}

impl From<ViewOverlays> for OverlayVisibility {
    fn from(value: ViewOverlays) -> Self {
        Self {
            axes: value.axes,
            grid: value.grid,
            selection_highlight: value.selection_highlight,
            snap_hints: value.snap_hints,
        }
    }
}

impl ViewBackground {
    /// The background colour of the space being displayed.
    ///
    /// Paper space has one colour for every layout (AutoCAD also has a single
    /// paper background), so the caller only reports whether the frame shows
    /// paper space rather than naming a layout.
    pub fn resolve(&self, paper: bool) -> HexColor {
        if paper {
            self.paper
        } else {
            self.model
        }
    }
}

/// Capability visibility resolved for the shell entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureVisibility {
    pub measure: bool,
}

/// The pure presentation model the shell renders. No CAD state is embedded.
#[derive(Debug, Clone, PartialEq)]
pub struct UiPresentationModel {
    pub layout: LayoutMode,
    pub application_ui: bool,
    pub ribbon: bool,
    pub layer_panel: bool,
    pub properties_panel: bool,
    pub layer_panel_initially_open: bool,
    pub properties_panel_initially_open: bool,
    pub navigation: bool,
    pub command_bar: bool,
    pub layout_tabs: bool,
    pub status_bar: bool,
    pub pointer: bool,
    pub touch: bool,
    pub keyboard_shortcuts: bool,
    pub overlays: OverlayVisibility,
    pub features: FeatureVisibility,
    /// Effective per-command visibility: application-UI × feature × override.
    pub command_visibility: BTreeMap<String, bool>,
    pub touch_target: f32,
    pub dock_width: f32,
}

impl UiPresentationModel {
    /// Whether any dock panel is visible (a convenience over the two flags).
    pub fn panels(&self) -> bool {
        self.layer_panel || self.properties_panel
    }

    /// Insets are top/right/bottom/left. Short landscape surfaces use compact chrome.
    pub fn resolve(config: &ViewerConfig, size: [f64; 2], insets: [f64; 4], touch: bool) -> Self {
        let width = (size[0] - insets[1] - insets[3]).max(0.0);
        let height = (size[1] - insets[0] - insets[2]).max(0.0);
        let layout = match config.ui.layout.mode {
            LayoutMode::Auto if width < config.ui.layout.breakpoints.mobile_below => {
                LayoutMode::Mobile
            }
            LayoutMode::Auto
                if width < config.ui.layout.breakpoints.compact_below || height < 540.0 =>
            {
                LayoutMode::Compact
            }
            LayoutMode::Auto => LayoutMode::Desktop,
            mode => mode,
        };
        let preset = preset_components(config.ui.preset);
        // The application frame is on for every preset except `CanvasOnly`,
        // which `preset_components` marks by turning off all chrome.
        let visible = config.ui.preset != Preset::CanvasOnly;
        let full = config.ui.preset == Preset::Full;
        let c = &config.ui.components;
        let layer_panel = preset.layer_panel && c.layer_panel.visible;
        let properties_panel = preset.properties_panel && c.properties_panel.visible;
        let features = FeatureVisibility {
            measure: config.features.measure,
        };
        let mut command_visibility = BTreeMap::new();
        for id in COMMAND_IDS {
            let mut entry_visible = preset_command_visible(config.ui.preset, id);
            if let Some(required) = required_feature(id) {
                entry_visible &= required(&features);
            }
            if let Some(over) = config.ui.command_overrides.get(*id) {
                entry_visible &= over.visible;
            }
            command_visibility.insert((*id).to_string(), entry_visible);
        }
        Self {
            layout,
            application_ui: visible,
            ribbon: preset.ribbon && c.ribbon.visible,
            layer_panel,
            properties_panel,
            // `initiallyOpen` is the desktop dock default. The compact/mobile
            // shell never auto-opens its drawer from a resize: there it is the
            // user's explicit toggle that opens the panel, so a breakpoint
            // change must not surprise the canvas with an overlay drawer.
            layer_panel_initially_open: layer_panel
                && c.layer_panel.initially_open
                && layout == LayoutMode::Desktop,
            properties_panel_initially_open: properties_panel
                && c.properties_panel.initially_open
                && layout == LayoutMode::Desktop,
            navigation: preset.navigation && c.navigation_toolbar.visible,
            command_bar: preset.command_bar && c.command_bar.visible,
            layout_tabs: preset.layout_tabs && c.layout_tabs.visible,
            status_bar: preset.status_bar && c.status_bar.visible,
            pointer: config.interaction.pointer,
            touch: config.interaction.touch,
            keyboard_shortcuts: config.interaction.keyboard_shortcuts,
            overlays: OverlayVisibility {
                axes: config.view.overlays.axes,
                grid: config.view.overlays.grid,
                selection_highlight: config.view.overlays.selection_highlight,
                snap_hints: config.view.overlays.snap_hints,
            },
            features,
            command_visibility,
            touch_target: if touch || layout == LayoutMode::Mobile {
                48.0
            } else {
                36.0
            },
            // Only the full desktop arrangement reserves a dock gutter. Minimal
            // keeps its panels togglable but never auto-reserves width, so it
            // reports `0.0` here.
            dock_width: if visible
                && full
                && layout == LayoutMode::Desktop
                && (layer_panel || properties_panel)
            {
                240.0
            } else {
                0.0
            },
        }
    }
}

/// Feature gate for a command id, if any.
fn required_feature(id: &str) -> Option<fn(&FeatureVisibility) -> bool> {
    if id.starts_with("measure.") {
        return Some(|f: &FeatureVisibility| f.measure);
    }
    None
}
