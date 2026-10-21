//! Typed, closed settings surface for the Options dialog.
//!
//! Only the settings this host actually honours are representable: the four
//! `view.overlays.*` leaves drawn by the render path, the model/paper canvas
//! backgrounds and the crosshair size those frames really use, the
//! `ui.export.plotFormat` default, the two session interaction switches, and the
//! read-only `--fonts-dir` display value. Every write goes through the same
//! validated store/persistence path as [`HostCore::apply_user_preference_json`];
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
}

impl CadSettingKey {
    /// Every writable key, in a stable order.
    pub const ALL: [Self; 10] = [
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
    ];

    /// The stable camelCase wire key (also the config leaf's final segment).
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
    /// two representable vector formats, the two backgrounds accept only a
    /// canonical `#rrggbb` colour, `crosshairSize` accepts only an integer
    /// percentage in `1..=100` (rejected, never clamped) and every switch is a
    /// boolean.
    pub fn validate(self, value: &CadSettingValue) -> Result<(), &'static str> {
        match self {
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
/// non-negative integer (the crosshair percentage).
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
            fonts_dir: self
                .options
                .fonts_dir
                .as_ref()
                .map(|path| path.display().to_string()),
        }
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
        }
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
}
