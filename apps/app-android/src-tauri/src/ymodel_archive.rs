//! The Android carrier's `.ymodel` archive adapter.
//!
//! This is a deliberate copy of `apps/app-linux/src/ymodel_archive.rs`, not a
//! second implementation: the shape is fixed by the seam
//! (`cad_platform::DocumentArchive` over `ZipDocumentStore` with
//! `MemorySource`/`MemorySink`), and both carriers must behave byte-identically.
//! It cannot live in a shared crate: `scripts/check-architecture.py` keeps
//! `cad-store-zip` out of every shared `cad-*` layer and allows it only in an
//! `apps/app-*` carrier, and `store-zip -> cad-platform` is the fixed direction
//! of that seam (the shared layers never name the container format). A shared
//! home would therefore need a new `cad-*` crate that depends on
//! `cad-store-zip`, which is exactly what the architecture rule forbids.
//!
//! Deliberately absent here, exactly like the Linux adapter: temp files,
//! `fsync`, `.bak`, rename, conflict checks. The write ordering belongs to the
//! file layer (`platform.rs` for SAF) and the conflict decision to `cad-host`;
//! duplicating them here would give the app two replace implementations that can
//! disagree.
//!
//! Installed by [`crate::host::tick`] on the `native-below` path only. Without
//! it every `.ymodel` open/save on this carrier refuses with
//! `unsupported-runtime: ymodel` (ADR-0041 D1: `SAVE` writes only `.ymodel`, so
//! a carrier without this adapter cannot save anything at all).

use std::sync::Arc;

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
    use cad_db::{
        ChangeScope, DocumentSnapshot, ModelKind, ModelRecord, ShardSnapshot, TableSnapshot,
    };
    use cad_domain::ymodel::{DocumentHeader, IdCounters, ModelKey, Shard};
    use cad_domain::{DatabaseId, DocumentIdentity, Revision, UnitContext};

    /// A minimal, real snapshot: one model-space model and one empty shard.
    fn snapshot() -> DocumentSnapshot {
        DocumentSnapshot {
            header: DocumentHeader {
                database: DatabaseId(1),
                units: UnitContext::drawing_units(),
                identity: DocumentIdentity::Temporary(7),
                resource_keys: Vec::new(),
                embed_resources: false,
                variables: serde_json::json!({}),
                next: IdCounters::default(),
            },
            revision: Revision(3),
            models: vec![ModelRecord {
                key: ModelKey::ModelSpace,
                kind: ModelKind::ModelSpace,
                name: None,
                owner: None,
            }],
            shards: vec![ShardSnapshot {
                model: ModelKey::ModelSpace,
                shard: Shard(0),
                elements: Vec::new(),
            }],
            tables: TableSnapshot::default(),
            textures: Default::default(),
            resources: Default::default(),
            resource_names: Default::default(),
            resource_sources: Default::default(),
        }
    }

    #[test]
    fn a_snapshot_round_trips_through_the_adapter() {
        // (a) The adapter is the only thing the shared host can reach, so the
        // carrier's claim "SAVE/OPEN work on Android" starts here: encode a real
        // snapshot, then decode the produced bytes back.
        let archive = ZipArchive::new();
        let snapshot = snapshot();
        let dirty = ChangeScope::default();
        let written = archive
            .write(&ArchiveWrite {
                snapshot: &snapshot,
                dirty: &dirty,
                previous: None,
                document_uuid: None,
                open_revision: u64::MAX,
                saved_at: None,
            })
            .expect("the snapshot encodes");
        assert!(
            written.bytes.starts_with(b"PK\x03\x04"),
            "the native format is the zip-container .ymodel"
        );

        let loaded = archive.read(&written.bytes).expect("the bytes decode");
        assert_eq!(loaded.snapshot.revision, Revision(3));
        assert_eq!(loaded.manifest.saved_revision, written.saved_revision);
        assert_eq!(loaded.snapshot.models.len(), 1);
    }

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
