//! The per-document section plane (`tables/sections.json`, `.ymodel` minor 32).
//!
//! One optional plane: origin, unit normal and enabled flag. A write replaces the
//! whole slot as **one** revision and one `tables/sections.json` dirty mark;
//! undo/redo replays the before/after sides through the same apply path. The
//! scene builder reads [`DrawingDatabase::section_plane`] to clip model geometry.

use cad_domain::section::{markers, SectionPlane};
use cad_domain::{CadError, CadResult, Revision, TransactionId};

use super::DrawingDatabase;
use crate::scope::TableKind;
use crate::{ChangeScope, ChangeSet};

/// A reversible section-plane write.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionPatch {
    /// The slot before the write.
    pub before: Option<SectionPlane>,
    /// The slot after the write.
    pub after: Option<SectionPlane>,
}

impl SectionPatch {
    /// The inverse patch (undo/redo replay).
    pub fn inverse(&self) -> SectionPatch {
        SectionPatch {
            before: self.after,
            after: self.before,
        }
    }
}

impl DrawingDatabase {
    /// The document's section plane, or `None` when it has no row.
    pub fn section_plane(&self) -> Option<SectionPlane> {
        self.section
    }

    /// Validate and apply a whole section slot as one revision.
    pub fn apply_section_plane(
        &mut self,
        next: Option<SectionPlane>,
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        if let Some(plane) = next {
            plane.validate()?;
        }
        if next == self.section {
            return Err(CadError::InvalidInput(format!(
                "{}: the write would not change the section plane",
                markers::SECTION_NO_CHANGE
            )));
        }
        let before = self.revision;
        self.section = next;
        let after = Revision(before.0 + 1);
        self.revision = after;
        let mut scope = ChangeScope::default();
        scope.table(TableKind::Sections);
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

    /// Apply one whole-slot section patch (the undo/redo replay path).
    pub fn apply_section_patches(
        &mut self,
        patches: &[SectionPatch],
        transaction: TransactionId,
        reason: &str,
    ) -> CadResult<ChangeSet> {
        let [patch] = patches else {
            return Err(CadError::InvalidInput(
                "section.invalid_patch: a section step carries exactly one whole-slot patch".into(),
            ));
        };
        if patch.before != self.section {
            return Err(CadError::InvalidInput(
                "section.stale_patch: the patch's before slot is not the current one".into(),
            ));
        }
        self.apply_section_plane(patch.after, transaction, reason)
    }
}

impl crate::DrawingDatabaseBuilder {
    /// Install the decoded section plane (a load path).
    pub fn set_section_plane(&mut self, plane: Option<SectionPlane>) -> CadResult<()> {
        if let Some(plane) = plane {
            plane.validate()?;
        }
        self.database.section = plane;
        Ok(())
    }
}
