//! Undo/redo and leave-preparation commands for [`Application`].
use super::*;

impl Application {
    pub(crate) fn undo(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        // A shared history interleaves annotation and drawing steps. The
        // top-of-stack marker decides which database the record must be applied
        // to, so a drawing patch can never reach the annotation store.
        let document_id = command.document;
        let is_drawing = self
            .history
            .get(&document_id)
            .map(|h| h.pop_drawing_for_undo())
            .unwrap_or(false);
        if is_drawing {
            return self.undo_drawing(document_id);
        }
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let history = self.history.entry(document_id).or_default();
        let changes = history.undo(&mut document.annotations)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(changes),
            diagnostics: Vec::new(),
            measurement: None,
            annotation: None,
        })
    }

    pub(crate) fn redo(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let is_drawing = self
            .history
            .get(&document_id)
            .map(|h| h.next_redo_is_drawing())
            .unwrap_or(false);
        if is_drawing {
            return self.redo_drawing(document_id);
        }
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let history = self.history.entry(document_id).or_default();
        let changes = history.redo(&mut document.annotations)?;
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(changes),
            diagnostics: Vec::new(),
            measurement: None,
            annotation: None,
        })
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
            annotation: None,
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
            annotation: None,
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

    /// Guard a document switch/exit; an unsaved annotation is never discarded
    /// without an explicit user decision (spec §16.3).
    ///
    /// This is the decision-only half of the leave flow: the host owns the real
    /// save/recovery writes, so `Save`/`PreserveRecovery` are treated as
    /// "recorded intent" here and the host confirms the durable write through
    /// [`Application::resolve_leave`] with the actual write results.
    pub fn prepare_leave(
        &mut self,
        document: DocumentId,
        decision: UnsavedDecision,
    ) -> CadResult<()> {
        match self.resolve_leave(document, decision, true, true) {
            UnsavedOutcome::Proceed => Ok(()),
            UnsavedOutcome::Cancelled => Err(CadError::Cancelled),
            UnsavedOutcome::SaveFailed => Err(CadError::Unsupported(
                "annotation save must be confirmed by the platform host".into(),
            )),
            UnsavedOutcome::RecoveryFailed => Err(CadError::Invariant(
                "recovery snapshot could not be persisted".into(),
            )),
        }
    }

    /// Apply the unsaved-work decision model and return its structured outcome.
    ///
    /// `save_succeeded` / `recovery_succeeded` are the host's results for the
    /// real atomic export and recovery write. A failed save never proceeds and
    /// never clears dirty (audit B07); a `Cancel` keeps the current document and
    /// any recovery data (audit U09). This method performs no writes itself.
    pub fn resolve_leave(
        &self,
        document: DocumentId,
        decision: UnsavedDecision,
        save_succeeded: bool,
        recovery_succeeded: bool,
    ) -> UnsavedOutcome {
        let dirty = self
            .workspace
            .documents
            .get(&document)
            .map(|d| d.annotations.is_dirty())
            .unwrap_or(false);
        UnsavedFlow::new(dirty).apply(decision, save_succeeded, recovery_succeeded)
    }
}
