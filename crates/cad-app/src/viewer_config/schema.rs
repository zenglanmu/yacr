use super::*;

use cad_domain::TolerancePolicy;

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
///
/// `acisVersion` selects the ACIS payload version a Save-As writes and
/// `dxfVersion` the DXF container. Both reuse the exporter's own public enums
/// (and therefore its wire labels), so the persisted token, the settings token
/// and the written header cannot drift.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportConfig {
    pub plot_format: PlotExportFormat,
    /// ACIS payload version (`"7.0"` / `"21.0"`), default `"7.0"`.
    pub acis_version: cad_connector_acadrust::export::AcisVersion,
    /// DXF container version (`"2010"` / `"2013"` / `"2018"`), default
    /// `"2010"`. A DWG is always AC1032 (R2018); see
    /// [`cad_connector_acadrust::export::DxfTarget`].
    pub dxf_version: cad_connector_acadrust::export::DxfTarget,
}

/// One drawing/display unit token (ADR-0037 D9).
///
/// The persisted/wire spelling is the stable lowercase token — `drawing`, `mm`,
/// `cm`, `m`, `in`, `ft` — so `renameAll` is genuinely needed: the Rust variant
/// names are `Mm`/`Cm`/`In`/`Ft`, and `camelCase` renders them as the
/// single-word lowercase tokens above (`Drawing` → `drawing`, `M` → `m`).
/// [`Unit::DrawingUnits`] is the schema's "unknown units: values are drawing
/// units, never assumed millimetres" (`UnitContext::drawing_units`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnitToken {
    #[default]
    Drawing,
    Mm,
    Cm,
    M,
    In,
    Ft,
}

impl UnitToken {
    /// Every token, in the schema's declaration order.
    pub const ALL: [Self; 6] = [
        Self::Drawing,
        Self::Mm,
        Self::Cm,
        Self::M,
        Self::In,
        Self::Ft,
    ];

    /// The stable lowercase token (also the `cad_set_setting` value).
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Drawing => "drawing",
            Self::Mm => "mm",
            Self::Cm => "cm",
            Self::M => "m",
            Self::In => "in",
            Self::Ft => "ft",
        }
    }

    /// Parse an exact token; anything else is `None`, never a default.
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|unit| unit.as_token() == token)
    }

    /// The domain unit this token names.
    pub fn to_unit(self) -> cad_domain::Unit {
        match self {
            Self::Drawing => cad_domain::Unit::DrawingUnits,
            Self::Mm => cad_domain::Unit::Millimeter,
            Self::Cm => cad_domain::Unit::Centimeter,
            Self::M => cad_domain::Unit::Meter,
            Self::In => cad_domain::Unit::Inch,
            Self::Ft => cad_domain::Unit::Foot,
        }
    }

    /// The domain unit back as a token.
    pub fn from_unit(unit: &cad_domain::Unit) -> Self {
        match unit {
            cad_domain::Unit::DrawingUnits => Self::Drawing,
            cad_domain::Unit::Millimeter => Self::Mm,
            cad_domain::Unit::Centimeter => Self::Cm,
            cad_domain::Unit::Meter => Self::M,
            cad_domain::Unit::Inch => Self::In,
            cad_domain::Unit::Foot => Self::Ft,
        }
    }

    /// Exact metres per one of this unit.
    ///
    /// The factors are exact by construction (`in` is defined as `0.0254 m` and
    /// `ft` as `12 in`), so a displayed length is a division by an exactly
    /// representable ratio rather than a fitted approximation.
    /// [`Self::Drawing`] has no factor: an untyped drawing unit is not a metre
    /// count.
    pub fn metres_per_unit(self) -> Option<f64> {
        match self {
            Self::Drawing => None,
            Self::Mm => Some(0.001),
            Self::Cm => Some(0.01),
            Self::M => Some(1.0),
            Self::In => Some(0.0254),
            Self::Ft => Some(0.3048),
        }
    }
}

/// Drawing-unit and display-unit preference (ADR-0037 D9).
///
/// Defaults reproduce [`cad_domain::UnitContext::default`] exactly: both units
/// are the untyped `drawing` token and the precision is `3`. A typed display
/// unit over an untyped source is refused by [`ViewerConfig::validate`] (see
/// [`Self::unit_context`]) because it would claim a scale the coordinates do not
/// have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitsConfig {
    pub drawing_unit: UnitToken,
    pub display_unit: UnitToken,
    pub decimal_places: u8,
}

impl Default for UnitsConfig {
    fn default() -> Self {
        let context = cad_domain::UnitContext::default();
        Self {
            drawing_unit: UnitToken::from_unit(&context.source),
            display_unit: UnitToken::from_unit(&context.display),
            decimal_places: context.decimal_places,
        }
    }
}

impl UnitsConfig {
    /// The largest accepted precision, in decimal places.
    ///
    /// Eight is the point where a `mm` length still distinguishes one nanometre
    /// while staying inside the shortest-round-trip formatting the app uses;
    /// a larger value is refused by name, never clamped.
    pub const MAX_DECIMAL_PLACES: u8 = 8;

    /// Build the document's [`cad_domain::UnitContext`] from these tokens.
    ///
    /// `display_per_source` is `Some(metres_per_source / metres_per_display)`
    /// when **both** tokens are typed (so `cm → m` is `0.01`, `in → mm` is
    /// `25.4`, and `m → m` is exactly `1.0`), and `None` when both are the
    /// untyped `drawing` token. A typed source shown as the untyped display unit
    /// keeps the stored numbers (`Some(1.0)`: no unit is claimed for them), while
    /// a **typed display over an untyped source** is an explicit refusal — it
    /// would present untyped coordinates as a measured length.
    pub fn unit_context(&self) -> Result<cad_domain::UnitContext, ConfigError> {
        let source = self.drawing_unit.metres_per_unit();
        let display = self.display_unit.metres_per_unit();
        let display_per_source = match (source, display) {
            (Some(source), Some(display)) => {
                let ratio = source / display;
                if !ratio.is_finite() || ratio <= 0.0 {
                    return Err(ConfigError::new(
                        "ui.units.displayUnit",
                        "the display unit has no finite positive ratio to the drawing unit",
                    ));
                }
                Some(ratio)
            }
            (None, None) => None,
            (None, Some(_)) => {
                return Err(ConfigError::new(
                    "ui.units.displayUnit",
                    "a display unit requires a drawing unit; untyped drawing units cannot be displayed as a named unit",
                ))
            }
            // A typed source displayed as "drawing units" shows the stored
            // numbers unchanged (no unit is claimed for them), so the ratio is
            // exactly one rather than an invented conversion.
            (Some(_), None) => Some(1.0),
        };
        Ok(cad_domain::UnitContext {
            source: self.drawing_unit.to_unit(),
            display: self.display_unit.to_unit(),
            display_per_source,
            decimal_places: self.decimal_places,
        })
    }
}

/// The default recovery interval, in seconds (ADR-0037 D12's "at most once per
/// minute"). The host's `RECOVERY_INTERVAL` constant is this same value.
pub const DEFAULT_RECOVERY_INTERVAL_SECONDS: u32 = 60;

/// Recovery-snapshot write interval (ADR-0037 D12).
///
/// `intervalSeconds` is how often a dirty document may rewrite its snapshot;
/// `0` disables the writes entirely. The default `60` is the ADR's "at most once
/// per minute" and the same value as
/// [`crate::viewer_config::DEFAULT_RECOVERY_INTERVAL_SECONDS`]. Out-of-range
/// values are refused by [`ViewerConfig::validate`], never clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryConfig {
    pub interval_seconds: u32,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            interval_seconds: DEFAULT_RECOVERY_INTERVAL_SECONDS,
        }
    }
}

impl RecoveryConfig {
    /// The largest accepted interval: one hour.
    ///
    /// Above it the "bounded window" the snapshot exists for (ADR-0037 D12) is
    /// longer than a normal editing session, so a longer value is refused by
    /// name rather than silently accepted.
    pub const MAX_INTERVAL_SECONDS: u32 = 3600;
}

/// Filesystem locations a user preference may choose (`ui.paths.*`).
///
/// `fontsDir` is an **absolute** directory path, or absent/`null` (and `""` for a
/// cleared preference) when the active directory comes from discovery instead.
/// Existence is deliberately not a schema rule: a persisted directory can be
/// removed between runs, and that must not fail the whole configuration load —
/// the host diagnoses it and falls back to its own discovery order instead.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPaths {
    /// CAD font directory override, or `None`/`""` when the user cleared it.
    pub fonts_dir: Option<String>,
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
    pub units: UnitsConfig,
    pub recovery: RecoveryConfig,
    pub paths: UiPaths,
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

/// Model-space and paper-space canvas backgrounds.
///
/// AutoCAD's display colours distinguish model space from paper space, and this
/// repository can honour that distinction honestly: the renderer clears **one**
/// target per frame, and the frame's space is already part of the view the host
/// publishes. The host therefore resolves the active space to one clear colour
/// per frame, so editing either leaf changes that space's next frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewBackground {
    /// Background while a model-space view is displayed.
    pub model: HexColor,
    /// Background while any paper-space layout is displayed.
    pub paper: HexColor,
}
impl Default for ViewBackground {
    fn default() -> Self {
        Self {
            model: HexColor::BLACK,
            paper: HexColor::BLACK,
        }
    }
}

/// Crosshair sizing for the 2D pointer overlay: the AutoCAD `CURSORSIZE` option.
///
/// `sizePercent` is the **total** crosshair span as an integer percentage of the
/// viewport dimension, split into two equal arms about the cursor and clipped at
/// the viewport edges. `5` is AutoCAD's initial value; `100` spans the whole
/// viewport and reproduces the historical full-view crosshair exactly.
/// Out-of-range values are rejected by [`ViewerConfig::validate`], never
/// clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewCrosshair {
    pub size_percent: u32,
}
impl Default for ViewCrosshair {
    fn default() -> Self {
        Self { size_percent: 5 }
    }
}

/// Pickbox sizing: the AutoCAD `PICKBOX` range (Options ▸ Selection ▸ Pickbox
/// size).
///
/// The pickbox is the square at the crosshair intersection, and its **half**-side
/// is this value in logical pixels converted to world units by the view scale, so
/// the box circumscribes exactly the region a click can hit. AutoCAD reports the
/// option in pixels and so do we (logical, device-independent pixels, the unit
/// the whole overlay/pointer contract uses). `1..=50` mirrors AutoCAD's own
/// `0..=50` range except that `0` is deliberately rejected: "no pickbox /
/// selection preview off" is not modelled here, and accepting `0` would silently
/// disable picking. The default `3` is both AutoCAD's initial value and the value
/// this project has always picked with, so the geometry is unchanged until a user
/// overrides it. Out-of-range and non-integer values are rejected by
/// [`ViewerConfig::validate`], never clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewPickbox {
    pub size_pixels: u32,
}
impl Default for ViewPickbox {
    fn default() -> Self {
        Self { size_pixels: 3 }
    }
}

impl ViewPickbox {
    /// AutoCAD's accepted pickbox range here: `1..=50` (AutoCAD allows `0`, but
    /// "selection preview off" is not modelled, so `0` is rejected).
    pub const RANGE: std::ops::RangeInclusive<u32> = 1..=50;
}

/// Object-snap aperture sizing: the AutoCAD `APERTURE` range (Options ▸ Drafting
/// ▸ Aperture size).
///
/// The aperture is the radius, in logical pixels, within which the object-snap
/// engine considers a snap candidate. It is deliberately **independent** of the
/// pickbox (AutoCAD keeps them as two separate options too) and it never affects
/// picking or the hover highlight. AutoCAD reports the option in pixels and so do
/// we. `1..=50` is AutoCAD's own range; the default `12` is this project's
/// existing aperture (AutoCAD's initial value is 10 — a pre-existing, deliberate
/// difference, not a parity claim). Out-of-range and non-integer values are
/// rejected by [`ViewerConfig::validate`], never clamped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewAperture {
    pub size_pixels: u32,
}
impl Default for ViewAperture {
    fn default() -> Self {
        Self { size_pixels: 12 }
    }
}

impl ViewAperture {
    /// AutoCAD's range for the Options ▸ Drafting aperture size, in pixels.
    pub const RANGE: std::ops::RangeInclusive<u32> = 1..=50;
}

/// GPU memory budget for CAD rendering: an integer percentage of the graphics
/// card's memory.
///
/// This is deliberately a **budget**, not the card's capacity: the renderer
/// resolves the percentage against the device memory it can actually determine
/// and treats the result as the memory CAD may use, never as the card's total.
/// `10..=100` keeps a usable floor and never asks for more than the whole card;
/// values outside it are rejected by [`ViewerConfig::validate`], never clamped.
/// The default `90` leaves headroom for the window compositor and the WebView.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewGpuBudget {
    pub percent: u32,
}
impl Default for ViewGpuBudget {
    fn default() -> Self {
        Self { percent: 90 }
    }
}

impl ViewGpuBudget {
    /// The accepted percentage range of the graphics card's memory.
    pub const RANGE: std::ops::RangeInclusive<u32> = 10..=100;
}

/// Canvas anti-aliasing: the user's MSAA choice (`view.msaa.mode`).
///
/// The persisted/wire spelling is the user-visible token — `off`, `2x` or `4x` —
/// which is why each variant carries an explicit `rename` (serde cannot derive
/// `2x` from an identifier). The choice maps to the renderer's sample count in
/// [`Self::samples`]; the renderer clamps a request to what the device supports
/// and reports the effective count, so this enum is only the *requested* value.
/// The default `4x` is the shipped canvas quality; `off` is exactly one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MsaaMode {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "2x")]
    X2,
    #[default]
    #[serde(rename = "4x")]
    X4,
}

impl MsaaMode {
    /// Every token, in the schema's declaration order.
    pub const ALL: [Self; 3] = [Self::Off, Self::X2, Self::X4];

    /// The stable persisted/wire token (`off`/`2x`/`4x`), also the
    /// `cad_set_setting` value.
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::X2 => "2x",
            Self::X4 => "4x",
        }
    }

    /// Parse an exact token; anything else is `None`, never a default.
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_token() == token)
    }

    /// The MSAA sample count the renderer is asked for: `1` (off), `2` or `4`.
    pub fn samples(self) -> u32 {
        match self {
            Self::Off => 1,
            Self::X2 => 2,
            Self::X4 => 4,
        }
    }
}

/// The `view.msaa` block: one canvas anti-aliasing choice.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewMsaa {
    /// `off`/`2x`/`4x`; the default is `4x` ([`MsaaMode::X4`]).
    pub mode: MsaaMode,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewConfig {
    pub overlays: ViewOverlays,
    pub background: ViewBackground,
    pub crosshair: ViewCrosshair,
    pub pickbox: ViewPickbox,
    pub aperture: ViewAperture,
    pub gpu_budget: ViewGpuBudget,
    pub msaa: ViewMsaa,
}

impl ViewConfig {
    /// The one interaction tolerance policy this view's settings describe.
    ///
    /// This is the single place the configuration is turned into a
    /// [`TolerancePolicy`]: the pickbox leaf becomes
    /// [`TolerancePolicy::pickbox_logical_pixels`] (pick + hover) and the
    /// aperture leaf becomes [`TolerancePolicy::interaction_logical_pixels`]
    /// (object-snap candidates only). Every other field keeps the domain default,
    /// so a default configuration reproduces [`TolerancePolicy::default`] **bit
    /// for bit** — `3` and `12` are exactly representable, so the pinned geometry
    /// and tolerances are unchanged.
    pub fn tolerance_policy(&self) -> TolerancePolicy {
        TolerancePolicy {
            pickbox_logical_pixels: f64::from(self.pickbox.size_pixels),
            interaction_logical_pixels: f64::from(self.aperture.size_pixels),
            ..TolerancePolicy::default()
        }
    }
}

/// Interaction defaults the Options dialog may persist.
///
/// The optional fields are nullable on purpose: `None` means "no user
/// preference", so a host that never configured them keeps the live session's
/// own default (`SessionState` starts object snap/hover preview enabled, grid
/// snap and ortho disabled, and the snap spacing at
/// [`crate::snap::DEFAULT_SNAP_SPACING`]). A persisted `false` is an explicit
/// user choice and must survive a restart, and so is a persisted spacing.
///
/// `snapSpacing` is the `SNAPUNIT` session spacing in drawing units; `None`
/// leaves the session default. A persisted value must be finite and strictly
/// positive ([`crate::snap::snap_spacing_is_valid`]); an invalid one is rejected
/// by [`ViewerConfig::validate`], never clamped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionConfig {
    pub pointer: bool,
    pub touch: bool,
    pub keyboard_shortcuts: bool,
    pub snap_enabled: Option<bool>,
    pub hover_preview: Option<bool>,
    /// `SNAP` (grid snap, F9) session switch; `None` keeps the session default
    /// (`off`).
    pub grid_snap_enabled: Option<bool>,
    /// `ORTHO` (F8) session switch; `None` keeps the session default (`off`).
    pub ortho_enabled: Option<bool>,
    /// `SNAPUNIT` session spacing in drawing units; `None` keeps the session
    /// default ([`crate::snap::DEFAULT_SNAP_SPACING`]).
    pub snap_spacing: Option<f64>,
    /// AutoCAD-like input scheme: when on, a primary-button drag in the 2D view
    /// pans instead of running a box selection. `false` keeps the default
    /// (left drag = box select, middle drag = pan). Unlike the session switches
    /// above this is a plain preference: the gesture reads the effective config
    /// per dispatch, so there is no session state to seed.
    pub left_drag_pans: bool,
    /// 3D orbit sensitivity as a **percentage** of the host's base rate: `100`
    /// (the default) is the base rate, `50` halves the rotation per logical
    /// pixel and `200` doubles it. A plain preference like [`Self::left_drag_pans`]:
    /// the orbit path reads the effective config per event, so there is no
    /// session state to seed and a write is live on the next drag.
    ///
    /// Valid range is `25.0..=200.0` inclusive; a non-finite or out-of-range
    /// value is rejected by [`ViewerConfig::validate`], never clamped. The
    /// gesture additionally clamps the multiplier defensively, so a value that
    /// was hand-edited past validation can never produce a runaway angle.
    pub orbit_sensitivity_percent: f64,
}
impl Default for InteractionConfig {
    fn default() -> Self {
        Self {
            pointer: true,
            touch: true,
            keyboard_shortcuts: true,
            snap_enabled: None,
            hover_preview: None,
            grid_snap_enabled: None,
            ortho_enabled: None,
            snap_spacing: None,
            left_drag_pans: false,
            orbit_sensitivity_percent: 100.0,
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
