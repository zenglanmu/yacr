//! Per-document display settings (`tables/display.json`, `.ymodel` minor 22).
//!
//! One record: SketchUp's **Shadows** sun (for shading) and **Fog** band. The
//! table lives on [`DrawingDatabase`], is written only when it differs from the
//! default, and is decoded by `from_snapshot`. A write replaces the whole record
//! as **one** revision and one `tables/display.json` dirty mark; undo/redo
//! replays the before/after sides through the same apply path.

use cad_domain::display::DisplaySettings;
use cad_domain::{CadError, CadResult, Revision, TransactionId};

use super::DrawingDatabase;
use crate::scope::TableKind;
use crate::{ChangeScope, ChangeSet};

/// A reversible display-settings write.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayPatch {
    /// The record before the write.
    pub before: DisplaySettings,
    /// The record after the write.
    pub after: DisplaySettings,
}

impl DisplayPatch {
    /// The inverse patch (undo/redo replay).
    pub fn inverse(&self) -> DisplayPatch {
        DisplayPatch {
            before: self.after,
            after: self.before,
        }
    }
}

impl DrawingDatabase {
    /// The document's display settings (the default when it has none).
    pub fn display_settings(&self) -> DisplaySettings {
        self.display
    }

    /// Validate and apply a whole display record as one revision.
    pub fn apply_display_settings(
        &mut self,
        next: DisplaySettings,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        next.validate()?;
        if next == self.display {
            return Err(CadError::InvalidInput(
                "display.no_change: the write would not change the display settings".into(),
            ));
        }
        let before = self.revision;
        self.display = next;
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.table(TableKind::Display);
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope,
        })
    }
}

impl DrawingDatabase {
    /// Apply one whole-record display patch (the undo/redo replay path).
    ///
    /// Exactly one patch is accepted, its `before` side must equal the current
    /// record (otherwise the history step is stale) and the write is one
    /// revision, like [`Self::apply_display_settings`].
    pub fn apply_display_patches(
        &mut self,
        patches: &[DisplayPatch],
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        let [patch] = patches else {
            return Err(CadError::InvalidInput(
                "display.invalid_patch: a display step carries exactly one whole-record patch"
                    .into(),
            ));
        };
        if patch.before != self.display {
            return Err(CadError::InvalidInput(
                "display.stale_patch: the patch's before record is not the current one".into(),
            ));
        }
        self.apply_display_settings(patch.after, transaction, reason)
    }
}

impl crate::DrawingDatabaseBuilder {
    /// Install the decoded display record (a load path).
    pub fn set_display_settings(&mut self, settings: DisplaySettings) -> CadResult<()> {
        settings.validate()?;
        self.database.display = settings;
        Ok(())
    }
}
