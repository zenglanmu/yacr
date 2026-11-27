//! Replay of a material history step (`tables/materials.json`).
//!
//! Split out of `helpers.rs` for the file-size gate, exactly like the view
//! replay. A paint step carries the whole material document on both sides, so
//! redo applies `after` and undo applies `before` through the same validated
//! [`cad_db::DrawingDatabase::apply_material_document`] path as the command.

use cad_db::{ChangeSet, DrawingDatabase};
use cad_domain::{CadResult, TransactionId};

/// The material branch of [`super::helpers::apply_drawing_patches`].
///
/// Returns `None` when the step carries no material patch (the caller continues
/// to its other lanes). Otherwise the whole document travels on both sides, so
/// undo applies `before` and redo applies `after`; the history validator refuses
/// a mixed step, so this branch cannot drop entity/table rows.
pub(crate) fn material_step(
    drawing: &mut DrawingDatabase,
    transaction: TransactionId,
    reason: &str,
    step: &cad_history::DrawingHistoryStep,
    forward: bool,
) -> Option<CadResult<ChangeSet>> {
    if step.material_patches.is_empty() {
        return None;
    }
    Some(replay_material_patches(
        drawing,
        transaction,
        reason,
        step,
        forward,
    ))
}

/// Apply one history step's material patch in the requested direction.
fn replay_material_patches(
    drawing: &mut DrawingDatabase,
    transaction: TransactionId,
    reason: &str,
    step: &cad_history::DrawingHistoryStep,
    forward: bool,
) -> CadResult<ChangeSet> {
    let patch = step
        .material_patches
        .first()
        .expect("the history validator requires at least one material patch");
    let next = if forward {
        patch.after.clone()
    } else {
        patch.before.clone()
    };
    drawing.apply_material_document(next, transaction, reason)
}
