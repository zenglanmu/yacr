//! Document-level metadata that lives outside the database but is persisted with
//! it (ADR-0037 D11/D9).
//!
//! `UnitContext`, the document identity and the resource keys live on
//! `cad_app::Document` rather than in `DrawingDatabase` (they are not rows), yet
//! they are written into `document.json`. A change to one of them therefore has to
//! raise the drawing revision, or `dirty = revision != saved_revision` stays false
//! and the next save silently keeps the old header.
//!
//! This is the same shape as a header-variable write (`drawing::header_geometry`):
//! one revision bump, no object changes, and a scope that marks the header dirty.

use cad_domain::{Revision, TransactionId};

use crate::change::ChangeSet;
use crate::drawing::DrawingDatabase;
use crate::scope::ChangeScope;

impl DrawingDatabase {
    /// Record that persisted document-level metadata changed.
    ///
    /// Called after the caller has updated `cad_app::Document::units` (or another
    /// header-owned value), never before: a refused mutation must not bump the
    /// revision. Returns the commit notification, exactly like a write path.
    ///
    /// # Why a transaction id
    ///
    /// The change is a document edit like any other, so it participates in the
    /// same revision chain and can be reported to consumers that only understand
    /// `ChangeSet`.
    pub fn note_document_header_change(
        &mut self,
        transaction: TransactionId,
        reason: &str,
    ) -> ChangeSet {
        let before = self.revision;
        self.invalidate_bounds_cache();
        let after = Revision(before.0 + 1);
        self.revision = after;
        // No element or table row moved; only `document.json` is stale.
        let mut scope = ChangeScope::default();
        scope.header();
        ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: Vec::new(),
            scope,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::DatabaseId;

    fn database() -> DrawingDatabase {
        crate::DrawingDatabaseBuilder::new(DatabaseId(1))
            .finish()
            .expect("an empty database")
    }

    #[test]
    fn a_document_header_change_raises_one_revision_and_dirties_only_the_header() {
        let mut db = database();
        let before = db.revision();
        let changes = db.note_document_header_change(TransactionId(1), "units changed");
        assert_eq!(changes.before, before);
        assert_eq!(changes.after, Revision(before.0 + 1));
        assert_eq!(db.revision(), changes.after);
        assert!(
            changes.changes.is_empty(),
            "no element or table row moved: {:?}",
            changes.changes
        );
        assert!(changes.scope.header, "document.json must be rewritten");
        assert!(changes.scope.shards.is_empty());
        assert!(changes.scope.tables.is_empty());

        // Two changes are two revisions: the caller cannot accidentally coalesce
        // and lose the dirty state.
        let second = db.note_document_header_change(TransactionId(2), "units changed again");
        assert_eq!(second.before, changes.after);
        assert_eq!(second.after, Revision(before.0 + 2));
    }
}
