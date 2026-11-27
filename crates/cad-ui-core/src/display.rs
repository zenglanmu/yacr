//! The Shadows/Fog panels' wire row: `cad_display` / `kind: "display"`.
//!
//! Both SketchUp display panels read the one per-document record
//! (`tables/display.json`, `.ymodel` minor 22): the Shadows sun used for shading
//! and the Fog distance band. The session projects the open document's current
//! values; a carrier without a seam answers [`DISPLAY_UNSUPPORTED`] instead of a
//! default record that would read as "the document is known to have shadows off".
//!
//! Writing is not this read: a panel dispatches `SHADOWS ...` / `FOG ...` through
//! the ordinary command path, and the answer below reflects what the next read
//! sees.

use serde::{Deserialize, Serialize};

/// The wire spelling of the display-settings request kind.
pub const DISPLAY_KIND: &str = "display";

/// Stable refusal for a carrier with no display-settings lane.
pub const DISPLAY_UNSUPPORTED: &str = "unsupported-runtime: display";

/// The Shadows panel's values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowsRow {
    /// Whether the sun replaces the renderer's camera-relative key light.
    pub enabled: bool,
    /// Degrees clockwise from world `+Y`, in `[0, 360)`.
    pub azimuth: f64,
    /// Degrees above the horizon, in `[0, 90]`.
    pub elevation: f64,
}

/// The Fog panel's values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FogRow {
    /// Whether fog is rendered.
    pub enabled: bool,
    /// Distance where the fog begins, in model units.
    pub start: f64,
    /// Distance where geometry is fully fogged, in model units.
    pub end: f64,
    /// sRGB fog colour.
    pub color: [u8; 3],
}

/// One document's display settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayResponse {
    /// The Shadows panel.
    pub shadows: ShadowsRow,
    /// The Fog panel.
    pub fog: FogRow,
}

impl DisplayResponse {
    /// Project the document record onto the wire row.
    pub fn from_host(settings: cad_domain::display::DisplaySettings) -> Self {
        DisplayResponse {
            shadows: ShadowsRow {
                enabled: settings.shadows.enabled,
                azimuth: settings.shadows.azimuth,
                elevation: settings.shadows.elevation,
            },
            fog: FogRow {
                enabled: settings.fog.enabled,
                start: settings.fog.start,
                end: settings.fog.end,
                color: settings.fog.color,
            },
        }
    }
}
