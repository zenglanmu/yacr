//! Typed settings wire surface for the Options dialog.
//!
//! The session exposes the same closed twelve-key surface the host persists; it
//! adds no second validation and no raw-JSON entry point. A UI writes one typed
//! key/value pair, the host validates it against the closed table, persists the
//! matching preference leaf and applies it to the live scene/session.

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
/// read-only display data sourced from the host's `--fonts-dir`; the other twelve
/// fields correspond one-to-one with [`CadSettingKey`], the two backgrounds as
/// canonical `#rrggbb` strings, the crosshair as an integer percentage and the
/// pickbox/aperture as integer pixel counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Model-space canvas background, canonical lower-case `#rrggbb`.
    pub background_model: String,
    /// Paper-space canvas background, canonical lower-case `#rrggbb`.
    pub background_paper: String,
    /// Crosshair arm length as an integer percentage of the viewport (`1..=100`).
    pub crosshair_percent: u32,
    /// Pickbox half-side in logical pixels (`1..=20`, the AutoCAD `PICKBOX`
    /// option).
    pub pickbox_size: u32,
    /// Object-snap aperture in logical pixels (`1..=50`, the AutoCAD `APERTURE`
    /// option).
    pub aperture_size: u32,
    /// Read-only: the configured CAD font directory, or `null` when unset.
    pub fonts_dir: Option<String>,
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
            background_model: value.background_model,
            background_paper: value.background_paper,
            crosshair_percent: value.crosshair_percent,
            pickbox_size: value.pickbox_size,
            aperture_size: value.aperture_size,
            fonts_dir: value.fonts_dir,
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
