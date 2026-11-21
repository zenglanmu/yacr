//! invocation module.

use super::*;
use cad_app::StandardView;

/// Machine schema version for CLI result and error documents.
///
/// This is a stable contract identifier: it does not change with `--locale`.
pub const CLI_SCHEMA_VERSION: u32 = 1;

/// Default locale for human-facing messages (simplified Chinese).
pub const DEFAULT_LOCALE: &str = "zh-CN";

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliOperation {
    Scan,
    ProxyReport,
    Measure,
    BuildRepresentation,
    FixedViewportRender,
    Plot,
    Benchmark,
}

impl CliOperation {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "scan" => Some(Self::Scan),
            "proxy-report" => Some(Self::ProxyReport),
            "measure" => Some(Self::Measure),
            "build-representation" => Some(Self::BuildRepresentation),
            "render" => Some(Self::FixedViewportRender),
            "plot" => Some(Self::Plot),
            "benchmark" => Some(Self::Benchmark),
            _ => None,
        }
    }

    /// The stable machine key reported in result and error documents.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::ProxyReport => "proxy-report",
            Self::Measure => "measure",
            Self::BuildRepresentation => "build-representation",
            Self::FixedViewportRender => "render",
            Self::Plot => "plot",
            Self::Benchmark => "benchmark",
        }
    }
}

// ---------------------------------------------------------------------------
// Human locale (never affects machine output)
// ---------------------------------------------------------------------------

/// Locale for human-facing strings printed on stderr.
///
/// Machine output (stdout JSON and error `code`s) is locale-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    ZhCn,
    En,
}

impl Locale {
    /// Normalize an input tag; unsupported tags fall back to `zh-CN`.
    pub fn parse(tag: &str) -> Self {
        let lower = tag.trim().to_ascii_lowercase();
        if lower == "en" || lower.starts_with("en-") {
            Locale::En
        } else {
            Locale::ZhCn
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Locale::ZhCn => "zh-CN",
            Locale::En => "en",
        }
    }
}

// ---------------------------------------------------------------------------
// Plot output format
// ---------------------------------------------------------------------------

/// Output format for the `plot` operation.
///
/// `Png` is the raster path (headless GPU + PNG encode, the default). `Svg` and
/// `Pdf` are the CPU-only vector path: they never create a GPU device, so they
/// work on a machine with no Vulkan adapter at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlotFormat {
    #[default]
    Png,
    Svg,
    Pdf,
}

impl PlotFormat {
    /// Parse a user-supplied `--plot-format` value. Unknown values stay `None`
    /// so the caller can report a usage error instead of guessing.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "svg" => Some(Self::Svg),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    /// The stable machine key (extension-friendly).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
        }
    }

    /// True for the CPU-only vector formats, which must not touch a GPU.
    pub fn is_vector(self) -> bool {
        matches!(self, Self::Svg | Self::Pdf)
    }
}

// ---------------------------------------------------------------------------
// GPU adapter preference (CLI-local so wasm stays free of `cad-render-wgpu`)
// ---------------------------------------------------------------------------

/// Adapter preference for `render`/`plot` (`--gpu auto|high|low`).
///
/// Mirrors `cad_render_wgpu::GpuSelection` semantics; the conversion to the
/// renderer type exists only on native (see `ops.rs::render_gpu`), because
/// `cad-cli-tools` depends on the renderer only natively. `render`/`plot` are
/// themselves native operations, so the field is absent on wasm.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuPreference {
    /// First adapter of the highest-priority backend.
    #[default]
    Auto,
    /// Prefer a discrete GPU; fall back to the backend default when absent.
    HighPerformance,
    /// Prefer an integrated GPU; fall back to the backend default when absent.
    LowPower,
}

#[cfg(not(target_arch = "wasm32"))]
impl GpuPreference {
    /// Stable CLI/UI name.
    pub fn as_str(self) -> &'static str {
        match self {
            GpuPreference::Auto => "auto",
            GpuPreference::HighPerformance => "high",
            GpuPreference::LowPower => "low",
        }
    }
    /// Parse the stable name; unknown values are `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(GpuPreference::Auto),
            "high" => Some(GpuPreference::HighPerformance),
            "low" => Some(GpuPreference::LowPower),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// GPU memory budget flag (CLI-local so wasm stays free of `cad-render-wgpu`)
// ---------------------------------------------------------------------------

/// The existence of `--gpu-budget-mb` in the process arguments.
///
/// Tri-state on purpose: an **absent** flag must leave the renderer's own policy
/// (`YACR_GPU_BUDGET_MB`, else the percentage of probed VRAM, else no cap at all
/// when the backend cannot be probed) untouched, while `--gpu-budget-mb 0` is an explicit
/// "uncapped" that must beat both. Collapsing the two into `Option<u64>` would
/// make `0` indistinguishable from "not given".
///
/// Native-only meaning (`render`/`plot` are native operations), but the type
/// itself is plain data so the wasm build keeps compiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuBudgetFlag {
    /// `--gpu-budget-mb` was not passed: the renderer's env/default policy holds.
    #[default]
    Absent,
    /// `--gpu-budget-mb 0`: explicitly no cap (reported as `uncapped`).
    Uncapped,
    /// `--gpu-budget-mb <n>`: the cap, already converted to **bytes**.
    Capped(u64),
}

/// One mebibyte: the unit of `--gpu-budget-mb` and `YACR_GPU_BUDGET_MB`.
pub(crate) const MIB: u64 = 1024 * 1024;

/// Parse a `--gpu-budget-mb` value into [`GpuBudgetFlag`].
///
/// `0` is the documented explicit "uncapped"; anything else must be a whole
/// number of MiB that still fits in a `u64` of bytes. A bad value is an error
/// (never a silent fall-back to the default), because a typo must not look
/// applied.
pub fn parse_gpu_budget_flag(value: &str) -> Result<GpuBudgetFlag, String> {
    let trimmed = value.trim();
    let mib: u64 = trimmed
        .parse()
        .map_err(|_| format!("--gpu-budget-mb needs an integer >= 0 (got '{value}')"))?;
    if mib == 0 {
        return Ok(GpuBudgetFlag::Uncapped);
    }
    let bytes = mib.checked_mul(MIB).ok_or_else(|| {
        format!("--gpu-budget-mb value is too large to express in bytes (got '{value}')")
    })?;
    Ok(GpuBudgetFlag::Capped(bytes))
}

/// The existence of `--gpu-budget-percent` in the process arguments.
///
/// Same tri-state reason as [`GpuBudgetFlag`]: absent must leave the renderer's
/// own policy (`YACR_GPU_BUDGET_PERCENT`, else 90 % of the probed VRAM) alone,
/// while an explicit value replaces the percentage even when the environment set
/// one. An explicit byte cap (`--gpu-budget-mb` / `YACR_GPU_BUDGET_MB`) still
/// wins over any percentage — that precedence belongs to `cad-render-wgpu`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuBudgetPercentFlag {
    /// `--gpu-budget-percent` was not passed: `YACR_GPU_BUDGET_PERCENT` or the
    /// renderer's 90 % default applies.
    #[default]
    Absent,
    /// `--gpu-budget-percent <10..=100>`: share of the probed device-local VRAM.
    Percent(u8),
}

/// Parse a `--gpu-budget-percent` value into [`GpuBudgetPercentFlag`].
///
/// The accepted range is the renderer's own `10..=100` (below it ordinary CAD
/// scenes would be refused on any card; a caller that wants a tiny fixed cap must
/// pass `--gpu-budget-mb`). A bad value is a usage error, never a silent fall-back
/// to the default.
pub fn parse_gpu_budget_percent_flag(value: &str) -> Result<GpuBudgetPercentFlag, String> {
    let trimmed = value.trim();
    let percent: u8 = trimmed
        .parse()
        .map_err(|_| format!("--gpu-budget-percent needs a whole percentage (got '{value}')"))?;
    if !(MIN_GPU_BUDGET_PERCENT..=MAX_GPU_BUDGET_PERCENT).contains(&percent) {
        return Err(format!(
            "--gpu-budget-percent must be {MIN_GPU_BUDGET_PERCENT}..={MAX_GPU_BUDGET_PERCENT} \
             (got '{value}')"
        ));
    }
    Ok(GpuBudgetPercentFlag::Percent(percent))
}

/// The smallest accepted `--gpu-budget-percent`, mirrored from `cad-render-wgpu`.
pub const MIN_GPU_BUDGET_PERCENT: u8 = 10;

/// The largest accepted `--gpu-budget-percent`, mirrored from `cad-render-wgpu`.
pub const MAX_GPU_BUDGET_PERCENT: u8 = 100;

// ---------------------------------------------------------------------------
// 3D camera/view for `render`
// ---------------------------------------------------------------------------

/// Default vertical field of view for a 3D `render` view, in degrees (`--fov`).
///
/// Matches the app camera's own 3D entry default (`π/4` radians = 45°), so the
/// CLI frames a solid the same way the interactive host does.
pub const DEFAULT_RENDER_FOV_DEGREES: f64 = 45.0;

/// Accepted `--fov` range, inclusive, in degrees.
pub const MIN_RENDER_FOV_DEGREES: f64 = 1.0;
/// Accepted `--fov` range, inclusive, in degrees.
pub const MAX_RENDER_FOV_DEGREES: f64 = 179.0;
/// Accepted `--azimuth` range, inclusive, in degrees.
pub const MIN_RENDER_AZIMUTH_DEGREES: f64 = -360.0;
/// Accepted `--azimuth` range, inclusive, in degrees.
pub const MAX_RENDER_AZIMUTH_DEGREES: f64 = 360.0;
/// Accepted `--elevation` range, inclusive, in degrees.
pub const MIN_RENDER_ELEVATION_DEGREES: f64 = -90.0;
/// Accepted `--elevation` range, inclusive, in degrees.
pub const MAX_RENDER_ELEVATION_DEGREES: f64 = 90.0;

/// A requested 3D view for `render`: a named preset or explicit orbit angles.
///
/// `render` without one of these keeps the unchanged 2D plan path; this type is
/// only produced by [`CliInvocation::requested_view`], which also refuses the
/// inconsistent flag combinations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RequestedView {
    /// One of [`StandardView::ALL`] (`--view <preset>`).
    Preset(StandardView),
    /// `--azimuth <deg>` plus `--elevation <deg>` in the CAD `+Z`-up frame.
    Angles {
        azimuth_degrees: f64,
        elevation_degrees: f64,
    },
}

impl RequestedView {
    /// Unit vector from the target toward the eye in the CAD `+Z`-up world frame.
    ///
    /// A preset uses [`StandardView::eye_offset`]; angles use
    /// `(cos(el)·cos(az), cos(el)·sin(az), sin(el))`.
    pub fn direction(self) -> Point3 {
        match self {
            RequestedView::Preset(view) => view.eye_offset(),
            RequestedView::Angles {
                azimuth_degrees,
                elevation_degrees,
            } => {
                let azimuth = azimuth_degrees.to_radians();
                let elevation = elevation_degrees.to_radians();
                let cos_elevation = elevation.cos();
                Point3 {
                    x: cos_elevation * azimuth.cos(),
                    y: cos_elevation * azimuth.sin(),
                    z: elevation.sin(),
                }
            }
        }
    }

    /// The up hint for this view.
    ///
    /// Presets use [`StandardView::up_hint`]. Angles use world `+Z`, except when
    /// the direction is near-vertical (`|dir.z| > 0.999`), where a `+Z` hint
    /// would be parallel to the line of sight and the camera basis degenerate;
    /// world `+Y` is used there instead (the documented top-down convention).
    pub fn up_hint(self) -> Point3 {
        match self {
            RequestedView::Preset(view) => view.up_hint(),
            RequestedView::Angles { .. } => {
                let z = Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                };
                let y = Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                };
                if self.direction().z.abs() > 0.999 {
                    y
                } else {
                    z
                }
            }
        }
    }

    /// The `StandardView::key` this view names, or `None` for explicit angles.
    ///
    /// This is the `camera.view` value of the `render` result JSON: an unnamed
    /// angle pair reports `null` rather than inventing a preset name.
    pub fn preset_key(self) -> Option<&'static str> {
        match self {
            RequestedView::Preset(view) => Some(view.key()),
            RequestedView::Angles { .. } => None,
        }
    }
}

/// Parse a `--view` preset case-insensitively against [`StandardView::ALL`].
///
/// `None` means "not a known preset"; the caller reports the valid names so a
/// typo can never be silently ignored.
pub fn parse_view_preset(value: &str) -> Option<StandardView> {
    let wanted = value.trim().to_ascii_lowercase();
    StandardView::ALL
        .iter()
        .copied()
        .find(|view| view.key() == wanted)
}

/// Validate an azimuth value in degrees, refusing rather than clamping.
pub fn validate_azimuth_degrees(value: f64) -> Result<f64, String> {
    if !value.is_finite()
        || !(MIN_RENDER_AZIMUTH_DEGREES..=MAX_RENDER_AZIMUTH_DEGREES).contains(&value)
    {
        return Err(format!(
            "--azimuth must be a finite number in \
             {MIN_RENDER_AZIMUTH_DEGREES}..={MAX_RENDER_AZIMUTH_DEGREES} degrees (got '{value}')"
        ));
    }
    Ok(value)
}

/// Validate an elevation value in degrees, refusing rather than clamping.
pub fn validate_elevation_degrees(value: f64) -> Result<f64, String> {
    if !value.is_finite()
        || !(MIN_RENDER_ELEVATION_DEGREES..=MAX_RENDER_ELEVATION_DEGREES).contains(&value)
    {
        return Err(format!(
            "--elevation must be a finite number in \
             {MIN_RENDER_ELEVATION_DEGREES}..={MAX_RENDER_ELEVATION_DEGREES} degrees (got '{value}')"
        ));
    }
    Ok(value)
}

/// Parse a `--fov` value in degrees.
///
/// `1..=179` inclusive; non-finite and out-of-range values are usage errors, not
/// a clamp, so a bad field of view can never silently frame a different picture.
pub fn parse_fov_degrees(value: &str) -> Result<f64, String> {
    match value.trim().parse::<f64>() {
        Ok(fov) => validate_fov_degrees(fov),
        Err(_) => Err(format!(
            "--fov must be a finite number in \
             {MIN_RENDER_FOV_DEGREES}..={MAX_RENDER_FOV_DEGREES} degrees (got '{value}')"
        )),
    }
}

/// Validate a field of view in degrees, refusing rather than clamping.
pub fn validate_fov_degrees(value: f64) -> Result<f64, String> {
    if !value.is_finite() || !(MIN_RENDER_FOV_DEGREES..=MAX_RENDER_FOV_DEGREES).contains(&value) {
        return Err(format!(
            "--fov must be a finite number in \
             {MIN_RENDER_FOV_DEGREES}..={MAX_RENDER_FOV_DEGREES} degrees (got '{value}')"
        ));
    }
    Ok(value)
}

/// Parse a `--azimuth` value in degrees (see [`validate_azimuth_degrees`]).
pub fn parse_azimuth_degrees(value: &str) -> Result<f64, String> {
    match value.trim().parse::<f64>() {
        Ok(azimuth) => validate_azimuth_degrees(azimuth),
        Err(_) => Err(format!(
            "--azimuth must be a finite number in \
             {MIN_RENDER_AZIMUTH_DEGREES}..={MAX_RENDER_AZIMUTH_DEGREES} degrees (got '{value}')"
        )),
    }
}

/// Parse an `--elevation` value in degrees (see [`validate_elevation_degrees`]).
pub fn parse_elevation_degrees(value: &str) -> Result<f64, String> {
    match value.trim().parse::<f64>() {
        Ok(elevation) => validate_elevation_degrees(elevation),
        Err(_) => Err(format!(
            "--elevation must be a finite number in \
             {MIN_RENDER_ELEVATION_DEGREES}..={MAX_RENDER_ELEVATION_DEGREES} degrees (got '{value}')"
        )),
    }
}

// ---------------------------------------------------------------------------
// Invocation
// ---------------------------------------------------------------------------

/// A versioned invocation record; re-running it reproduces the command.
#[derive(Debug, Clone)]
pub struct CliInvocation {
    pub schema_version: u32,
    pub operation: CliOperation,
    pub input: PathBuf,
    /// Measure input in the input file's coordinate space.
    pub points: Vec<Point3>,
    /// Fonts to shape text with, as `(key, path)`.
    pub fonts: Vec<(String, PathBuf)>,
    /// Explicit directory to resolve drawing image references from
    /// (`--images-dir`). `None` resolves images relative to the drawing's own
    /// directory only. Native render/plot only.
    pub images_dir: Option<PathBuf>,
    /// Optional file that receives the JSON result atomically.
    pub out: Option<PathBuf>,
    /// Optional PNG that `render` writes the offscreen frame to.
    pub png: Option<PathBuf>,
    /// Offscreen frame width in pixels for `render`/`plot`.
    pub render_width: u32,
    /// Offscreen frame height in pixels for `render`/`plot`.
    pub render_height: u32,
    /// `--view <preset>` for `render`: a named 3D standard view
    /// ([`StandardView::ALL`]). `None` keeps the 2D plan path unless
    /// `--azimuth`/`--elevation` are also given.
    pub view_preset: Option<StandardView>,
    /// `--azimuth <deg>` for `render`: 3D orbit azimuth in the CAD `+Z`-up frame.
    /// Must be supplied together with [`Self::view_elevation_degrees`] and is
    /// mutually exclusive with [`Self::view_preset`].
    pub view_azimuth_degrees: Option<f64>,
    /// `--elevation <deg>` for `render`: 3D orbit elevation. Must be supplied
    /// together with [`Self::view_azimuth_degrees`].
    pub view_elevation_degrees: Option<f64>,
    /// `--fov <deg>` for `render`: vertical field of view of a 3D view. Only
    /// valid when a 3D view was requested (a lone `--fov` is a usage error).
    pub view_fov_degrees: Option<f64>,
    /// `--legacy-shading`: draw this frame with the legacy single-light PBR path
    /// (`Renderer::set_ibl_enabled(false)`) instead of the session's IBL default.
    pub legacy_shading: bool,
    /// Layout to plot by name (`plot`). `None` selects the first layout.
    pub layout: Option<String>,
    /// Plot resolution in dots per inch (`plot`). When set, the canvas is sized
    /// from the sheet; when absent the `--width`/`--height` canvas is used.
    pub plot_dpi: Option<f64>,
    /// Plot output format (`plot`). `Png` (default) runs the headless GPU
    /// raster path; `Svg`/`Pdf` run the CPU-only vector path and never create a
    /// GPU device.
    pub plot_format: PlotFormat,
    /// Adapter preference for `render`/`plot` (`--gpu auto|high|low`); native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub gpu: GpuPreference,
    /// Hard cap on accumulated render batches (`--max-batches N`, `0` = no
    /// cap). Protects the host against a block-reference explosion expanding
    /// into tens of millions of batches (memory protection, never an OOM kill).
    pub max_batches: Option<u64>,
    /// Hard cap on accumulated render vertices (`--max-vertices N`, `0` = no
    /// cap). Same protection as `max_batches`.
    pub max_vertices: Option<u64>,
    /// `--gpu-budget-mb` (render/plot): an explicit GPU memory cap in MiB.
    ///
    /// Precedence is **flag > `YACR_GPU_BUDGET_MB` > the percentage policy**: an
    /// `Absent` flag keeps the renderer's own env/default policy, and
    /// `Uncapped`/`Capped` replace the cap. An explicit cap always wins over
    /// `--gpu-budget-percent` / `YACR_GPU_BUDGET_PERCENT` (the renderer's
    /// documented `explicit_bytes > percent_of_vram`), and there is no cap at all
    /// when neither is available. The cap is never the card's VRAM itself.
    pub gpu_budget_flag: GpuBudgetFlag,
    /// `--gpu-budget-percent` (render/plot): share of the probed device-local VRAM.
    ///
    /// Precedence is **flag > `YACR_GPU_BUDGET_PERCENT` > the 90 % default**;
    /// `Absent` keeps the renderer's env/default percentage. `--gpu-budget-mb` /
    /// `YACR_GPU_BUDGET_MB` still wins: an explicit byte cap is not a percentage.
    pub gpu_budget_percent_flag: GpuBudgetPercentFlag,
    /// `--gpu-memory-detail` (render/plot): also print the per-category GPU
    /// memory breakdown on stderr.
    pub gpu_memory_detail: bool,
    /// Locale for human-facing stderr messages (never machine output).
    pub locale: Locale,
}

/// Default offscreen frame width for `render`/`plot`.
pub const DEFAULT_RENDER_WIDTH: u32 = 1280;
/// Default offscreen frame height for `render`/`plot`.
pub const DEFAULT_RENDER_HEIGHT: u32 = 720;
/// Default hard cap on accumulated render batches (0 in `CliInvocation` =
/// unlimited only when the user passes `--max-batches 0`).
pub const DEFAULT_MAX_RENDER_BATCHES: u64 = 4_000_000;
/// Default hard cap on accumulated render vertices.
pub const DEFAULT_MAX_RENDER_VERTICES: u64 = 128_000_000;

impl CliInvocation {
    pub fn new(operation: CliOperation, input: impl Into<PathBuf>) -> Self {
        CliInvocation {
            schema_version: CLI_SCHEMA_VERSION,
            operation,
            input: input.into(),
            points: Vec::new(),
            fonts: Vec::new(),
            images_dir: None,
            out: None,
            png: None,
            render_width: DEFAULT_RENDER_WIDTH,
            render_height: DEFAULT_RENDER_HEIGHT,
            view_preset: None,
            view_azimuth_degrees: None,
            view_elevation_degrees: None,
            view_fov_degrees: None,
            legacy_shading: false,
            layout: None,
            plot_dpi: None,
            plot_format: PlotFormat::default(),
            #[cfg(not(target_arch = "wasm32"))]
            gpu: GpuPreference::Auto,
            max_batches: Some(DEFAULT_MAX_RENDER_BATCHES),
            max_vertices: Some(DEFAULT_MAX_RENDER_VERTICES),
            gpu_budget_flag: GpuBudgetFlag::Absent,
            gpu_budget_percent_flag: GpuBudgetPercentFlag::Absent,
            gpu_memory_detail: false,
            locale: Locale::default(),
        }
    }

    /// Resolve the requested `render` 3D view.
    ///
    /// `Ok(None)` is the unchanged 2D plan path. Every inconsistent combination
    /// is a `usage` error (exit code 2) instead of a silent ignore:
    ///
    /// * `--view` together with `--azimuth`/`--elevation`;
    /// * `--azimuth` without `--elevation` (or vice versa);
    /// * `--fov` without any 3D view;
    /// * an out-of-range angle or field of view (also refused when the value was
    ///   set programmatically, not only through the argument parser).
    pub fn requested_view(&self) -> Result<Option<RequestedView>, CliError> {
        let has_angles =
            self.view_azimuth_degrees.is_some() || self.view_elevation_degrees.is_some();
        if self.view_preset.is_some() && has_angles {
            return Err(CliError::usage(
                "--view and --azimuth/--elevation are mutually exclusive; use one way to \
                 choose the 3D view",
            ));
        }
        if has_angles
            && (self.view_azimuth_degrees.is_none() || self.view_elevation_degrees.is_none())
        {
            return Err(CliError::usage(
                "--azimuth and --elevation must be supplied together",
            ));
        }
        if self.view_fov_degrees.is_some() && self.view_preset.is_none() && !has_angles {
            return Err(CliError::usage(
                "--fov only applies to a 3D view; pass --view, or --azimuth with --elevation",
            ));
        }
        if let Some(fov) = self.view_fov_degrees {
            validate_fov_degrees(fov).map_err(CliError::usage)?;
        }
        match (
            self.view_preset,
            self.view_azimuth_degrees,
            self.view_elevation_degrees,
        ) {
            (Some(preset), None, None) => Ok(Some(RequestedView::Preset(preset))),
            (None, Some(azimuth), Some(elevation)) => {
                let azimuth = validate_azimuth_degrees(azimuth).map_err(CliError::usage)?;
                let elevation = validate_elevation_degrees(elevation).map_err(CliError::usage)?;
                Ok(Some(RequestedView::Angles {
                    azimuth_degrees: azimuth,
                    elevation_degrees: elevation,
                }))
            }
            _ => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Structured errors
// ---------------------------------------------------------------------------

/// Stable machine error codes. These never change with locale.
pub mod error_code {
    pub const INVALID_INPUT: &str = "invalid_input";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const NOT_IMPLEMENTED: &str = "not_implemented";
    pub const RESOURCE_MISSING: &str = "resource_missing";
    pub const CORRUPT_DATA: &str = "corrupt_data";
    pub const GPU_FAILURE: &str = "gpu_failure";
    pub const INVARIANT: &str = "invariant";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const CANCELLED: &str = "cancelled";
    pub const STALE_RESULT: &str = "stale_result";
    /// The operation's input contract was violated (bad option combination).
    pub const USAGE: &str = "usage";
    /// A result could not be written to `--out`.
    pub const OUTPUT_WRITE_FAILED: &str = "output_write_failed";
    /// A GPU allocation was refused **for memory**: the renderer's application
    /// budget or the device reporting out-of-memory. Exit code
    /// [`EXIT_GPU_MEMORY_REFUSED`]; never a success and never a blank PNG.
    pub const GPU_MEMORY_REFUSED: &str = "gpu_memory_refused";
}

/// The process exit code of a GPU-memory refusal (budget or device OOM).
///
/// Distinct from every other code this CLI produces, so a script can tell a
/// memory refusal apart from a usage error (2) and from a generic failure (1)
/// without parsing text. Documented in `docs/build.md`.
pub const EXIT_GPU_MEMORY_REFUSED: u8 = 7;

/// A structured CLI failure ready to be serialized to stderr.
///
/// `code` and `context` are stable machine data; `message` is human text whose
/// language may follow `locale`.
#[derive(Debug, Clone)]
pub struct CliError {
    pub code: String,
    pub message: String,
    /// Boxed because `serde_json::Value` is 72 bytes in this workspace
    /// (`preserve_order`/`indexmap` is unified in through the dependency graph),
    /// which would make the error variant exactly the 128-byte
    /// `clippy::result_large_err` ceiling. The JSON document is unchanged; only
    /// the in-memory layout differs.
    pub context: Box<serde_json::Value>,
    /// Locale for the human-facing `message` line only.
    pub locale: Locale,
}

impl CliError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        CliError {
            code: code.into(),
            message: message.into(),
            context: Box::new(serde_json::Value::Null),
            locale: Locale::default(),
        }
    }

    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = Box::new(context);
        self
    }

    /// Set the human-message locale. Machine keys are unaffected.
    pub fn with_locale(mut self, locale: Locale) -> Self {
        self.locale = locale;
        self
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// A usage error (exit code 2) for bad argument combinations.
    pub fn usage(message: impl Into<String>) -> Self {
        CliError::new(error_code::USAGE, message)
    }

    /// The error document written to stderr.
    ///
    /// `operation` and every key are stable; only `error.message` is localized.
    pub fn to_json(&self, operation: CliOperation) -> serde_json::Value {
        serde_json::json!({
            "schema_version": CLI_SCHEMA_VERSION,
            "operation": operation.as_str(),
            "error": {
                "code": self.code,
                "message": self.message,
                "context": self.context.as_ref(),
            }
        })
    }

    /// Process exit status: 2 for usage errors, 7 for a GPU-memory refusal
    /// (application budget or device out-of-memory), 1 for everything else.
    pub fn exit_code(&self) -> u8 {
        if self.code == error_code::USAGE {
            2
        } else if self.code == error_code::GPU_MEMORY_REFUSED {
            EXIT_GPU_MEMORY_REFUSED
        } else {
            1
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

/// Map a domain error to a stable machine code.
pub fn error_code_for(error: &CadError) -> &'static str {
    match error {
        CadError::NotImplemented(_) => error_code::NOT_IMPLEMENTED,
        CadError::InvalidInput(_) => error_code::INVALID_INPUT,
        CadError::Unsupported(_) => error_code::UNSUPPORTED,
        CadError::ResourceMissing(_) => error_code::RESOURCE_MISSING,
        CadError::CorruptData(_) => error_code::CORRUPT_DATA,
        CadError::GpuFailure(_) => error_code::GPU_FAILURE,
        CadError::Invariant(_) => error_code::INVARIANT,
        CadError::PermissionDenied => error_code::PERMISSION_DENIED,
        CadError::Cancelled => error_code::CANCELLED,
        CadError::StaleResult => error_code::STALE_RESULT,
    }
}

/// Convert a domain error into a structured CLI error with redacted context.
///
/// Only the technical error kind is attached as context; file contents, paths
/// and identity values are never copied into the default machine document.
pub fn cli_error_from_domain(error: CadError) -> CliError {
    let code = error_code_for(&error);
    let message = error.to_string();
    CliError::new(code, message)
}

/// So `?` works on a `CadResult` inside an operation that returns
/// `Result<_, CliError>` (the native render/plot paths, which must keep a
/// GPU-memory refusal's own code and exit status instead of flattening it into
/// `gpu_failure`).
impl From<CadError> for CliError {
    fn from(error: CadError) -> Self {
        cli_error_from_domain(error)
    }
}

pub(crate) fn domain<T>(result: CadResult<T>) -> Result<T, CliError> {
    result.map_err(cli_error_from_domain)
}

// ---------------------------------------------------------------------------
// Atomic output
// ---------------------------------------------------------------------------
