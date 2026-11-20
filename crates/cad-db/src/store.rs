//! The archive-independent store contract (ADR-0037 D1/D11/D12/D16).
//!
//! [`DocumentStore`] is the seam between the database and any container format.
//! It is deliberately small: read bytes into a [`DocumentSnapshot`], write a
//! snapshot into bytes. Everything that knows about the *document* stays in
//! `cad-db`; everything that knows about *files* stays in the carrier.
//!
//! # Why the byte traits look like this
//!
//! * [`ByteSource`] is read-whole. A zip archive needs `Read + Seek`, and the
//!   carriers that cannot provide that (Android SAF, browser `File`) copy the
//!   bytes in first — a decision recorded in D16, not an oversight. A future
//!   seekable source can be added without changing this trait's callers.
//! * [`ByteSink`] is **chunked and abortable** on purpose. The browser has a
//!   hard memory budget; a `write_all(&[u8])` sink would force the whole archive
//!   into one buffer before any size check could run, so the cap would only be
//!   enforced after the allocation that kills the tab. The store streams
//!   shard-by-shard and calls [`ByteSink::abort`] mid-stream when the budget is
//!   exceeded ([`cad_domain::ymodel::markers::PERSISTENCE_TOO_LARGE`]).
//! * "Saved" means [`ByteSink::commit`] returned `Ok`. A carrier that flushes on
//!   `write_chunk` but fails on commit must not be reported as saved, and a
//!   caller must not clear `dirty` before this returns.
//!
//! # Deliberately absent
//!
//! No lock files, no directory scanning, no `Path`: those are carrier concerns.
//! The save-time conflict check (`sha256` + `mtime` compared against the values
//! recorded at open) needs a `mtime`, which a pure byte sink does not have, so it
//! lives in the carrier adapter next to the file handle. Everything the archive
//! itself must guarantee — hashes, ordering, the manifest-last rule — is the
//! store's job.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use cad_domain::Revision;

use crate::scope::ChangeScope;
use crate::snapshot::DocumentSnapshot;

/// The `Read.Write.Minor` triple from `manifest.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatVersion {
    pub read: u32,
    pub write: u32,
    pub minor: u32,
}

/// The parts of `manifest.json` a caller needs after opening.
///
/// The per-entry hash table is deliberately not exposed: it is the store's own
/// integrity mechanism, and a caller that used it would couple to an
/// implementation instead of to the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestSummary {
    pub format: FormatVersion,
    /// Stable per-document identity, distinct from the database id (D3: Save-As
    /// mints a new one and keeps element GUIDs).
    pub document_uuid: String,
    /// The import baseline. **Never advances** across saves (D2).
    pub open_revision: Revision,
    /// The state this archive holds (D2). Seeds the in-memory revision on load.
    pub saved_revision: Revision,
    /// Entries this build does not model and must copy through untouched.
    pub foreign_entries: Vec<String>,
}

/// One successfully opened archive.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedDocument {
    pub snapshot: DocumentSnapshot,
    pub manifest: ManifestSummary,
    /// The document's base-colour textures, mirrored from
    /// [`DocumentSnapshot::textures`] (ADR-0038 D3).
    ///
    /// A carrier that forwards only the parsed document (or destructures the
    /// loaded value) can reach the bytes without threading the snapshot through;
    /// the store fills both from one decode, so the two always agree. The snapshot
    /// stays the authority for what a save writes.
    pub textures: BTreeMap<String, Arc<[u8]>>,
}

/// What one save actually did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    /// The revision now recorded as saved. The caller may only advance
    /// `saved_revision` to this value after [`ByteSink::commit`] succeeded.
    pub saved_revision: Revision,
    pub bytes_written: u64,
    /// Entries encoded from the snapshot.
    pub entries_written: usize,
    /// Unchanged entries copied through at the compressed-byte level.
    pub entries_copied: usize,
    /// Hash of the committed archive, when the sink can report it.
    pub sha256: Option<String>,
}

/// A store failure, identified by its stable marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError {
    /// One of `cad_domain::ymodel::markers`; match on this, not on the text.
    pub marker: &'static str,
    pub detail: String,
}

impl StoreError {
    pub fn new(marker: &'static str, detail: impl Into<String>) -> StoreError {
        StoreError {
            marker,
            detail: detail.into(),
        }
    }

    /// A malformed archive: missing entry, bad hash, bad size, unparsable JSON.
    pub fn invalid(detail: impl Into<String>) -> StoreError {
        StoreError::new(cad_domain::ymodel::markers::INVALID_DOCUMENT, detail)
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.marker, self.detail)
    }
}

impl std::error::Error for StoreError {}

/// Bytes to read an archive from.
pub trait ByteSource {
    /// Total length when known. A store may use it to reject an absurd archive
    /// before allocating, but must not require it.
    fn len_hint(&self) -> Option<u64>;

    /// The whole archive.
    fn read_all(&mut self) -> Result<Vec<u8>, StoreError>;
}

/// Bytes to write an archive to, in bounded chunks.
pub trait ByteSink {
    /// Start a new output. `len_hint` is the planned size when the store can
    /// compute it; a carrier may use it to refuse early or to reserve space.
    fn begin(&mut self, len_hint: Option<u64>) -> Result<(), StoreError>;

    /// Append one chunk. Implementations must not buffer unboundedly: the sum of
    /// chunks is the archive.
    fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), StoreError>;

    /// Publish the output durably. Only after this returns `Ok` may the caller
    /// treat the document as saved and clear its dirty state.
    fn commit(&mut self) -> Result<(), StoreError>;

    /// Discard everything written. Must leave no partial file behind.
    ///
    /// Called on budget overflow and on any error after [`ByteSink::begin`].
    fn abort(&mut self);
}

/// Read and write one archive format for one document.
pub trait DocumentStore {
    /// Parse an archive. Must refuse a document it cannot represent faithfully
    /// instead of opening it partially — a newer `minor` is
    /// [`cad_domain::ymodel::markers::UNSUPPORTED_MINOR`], not a silent downgrade.
    fn read(&mut self, source: &mut dyn ByteSource) -> Result<LoadedDocument, StoreError>;

    /// Encode a snapshot.
    ///
    /// `dirty` selects what may be copied through at the compressed-byte level
    /// instead of re-encoded; entries not in the live set are dropped, so a
    /// deleted model cannot reappear. The caller must have verified that the
    /// target file is unchanged since open (see the module docs).
    fn write(
        &mut self,
        snapshot: &DocumentSnapshot,
        dirty: &ChangeScope,
        sink: &mut dyn ByteSink,
    ) -> Result<SaveReport, StoreError>;
}

/// An in-memory source, for tests and for carriers that already hold the bytes.
#[derive(Debug, Clone, Default)]
pub struct MemorySource {
    bytes: Vec<u8>,
}

impl MemorySource {
    pub fn new(bytes: Vec<u8>) -> MemorySource {
        MemorySource { bytes }
    }
}

impl ByteSource for MemorySource {
    fn len_hint(&self) -> Option<u64> {
        Some(self.bytes.len() as u64)
    }

    fn read_all(&mut self) -> Result<Vec<u8>, StoreError> {
        Ok(std::mem::take(&mut self.bytes))
    }
}

/// An in-memory sink that records what was written and whether it committed.
///
/// `abort` clears the buffer, so a test can assert that a failed save leaves no
/// partial archive and that `commit` is what makes the bytes observable.
#[derive(Debug, Clone, Default)]
pub struct MemorySink {
    bytes: Vec<u8>,
    chunks: usize,
    begun: bool,
    committed: bool,
    aborted: bool,
    len_hint: Option<u64>,
}

impl MemorySink {
    pub fn new() -> MemorySink {
        MemorySink::default()
    }

    /// The committed bytes. `None` until `commit` succeeds.
    pub fn committed_bytes(&self) -> Option<&[u8]> {
        self.committed.then_some(self.bytes.as_slice())
    }

    /// The committed bytes, moved out of the sink.
    ///
    /// [`Self::committed_bytes`] borrows, so a caller that needs an owned
    /// `Vec<u8>` (the archive seam hands `ArchiveBytes { bytes: Vec<u8> }` to the
    /// carrier) would copy the whole archive. With resources embedded that copy
    /// is an extra tens of MiB per save for no reason, so the sink exposes the
    /// move instead. `None` until `commit` succeeds, exactly like the borrow.
    pub fn into_committed_bytes(self) -> Option<Vec<u8>> {
        self.committed.then_some(self.bytes)
    }

    /// Bytes written so far, committed or not — for asserting abort behaviour.
    pub fn buffered(&self) -> &[u8] {
        &self.bytes
    }

    pub fn chunks(&self) -> usize {
        self.chunks
    }

    pub fn was_aborted(&self) -> bool {
        self.aborted
    }

    pub fn len_hint(&self) -> Option<u64> {
        self.len_hint
    }
}

impl ByteSink for MemorySink {
    fn begin(&mut self, len_hint: Option<u64>) -> Result<(), StoreError> {
        if self.begun && !self.committed && !self.aborted {
            return Err(StoreError::invalid("sink already begun"));
        }
        self.bytes.clear();
        self.chunks = 0;
        self.begun = true;
        self.committed = false;
        self.aborted = false;
        self.len_hint = len_hint;
        Ok(())
    }

    fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), StoreError> {
        if !self.begun || self.committed || self.aborted {
            return Err(StoreError::invalid("sink is not accepting bytes"));
        }
        self.bytes.extend_from_slice(chunk);
        self.chunks += 1;
        Ok(())
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        if !self.begun || self.aborted {
            return Err(StoreError::invalid("sink cannot commit"));
        }
        self.committed = true;
        Ok(())
    }

    fn abort(&mut self) {
        self.bytes.clear();
        self.chunks = 0;
        self.aborted = true;
        self.committed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A carrier must be able to hold the store behind `Box<dyn DocumentStore>`.
    #[allow(dead_code)]
    fn _assert_object_safe_store(_: &dyn DocumentStore) {}
    #[allow(dead_code)]
    fn _assert_object_safe_sink(_: &dyn ByteSink) {}
    #[allow(dead_code)]
    fn _assert_object_safe_source(_: &dyn ByteSource) {}

    #[test]
    fn memory_sink_only_exposes_committed_bytes() {
        let mut sink = MemorySink::new();
        sink.begin(Some(4)).expect("begin");
        sink.write_chunk(b"ab").expect("chunk");
        assert_eq!(sink.committed_bytes(), None);
        assert_eq!(sink.buffered(), b"ab");
        sink.commit().expect("commit");
        assert_eq!(sink.committed_bytes(), Some(&b"ab"[..]));
        assert_eq!(sink.chunks(), 1);
        assert_eq!(sink.len_hint(), Some(4));
    }

    #[test]
    fn abort_leaves_no_partial_archive() {
        let mut sink = MemorySink::new();
        sink.begin(None).expect("begin");
        sink.write_chunk(b"partial").expect("chunk");
        sink.abort();
        assert!(sink.was_aborted());
        assert_eq!(sink.committed_bytes(), None);
        assert_eq!(sink.buffered(), b"");
        // A sink that never began cannot accept bytes.
        let mut fresh = MemorySink::new();
        assert!(fresh.write_chunk(b"x").is_err());
        assert!(fresh.commit().is_err());
    }

    #[test]
    fn begin_resets_a_previous_run() {
        let mut sink = MemorySink::new();
        sink.begin(None).expect("begin");
        sink.write_chunk(b"first").expect("chunk");
        sink.commit().expect("commit");
        sink.begin(Some(2)).expect("begin again");
        assert_eq!(sink.committed_bytes(), None);
        sink.write_chunk(b"ok").expect("chunk");
        sink.commit().expect("commit");
        assert_eq!(sink.committed_bytes(), Some(&b"ok"[..]));
    }

    #[test]
    fn memory_source_reports_its_length_once() {
        let mut source = MemorySource::new(b"archive".to_vec());
        assert_eq!(source.len_hint(), Some(7));
        assert_eq!(source.read_all().expect("read"), b"archive");
    }

    #[test]
    fn error_markers_are_the_contract() {
        let error = StoreError::invalid("missing manifest.json");
        assert_eq!(error.marker, cad_domain::ymodel::markers::INVALID_DOCUMENT);
        assert!(error.to_string().starts_with("ymodel.invalid_document: "));
    }
}
