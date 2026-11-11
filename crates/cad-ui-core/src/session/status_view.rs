//! The status `view` orientation and `navigation` switch blocks.
//!
//! Split out of `status.rs` for the code-size gate; re-exported from there so
//! every existing path is unchanged.

use serde::Serialize;

/// The current view's orientation, projected from the host's **authoritative
/// camera** (host truth, never a UI approximation).
///
/// `mode` is the host's real observation mode (`"2d"` for a plan view, `"3d"`
/// for an orbit view) and `projection` the camera's real projection
/// (`"orthographic"` / `"perspective"`). `eye`/`target`/`up` are the camera's
/// world-space basis, so a renderer or a ViewCube can orient itself without
/// re-deriving the view. The block is `None` when there is no authoritative
/// camera or when any component is non-finite — never a fabricated basis.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadViewOrientation {
    /// `"2d"` (plan) or `"3d"` (orbit).
    pub mode: String,
    /// `"orthographic"` or `"perspective"`.
    pub projection: String,
    /// Camera eye point in world coordinates.
    pub eye: [f64; 3],
    /// Camera target point in world coordinates.
    pub target: [f64; 3],
    /// Camera up hint in world coordinates.
    pub up: [f64; 3],
}

/// The host's real navigation/ViewCube switches (host truth for the chrome).
///
/// These are the same values the Options surface persists, projected without a
/// second derivation: `navbar_visible` is the navigation-bar switch,
/// `view_cube_display` the two-bit ViewCube class mask (bit 1 = 3D, bit 2 = 2D)
/// and the two `view_cube_in_*` flags the per-class visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadNavigation {
    pub navbar_visible: bool,
    pub view_cube_display: u32,
    pub view_cube_in_2d: bool,
    pub view_cube_in_3d: bool,
}

impl Default for CadNavigation {
    /// The documented host defaults (no user choice yet): navigation bar on,
    /// both ViewCube view classes on.
    fn default() -> Self {
        Self {
            navbar_visible: true,
            view_cube_display: 3,
            view_cube_in_2d: true,
            view_cube_in_3d: true,
        }
    }
}
