//! Replay of a section-plane history step (`tables/sections.json`).
//!
//! Undo passes the swapped patch sides and the whole-slot write is one revision.

use cad_db::{ChangeSet, DrawingDatabase, SectionPatch};
use cad_domain::{CadResult, TransactionId};

/// The section branch of `apply_drawing_patches`; `None` when the step carries
/// no section patch.
pub(crate) fn section_step(
    drawing: &mut DrawingDatabase,
    transaction: TransactionId,
    reason: &str,
    step: &cad_history::DrawingHistoryStep,
    forward: bool,
) -> Option<CadResult<ChangeSet>> {
    let patch = step.section_patches.first()?;
    let patch: SectionPatch = if forward {
        patch.clone()
    } else {
        patch.inverse()
    };
    Some(drawing.apply_section_patches(&[patch], transaction, reason))
}
