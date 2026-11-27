//! Replay of a scene history step (`tables/scenes.json`).
//!
//! Split out of `helpers.rs` for the code-size gate, like the view/material
//! replays. Undo passes the swapped patch sides and the table write is one
//! revision.

use cad_db::{ChangeSet, DrawingDatabase, ScenePatch};
use cad_domain::{CadResult, TransactionId};

/// The scene branch of `apply_drawing_patches`; `None` when the step has no
/// scene patch.
pub(crate) fn scene_step(
    drawing: &mut DrawingDatabase,
    transaction: TransactionId,
    reason: &str,
    step: &cad_history::DrawingHistoryStep,
    forward: bool,
) -> Option<CadResult<ChangeSet>> {
    if step.scene_patches.is_empty() {
        return None;
    }
    let patches: Vec<ScenePatch> = step
        .scene_patches
        .iter()
        .map(|patch| {
            if forward {
                patch.clone()
            } else {
                patch.inverse()
            }
        })
        .collect();
    Some(drawing.apply_scene_patches(&patches, transaction, reason))
}
