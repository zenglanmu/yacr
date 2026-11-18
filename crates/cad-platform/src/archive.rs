//! The document-archive seam (ADR-0037 D1/D12/D16).
//!
//! `cad-store-zip` may only be linked by an `apps/app-*` carrier
//! (`scripts/check-architecture.py`), so the shared layers cannot name it. This
//! trait is how they use it anyway: the host asks "decode these bytes" or
//! "encode this snapshot", and the carrier supplies the implementation.
//!
//! It is deliberately narrower than [`cad_db::DocumentStore`]:
//!
//! * no `ByteSink`, because a carrier that already holds a native file handle
//!   wants whole bytes and does its own write ordering (temp, fsync, `.bak`,
//!   rename). The store's chunked sink still exists for the browser budget.
//! * `write` takes the **previous archive bytes**, because foreign-entry
//!   preservation copies entries through instead of re-encoding them. Passing
//!   them per call also means the carrier can read the target fresh at save time
//!   and detect an external edit in the same step.
//! * no paths. `FileTarget` and the chooser stay in [`crate::files`].
//!
//! A carrier with no archive support returns `None` where this is requested and
//! the host refuses by name (`unsupported-runtime: ymodel`), never by silently
//! exporting something else.

use std::fmt;

use cad_db::store::ManifestSummary;
use cad_db::{ChangeScope, DocumentSnapshot, StoreError};
use cad_domain::Revision;

/// One archive decoded into a snapshot plus the manifest facts a session needs.
#[derive(Debug, Clone)]
pub struct LoadedArchive {
    pub snapshot: DocumentSnapshot,
    pub manifest: ManifestSummary,
}

/// One archive to encode.
#[derive(Debug, Clone)]
pub struct ArchiveWrite<'a> {
    pub snapshot: &'a DocumentSnapshot,
    /// Which entries must be rewritten. Phase 1 re-encodes everything; Phase 2
    /// uses this to copy unchanged entries at the compressed-byte level.
    pub dirty: &'a ChangeScope,
    /// The archive currently on disk, if any. Foreign entries are copied from it
    /// and it is the only way an unmodelled entry survives a save.
    pub previous: Option<&'a [u8]>,
    /// Stable per-document identity. `None` mints one on first save; a Save As
    /// must pass a **new** value while element GUIDs stay unchanged (D3).
    pub document_uuid: Option<&'a str>,
    /// The import baseline (D2). Never advances across saves; `u64::MAX` when the
    /// carrier has no baseline so the L0 passthrough predicate cannot be
    /// satisfied by accident.
    pub open_revision: u64,
    /// ISO-8601 timestamp for the manifest, or `None` when the carrier has no
    /// clock. Never invented here.
    pub saved_at: Option<&'a str>,
}

/// One encoded archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveBytes {
    pub bytes: Vec<u8>,
    /// Echoed from the manifest. The caller may only advance its saved revision
    /// to this value after the bytes are durably on disk.
    pub saved_revision: Revision,
}

/// Why an archive could not be decoded or encoded.
///
/// `marker` is one of `cad_domain::ymodel::markers`; match on it, never on the
/// detail text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveError {
    pub marker: &'static str,
    pub detail: String,
}

impl ArchiveError {
    pub fn new(marker: &'static str, detail: impl Into<String>) -> ArchiveError {
        ArchiveError {
            marker,
            detail: detail.into(),
        }
    }
}

impl From<StoreError> for ArchiveError {
    fn from(error: StoreError) -> Self {
        ArchiveError {
            marker: error.marker,
            detail: error.detail,
        }
    }
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.marker, self.detail)
    }
}

impl std::error::Error for ArchiveError {}

/// Encode and decode the native document archive.
///
/// Implemented by a carrier (currently the Linux host, over `cad-store-zip`).
pub trait DocumentArchive: Send + Sync {
    /// Decode a complete archive.
    ///
    /// Must refuse a document it cannot represent faithfully: a newer `read` or
    /// `minor` is [`cad_domain::ymodel::markers::UNSUPPORTED_VERSION`] /
    /// [`cad_domain::ymodel::markers::UNSUPPORTED_MINOR`], never a partial load.
    fn read(&self, bytes: &[u8]) -> Result<LoadedArchive, ArchiveError>;

    /// Encode a snapshot.
    fn write(&self, request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoArchive;

    impl DocumentArchive for NoArchive {
        fn read(&self, _bytes: &[u8]) -> Result<LoadedArchive, ArchiveError> {
            Err(ArchiveError::new(
                cad_domain::ymodel::markers::UNSUPPORTED_VERSION,
                "no archive support in this build",
            ))
        }

        fn write(&self, _request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError> {
            Err(ArchiveError::new(
                cad_domain::ymodel::markers::UNSUPPORTED_VERSION,
                "no archive support in this build",
            ))
        }
    }

    fn _assert_object_safe(_: &dyn DocumentArchive) {}

    #[test]
    fn the_seam_is_object_safe_and_carries_markers() {
        let archive = NoArchive;
        let error = archive.read(b"not an archive").expect_err("refused");
        assert_eq!(
            error.marker,
            cad_domain::ymodel::markers::UNSUPPORTED_VERSION
        );
        // The marker, not the prose, is the contract: it is what the wire and the
        // exit-code mapping read.
        assert!(error.to_string().contains("ymodel.unsupported_version"));
    }

    #[test]
    fn store_errors_keep_their_marker() {
        let error: ArchiveError = StoreError::invalid("missing manifest.json").into();
        assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
        assert_eq!(error.detail, "missing manifest.json");
    }
}
