//! Where the current document stands relative to its file (ADR-0037 D2/D12).
//!
//! `cad-db` is the authoritative content; this record is the *persistence*
//! state around it: which revision is on disk, which revision the document was
//! imported/opened at (the L0 import baseline), the archive identity, and the
//! hash of the bytes the document was loaded from.
//!
//! It deliberately lives on [`crate::Document`] rather than in `DrawingDatabase`:
//! the database never sees an archive (D1), and `saved_revision` must survive a
//! content replacement exactly as long as the document slot does.
//!
//! **Two revisions, never one** (D2): [`PersistenceState::saved_revision`]
//! advances on every successful save; [`PersistenceState::open_revision`] is the
//! import baseline and **never** advances. Collapsing them would make an edited
//! document satisfy the L0 passthrough predicate (`revision == open_revision`)
//! and export divergent content as the original bytes.

use cad_domain::Revision;
use std::sync::Arc;

/// The persistence state of one document slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistenceState {
    /// The revision last written to the document's file. `Revision(0)` for a
    /// document that has never been saved, which is also the revision a fresh
    /// blank/imported database starts at — so a brand new document is not dirty.
    pub saved_revision: Revision,
    /// Import baseline (D2). `Some(Revision(0))` right after an import; the
    /// manifest value after a `.ymodel` open; `None` for a document that never
    /// had one (a blank document, or a close).
    pub open_revision: Option<Revision>,
    /// Stable per-document identity from the archive manifest.
    ///
    /// Minted once when the document is created, never derived from content: the
    /// store's fallback (`derive_document_uuid`) hashes the header, so a
    /// content-derived identity would drift on every save of an edited drawing.
    /// A Save As must mint a **new** value (ADR-0037 D3) and keep element GUIDs.
    pub document_uuid: Option<String>,
    /// Entries the archive carries that this build does not model. They are
    /// copied through on save; never parsed and never dropped.
    pub foreign_entries: Vec<String>,
    /// `sha256` (lowercase hex) of the bytes this document was opened from, for
    /// save-time conflict detection. `None` when the document was not loaded
    /// from a file (blank/imported-from-memory), so a save cannot claim to know
    /// what the file looked like.
    pub source_sha256: Option<String>,
    /// The document's **own** native archive bytes, as last opened or saved
    /// (ADR-0037 D8/D11/D12).
    ///
    /// This is the base a save encodes **from**: its live entries are the ones a
    /// narrowed write keeps, its unknown entries are copied through
    /// (`foreign_entries` names them, but the bytes carry the payloads), and its
    /// header/manifest supplies identity and baseline. It is therefore **never**
    /// the save target: a Save As must carry the *source* archive forward rather
    /// than adopt the destination file's contents, and an in-place save must not
    /// need a re-read to preserve what was already there.
    ///
    /// `None` for a document with no native archive — a blank or
    /// connector-imported drawing — which is also the honest statement that
    /// "there is nothing to copy through or passthrough".
    pub archive_bytes: Option<Arc<[u8]>>,
}

/// A fresh document identity.
///
/// Called once per document creation (and once per Save As). Not deterministic on
/// purpose: two documents with identical content are still two documents.
pub fn new_document_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl PersistenceState {
    /// A document that has no file: never saved, never imported, no identity.
    ///
    /// `saved_revision == Revision(0)` matches the revision a freshly built
    /// database carries, so an untouched blank document is not dirty.
    pub fn untitled() -> Self {
        PersistenceState {
            saved_revision: Revision(0),
            open_revision: None,
            document_uuid: Some(new_document_uuid()),
            foreign_entries: Vec::new(),
            source_sha256: None,
            archive_bytes: None,
        }
    }

    /// A document imported from a connector format (DWG/DXF/...).
    ///
    /// The import became revision 0 of the in-memory database, so `(0, 0)` is
    /// the honest "nothing edited since the import" pair. It has no archive
    /// identity yet (D3: an import is an untitled document the user saves) and
    /// no source hash — the connector bytes are not the document's own bytes.
    pub fn imported() -> Self {
        PersistenceState {
            saved_revision: Revision(0),
            open_revision: Some(Revision(0)),
            document_uuid: Some(new_document_uuid()),
            foreign_entries: Vec::new(),
            source_sha256: None,
            archive_bytes: None,
        }
    }

    /// Whether the drawing changed since the last save (D12: `dirty` is exactly
    /// `revision != saved_revision`).
    pub fn is_dirty(&self, current: Revision) -> bool {
        current != self.saved_revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_document_is_not_dirty_and_has_no_baseline() {
        let state = PersistenceState::untitled();
        assert_eq!(state.saved_revision, Revision(0));
        assert_eq!(state.open_revision, None);
        assert!(
            state.document_uuid.is_some(),
            "a document identity is minted at creation, not derived from content"
        );
        assert!(state.foreign_entries.is_empty());
        assert_eq!(state.source_sha256, None);
        assert_eq!(
            state.archive_bytes, None,
            "a blank document has no archive to copy through"
        );
        assert!(!state.is_dirty(Revision(0)));
        assert!(state.is_dirty(Revision(1)));
    }

    #[test]
    fn an_imported_document_has_the_import_baseline_and_a_fresh_identity() {
        let state = PersistenceState::imported();
        assert_eq!(state.saved_revision, Revision(0));
        assert_eq!(state.open_revision, Some(Revision(0)));
        assert!(
            state.document_uuid.is_some(),
            "an import is a new document (D3), so it gets its own identity"
        );
        assert!(!state.is_dirty(Revision(0)));
        assert!(state.is_dirty(Revision(3)));
        assert_eq!(
            state.archive_bytes, None,
            "connector bytes are not the document's own archive"
        );
    }

    #[test]
    fn two_documents_never_share_a_minted_identity() {
        // The store's fallback hashes the header, which would make two identical
        // blank documents share an id and make an edited document's id drift.
        let first = PersistenceState::untitled();
        let second = PersistenceState::untitled();
        assert_ne!(first.document_uuid, second.document_uuid);
        assert_ne!(
            first.document_uuid,
            PersistenceState::imported().document_uuid
        );
    }

    #[test]
    fn dirty_is_a_revision_comparison_not_a_baseline_comparison() {
        // After a save the baseline stays put (D2) while the saved revision
        // advances: a document edited-saved-edited is dirty at revision 3 even
        // though its import baseline is still 0.
        let state = PersistenceState {
            saved_revision: Revision(2),
            open_revision: Some(Revision(0)),
            document_uuid: Some("doc".into()),
            foreign_entries: Vec::new(),
            source_sha256: None,
            archive_bytes: None,
        };
        assert!(!state.is_dirty(Revision(2)));
        assert!(state.is_dirty(Revision(3)));
        assert_eq!(state.open_revision, Some(Revision(0)));
    }
}
