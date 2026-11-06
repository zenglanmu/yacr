//! Snapshot-bound rendering receipts issued only by the command funnel.
use super::*;
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
            DrawingEditReceipt::from_create_line_trace(before, after, &traced.commands)
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
            DrawingEditReceipt::from_create_line_trace(before, after, &traced.commands)
        });
        Ok((traced, receipt))
    }
}
