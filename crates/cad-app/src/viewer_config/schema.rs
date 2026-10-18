use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutMode {
    #[default]
    Auto,
    Desktop,
    Compact,
    Mobile,
}

/// Dock/side placement of a panel or toolbar. `auto` lets the layout decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    #[default]
    Auto,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Breakpoints {
    pub compact_below: f64,
    pub mobile_below: f64,
}
impl Default for Breakpoints {
    fn default() -> Self {
        Self {
            compact_below: 1200.0,
            mobile_below: 720.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutConfig {
    pub mode: LayoutMode,
    pub breakpoints: Breakpoints,
}

/// One ribbon group: a stable id, an i18n catalog key label and command ids.
///
/// `label` is a catalog key (for example `"ribbon.review"`) resolved by the UI
/// through `MessageSource`; configuration must not carry literal user-facing
/// text (AGENTS i18n rule). `id` is a stable, non-user-visible identifier.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RibbonGroup {
    pub id: String,
    pub label: String,
    pub commands: Vec<String>,
    /// How this group's commands render. Defaults to `iconAndLabel`, which is
    /// exactly the pre-existing behavior.
    pub display: RibbonCommandDisplay,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RibbonTab {
    pub id: String,
    pub label: String,
    pub groups: Vec<RibbonGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RibbonComponent {
    pub visible: bool,
    /// Empty means "use the shipped catalog ribbon". A non-empty list replaces
    /// the tab model wholesale (arrays replace, never merge).
    pub tabs: Vec<RibbonTab>,
}
impl Default for RibbonComponent {
    fn default() -> Self {
        Self {
            visible: true,
            tabs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelComponent {
    pub visible: bool,
    pub placement: Placement,
    /// Whether the panel starts expanded. Docked panels default to open so the
    /// shipped desktop arrangement is unchanged; the protocol example sets it
    /// explicitly to `false`.
    pub initially_open: bool,
}
impl Default for PanelComponent {
    fn default() -> Self {
        Self {
            visible: true,
            placement: Placement::Auto,
            initially_open: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct NavigationToolbarComponent {
    pub visible: bool,
    pub placement: Placement,
    pub commands: Vec<String>,
}
impl Default for NavigationToolbarComponent {
    fn default() -> Self {
        Self {
            visible: true,
            placement: Placement::Right,
            commands: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ToggleComponent {
    pub visible: bool,
}
impl Default for ToggleComponent {
    fn default() -> Self {
        Self { visible: true }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Components {
    pub ribbon: RibbonComponent,
    pub layer_panel: PanelComponent,
    pub properties_panel: PanelComponent,
    pub navigation_toolbar: NavigationToolbarComponent,
    pub command_bar: ToggleComponent,
    pub layout_tabs: ToggleComponent,
    pub status_bar: ToggleComponent,
}

/// Per-command override. Only `visible: false` is meaningful today; an omitted
/// override leaves the command's feature/preset visibility untouched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandOverride {
    pub visible: bool,
}
impl Default for CommandOverride {
    fn default() -> Self {
        Self { visible: true }
    }
}

/// Paths a host allows a user preference to touch. A path is a dotted JSON path
/// such as `ui.components.layerPanel.initiallyOpen`; a leaf is allowed when it
/// equals or is a descendant of one of these paths.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct UserCustomization {
    pub allowed_paths: Vec<String>,
}

/// The vector format a host-owned `PLOT` export defaults to.
///
/// The desktop save dialog still lets the user type any supported extension; the
/// web host has no dialog and uses this value directly as the download name's
/// extension. Only the two vector formats the CPU path can actually produce are
/// representable: `.png` needs surface readback and stays an explicit
/// `plot.png_cli_only` refusal, so it is not a legal value here. Serde rejects an
/// unknown value with a precise path, never a silent fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlotExportFormat {
    #[default]
    Svg,
    Pdf,
}

impl PlotExportFormat {
    /// The lowercased file extension for this format (`"svg"`/`"pdf"`).
    pub fn as_str(self) -> &'static str {
        match self {
            PlotExportFormat::Svg => "svg",
            PlotExportFormat::Pdf => "pdf",
        }
    }
}

/// Host-owned export defaults (no executable content).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportConfig {
    pub plot_format: PlotExportFormat,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct UiConfig {
    pub preset: Preset,
    pub layout: LayoutConfig,
    pub components: Components,
    pub command_overrides: BTreeMap<String, CommandOverride>,
    pub user_customization: UserCustomization,
    pub export: ExportConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FeaturesConfig {
    pub measure: bool,
}
impl Default for FeaturesConfig {
    fn default() -> Self {
        Self { measure: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewOverlays {
    pub axes: bool,
    pub grid: bool,
    pub selection_highlight: bool,
    pub snap_hints: bool,
}
impl Default for ViewOverlays {
    fn default() -> Self {
        Self {
            axes: true,
            // AutoCAD starts with the grid off and the ui-spec example uses
            // `grid: false`; a host that wants it enables it explicitly.
            grid: false,
            selection_highlight: true,
            snap_hints: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewConfig {
    pub overlays: ViewOverlays,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionConfig {
    pub pointer: bool,
    pub touch: bool,
    pub keyboard_shortcuts: bool,
}
impl Default for InteractionConfig {
    fn default() -> Self {
        Self {
            pointer: true,
            touch: true,
            keyboard_shortcuts: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewerConfig {
    pub schema_version: u32,
    pub ui: UiConfig,
    pub features: FeaturesConfig,
    pub view: ViewConfig,
    pub interaction: InteractionConfig,
}
impl Default for ViewerConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            ui: UiConfig::default(),
            features: FeaturesConfig::default(),
            view: ViewConfig::default(),
            interaction: InteractionConfig::default(),
        }
    }
}
