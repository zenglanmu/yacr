//! Persisted construction guides (`tables/guides.json`, `.ymodel` minor 26).
//!
//! The table is whole-table state (bounded to 64 rows), like the display record:
//! one `GUIDE`/measurement-guide write replaces it as one revision and one undo
//! step. Undo/redo swaps the patch sides and replays through the same method.

use crate::scope::TableKind;
use crate::{ChangeScope, ChangeSet};
use cad_domain::guide::GuideTable;
use cad_domain::{CadError, CadResult, Revision, TransactionId};

use super::DrawingDatabase;

/// A reversible guide-table write (the whole table before and after).
#[derive(Debug, Clone, PartialEq)]
pub struct GuidePatch {
    pub before: GuideTable,
    pub after: GuideTable,
}

impl GuidePatch {
    /// The inverse patch (undo/redo replay).
    pub fn inverse(&self) -> GuidePatch {
        GuidePatch {
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

impl DrawingDatabase {
    /// The document's guides.
    pub fn guides(&self) -> &GuideTable {
        &self.guides
    }

    /// Apply one whole-table guide write as one revision.
    pub fn apply_guides(
        &mut self,
        next: GuideTable,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        next.validate()
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        if next == self.guides {
            return Err(CadError::InvalidInput(
                "guide.no_change: the guide table already holds this state".into(),
            ));
        }
        let before = self.revision;
        self.guides = next;
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.table(TableKind::Guides);
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

impl crate::DrawingDatabaseBuilder {
    /// Install decoded guides (a load path).
    pub fn set_guides(&mut self, table: GuideTable) -> CadResult<()> {
        table
            .validate()
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        self.database.guides = table;
        Ok(())
    }
}
