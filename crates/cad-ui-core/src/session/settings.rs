//! Typed settings wire surface for the Options dialog.
//!
//! The session exposes the same closed thirty-two-key surface the host
//! persists; it adds no second validation and no raw-JSON entry point. A UI
//! writes one typed key/value pair, the host validates it against the closed
//! table, persists the matching preference leaf and applies it to the live
//! scene/session.

use serde::{Deserialize, Serialize};

use super::diagnostics::cad_error_text;
use super::CadSession;

pub use cad_host::{
    CadSettingKey, CadSettingValue, HostSettings, SETTINGS_INVALID_PLOT_FORMAT,
    SETTINGS_INVALID_VALUE, SETTINGS_UNKNOWN_KEY,
};

/// The effective settings an Options dialog reads and writes.
///
/// A flat camelCase wire object (like the status/session types). `fontsDir` is
/// the font directory actually used and `fontsDirOverride` the persisted choice
/// behind it; both are omitted when the carrier cannot answer them (the Web
/// `dom-canvas` host has no local font directory), which renders the row
/// disabled rather than inventing a path. The other thirty-one fields correspond
/// one-to-one with [`CadSettingKey`] (`fontsDir` included), the two
/// backgrounds as canonical `#rrggbb` strings, the crosshair and the GPU memory
/// budget as integer percentages, the pickbox/aperture as integer pixel counts,
/// the snap spacing as a real number of drawing units, the orbit sensitivity as
/// a real percentage of the host's base rate, the graphics backend as
/// one of `auto`/`vulkan`/`dx12`, the theme as `dark`/`light`, the
/// navigation/ViewCube switches as their host-owned values, the drawing/display
/// units as their schema tokens, and the unit precision / recovery interval as
/// integers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CadSettings {
    pub grid: bool,
    pub axes: bool,
    pub selection_highlight: bool,
    pub snap_hints: bool,
    /// `"svg"` or `"pdf"` only.
    pub plot_format: String,
    pub snap_enabled: bool,
    pub hover_preview: bool,
    /// `SNAP` (grid snap, F9) session switch, persisted as
    /// `interaction.gridSnapEnabled`.
    pub grid_snap_enabled: bool,
    /// `ORTHO` (F8) session switch, persisted as `interaction.orthoEnabled`.
    pub ortho_enabled: bool,
    /// `SNAPUNIT` session spacing in drawing units, persisted as
    /// `interaction.snapSpacing`. Finite and strictly positive.
    pub snap_spacing: f64,
    /// 2D input scheme: when `true`, a primary-button drag in the 2D view pans
    /// instead of box-selecting. Persisted as `interaction.leftDragPans`,
    /// default `false`.
    pub left_drag_pans: bool,
    /// 3D orbit sensitivity as a **percentage** of the host's base rate: `100`
    /// (the default) is the base rate, `50` halves the rotation per logical
    /// pixel and `200` doubles it. Persisted as
    /// `interaction.orbitSensitivityPercent`. Finite and `25.0..=200.0`; the
    /// preference is read per gesture, so a write is live on the next drag.
    pub orbit_sensitivity_percent: f64,
    /// Model-space canvas background, canonical lower-case `#rrggbb`.
    pub background_model: String,
    /// Paper-space canvas background, canonical lower-case `#rrggbb`.
    pub background_paper: String,
    /// Crosshair total span as an integer percentage of the viewport (`1..=100`),
    /// split into equal arms about the cursor and clipped at the edges.
    pub crosshair_percent: u32,
    /// Pickbox half-side in logical pixels (`1..=50`, the AutoCAD `PICKBOX`
    /// option).
    pub pickbox_size: u32,
    /// Object-snap aperture in logical pixels (`1..=50`, the AutoCAD `APERTURE`
    /// option).
    pub aperture_size: u32,
    /// GPU memory budget as an integer percentage of the graphics card's memory
    /// (`10..=100`): a budget for CAD rendering, not the card's total memory.
    pub gpu_budget_percent: u32,
    /// Graphics backend preference: `"auto"` (default), `"vulkan"` or `"dx12"`.
    ///
    /// Deliberately the **snake_case** wire key, unlike every other field: the
    /// `cad_set_setting` key and the persisted `preference.json` leaf are the
    /// same `wgpu_backend`, so the read field and the write key cannot drift
    /// apart. Takes effect on the next presenter construction.
    #[serde(rename = "wgpu_backend")]
    pub wgpu_backend: String,
    /// UI theme: `"dark"` (default) or `"light"`
    /// (root `preference.json` leaf `theme`).
    pub theme: String,
    /// Navigation-bar visibility (root leaf `navbarVisible`, default `true`).
    pub navbar_visible: bool,
    /// ViewCube display bitmask `0..=3` (root leaf `navvcubeDisplay`, default
    /// `3`). Bit 1 is the 3D view class, bit 2 the 2D view class.
    pub navvcube_display: u32,
    /// ViewCube visibility in 2D views (root leaf `viewCubeIn2d`, default
    /// `true`).
    pub view_cube_in_2d: bool,
    /// ViewCube visibility in 3D views (root leaf `viewCubeIn3d`, default
    /// `true`).
    pub view_cube_in_3d: bool,
    /// Reverse-engineered-format notice (root leaf `formatNoticeEnabled`, default
    /// `true`). When `true` the shell shows the post-open hint that the drawing
    /// came from a reverse-engineered format; `false` silences it. Written with
    /// the same key through `cad_set_setting`.
    pub format_notice_enabled: bool,
    /// Drawing unit of the document's coordinates (`ui.units.drawingUnit`):
    /// `drawing` (default), `mm`, `cm`, `m`, `in` or `ft`.
    pub drawing_unit: String,
    /// Unit lengths are displayed in (`ui.units.displayUnit`), the same closed
    /// token set. A typed token over the `drawing` source is refused.
    pub display_unit: String,
    /// Decimal places the display uses (`ui.units.decimalPlaces`, `0..=8`).
    pub unit_precision: u32,
    /// Recovery-snapshot interval in seconds (`ui.recovery.intervalSeconds`,
    /// `0..=3600`); `0` disables the writes.
    pub recovery_interval: u32,
    /// The font directory actually used for font loading (`--fonts-dir` > a
    /// usable persisted `ui.paths.fontsDir` > discovery), or absent when the
    /// carrier has no local directory (Web) or nothing resolves.
    ///
    /// Omitted rather than `null` when `None`: a carrier that cannot answer the
    /// question must not be read as "answered with an empty value". The Options
    /// dialog renders an omitted key as a disabled row with the missing badge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fonts_dir: Option<String>,
    /// The persisted font-directory choice (`ui.paths.fontsDir`), absent when the
    /// user never chose one, cleared it, or the carrier has no local
    /// filesystem (Web). This is the **editable** value: it is what the row
    /// shows and what `cad_set_setting` writes with the key `fontsDir`.
    ///
    /// A choice that no longer exists is still reported here, while
    /// [`Self::fonts_dir`] carries the directory in use (the CLI value or the
    /// discovered default); [`Self::fonts_dir_missing`] is the row state the
    /// dialog turns into `options.status.fontsDirMissing`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fonts_dir_override: Option<String>,
    /// Whether [`Self::fonts_dir_override`] no longer names a usable directory.
    ///
    /// `false` when the user never chose one (and on the Web carrier, which has
    /// no saved local directory at all); the font-directory row there is disabled
    /// by the omitted [`Self::fonts_dir`], not by this flag.
    pub fonts_dir_missing: bool,
}

impl From<HostSettings> for CadSettings {
    fn from(value: HostSettings) -> Self {
        Self {
            grid: value.grid,
            axes: value.axes,
            selection_highlight: value.selection_highlight,
            snap_hints: value.snap_hints,
            plot_format: value.plot_format,
            snap_enabled: value.snap_enabled,
            hover_preview: value.hover_preview,
            grid_snap_enabled: value.grid_snap_enabled,
            ortho_enabled: value.ortho_enabled,
            snap_spacing: value.snap_spacing,
            left_drag_pans: value.left_drag_pans,
            orbit_sensitivity_percent: value.orbit_sensitivity_percent,
            background_model: value.background_model,
            background_paper: value.background_paper,
            crosshair_percent: value.crosshair_percent,
            pickbox_size: value.pickbox_size,
            aperture_size: value.aperture_size,
            gpu_budget_percent: value.gpu_budget_percent,
            wgpu_backend: value.wgpu_backend,
            theme: value.theme,
            navbar_visible: value.navbar_visible,
            navvcube_display: value.navvcube_display,
            view_cube_in_2d: value.view_cube_in_2d,
            view_cube_in_3d: value.view_cube_in_3d,
            format_notice_enabled: value.format_notice_enabled,
            drawing_unit: value.drawing_unit,
            display_unit: value.display_unit,
            unit_precision: value.unit_precision,
            recovery_interval: value.recovery_interval,
            fonts_dir: value.fonts_dir,
            fonts_dir_override: value.fonts_dir_override,
            fonts_dir_missing: value.fonts_dir_missing,
        }
    }
}

impl CadSession {
    /// The current effective settings (config defaults + `config.json` +
    /// `preference.json`, plus the live session switches).
    pub fn settings(&self) -> CadSettings {
        self.core.settings().into()
    }

    /// Apply one typed setting and return the new effective snapshot.
    ///
    /// The key/value pair is validated by the host's closed table *before* any
    /// durable write; an unknown key cannot reach this method (the wire parse
    /// rejects it), and a wrong type/format returns the stable refusal code
    /// with no file change.
    pub fn apply_setting(
        &self,
        key: CadSettingKey,
        value: CadSettingValue,
    ) -> Result<CadSettings, String> {
        // Validate here as well so a bad key/value pair returns the stable code
        // as-is (`CadError::InvalidInput` would otherwise render as a Debug
        // wrapper). The host validates again before it persists anything.
        if let Err(code) = key.validate(&value) {
            let text = code.to_string();
            *self.error.borrow_mut() = Some(text.clone());
            return Err(text);
        }
        match self.core.apply_setting(key, value) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(self.settings())
            }
            Err(error) => {
                // Surface a real persistence/live-apply refusal through the
                // session error (and therefore `cad_status.error`), never only
                // in a log line.
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
