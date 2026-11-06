//! Snapshot-bound rendering receipts issued only by the command funnel.
use super::*;
use cad_db::ChangeSet;

use crate::render_scene::DrawingEditReceipt;

impl HostController {
    /// Execute and issue an opaque receipt bound to the actual command snapshots.
    pub fn execute_traced_with_edit(
        &mut self,
        command: Command,
    ) -> CadResult<(crate::TracedCommandOutcome, Option<DrawingEditReceipt>)> {
        let before = self.drawing();
        let traced = self.execute_traced(command)?;
        let receipt = before.zip(self.drawing()).and_then(|(before, after)| {
            let changes = committed_changes(&traced, before.as_ref())?;
            DrawingEditReceipt::from_commit(before, after, changes)
        });
        Ok((traced, receipt))
    }

    /// Confirm a capture without letting a caller manufacture its edit witness.
    pub fn confirm_draw_traced_with_edit(
        &mut self,
    ) -> CadResult<(crate::TracedCommandOutcome, Option<DrawingEditReceipt>)> {
        let before = self.drawing();
        let traced = self.confirm_draw_traced()?;
        let receipt = before.zip(self.drawing()).and_then(|(before, after)| {
            let changes = committed_changes(&traced, before.as_ref())?;
            DrawingEditReceipt::from_commit(before, after, changes)
        });
        Ok((traced, receipt))
    }
}

/// The changeset the commit published, read from the trace list.
///
/// `TracedCommandOutcome` exposes no `changes` field of its own, so the changeset
/// comes from the per-trace `result`: the last **non-empty** one is the applied
/// commit. It must still `follows` the pre-command revision, otherwise no receipt
/// is issued — declining to the full rebuild is always safe.
fn committed_changes<'a>(
    traced: &'a crate::TracedCommandOutcome,
    before: &DrawingDatabase,
) -> Option<&'a ChangeSet> {
    let changes = traced
        .commands
        .iter()
        .rev()
        .filter_map(|trace| trace.result.as_ref().ok()?.changes.as_deref())
        .find(|changes| !changes.is_empty())?;
    changes
        .follows(before.id(), before.revision())
        .then_some(changes)
}
