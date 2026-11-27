//! The Scenes tray's wire row: `cad_scenes` / `kind: "scenes"`.
//!
//! The panel needs only what it lists and applies: each scene's name, whether it
//! captured the 3D mode, its space label and its style token. The session
//! projects the open document's `tables/scenes.json`; a carrier without a
//! seam answers [`SCENES_UNSUPPORTED`] instead of an empty list that would read
//! as "this document has no scenes".
//!
//! Applying a scene is not this read: the panel dispatches
//! `SCENE RESTORE <name>` through the ordinary command path.

use serde::{Deserialize, Serialize};

/// The wire spelling of the scenes request kind.
pub const SCENES_KIND: &str = "scenes";

/// Stable refusal for a carrier with no scenes lane.
pub const SCENES_UNSUPPORTED: &str = "unsupported-runtime: scenes";

/// The maximum rows one answer carries (the host table is capped lower).
pub const SCENES_DEFAULT_LIMIT: usize = 64;

/// One scene row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneRow {
    /// Stored name.
    pub name: String,
    /// Whether the scene captured the 3D observation mode.
    pub is_3d: bool,
    /// Display name of the active space (`Model` or the layout's name).
    pub space: String,
    /// The visual-style token (`WIREFRAME`/`SHADED`).
    pub style: String,
}

/// The scene list and its page facts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenesResponse {
    /// Rows in save order (oldest first).
    pub scenes: Vec<SceneRow>,
    /// Total scenes in the document table.
    pub total: usize,
    /// Whether `scenes` was cut at the wire budget.
    pub truncated: bool,
}

impl ScenesResponse {
    /// Project the host's read onto the wire row.
    pub fn from_host(list: cad_host::SceneList) -> Self {
        ScenesResponse {
            scenes: list
                .rows
                .into_iter()
                .map(|row| SceneRow {
                    name: row.name,
                    is_3d: row.is_3d,
                    space: row.space,
                    style: row.style,
                })
                .collect(),
            total: list.total,
            truncated: list.truncated,
        }
    }
}
