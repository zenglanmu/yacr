//! The `dom-canvas` carrier's `.ymodel` archive adapter.
//!
//! `scripts/check-architecture.py` keeps `cad-store-zip` out of every shared
//! `cad-*` layer, so an `apps/app-*` carrier is the only place the container
//! crate is linked. This is the wasm carrier's adapter: it implements the narrow
//! [`cad_platform::DocumentArchive`] seam over the store — bytes in,
//! `DocumentSnapshot` out, and back — with no filesystem anywhere.
//!
//! A page has no file to write, so `Save` is a real browser download
//! ([`crate::download`]) and the archive is encoded entirely in memory. What the
//! native carriers get from their file layer is deliberately absent here: no
//! temp file, no `fsync`, no `.bak`, no rename and no on-disk conflict check.
//! There is nothing durable to conflict with — each save publishes a new
//! download — so the host's in-place rule never applies on this carrier.
//!
//! The adapter is installed through [`cad_ui_core::session::BytesDocument::archive`]
//! before the first document is delivered (see [`crate::live`]), so an uploaded
//! `.ymodel` opens as the first document instead of being refused for want of an
//! adapter.

use cad_db::store::{MemorySink, MemorySource};
use cad_db::DocumentStore;
use cad_platform::{ArchiveBytes, ArchiveError, ArchiveWrite, DocumentArchive, LoadedArchive};
use cad_store_zip::ZipDocumentStore;

/// Stateless: one store instance is built per operation, because the archive
/// parameters (`previous` bytes, document uuid, import baseline) differ per call
/// and a cached instance would carry the last save's state into the next read.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZipArchive;

impl ZipArchive {
    /// The carrier's archive adapter, ready to hand to a session.
    pub fn new() -> std::sync::Arc<ZipArchive> {
        std::sync::Arc::new(ZipArchive)
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
        let bytes = sink.into_committed_bytes().ok_or_else(|| {
            ArchiveError::new(
                cad_domain::ymodel::markers::INVALID_DOCUMENT,
                "the archive sink did not commit",
            )
        })?;
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
        let archive: std::sync::Arc<dyn DocumentArchive> = ZipArchive::new();
        assert!(archive.read(b"").is_err());
    }
}
