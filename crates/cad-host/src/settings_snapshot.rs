//! The effective settings snapshot an Options dialog reads.
//!
//! Split out of [`crate::settings`] so both files stay inside the file-size gate;
//! the closed key table, its validation and the write path stay there.
use super::*;

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
    /// ACIS payload version for a Save-As (`ui.export.acisVersion`): `"7.0"`
    /// (default) or `"21.0"`. Selects the payload header version only; it never
    /// changes which bodies are exportable.
    pub acis_version: String,
    /// DXF container version for a Save-As (`ui.export.dxfVersion`): `"2010"`
    /// (default), `"2013"` or `"2018"`. A DWG is always AC1032 (R2018) because
    /// the DWG writer's version derives from the document.
    pub dxf_version: String,
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
    /// 3D orbit sensitivity as a percentage of the host's base rate
    /// (`interaction.orbitSensitivityPercent`, default `100`, valid
    /// `25.0..=200.0`).
    pub orbit_sensitivity_percent: f64,
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
        let (acis_version, dxf_version) = {
            let store = self.viewer_config.borrow();
            let export = &store.effective().ui.export;
            (
                export.acis_version.label().to_string(),
                export.dxf_version.label().to_string(),
            )
        };
        let orbit_sensitivity_percent = self.orbit_sensitivity_percent();
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
            acis_version,
            dxf_version,
            snap_enabled,
            hover_preview,
            grid_snap_enabled,
            ortho_enabled,
            snap_spacing,
            left_drag_pans,
            orbit_sensitivity_percent,
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
}
