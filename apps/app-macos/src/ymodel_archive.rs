//! The macOS carrier's `.ymodel` archive adapter.
//!
//! **Why this is a copy of `apps/app-linux/src/ymodel_archive.rs`**: the carriers
//! are separate binary crates with no shared crate between them, and
//! `scripts/check-architecture.py` keeps `cad-store-zip` out of every shared
//! `cad-*` layer, so the only place the container crate can be linked is an
//! `apps/app-*` carrier. `cad_platform::DocumentArchive` is the seam that makes
//! the shared layers container-format-free; this adapter is the macOS-side
//! implementation of that seam, byte-for-byte the same mapping as Linux (bytes
//! in, `DocumentSnapshot` out, and back) so the two carriers cannot disagree
//! about the format.
//!
//! Deliberately absent here: temp files, `fsync`, `.bak`, rename and the
//! save-time conflict check. Those belong to the file layer
//! (`apps/app-macos/src/platform/fs.rs` for the write, `cad-host` for the
//! conflict decision); duplicating them here would give the app two
//! atomic-replace implementations that could disagree.

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
    use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
    use cad_domain::DocumentIdentity;
    use cad_domain::{
        DatabaseId, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId, Unit,
        UnitContext,
    };

    fn units() -> UnitContext {
        UnitContext {
            source: Unit::Millimeter,
            display: Unit::Millimeter,
            display_per_source: Some(1.0),
            decimal_places: 3,
        }
    }

    fn sample() -> DrawingDatabase {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .expect("layer 0");
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(1),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(1),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Line {
                    start: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    end: Point3 {
                        x: 10.0,
                        y: 0.0,
                        z: 0.0,
                    },
                },
                draw_order: 0,
            })
            .expect("entity");
        builder.finish().expect("a consistent database")
    }

    fn snapshot(db: &DrawingDatabase) -> cad_db::DocumentSnapshot {
        db.snapshot(
            units(),
            DocumentIdentity::Temporary(11),
            Vec::new(),
            Revision(0),
        )
        .expect("snapshot")
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

    /// (f) `snapshot -> write -> bytes -> read -> snapshot` through this
    /// carrier's real adapter, including the document identity a reopen reuses.
    #[test]
    fn the_carrier_adapter_round_trips_a_document() {
        let archive = ZipArchive::new();
        let original = snapshot(&sample());

        let written = archive
            .write(&ArchiveWrite {
                snapshot: &original,
                dirty: &cad_db::ChangeScope::default(),
                previous: None,
                document_uuid: Some("macos-carrier-test"),
                open_revision: 0,
                saved_at: Some("2026-11-21T00:00:00Z"),
            })
            .expect("the adapter writes an archive");

        let loaded = archive.read(&written.bytes).expect("reads it back");
        assert_eq!(
            loaded.manifest.document_uuid, "macos-carrier-test",
            "a reopen reuses the identity the archive was given"
        );
        assert_eq!(loaded.manifest.open_revision, Revision(0));
        assert_eq!(
            loaded.snapshot, original,
            "the snapshot survives the container"
        );

        let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
        assert_eq!(reopened.entities().count(), 1);
    }
}
