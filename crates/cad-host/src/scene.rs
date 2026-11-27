//! The Scenes tray's read: the document's persisted pages.
//!
//! Host content only: the projection reads `tables/scenes.json` off the current
//! database and never mutates it. A document without scenes answers an empty
//! list (the panel's empty state), never an error; a carrier with no seam still
//! answers the kind's own refusal at the wire layer.

use cad_domain::scene::SceneTable;
use cad_domain::SpaceId;

use super::HostCore;

/// The maximum rows one read returns (the table itself is capped at 32; this is
/// the wire budget, not a second scene cap).
pub const SCENE_LIST_MAX_ROWS: usize = 64;

/// One scene row the panel can list and apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneListRow {
    /// Stored name.
    pub name: String,
    /// Whether the scene captured the 3D observation mode.
    pub is_3d: bool,
    /// Display name of the active space (`Model` or the layout's name).
    pub space: String,
    /// The visual-style token (`WIREFRAME`/`SHADED`).
    pub style: String,
}

/// The scene list plus its page facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneList {
    /// Rows in save order (oldest first).
    pub rows: Vec<SceneListRow>,
    /// Total scenes in the table (equal to `rows.len()` unless truncated).
    pub total: usize,
    /// Whether `rows` was cut at [`SCENE_LIST_MAX_ROWS`].
    pub truncated: bool,
}

impl HostCore {
    /// The open document's scenes, in save order.
    pub fn scenes(&self) -> SceneList {
        let controller = self.controller.borrow();
        let Some(database) = controller.drawing() else {
            return SceneList::default();
        };
        let table: &SceneTable = database.scenes();
        let total = table.len();
        let rows = table
            .scenes()
            .take(SCENE_LIST_MAX_ROWS)
            .map(|scene| SceneListRow {
                name: scene.name().to_string(),
                is_3d: scene.view.is_3d,
                space: space_name(database.as_ref(), &scene.view.space),
                style: scene.style.token().to_string(),
            })
            .collect::<Vec<_>>();
        SceneList {
            truncated: total > rows.len(),
            total,
            rows,
        }
    }
}

/// A stable display name for a scene's space.
fn space_name(database: &cad_db::DrawingDatabase, space: &SpaceId) -> String {
    match space {
        SpaceId::Model => "Model".to_string(),
        SpaceId::Paper(layout) => database
            .layout(*layout)
            .map(|layout| layout.name.clone())
            .unwrap_or_else(|| format!("#{}", layout.0)),
        SpaceId::Block(block) => format!("Block {}", block.0),
    }
}
