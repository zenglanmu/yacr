//! Typed, closed settings surface for the Options dialog.
//!
//! Only the settings this host actually honours are representable: the four
//! `view.overlays.*` leaves drawn by the render path, the model/paper canvas
//! backgrounds and the crosshair size those frames really use, the
//! `ui.export.plotFormat` default, the two session interaction switches, the
//! renderer backend preference (`wgpu_backend`, read by the next presenter
//! construction), the GPU memory budget percentage the renderer lane reads
//! (`view.gpuBudget.percent`) and the read-only `--fonts-dir` display value.
//! Every write goes through the same validated store/persistence path as
//! [`HostCore::apply_user_preference_json`] — the backend leaf is merged into the
//! same durable preference document, see [`merge_wgpu_backend_into_preference`];
//! no raw JSON is accepted from a UI.
use super::*;

use cad_app::viewer_config::HexColor;

/// A key outside the closed writable set. Stable, locale-neutral error code.
pub const SETTINGS_UNKNOWN_KEY: &str = "settings.unknown_key";
/// A value whose JSON/typed shape does not match the key (for example a string
/// for a boolean setting). Stable, locale-neutral error code.
pub const SETTINGS_INVALID_VALUE: &str = "settings.invalid_value";
/// A plot format other than the two the CPU vector path can produce.
pub const SETTINGS_INVALID_PLOT_FORMAT: &str = "settings.invalid_plot_format";

/// The durable preference key of the renderer backend choice.
///
/// It is a **root-level** leaf of the host preference document (`preference.json`)
/// with exactly this spelling — the same key the UI writes and the hosts' late
/// presenter construction reads. Unlike the other settings it is not camelCased
/// into the `ViewerConfig` schema: it is renderer state, so the host owns it.
pub const WGPU_BACKEND_PREFERENCE_KEY: &str = "wgpu_backend";

/// The closed set of renderer backend tokens, `auto` first (the default).
///
/// These are the wire values of the `wgpu_backend` preference *and* of
/// `CadSettingValue::Text` for [`CadSettingKey::WgpuBackend`]: the Options dialog,
/// the durable preference and the renderer policy parse the same three strings,
/// so a new spelling would have to be added in all three places at once.
pub const WGPU_BACKEND_VALUES: [&str; 3] = ["auto", "vulkan", "dx12"];

/// Normalize a raw durable value to the closed set; absent/unknown → `auto`.
pub fn normalize_wgpu_backend(raw: Option<&str>) -> &'static str {
    let Some(raw) = raw else {
        return WGPU_BACKEND_VALUES[0];
    };
    WGPU_BACKEND_VALUES
        .into_iter()
        .find(|candidate| *candidate == raw)
        .unwrap_or(WGPU_BACKEND_VALUES[0])
}

/// Extract the `wgpu_backend` leaf from a durable preference document.
///
/// The root key is canonical; a nested `ui.wgpu_backend` is tolerated so a
/// projection that nests it cannot silently disable the setting. A non-string or
/// an unknown token yields `None` (the caller falls back to `auto`), never an
/// invented value.
pub(crate) fn wgpu_backend_from_preference(text: &str) -> Option<String> {
    let document: serde_json::Value = serde_json::from_str(text).ok()?;
    let raw = document
        .get(WGPU_BACKEND_PREFERENCE_KEY)
        .or_else(|| {
            document
                .get("ui")
                .and_then(|ui| ui.get(WGPU_BACKEND_PREFERENCE_KEY))
        })
        .and_then(serde_json::Value::as_str)?;
    WGPU_BACKEND_VALUES.contains(&raw).then(|| raw.to_string())
}

/// Remove the renderer backend leaf from a durable preference document.
///
/// The store's allowed-path validation is strict: one leaf outside the host
/// allowances rejects the whole patch. The backend choice is the one leaf the
/// `ViewerConfig` schema cannot express, so it is captured separately
/// ([`wgpu_backend_from_preference`]) and stripped here before the rest of the
/// document is applied — every other leaf is passed through unchanged. Text that
/// is not valid JSON is returned as-is so the store reports its own parse error
/// instead of this helper inventing one.
pub(crate) fn without_wgpu_backend(text: &str) -> String {
    let Ok(mut document) = serde_json::from_str::<serde_json::Value>(text) else {
        return text.to_string();
    };
    if let Some(object) = document.as_object_mut() {
        object.remove(WGPU_BACKEND_PREFERENCE_KEY);
        if let Some(ui) = object.get_mut("ui").and_then(|ui| ui.as_object_mut()) {
            ui.remove(WGPU_BACKEND_PREFERENCE_KEY);
        }
    }
    serde_json::to_string(&document).unwrap_or_else(|_| text.to_string())
}

/// Merge the renderer backend choice into the projected preference document.
///
/// `projected` is the store's allowed-leaf projection (exactly what the host
/// persists). The backend leaf is added at the root — or removed when the choice
/// is `None` — so every preference write keeps the one leaf the schema cannot
/// express. A projection that is not a JSON object is an explicit error, never a
/// silently rewritten file.
pub(crate) fn merge_wgpu_backend_into_preference(
    projected: &str,
    backend: Option<&str>,
) -> CadResult<String> {
    let mut document: serde_json::Value = serde_json::from_str(projected).map_err(|error| {
        CadError::InvalidInput(format!("preference projection is not JSON: {error}"))
    })?;
    let object = document.as_object_mut().ok_or_else(|| {
        CadError::InvalidInput("preference projection is not a JSON object".into())
    })?;
    match backend {
        Some(token) => {
            object.insert(
                WGPU_BACKEND_PREFERENCE_KEY.to_string(),
                serde_json::Value::String(token.to_string()),
            );
        }
        None => {
            object.remove(WGPU_BACKEND_PREFERENCE_KEY);
        }
    }
    serde_json::to_string(&document)
        .map_err(|error| CadError::InvalidInput(format!("preference write is not JSON: {error}")))
}

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
}

impl CadSettingKey {
    /// Every writable key, in a stable order.
    pub const ALL: [Self; 14] = [
        Self::Grid,
        Self::Axes,
        Self::SelectionHighlight,
        Self::SnapHints,
        Self::PlotFormat,
        Self::SnapEnabled,
        Self::HoverPreview,
        Self::BackgroundModel,
        Self::BackgroundPaper,
        Self::CrosshairSize,
        Self::PickboxSize,
        Self::ApertureSize,
        Self::GpuBudgetPercent,
        Self::WgpuBackend,
    ];

    /// The stable camelCase wire key (also the `cad_set_setting` key).
    ///
    /// Every key but [`Self::WgpuBackend`] and [`Self::GpuBudgetPercent`] is the
    /// final segment of a camelCase `ViewerConfig` path; the backend choice is a
    /// root-level renderer preference, so its wire key, its `cad_set_setting` key
    /// and its persisted key are the identical `wgpu_backend`; the GPU budget is
    /// persisted under `view.gpuBudget.percent`, whose leaf is `percent`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Axes => "axes",
            Self::SelectionHighlight => "selectionHighlight",
            Self::SnapHints => "snapHints",
            Self::PlotFormat => "plotFormat",
            Self::SnapEnabled => "snapEnabled",
            Self::HoverPreview => "hoverPreview",
            Self::BackgroundModel => "backgroundModel",
            Self::BackgroundPaper => "backgroundPaper",
            Self::CrosshairSize => "crosshairSize",
            Self::PickboxSize => "pickboxSize",
            Self::ApertureSize => "apertureSize",
            Self::GpuBudgetPercent => "gpuBudgetPercent",
            Self::WgpuBackend => WGPU_BACKEND_PREFERENCE_KEY,
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
    /// `1..=20`, `apertureSize` an integer pixel count in `1..=50` and
    /// `gpuBudgetPercent` an integer percentage of the graphics card's memory in
    /// `10..=100` (all four rejected, never clamped), and every switch is a
    /// boolean.
    pub fn validate(self, value: &CadSettingValue) -> Result<(), &'static str> {
        match self {
            Self::WgpuBackend => match value {
                CadSettingValue::Text(token) if WGPU_BACKEND_VALUES.contains(&token.as_str()) => {
                    Ok(())
                }
                _ => Err(SETTINGS_INVALID_VALUE),
            },
            Self::PlotFormat => match value {
                CadSettingValue::Text(format) if format == "svg" || format == "pdf" => Ok(()),
                CadSettingValue::Text(_) => Err(SETTINGS_INVALID_PLOT_FORMAT),
                CadSettingValue::Bool(_) | CadSettingValue::Number(_) => {
                    Err(SETTINGS_INVALID_VALUE)
                }
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
            | Self::HoverPreview => match value {
                CadSettingValue::Bool(_) => Ok(()),
                CadSettingValue::Text(_) | CadSettingValue::Number(_) => {
                    Err(SETTINGS_INVALID_VALUE)
                }
            },
        }
    }
}

/// A typed setting value: the wire surface carries one boolean, one string or one
/// non-negative integer (a percentage or a pixel count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CadSettingValue {
    Bool(bool),
    Text(String),
    Number(u32),
}

/// The effective settings an Options dialog reads.
///
/// Derived from the effective config (defaults, then `config.json`, then the
/// host-allowed `preference.json`) plus the live session state. `fonts_dir` is
/// read-only display data from [`HostOptions`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSettings {
    pub grid: bool,
    pub axes: bool,
    pub selection_highlight: bool,
    pub snap_hints: bool,
    pub plot_format: String,
    pub snap_enabled: bool,
    pub hover_preview: bool,
    /// Canonical lower-case `#rrggbb` of the model-space background.
    pub background_model: String,
    /// Canonical lower-case `#rrggbb` of the paper-space background.
    pub background_paper: String,
    /// Crosshair arm length as an integer percentage (`1..=100`).
    pub crosshair_percent: u32,
    /// Pickbox half-side in logical pixels (`1..=20`, the AutoCAD `PICKBOX`
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
        let (snap_enabled, hover_preview) = {
            let controller = self.controller.borrow();
            (
                controller.snap_enabled(),
                controller.hover_preview_enabled(),
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
            background_model,
            background_paper,
            crosshair_percent,
            pickbox_size,
            aperture_size,
            gpu_budget_percent,
            wgpu_backend: normalize_wgpu_backend(self.wgpu_backend.borrow().as_deref()).to_string(),
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

    /// Apply one typed setting: persist it under the allowed preference leaves
    /// and make it take effect on the live scene/session immediately.
    ///
    /// The value is validated against the closed key table first, so a wrong
    /// type or an unsupported plot format is refused before any file write.
    pub fn apply_setting(&self, key: CadSettingKey, value: CadSettingValue) -> CadResult<()> {
        key.validate(&value)
            .map_err(|code| CadError::InvalidInput(code.to_string()))?;
        match key {
            CadSettingKey::Grid => self.apply_overlay_preference("grid", &value),
            CadSettingKey::Axes => self.apply_overlay_preference("axes", &value),
            CadSettingKey::SelectionHighlight => {
                self.apply_overlay_preference("selectionHighlight", &value)
            }
            CadSettingKey::SnapHints => self.apply_overlay_preference("snapHints", &value),
            CadSettingKey::PlotFormat => self.apply_plot_format(&value),
            CadSettingKey::SnapEnabled => self.apply_session_switch(key, &value),
            CadSettingKey::HoverPreview => self.apply_session_switch(key, &value),
            CadSettingKey::BackgroundModel => self.apply_background_preference("model", &value),
            CadSettingKey::BackgroundPaper => self.apply_background_preference("paper", &value),
            CadSettingKey::CrosshairSize => self.apply_crosshair_size(&value),
            CadSettingKey::PickboxSize => self.apply_pixel_size("pickbox", &value),
            CadSettingKey::ApertureSize => self.apply_pixel_size("aperture", &value),
            CadSettingKey::GpuBudgetPercent => self.apply_gpu_budget_percent(&value),
            CadSettingKey::WgpuBackend => self.apply_wgpu_backend(&value),
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
            return Err(error);
        }
        Ok(())
    }

    /// Persist one `view.background.<leaf>` colour.
    ///
    /// The value is already validated, so the canonical spelling written here is
    /// the same colour the store parses back; the write path republishes the
    /// render inputs through `sync_render`, so the active space's next frame
    /// clears to the new colour without reopening the document.
    fn apply_background_preference(&self, leaf: &str, value: &CadSettingValue) -> CadResult<()> {
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

    /// Apply the persisted interaction defaults to the live session.
    ///
    /// Called after startup config load and after every authoritative document
    /// replacement (a replacement rebuilds `SessionState`, which would
    /// otherwise reset the two switches to their defaults). `None` means "the
    /// user never chose", so the session default is left untouched.
    pub(crate) fn seed_session_interaction(&self) {
        let (snap_enabled, hover_preview) = {
            let store = self.viewer_config.borrow();
            let interaction = &store.effective().interaction;
            (interaction.snap_enabled, interaction.hover_preview)
        };
        if snap_enabled.is_none() && hover_preview.is_none() {
            return;
        }
        let mut controller = self.controller.borrow_mut();
        if let Some(enabled) = snap_enabled {
            controller.session.set_snap_enabled(enabled);
        }
        if let Some(enabled) = hover_preview {
            controller.session.set_hover_preview(enabled);
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
