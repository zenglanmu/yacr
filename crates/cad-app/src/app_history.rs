//! Undo/redo commands for [`Application`].
//!
//! Every retained history record is a drawing-entity edit, so undo and redo
//! apply the patch to the drawing database through its validated write path.

use super::*;

impl Application {
    pub(crate) fn undo(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        self.undo_drawing(command.document)
    }

    pub(crate) fn redo(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        self.redo_drawing(command.document)
    }

    /// Undo the top drawing record against the drawing database.
    ///
    /// The record is only committed to the redo stack after the database
    /// accepted the `before` entities, so a failed apply leaves the step pending
    /// and the database unchanged.
    fn undo_drawing(&mut self, document_id: DocumentId) -> CadResult<CommandOutcome> {
        let record = self
            .history
            .entry(document_id)
            .or_default()
            .begin_drawing_undo()?;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let drawing = Arc::make_mut(&mut document.drawing);
        let changes = match crate::app_drawing::apply_drawing_patches(
            drawing,
            &format!("undo: {}", record.label),
            record.transaction,
            &record.patches,
            false,
        ) {
            Ok(changes) => changes,
            Err(error) => {
                // The step was popped before the apply; put it back so a
                // rejected patch can never silently drop an undo step.
                self.history
                    .entry(document_id)
                    .or_default()
                    .cancel_pending_drawing();
                return Err(error);
            }
        };
        self.history
            .entry(document_id)
            .or_default()
            .finish_drawing()?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(changes),
            diagnostics: Vec::new(),
            measurement: None,
        })
    }

    /// Redo the next drawing record against the drawing database.
    fn redo_drawing(&mut self, document_id: DocumentId) -> CadResult<CommandOutcome> {
        let record = self
            .history
            .entry(document_id)
            .or_default()
            .begin_drawing_redo()?;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let drawing = Arc::make_mut(&mut document.drawing);
        let changes = match crate::app_drawing::apply_drawing_patches(
            drawing,
            &format!("redo: {}", record.label),
            record.transaction,
            &record.patches,
            true,
        ) {
            Ok(changes) => changes,
            Err(error) => {
                self.history
                    .entry(document_id)
                    .or_default()
                    .cancel_pending_drawing();
                return Err(error);
            }
        };
        self.history
            .entry(document_id)
            .or_default()
            .finish_drawing()?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(changes),
            diagnostics: Vec::new(),
            measurement: None,
        })
    }

    pub fn can_undo(&self, document: &DocumentId) -> bool {
        self.history
            .get(document)
            .map(|h| h.can_undo())
            .unwrap_or(false)
    }

    /// Whether the document has a redo record. Derived from the redo stack, not
    /// from `can_undo` (audit U11): after undoing to empty, redo stays available.
    pub fn can_redo(&self, document: &DocumentId) -> bool {
        self.history
            .get(document)
            .map(|h| h.can_redo())
            .unwrap_or(false)
    }

    /// Pure snapshot of undo/redo availability. UI refreshes call this instead
    /// of dispatching a command, so no duplicate command is emitted.
    pub fn history_availability(&self, document: &DocumentId) -> HistoryAvailability {
        HistoryAvailability {
            can_undo: self.can_undo(document),
            can_redo: self.can_redo(document),
        }
    }
}
