//! Undo/redo and leave-preparation commands for [`Application`].
use super::*;

impl Application {
    pub(crate) fn undo(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document = self
            .workspace
            .documents
            .get_mut(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let history = self.history.entry(command.document).or_default();
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
        let document = self
            .workspace
            .documents
            .get_mut(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let history = self.history.entry(command.document).or_default();
        let changes = history.redo(&mut document.annotations)?;
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
