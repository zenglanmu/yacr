//! The Overlays tray's wire row: `cad_overlays` / `kind: "overlays"`.
//!
//! The rows are the renderer-owned overlay channels this build draws, with their
//! current on/off state. There is no user-managed overlay model; the panel's
//! management action dispatches `OVERLAYS`, which answers
//! `overlays.not_implemented`.

use serde::{Deserialize, Serialize};

/// The wire spelling of the overlays request kind.
pub const OVERLAYS_KIND: &str = "overlays";

/// Stable refusal for a carrier with no overlays lane.
pub const OVERLAYS_UNSUPPORTED: &str = "unsupported-runtime: overlays";

/// One overlay channel row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayRowWire {
    /// Stable key (`axes`, `grid`, `selectionHighlight`, `snapHints`,
    /// `selectionBox`, `hoverTint`, `measurement`, `drawPreview`).
    pub key: String,
    /// Whether the channel is currently on/active.
    pub active: bool,
}

/// The overlay channel list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaysResponse {
    /// Rows in panel order.
    pub rows: Vec<OverlayRowWire>,
}

impl OverlaysResponse {
    /// Project the host's read onto the wire row.
    pub fn from_host(list: cad_host::OverlayList) -> Self {
        OverlaysResponse {
            rows: list
                .rows
                .into_iter()
                .map(|row| OverlayRowWire {
                    key: row.key.to_string(),
                    active: row.active,
                })
                .collect(),
        }
    }
}
