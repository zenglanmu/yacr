//! Replay of a display-settings history step (`tables/display.json`).
//!
//! Split out of `helpers.rs` for the code-size gate, like the scene/material
//! replays. Undo passes the swapped patch sides and the whole-record write is one
//! revision.

use cad_db::{ChangeSet, DisplayPatch, DrawingDatabase};
use cad_domain::{CadResult, TransactionId};

/// The display branch of `apply_drawing_patches`; `None` when the step carries
/// no display patch.
pub(crate) fn display_step(
    drawing: &mut DrawingDatabase,
    transaction: TransactionId,
    reason: &str,
    step: &cad_history::DrawingHistoryStep,
    forward: bool,
) -> Option<CadResult<ChangeSet>> {
    if step.display_patches.is_empty() {
        return None;
    }
    let patches: Vec<DisplayPatch> = step
        .display_patches
        .iter()
        .map(|patch| {
            if forward {
                patch.clone()
            } else {
                patch.inverse()
            }
        })
        .collect();
    Some(drawing.apply_display_patches(&patches, transaction, reason))
}
