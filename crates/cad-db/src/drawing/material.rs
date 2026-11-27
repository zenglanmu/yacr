//! Document materials (`tables/materials.json`, `.ymodel` minor 19).
//!
//! The table lives on [`DrawingDatabase`] as a [`MaterialDocument`]: the named
//! flat materials plus one assignment per painted entity. This module owns the
//! two write paths:
//!
//! * [`DrawingDatabase::apply_material_document`] validates a whole new document
//!   (the paint command's output) and applies it as **one** revision and one
//!   `tables/materials.json` dirty mark;
//! * [`DrawingDatabase::prune_material_assignments`] drops assignments whose
//!   entity no longer exists — the delete path calls it from `remove_entity`, so
//!   a deleted row can never leave a dangling assignment for the codec to refuse.
//!
//! Undo/redo replays one [`MaterialPatch`] (a before/after document) through the
//! same apply path; materials are a small table, so carrying both sides is
//! cheaper than a per-row patch lane.

use cad_domain::material::{MaterialDocument, MaterialError};
use cad_domain::{CadError, CadResult, Revision, TransactionId};

use super::DrawingDatabase;
use crate::scope::TableKind;
use crate::{ChangeScope, ChangeSet};

/// A reversible material-document write: the whole table before and after.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialPatch {
    /// The document before the write.
    pub before: MaterialDocument,
    /// The document after the write.
    pub after: MaterialDocument,
}

impl MaterialPatch {
    /// The inverse patch (undo/redo replay).
    pub fn inverse(&self) -> MaterialPatch {
        MaterialPatch {
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

impl DrawingDatabase {
    /// The document's materials (empty when it has none).
    pub fn materials(&self) -> &MaterialDocument {
        &self.materials
    }

    /// Validate and apply a whole material document as one revision.
    ///
    /// A document equal to the current one is refused as a no-op, so a paint that
    /// changes nothing records no undo step. The returned [`ChangeSet`] has no
    /// object changes; its scope marks `tables/materials.json` dirty.
    pub fn apply_material_document(
        &mut self,
        next: MaterialDocument,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        next.validate()
            .map_err(|error: MaterialError| CadError::InvalidInput(error.to_string()))?;
        if next == self.materials {
            return Err(CadError::InvalidInput(format!(
                "{}: the paint would not change the material table",
                cad_domain::material::markers::MATERIAL_NO_CHANGE
            )));
        }
        let before = self.revision;
        self.materials = next;
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.table(TableKind::Materials);
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
    /// Install the decoded material document (a load path).
    ///
    /// The snapshot codec already refused duplicates, an over-cap table and a
    /// dangling assignment; this validates once more so a hand-built table cannot
    /// bypass the name/transparency rules.
    pub fn set_materials(&mut self, document: MaterialDocument) -> CadResult<()> {
        document
            .validate()
            .map_err(|error: MaterialError| CadError::InvalidInput(error.to_string()))?;
        self.database.materials = document;
        Ok(())
    }
}
