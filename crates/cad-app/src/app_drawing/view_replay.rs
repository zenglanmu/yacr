//! Replay of a named-view history step (`tables/views.json`).
//!
//! Split out of `helpers.rs` for the file-size gate. The branch is the same
//! before/after swap every table patch uses: redo keeps the stored sides, undo
//! swaps them, and [`cad_db::DrawingDatabase::apply_named_view_patches`] applies
//! the selected side as one revision.

use cad_db::{ChangeSet, DrawingDatabase, ViewPatch};
use cad_domain::{CadResult, TransactionId};

/// Apply one history step's view patches in the requested direction.
pub(crate) fn replay_view_patches(
    drawing: &mut DrawingDatabase,
    transaction: TransactionId,
    reason: &str,
    step: &cad_history::DrawingHistoryStep,
    forward: bool,
) -> CadResult<ChangeSet> {
    let patches: Vec<ViewPatch> = step
        .view_patches
        .iter()
        .map(|patch| ViewPatch {
            name: patch.name.clone(),
            before: if forward {
                patch.before.clone()
            } else {
                patch.after.clone()
            },
            after: if forward {
                patch.after.clone()
            } else {
                patch.before.clone()
            },
        })
        .collect();
    drawing.apply_named_view_patches(&patches, transaction, reason)
}
