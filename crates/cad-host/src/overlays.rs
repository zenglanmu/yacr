//! The Overlays tray's read: the renderer-owned overlay channels this build
//! actually draws.
//!
//! SketchUp's Overlays panel lets a user manage overlay definitions. This build
//! has **no user-managed overlay model**: its overlays are renderer/scene-owned
//! channels (axes, grid, selection tint, snap hints, the gesture previews and the
//! measurement readout). The honest panel lists those channels and whether each
//! is currently active; it does not pretend a toggle would persist anywhere.
//!
//! A management action dispatches `OVERLAYS`, which answers the stable
//! `overlays.not_implemented` refusal.

use crate::HostCore;

/// The stable key of every row, in panel order.
pub const OVERLAY_ROW_KEYS: &[&str] = &[
    "axes",
    "grid",
    "selectionHighlight",
    "snapHints",
    "selectionBox",
    "hoverTint",
    "measurement",
    "drawPreview",
];

/// One overlay channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayRow {
    /// Stable key (the panel localises it; never a translated label).
    pub key: &'static str,
    /// Whether the channel is currently on/active.
    pub active: bool,
}

/// The overlay channel list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayList {
    /// Rows in [`OVERLAY_ROW_KEYS`] order.
    pub rows: Vec<OverlayRow>,
}

impl HostCore {
    /// The renderer-owned overlay channels and their current state.
    pub fn overlays(&self) -> OverlayList {
        let overlays = self.render.overlays();
        OverlayList {
            rows: vec![
                OverlayRow {
                    key: "axes",
                    active: overlays.visibility.axes,
                },
                OverlayRow {
                    key: "grid",
                    active: overlays.visibility.grid,
                },
                OverlayRow {
                    key: "selectionHighlight",
                    active: overlays.visibility.selection_highlight,
                },
                OverlayRow {
                    key: "snapHints",
                    active: overlays.visibility.snap_hints,
                },
                OverlayRow {
                    key: "selectionBox",
                    active: overlays.selection_box.is_some(),
                },
                OverlayRow {
                    key: "hoverTint",
                    active: !overlays.hovered.is_empty(),
                },
                OverlayRow {
                    key: "measurement",
                    active: overlays.measurement.is_some(),
                },
                OverlayRow {
                    key: "drawPreview",
                    active: overlays.draw_preview.is_some(),
                },
            ],
        }
    }
}
