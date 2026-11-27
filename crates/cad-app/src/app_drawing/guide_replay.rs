//! Replay of a construction-guide step (`tables/guides.json`).
//!
//! Undo passes the swapped whole-table patch and the write is one revision.

use cad_db::{ChangeSet, DrawingDatabase, GuidePatch};
use cad_domain::{CadResult, TransactionId};

/// The guide branch of `apply_drawing_patches`; `None` when the step has no
/// guide patch.
pub(crate) fn guide_step(
    drawing: &mut DrawingDatabase,
    transaction: TransactionId,
    reason: &str,
    step: &cad_history::DrawingHistoryStep,
    forward: bool,
) -> Option<CadResult<ChangeSet>> {
    let patch = step.guide_patches.first()?;
    let after = if forward {
        patch.after.clone()
    } else {
        patch.before.clone()
    };
    Some(drawing.apply_guides(after, transaction, reason))
}

/// Keep the patch type imported for documentation links and future callers.
#[allow(dead_code)]
fn _patch_type(_: &GuidePatch) {}
