//! The Linux carrier's `.ymodel` archive adapter.
//!
//! `scripts/check-architecture.py` keeps `cad-store-zip` out of every shared
//! `cad-*` layer, so this is the only place the container crate is linked. It
//! implements the narrow `cad_platform::DocumentArchive` seam over the store:
//! bytes in, `DocumentSnapshot` out, and back.
//!
//! Deliberately absent here: temp files, `fsync`, `.bak`, rename and the
//! save-time conflict check. Those belong to the file layer
//! (`apps/app-linux/src/platform.rs` for the write, `cad-host` for the conflict
//! decision) and both already exist there; duplicating them here would give the
//! app two atomic-replace implementations that could disagree.

use std::sync::Arc;

use cad_db::store::{MemorySink, MemorySource};
use cad_db::DocumentStore;
use cad_platform::{ArchiveBytes, ArchiveError, ArchiveWrite, DocumentArchive, LoadedArchive};
use cad_store_zip::ZipDocumentStore;

/// Stateless: one store instance is built per operation, because the archive
/// parameters (`previous` bytes, document uuid, import baseline) differ per
/// call and a cached instance would carry the last save's state into the next
/// read.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZipArchive;

impl ZipArchive {
    /// The carrier's archive adapter, ready to hand to
    /// `CadSession::set_document_archive`.
    pub fn new() -> Arc<ZipArchive> {
        Arc::new(ZipArchive)
    }
}

impl DocumentArchive for ZipArchive {
    fn read(&self, bytes: &[u8]) -> Result<LoadedArchive, ArchiveError> {
        let mut store = ZipDocumentStore::new();
        let mut source = MemorySource::new(bytes.to_vec());
        let loaded = store.read(&mut source).map_err(ArchiveError::from)?;
        Ok(LoadedArchive {
            snapshot: loaded.snapshot,
            manifest: loaded.manifest,
        })
    }

    fn write(&self, request: &ArchiveWrite<'_>) -> Result<ArchiveBytes, ArchiveError> {
        let mut store = ZipDocumentStore::new()
            .with_previous(request.previous.map(<[u8]>::to_vec))
            .with_document_uuid(request.document_uuid.map(str::to_string))
            .with_open_revision(Some(request.open_revision))
            .with_saved_at(request.saved_at.map(str::to_string));
        let mut sink = MemorySink::new();
        let report = store
            .write(request.snapshot, request.dirty, &mut sink)
            .map_err(ArchiveError::from)?;
        // `commit` is what makes the bytes observable; a sink that never
        // committed means the write did not complete, which must not be reported
        // as a saved document.
        let bytes = sink
            .committed_bytes()
            .ok_or_else(|| {
                ArchiveError::new(
                    cad_domain::ymodel::markers::INVALID_DOCUMENT,
                    "the archive sink did not commit",
                )
            })?
            .to_vec();
        Ok(ArchiveBytes {
            bytes,
            saved_revision: report.saved_revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_archive_is_refused_with_its_marker() {
        let error = ZipArchive::new()
            .read(b"definitely not a zip")
            .expect_err("refused");
        assert_eq!(
            error.marker,
            cad_domain::ymodel::markers::INVALID_DOCUMENT,
            "{error}"
        );
    }

    #[test]
    fn the_adapter_is_usable_behind_the_shared_seam() {
        // The host stores `Arc<dyn DocumentArchive>`; this proves the concrete
        // carrier type coerces without naming the store anywhere upstream.
        let archive: Arc<dyn DocumentArchive> = ZipArchive::new();
        assert!(archive.read(b"").is_err());
    }
}
