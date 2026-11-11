//! Typed, closed settings surface for the Options dialog.
//!
//! Only the settings this host actually honours are representable: the four
//! `view.overlays.*` leaves drawn by the render path, the model/paper canvas
//! backgrounds and the crosshair size those frames really use, the
//! `ui.export.plotFormat` default, the five persisted session interaction
//! values (object snap, hover preview, `SNAP` grid snap, `ORTHO` and the
//! `SNAPUNIT` spacing), the persisted 2D left-drag input scheme, the renderer
//! backend preference (`wgpu_backend`, read by the next presenter construction),
//! the GPU memory budget percentage the renderer lane reads
//! (`view.gpuBudget.percent`), the five host-owned appearance/navigation leaves
//! (`theme`, `navbarVisible`, `navvcubeDisplay`, `viewCubeIn2d`,
//! `viewCubeIn3d`) and the read-only `--fonts-dir` display value.
//! Every write goes through the same validated store/persistence path as
//! [`HostCore::apply_user_preference_json`] — the backend leaf and the five
//! appearance/navigation leaves are merged into the same durable preference
//! document (see [`merge_wgpu_backend_into_preference`] and
//! [`crate::settings_root::merge_root_preferences_into_preference`]); no raw JSON
//! is accepted from a UI.
use super::*;

use cad_app::viewer_config::HexColor;

use crate::settings_root::RootPreferences;
pub(crate) use crate::settings_root::{
    merge_wgpu_backend_into_preference, wgpu_backend_from_preference, without_wgpu_backend,
};
pub use crate::settings_root::{
    normalize_wgpu_backend, WGPU_BACKEND_PREFERENCE_KEY, WGPU_BACKEND_VALUES,
};

/// A key outside the closed writable set. Stable, locale-neutral error code.
pub const SETTINGS_UNKNOWN_KEY: &str = "settings.unknown_key";
/// A value whose JSON/typed shape does not match the key (for example a string
/// for a boolean setting). Stable, locale-neutral error code.
pub const SETTINGS_INVALID_VALUE: &str = "settings.invalid_value";
/// A plot format other than the two the CPU vector path can produce.
pub const SETTINGS_INVALID_PLOT_FORMAT: &str = "settings.invalid_plot_format";
/// The host accepted the key/value but could not make it durable (no preference
/// path, a read-only file, a failing write). Stable, locale-neutral prefix; the
/// host appends the underlying reason after `: ` for diagnostics.
///
/// A write with this refusal is **rolled back** in memory too, so neither the
/// snapshot nor the Options dialog reports a value that would disappear on the
/// next start.
pub const SETTINGS_NOT_PERSISTED: &str = "settings.not_persisted";

/// The closed set of settings the Options dialog may write.
///
/// Everything else AutoCAD's Options dialog exposes has no behavioural backing
/// in this repository and is deliberately not representable: an unknown key is
/// refused with [`SETTINGS_UNKNOWN_KEY`], never silently ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CadSettingKey {
    Grid,
    Axes,
    SelectionHighlight,
    SnapHints,
    PlotFormat,
    SnapEnabled,
    HoverPreview,
    /// `SNAP` grid-snap session switch (`interaction.gridSnapEnabled`).
    GridSnapEnabled,
    /// `ORTHO` drafting-constraint session switch (`interaction.orthoEnabled`).
    OrthoEnabled,
    /// `SNAPUNIT` session spacing in drawing units (`interaction.snapSpacing`).
    SnapSpacing,
    /// 2D input scheme: a primary-button drag pans instead of box-selecting
    /// (`interaction.leftDragPans`).
    LeftDragPans,
    BackgroundModel,
    BackgroundPaper,
    CrosshairSize,
    PickboxSize,
    ApertureSize,
    /// GPU memory budget: an integer percentage of the graphics card's memory
    /// (`view.gpuBudget.percent`).
    GpuBudgetPercent,
    /// Renderer graphics backend (`auto`/`vulkan`/`dx12`).
    WgpuBackend,
    /// UI theme (`dark`/`light`): a root preference leaf that also selects the
    /// model/paper canvas backgrounds.
    Theme,
    /// Navigation-bar visibility (root `navbarVisible` leaf).
    NavbarVisible,
    /// ViewCube display bitmask `0..=3` (root `navvcubeDisplay` leaf):
    /// bit 1 = the 3D view class, bit 2 = the 2D view class.
    NavvcubeDisplay,
    /// ViewCube visibility in 2D views (root `viewCubeIn2d` leaf).
    ViewCubeIn2d,
    /// ViewCube visibility in 3D views (root `viewCubeIn3d` leaf).
    ViewCubeIn3d,
}

impl CadSettingKey {
    /// Every writable key, in a stable order.
    pub const ALL: [Self; 23] = [
        Self::Grid,
        Self::Axes,
        Self::SelectionHighlight,
        Self::SnapHints,
        Self::PlotFormat,
        Self::SnapEnabled,
        Self::HoverPreview,
        Self::GridSnapEnabled,
        Self::OrthoEnabled,
        Self::SnapSpacing,
        Self::LeftDragPans,
        Self::BackgroundModel,
        Self::BackgroundPaper,
        Self::CrosshairSize,
        Self::PickboxSize,
        Self::ApertureSize,
        Self::GpuBudgetPercent,
        Self::WgpuBackend,
        Self::Theme,
        Self::NavbarVisible,
        Self::NavvcubeDisplay,
        Self::ViewCubeIn2d,
        Self::ViewCubeIn3d,
    ];

    /// The stable camelCase wire key (also the `cad_set_setting` key).
    ///
    /// Every key but [`Self::WgpuBackend`], [`Self::GpuBudgetPercent`] and the
    /// five [`ROOT_PREFERENCE_KEYS`] leaves is the final segment of a camelCase
    /// `ViewerConfig` path. The backend and the appearance/navigation keys are
    /// root-level host preferences, so their wire key, `cad_set_setting` key and
    /// persisted key are identical; the GPU budget is persisted under
    /// `view.gpuBudget.percent`, whose leaf is `percent`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Axes => "axes",
            Self::SelectionHighlight => "selectionHighlight",
            Self::SnapHints => "snapHints",
            Self::PlotFormat => "plotFormat",
            Self::SnapEnabled => "snapEnabled",
            Self::HoverPreview => "hoverPreview",
            Self::GridSnapEnabled => "gridSnapEnabled",
            Self::OrthoEnabled => "orthoEnabled",
            Self::SnapSpacing => "snapSpacing",
            Self::LeftDragPans => "leftDragPans",
            Self::BackgroundModel => "backgroundModel",
            Self::BackgroundPaper => "backgroundPaper",
            Self::CrosshairSize => "crosshairSize",
            Self::PickboxSize => "pickboxSize",
            Self::ApertureSize => "apertureSize",
            Self::GpuBudgetPercent => "gpuBudgetPercent",
            Self::WgpuBackend => WGPU_BACKEND_PREFERENCE_KEY,
            Self::Theme => "theme",
            Self::NavbarVisible => "navbarVisible",
            Self::NavvcubeDisplay => "navvcubeDisplay",
            Self::ViewCubeIn2d => "viewCubeIn2d",
            Self::ViewCubeIn3d => "viewCubeIn3d",
        }
    }

    /// Parse an exact wire key. `None` means the key is outside the closed set.
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.key() == key)
    }

    /// Validate a typed value for this key.
    ///
    /// Returns the stable refusal code on mismatch. `plotFormat` accepts only its
    /// two representable vector formats, `wgpu_backend` only the three
    /// [`WGPU_BACKEND_VALUES`] tokens (any other string, and any non-string, is
    /// [`SETTINGS_INVALID_VALUE`] — never coerced to `auto`), the two backgrounds
    /// accept only a canonical `#rrggbb` colour, `crosshairSize` accepts only an
    /// integer percentage in `1..=100`, `pickboxSize` an integer pixel count in
    /// `1..=50`, `apertureSize` an integer pixel count in `1..=50` and
    /// `gpuBudgetPercent` an integer percentage of the graphics card's memory in
    /// `10..=100` (all four rejected, never clamped), `snapSpacing` a finite
    /// spacing greater than zero drawing units (an integer or a real; `0`,
    /// negative, non-finite and a wrong shape are rejected, never defaulted), and
    /// every switch (including `gridSnapEnabled`, `orthoEnabled` and
    /// `leftDragPans`) is a boolean.
    pub fn validate(self, value: &CadSettingValue) -> Result<(), &'static str> {
        match self {
            Self::WgpuBackend => match value {
                CadSettingValue::Text(token) if WGPU_BACKEND_VALUES.contains(&token.as_str()) => {
                    Ok(())
                }
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            // The UI theme is one of the two tokens this build draws. An unknown
            // token is refused, never normalized to the default.
            Self::Theme => match value {
                CadSettingValue::Text(token) if THEME_VALUES.contains(&token.as_str()) => Ok(()),
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            // The ViewCube display bitmask is exactly `0..=3` (bit 1 = 3D class,
            // bit 2 = 2D class). Out of range is refused, never clamped.
            Self::NavvcubeDisplay => match value {
                CadSettingValue::Number(display) if *display <= 3 => Ok(()),
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            Self::PlotFormat => match value {
                CadSettingValue::Text(format) if format == "svg" || format == "pdf" => Ok(()),
                CadSettingValue::Text(_) => Err(SETTINGS_INVALID_PLOT_FORMAT),
                CadSettingValue::Bool(_)
                | CadSettingValue::Number(_)
                | CadSettingValue::Real(_) => Err(SETTINGS_INVALID_VALUE),
            },
            // The `SNAPUNIT` spacing: finite and strictly positive, the same rule
            // the session setter and the config validator enforce. An integer
            // wire number and a fractional one are both accepted, and neither is
            // silently clamped or truncated.
            Self::SnapSpacing => match value {
                CadSettingValue::Number(spacing) if *spacing > 0 => Ok(()),
                CadSettingValue::Real(spacing) if cad_app::snap_spacing_is_valid(*spacing) => {
                    Ok(())
                }
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            Self::BackgroundModel | Self::BackgroundPaper => match value {
                CadSettingValue::Text(text) if HexColor::parse(text).is_some() => Ok(()),
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            Self::CrosshairSize => match value {
                CadSettingValue::Number(percent) if (1..=100).contains(percent) => Ok(()),
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            // AutoCAD's own option ranges, in pixels (the same unit the schema
            // documents). Out-of-range values are refused, never clamped.
            Self::PickboxSize => match value {
                CadSettingValue::Number(pixels)
                    if cad_app::viewer_config::ViewPickbox::RANGE.contains(pixels) =>
                {
                    Ok(())
                }
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            Self::ApertureSize => match value {
                CadSettingValue::Number(pixels)
                    if cad_app::viewer_config::ViewAperture::RANGE.contains(pixels) =>
                {
                    Ok(())
                }
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            // The share of the graphics card's memory CAD may use. The renderer
            // lane reads the effective config; this table only refuses values
            // outside `10..=100`, never clamps them.
            Self::GpuBudgetPercent => match value {
                CadSettingValue::Number(percent)
                    if cad_app::viewer_config::ViewGpuBudget::RANGE.contains(percent) =>
                {
                    Ok(())
                }
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            Self::Grid
            | Self::Axes
            | Self::SelectionHighlight
            | Self::SnapHints
            | Self::SnapEnabled
            | Self::HoverPreview
            | Self::GridSnapEnabled
            | Self::OrthoEnabled
            | Self::LeftDragPans
            | Self::NavbarVisible
            | Self::ViewCubeIn2d
            | Self::ViewCubeIn3d => match value {
                CadSettingValue::Bool(_) => Ok(()),
                CadSettingValue::Text(_)
                | CadSettingValue::Number(_)
                | CadSettingValue::Real(_) => Err(SETTINGS_INVALID_VALUE),
            },
        }
    }
}

/// A typed setting value: the wire surface carries one boolean, one string, one
/// non-negative integer (a percentage or a pixel count) or one real.
#[derive(Debug, Clone, PartialEq)]
pub enum CadSettingValue {
    Bool(bool),
    Text(String),
    Number(u32),
    /// A real quantity: the [`CadSettingKey::SnapSpacing`] `SNAPUNIT` spacing in
    /// drawing units. Deliberately a separate shape from [`Self::Number`] so an
    /// integer-only leaf can never receive `90.5` and the spacing can carry
    /// `2.5`; the key table decides which of the two a key accepts.
    Real(f64),
}

/// The effective settings an Options dialog reads.
///
/// Derived from the effective config (defaults, then `config.json`, then the
/// host-allowed `preference.json`) plus the live session state. `fonts_dir` is
/// read-only display data from [`HostOptions`].
#[derive(Debug, Clone, PartialEq)]
pub struct HostSettings {
    pub grid: bool,
    pub axes: bool,
    pub selection_highlight: bool,
    pub snap_hints: bool,
    pub plot_format: String,
    pub snap_enabled: bool,
    pub hover_preview: bool,
    /// `SNAP` (grid snap, F9) session switch. `false` until the user turns it on.
    pub grid_snap_enabled: bool,
    /// `ORTHO` (F8) drafting-constraint session switch.
    pub ortho_enabled: bool,
    /// `SNAPUNIT` session spacing in drawing units
    /// ([`cad_app::DEFAULT_SNAP_SPACING`] until the user sets one).
    pub snap_spacing: f64,
    /// 2D input scheme: `true` means a primary-button drag pans instead of
    /// box-selecting (`interaction.leftDragPans`, default `false`).
    pub left_drag_pans: bool,
    /// Canonical lower-case `#rrggbb` of the model-space background.
    pub background_model: String,
    /// Canonical lower-case `#rrggbb` of the paper-space background.
    pub background_paper: String,
    /// Crosshair total span as an integer percentage of the viewport
    /// (`1..=100`); equal arms about the cursor, clipped at the edges.
    pub crosshair_percent: u32,
    /// Pickbox half-side in logical pixels (`1..=50`, the AutoCAD `PICKBOX`
    /// option).
    pub pickbox_size: u32,
    /// Object-snap aperture in logical pixels (`1..=50`, the AutoCAD `APERTURE`
    /// option).
    pub aperture_size: u32,
    /// GPU memory budget as an integer percentage of the graphics card's memory
    /// (`10..=100`). A budget for CAD rendering, not the card's total memory.
    pub gpu_budget_percent: u32,
    /// Renderer graphics backend: one of [`WGPU_BACKEND_VALUES`], `"auto"` when
    /// the user never chose one. Takes effect on the next presenter
    /// construction (the renderer backend cannot change inside a live surface).
    pub wgpu_backend: String,
    /// UI theme: one of [`THEME_VALUES`], `"dark"` when the user never chose one.
    /// Applying a theme also writes the model/paper canvas backgrounds
    /// ([`THEME_DARK_BACKGROUND`] / [`THEME_LIGHT_BACKGROUND`]).
    pub theme: String,
    /// Navigation-bar visibility (root `navbarVisible` leaf, default `true`).
    pub navbar_visible: bool,
    /// ViewCube display bitmask `0..=3` (root `navvcubeDisplay` leaf, default
    /// `3` = both view classes). Bit 1 is the 3D class, bit 2 the 2D class.
    pub navvcube_display: u32,
    /// ViewCube visibility in 2D views (root `viewCubeIn2d` leaf, default
    /// `true`).
    pub view_cube_in_2d: bool,
    /// ViewCube visibility in 3D views (root `viewCubeIn3d` leaf, default
    /// `true`).
    pub view_cube_in_3d: bool,
    pub fonts_dir: Option<String>,
}

impl HostCore {
    /// The effective settings snapshot (never a guess: config + live session).
    pub fn settings(&self) -> HostSettings {
        let (
            grid,
            axes,
            selection_highlight,
            snap_hints,
            plot_format,
            background_model,
            background_paper,
            crosshair_percent,
            pickbox_size,
            aperture_size,
            gpu_budget_percent,
        ) = {
            let store = self.viewer_config.borrow();
            let effective = store.effective();
            (
                effective.view.overlays.grid,
                effective.view.overlays.axes,
                effective.view.overlays.selection_highlight,
                effective.view.overlays.snap_hints,
                effective.ui.export.plot_format.as_str().to_string(),
                effective.view.background.model.to_hex(),
                effective.view.background.paper.to_hex(),
                effective.view.crosshair.size_percent,
                effective.view.pickbox.size_pixels,
                effective.view.aperture.size_pixels,
                effective.view.gpu_budget.percent,
            )
        };
        // A plain preference (no session state): the gesture reads the effective
        // config, so the snapshot reports it directly.
        let left_drag_pans = self
            .viewer_config
            .borrow()
            .effective()
            .interaction
            .left_drag_pans;
        let (snap_enabled, hover_preview, grid_snap_enabled, ortho_enabled, snap_spacing) = {
            let controller = self.controller.borrow();
            (
                controller.snap_enabled(),
                controller.hover_preview_enabled(),
                controller.snap_grid_enabled(),
                controller.ortho_enabled(),
                controller.snap_spacing(),
            )
        };
        HostSettings {
            grid,
            axes,
            selection_highlight,
            snap_hints,
            plot_format,
            snap_enabled,
            hover_preview,
            grid_snap_enabled,
            ortho_enabled,
            snap_spacing,
            left_drag_pans,
            background_model,
            background_paper,
            crosshair_percent,
            pickbox_size,
            aperture_size,
            gpu_budget_percent,
            wgpu_backend: normalize_wgpu_backend(self.wgpu_backend.borrow().as_deref()).to_string(),
            // The five host-owned appearance/navigation leaves live outside the
            // `ViewerConfig` schema; the documented defaults apply when the user
            // never chose one.
            theme: {
                let preferences = self.root_preferences.borrow();
                preferences.theme().to_string()
            },
            navbar_visible: self.root_preferences.borrow().navbar_visible(),
            navvcube_display: self.root_preferences.borrow().navvcube_display(),
            view_cube_in_2d: self.root_preferences.borrow().view_cube_in_2d(),
            view_cube_in_3d: self.root_preferences.borrow().view_cube_in_3d(),
            fonts_dir: self
                .options
                .fonts_dir
                .as_ref()
                .map(|path| path.display().to_string()),
        }
    }

    /// The one interaction tolerance policy the effective config describes.
    ///
    /// Built by [`cad_app::viewer_config::ViewConfig::tolerance_policy`], so the
    /// configuration is turned into a policy in exactly one place. Every pick,
    /// hover and snap path reads this value (the live paths call it per dispatch;
    /// [`HostCore::sync_render`] also installs it on the controller), which is why
    /// a settings write takes effect on the next interaction without reopening the
    /// document.
    pub(crate) fn tolerance_policy(&self) -> cad_domain::TolerancePolicy {
        self.viewer_config
            .borrow()
            .effective()
            .view
            .tolerance_policy()
    }

    /// The persisted 2D input scheme: `true` means a primary-button drag pans
    /// instead of box-selecting (`interaction.leftDragPans`, default `false`).
    ///
    /// Read from the effective config per call — the same source the gesture
    /// arbitration uses — so a settings write takes effect on the next drag
    /// without reopening the document and there is no session state to seed. A
    /// missing preference is the documented `false` default, never an invented
    /// `true`.
    ///
    /// The gesture arbitration in `input/pointer.rs` is the consumer; it is the
    /// one in-crate call site that does not exist yet, so the getter is allowed to
    /// be unused until that lane lands its arbitration.
    #[allow(dead_code)]
    pub(crate) fn left_drag_pans(&self) -> bool {
        self.viewer_config
            .borrow()
            .effective()
            .interaction
            .left_drag_pans
    }

    /// Apply one typed setting: persist it under the allowed preference leaves
    /// and make it take effect on the live scene/session immediately.
    ///
    /// The value is validated against the closed key table first, so a wrong
    /// type or an unsupported plot format is refused before any file write.
    pub fn apply_setting(&self, key: CadSettingKey, value: CadSettingValue) -> CadResult<()> {
        key.validate(&value)
            .map_err(|code| CadError::InvalidInput(code.to_string()))?;
        match key {
            CadSettingKey::Grid => {
                self.apply_overlay_preference("grid", &value)?;
                // The display grid has two gates: the persisted preference AND
                // the live session switch. An Options write is an explicit user
                // choice, so it also drives the live switch — otherwise checking
                // "Grid" would persist a preference that stays hidden until F7.
                // F7/`GRID` still toggles the session gate on its own.
                if let CadSettingValue::Bool(enabled) = value {
                    if self.controller.borrow().grid_enabled() != enabled {
                        self.controller.borrow_mut().set_grid_enabled(enabled);
                        self.sync_render()?;
                    }
                }
                Ok(())
            }
            CadSettingKey::Axes => self.apply_overlay_preference("axes", &value),
            CadSettingKey::SelectionHighlight => {
                self.apply_overlay_preference("selectionHighlight", &value)
            }
            CadSettingKey::SnapHints => self.apply_overlay_preference("snapHints", &value),
            CadSettingKey::PlotFormat => self.apply_plot_format(&value),
            CadSettingKey::SnapEnabled => self.apply_session_switch(key, &value),
            CadSettingKey::HoverPreview => self.apply_session_switch(key, &value),
            CadSettingKey::GridSnapEnabled | CadSettingKey::OrthoEnabled => {
                self.apply_drafting_switch(key, &value)
            }
            CadSettingKey::SnapSpacing => self.apply_snap_spacing(&value),
            CadSettingKey::LeftDragPans => self.apply_interaction_flag("leftDragPans", &value),
            CadSettingKey::BackgroundModel => self.apply_background_preference("model", &value),
            CadSettingKey::BackgroundPaper => self.apply_background_preference("paper", &value),
            CadSettingKey::CrosshairSize => self.apply_crosshair_size(&value),
            CadSettingKey::PickboxSize => self.apply_pixel_size("pickbox", &value),
            CadSettingKey::ApertureSize => self.apply_pixel_size("aperture", &value),
            CadSettingKey::GpuBudgetPercent => self.apply_gpu_budget_percent(&value),
            CadSettingKey::WgpuBackend => self.apply_wgpu_backend(&value),
            CadSettingKey::Theme => self.apply_theme(&value),
            CadSettingKey::NavbarVisible => {
                self.apply_root_flag(&value, |preferences| &mut preferences.navbar_visible)
            }
            CadSettingKey::NavvcubeDisplay => self.apply_navvcube_display(&value),
            CadSettingKey::ViewCubeIn2d => {
                self.apply_root_flag(&value, |preferences| &mut preferences.view_cube_in_2d)
            }
            CadSettingKey::ViewCubeIn3d => {
                self.apply_root_flag(&value, |preferences| &mut preferences.view_cube_in_3d)
            }
        }
    }

    /// Persist the renderer backend choice at the root of the preference document.
    ///
    /// This is the one setting that is not a `ViewerConfig` leaf, so it cannot go
    /// through [`Self::apply_user_preference_json`] (the host-allowed path
    /// projection would reject it). It is written through the same
    /// [`ConfigStore`] every other preference uses, merged into the projected
    /// document, and kept in memory so
    /// [`HostCore::persist_preference`](crate::HostCore) re-emits it on every
    /// later write instead of dropping it. A failed write rolls the in-memory
    /// value back and returns the error — never a silent success.
    fn apply_wgpu_backend(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Text(token) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        if !WGPU_BACKEND_VALUES.contains(&token.as_str()) {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        }
        let previous = self.wgpu_backend.replace(Some(token.clone()));
        if let Err(error) = self.persist_preference() {
            *self.wgpu_backend.borrow_mut() = previous;
            return Err(preference_persist_refusal(&error));
        }
        Ok(())
    }

    /// Persist one `view.background.<leaf>` colour.
    ///
    /// The value is already validated, so the canonical spelling written here is
    /// the same colour the store parses back; the write path republishes the
    /// render inputs through `sync_render`, so the active space's next frame
    /// clears to the new colour without reopening the document.
    pub(crate) fn apply_background_preference(
        &self,
        leaf: &str,
        value: &CadSettingValue,
    ) -> CadResult<()> {
        let CadSettingValue::Text(text) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        let colour =
            HexColor::parse(text).ok_or(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()))?;
        self.apply_user_preference_json(&format!(
            r#"{{"view":{{"background":{{"{leaf}":"{colour}"}}}}}}"#
        ))
    }

    /// Persist `view.crosshair.sizePercent`.
    ///
    /// The live overlay reads the effective config on the next `sync_render`, so a
    /// resize shows on the next frame.
    fn apply_crosshair_size(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Number(percent) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        self.apply_user_preference_json(&format!(
            r#"{{"view":{{"crosshair":{{"sizePercent":{percent}}}}}}}"#
        ))
    }

    /// Persist one integer pixel-size leaf: `view.pickbox.sizePixels` or
    /// `view.aperture.sizePixels`.
    ///
    /// The key table has already refused an out-of-range value, so the number
    /// written here is exactly what the schema validates back. The write path
    /// republishes through `sync_render`, which installs the derived
    /// [`cad_domain::TolerancePolicy`] on the session and on the cursor overlay
    /// inputs, so the next hover/pick/snap uses the new size without reopening the
    /// document.
    fn apply_pixel_size(&self, leaf: &str, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Number(pixels) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        self.apply_user_preference_json(&format!(
            r#"{{"view":{{"{leaf}":{{"sizePixels":{pixels}}}}}}}"#
        ))
    }

    /// Persist `view.gpuBudget.percent`.
    ///
    /// The value is already validated against `10..=100`, so the number written
    /// here is exactly what the schema validates back. The renderer lane that
    /// applies the budget reads the effective config (or the same durable
    /// preference document); this host only stores and republishes it, so an
    /// unrelated setting write keeps it.
    fn apply_gpu_budget_percent(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Number(percent) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        self.apply_user_preference_json(&format!(
            r#"{{"view":{{"gpuBudget":{{"percent":{percent}}}}}}}"#
        ))
    }

    /// Persist one `view.overlays.<leaf>` preference. `apply_user_preference_json`
    /// recomputes the effective config and republishes the render inputs through
    /// `sync_render`, so the overlay visibility changes immediately.
    fn apply_overlay_preference(&self, leaf: &str, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Bool(enabled) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        self.apply_user_preference_json(&format!(
            r#"{{"view":{{"overlays":{{"{leaf}":{enabled}}}}}}}"#
        ))
    }

    /// Persist `ui.export.plotFormat`; the save path reads it on the next plot.
    fn apply_plot_format(&self, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Text(format) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        self.apply_user_preference_json(&format!(
            r#"{{"ui":{{"export":{{"plotFormat":"{format}"}}}}}}"#
        ))
    }

    /// Persist an interaction switch and drive the existing session toggle when
    /// the live value differs.
    ///
    /// The toggle is the same `CommandId` path the F3/`HOVERPREVIEW` selectors
    /// use (never a second implementation), so its side effects — including the
    /// hover-tint clear when the preview is switched off — stay identical.
    fn apply_session_switch(&self, key: CadSettingKey, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Bool(enabled) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        let (leaf, command) = match key {
            CadSettingKey::SnapEnabled => ("snapEnabled", CommandId::ToggleSnap),
            CadSettingKey::HoverPreview => ("hoverPreview", CommandId::ToggleHoverPreview),
            _ => return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into())),
        };
        self.apply_user_preference_json(&format!(r#"{{"interaction":{{"{leaf}":{enabled}}}}}"#))?;
        let current = match key {
            CadSettingKey::SnapEnabled => self.controller.borrow().snap_enabled(),
            _ => self.controller.borrow().hover_preview_enabled(),
        };
        if current != *enabled {
            self.command(command, CommandPayload::None)?;
        }
        Ok(())
    }

    /// Persist a drafting-session switch (`SNAP` grid snap or `ORTHO`) and drive
    /// the existing session toggle when the live value differs.
    ///
    /// The toggle is the same `CommandId` path the F9/`F8` selectors and the
    /// typed `SNAP`/`ORTHO` lines use (never a second implementation), and it is
    /// dispatched **only** when the live value really differs: a write that
    /// matches the current state is a pure preference write, so it cannot flip the
    /// switch back and cannot stage anything.
    fn apply_drafting_switch(&self, key: CadSettingKey, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Bool(enabled) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        let (leaf, command) = match key {
            CadSettingKey::GridSnapEnabled => ("gridSnapEnabled", CommandId::ToggleSnapGrid),
            CadSettingKey::OrthoEnabled => ("orthoEnabled", CommandId::ToggleOrtho),
            _ => return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into())),
        };
        self.apply_user_preference_json(&format!(r#"{{"interaction":{{"{leaf}":{enabled}}}}}"#))?;
        let current = match key {
            CadSettingKey::GridSnapEnabled => self.controller.borrow().snap_grid_enabled(),
            _ => self.controller.borrow().ortho_enabled(),
        };
        if current != *enabled {
            self.command(command, CommandPayload::None)?;
        }
        Ok(())
    }

    /// Persist `interaction.snapSpacing` and apply it through the typed
    /// `SNAPUNIT` command path when the live spacing differs.
    ///
    /// The key table has already refused a non-finite or non-positive spacing, so
    /// the number written here is exactly the value the schema validates back. The
    /// live application reuses [`CommandId::SetSnapSpacing`], whose payload is
    /// validated by the session again; a spacing that already matches is not
    /// re-dispatched.
    fn apply_snap_spacing(&self, value: &CadSettingValue) -> CadResult<()> {
        let spacing = match value {
            CadSettingValue::Real(spacing) => *spacing,
            CadSettingValue::Number(spacing) => f64::from(*spacing),
            _ => return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into())),
        };
        if !cad_app::snap_spacing_is_valid(spacing) {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        }
        self.apply_user_preference_json(&format!(
            r#"{{"interaction":{{"snapSpacing":{spacing}}}}}"#
        ))?;
        if self.controller.borrow().snap_spacing() != spacing {
            self.command(
                CommandId::SetSnapSpacing,
                CommandPayload::SnapSpacing(spacing),
            )?;
        }
        Ok(())
    }

    /// Persist one boolean `interaction.<leaf>` flag (`leftDragPans`).
    ///
    /// Unlike the F8/F9 switches this flag has no session state to drive: the
    /// gesture arbitration reads the effective config per event, so persisting
    /// the leaf is the live application. The write goes through
    /// [`HostCore::apply_user_preference_json`], which persists the allowed
    /// projection, so the choice survives a restart and the next drag honours it
    /// without reopening the document.
    fn apply_interaction_flag(&self, leaf: &str, value: &CadSettingValue) -> CadResult<()> {
        let CadSettingValue::Bool(enabled) = value else {
            return Err(CadError::InvalidInput(SETTINGS_INVALID_VALUE.into()));
        };
        self.apply_user_preference_json(&format!(r#"{{"interaction":{{"{leaf}":{enabled}}}}}"#))
    }

    /// Apply the persisted interaction values to the live session.
    ///
    /// Called after startup config load and after every authoritative document
    /// replacement (a replacement rebuilds `SessionState`, which would otherwise
    /// reset the switches and the spacing to their defaults). `None` means "the
    /// user never chose", so the session default is left untouched.
    pub(crate) fn seed_session_interaction(&self) {
        let (snap_enabled, hover_preview, grid_snap_enabled, ortho_enabled, snap_spacing) = {
            let store = self.viewer_config.borrow();
            let interaction = &store.effective().interaction;
            (
                interaction.snap_enabled,
                interaction.hover_preview,
                interaction.grid_snap_enabled,
                interaction.ortho_enabled,
                interaction.snap_spacing,
            )
        };
        if snap_enabled.is_none()
            && hover_preview.is_none()
            && grid_snap_enabled.is_none()
            && ortho_enabled.is_none()
            && snap_spacing.is_none()
        {
            return;
        }
        let mut controller = self.controller.borrow_mut();
        if let Some(enabled) = snap_enabled {
            controller.session.set_snap_enabled(enabled);
        }
        if let Some(enabled) = hover_preview {
            controller.session.set_hover_preview(enabled);
        }
        if let Some(enabled) = grid_snap_enabled {
            controller.session.set_snap_grid_enabled(enabled);
        }
        if let Some(enabled) = ortho_enabled {
            controller.session.set_ortho_enabled(enabled);
        }
        if let Some(spacing) = snap_spacing {
            // The effective config is validated (`interaction.snapSpacing` must be
            // finite and > 0), so the session setter cannot refuse it; a refusal
            // here would be a validator bug, not a value to silently replace with
            // the default.
            controller
                .session
                .set_snap_spacing(spacing)
                .expect("a validated interaction.snapSpacing cannot be refused");
        }
    }

    /// Install the configured tolerance policy on the live session.
    ///
    /// The pickbox/aperture are not session switches: they are derived from the
    /// effective config every time they are read. This installer exists for the
    /// two paths that rebuild the controller state out from under the
    /// configuration — startup and document replacement — so a persisted
    /// pickbox/aperture survives open/new exactly like `snapEnabled` does, and for
    /// `sync_render` so a live write reaches the snap engine immediately.
    pub(crate) fn seed_interaction_tolerance(&self) {
        let policy = self.tolerance_policy();
        self.controller.borrow_mut().set_tolerance_policy(policy);
    }
}

/// The stable refusal a preference write returns when it cannot be made durable.
///
/// The code leads ([`SETTINGS_NOT_PERSISTED`]) so the wire/UI can map it to
/// catalog copy; the store's own reason follows for diagnostics. The caller has
/// already rolled the in-memory change back, so no layer can report a value that
/// would disappear on the next start.
pub(crate) fn preference_persist_refusal(error: &CadError) -> CadError {
    CadError::Unsupported(format!(
        "{SETTINGS_NOT_PERSISTED}: {}",
        store_error_reason(error)
    ))
}

/// The inner reason of a store error, without the `CadError` debug wrapper.
///
/// `CadError::Display` delegates to `Debug`, so `to_string()` on the common
/// string-bearing variants would render `InvalidInput("…")`; diagnostics should
/// carry the reason itself, not the variant name.
fn store_error_reason(error: &CadError) -> String {
    match error {
        CadError::NotImplemented(feature) => (*feature).to_string(),
        CadError::InvalidInput(reason)
        | CadError::Unsupported(reason)
        | CadError::ResourceMissing(reason)
        | CadError::CorruptData(reason)
        | CadError::GpuFailure(reason)
        | CadError::Invariant(reason) => reason.clone(),
        other => other.to_string(),
    }
}

impl HostCore {
    /// Apply a host-allowed user preference and persist the allowed projection.
    ///
    /// The durable write is part of the change, not a follow-up: the store's
    /// previous document is captured first, and a failed persistence restores it
    /// before the refusal is returned. A setting that cannot survive a restart is
    /// therefore **not applied in memory either**, so `cad_settings` (and the
    /// Options dialog's read-back) can never report a value that was never
    /// written. The projection is written from the store, not from a
    /// `ConfigObserver` (which would re-borrow the store while it is mutably
    /// borrowed).
    pub fn apply_user_preference_json(&self, text: &str) -> CadResult<()> {
        self.require_config_idle()?;
        let previous = self.viewer_config.borrow().user_preference_json();
        self.viewer_config
            .borrow_mut()
            .apply_user_preference_json(text)
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        self.clear_gestures();
        if let Err(error) = self.persist_preference() {
            self.rollback_user_preference(&previous);
            return Err(preference_persist_refusal(&error));
        }
        self.config_dirty.set(true);
        self.full_dirty.set(true);
        self.sync_render()
    }

    /// Apply a durable preference document that was just read at startup.
    ///
    /// The bytes already exist on disk, so this is the one path that does **not**
    /// require a successful write: a store that cannot re-persist its normalized
    /// projection still honors what the user saved for this session (the caller
    /// records that as a diagnostic). Runtime writes use
    /// [`Self::apply_user_preference_json`], which is all-or-nothing.
    pub(crate) fn load_user_preference(&self, text: &str) -> CadResult<()> {
        self.viewer_config
            .borrow_mut()
            .apply_user_preference_json(text)
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        self.clear_gestures();
        self.config_dirty.set(true);
        self.full_dirty.set(true);
        self.sync_render()
    }

    /// Restore a previously accepted preference document after a failed write.
    ///
    /// The snapshot was accepted by this store under the same host config, so
    /// resolving it again succeeds. If it somehow does not, the persistence
    /// refusal remains the authoritative error and the next write or the next
    /// start reconciles the store; this never panics.
    fn rollback_user_preference(&self, previous: &str) {
        let _ = self
            .viewer_config
            .borrow_mut()
            .restore_user_preference_json(previous);
    }

    /// Drop the stored user preference so the host defaults apply again.
    ///
    /// This is a real clear of the single [`ViewerConfigStore`]: the accumulated
    /// preference is emptied, the allowed (now empty) projection is persisted
    /// through the same policy as [`Self::apply_user_preference_json`], and the
    /// config + render inputs are republished. It never fakes success: a failed
    /// persistence restores the previous document (and the backend leaf) and
    /// returns [`SETTINGS_NOT_PERSISTED`], so nothing reads as cleared while the
    /// durable file still holds the old values.
    pub fn clear_user_preference(&self) -> CadResult<()> {
        self.require_config_idle()?;
        let previous = self.viewer_config.borrow().user_preference_json();
        let previous_backend = self.wgpu_backend.borrow().clone();
        let previous_root = self.root_preferences.borrow().clone();
        let previous_recent = self.recent_files.borrow().clone();
        self.viewer_config.borrow_mut().clear_user_preference();
        // The renderer backend choice is a user preference too, even though it
        // is not a `ViewerConfig` leaf: a real clear resets it to the `auto`
        // default and removes the leaf from the durable document. The five
        // appearance/navigation leaves are reset the same way (documented
        // defaults; no stale leaf survives). Recent drawings are user data, not
        // a setting, but a real clear drops them too: the `recentFiles` leaf is
        // removed with the rest.
        *self.wgpu_backend.borrow_mut() = None;
        *self.root_preferences.borrow_mut() = RootPreferences::default();
        self.recent_files.borrow_mut().clear();
        self.clear_gestures();
        self.config_dirty.set(true);
        self.full_dirty.set(true);
        if let Err(error) = self.persist_preference() {
            self.rollback_user_preference(&previous);
            *self.wgpu_backend.borrow_mut() = previous_backend;
            *self.root_preferences.borrow_mut() = previous_root;
            *self.recent_files.borrow_mut() = previous_recent;
            return Err(preference_persist_refusal(&error));
        }
        self.sync_render()
    }
}
