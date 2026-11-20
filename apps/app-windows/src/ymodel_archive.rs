//! The Windows carrier's `.ymodel` archive adapter.
//!
//! A copy of `apps/app-linux/src/ymodel_archive.rs` and
//! `apps/app-android/src-tauri/src/ymodel_archive.rs`: there is **no shared
//! crate** for this seam. `scripts/check-architecture.py` keeps `cad-store-zip`
//! out of every shared `cad-*` layer, so each carrier that can open or save the
//! native document format links it itself and exposes it through the narrow
//! `cad_platform::DocumentArchive` trait. The three adapters are identical by
//! construction; only the header comment names the host.
//!
//! It implements bytes in, `DocumentSnapshot` out, and back.
//!
//! Deliberately absent here: temp files, `fsync`, `.bak`, replace and the
//! save-time conflict check. Those belong to the file layer
//! (`apps/app-windows/src/platform.rs` for the write, `cad-host` for the conflict
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
    use cad_domain::{
        DatabaseId, DocumentIdentity, EntityId, LayerId, ObjectId, Point3, Revision,
        SemanticGeometry, SpaceId, Unit, UnitContext,
    };
    use cad_platform::{ArchiveWrite, DocumentArchive};

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
            .insert_layer(Layer {
                id: LayerId(1),
                name: "WALLS".into(),
                visible: true,
            })
            .expect("layer 1");
        for id in 1..=3u128 {
            builder
                .insert_entity(DbEntity {
                    object: DbObject {
                        id: ObjectId(id),
                        type_key: "AcDbLine".into(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id: EntityId(id),
                    layer: LayerId(id % 2),
                    space: SpaceId::Model,
                    geometry: SemanticGeometry::Line {
                        start: Point3 {
                            x: id as f64,
                            y: 0.0,
                            z: 0.0,
                        },
                        end: Point3 {
                            x: id as f64 + 10.0,
                            y: 0.0,
                            z: 0.0,
                        },
                    },
                    draw_order: 0,
                })
                .expect("entity");
        }
        builder.finish().expect("a consistent database")
    }

    fn snapshot(db: &DrawingDatabase, revision: u64) -> cad_db::DocumentSnapshot {
        db.snapshot(
            units(),
            DocumentIdentity::Temporary(11),
            Vec::new(),
            Revision(revision),
        )
        .expect("snapshot")
    }

    /// `DrawingDatabase::snapshot -> ZipArchive::write -> ZipArchive::read ->
    /// DrawingDatabase::from_snapshot`, i.e. what a save and a reopen actually run
    /// on this carrier (the same composition `app-linux` proves).
    #[test]
    fn a_document_round_trips_through_the_carrier_adapter() {
        let archive = ZipArchive::new();
        let original = snapshot(&sample(), 0);

        let written = archive
            .write(&ArchiveWrite {
                snapshot: &original,
                dirty: &cad_db::ChangeScope::default(),
                previous: None,
                document_uuid: Some("windows-carrier-test-document"),
                open_revision: 0,
                saved_at: Some("2026-11-21T00:00:00Z"),
            })
            .expect("the adapter writes an archive");
        assert!(written.bytes.starts_with(b"PK"), "a zip container");

        let loaded = archive
            .read(&written.bytes)
            .expect("the adapter reads it back");
        assert_eq!(
            loaded.manifest.document_uuid, "windows-carrier-test-document",
            "the archive keeps the identity it was given, so a reopen can reuse it"
        );
        assert_eq!(loaded.manifest.open_revision, Revision(0));
        assert_eq!(loaded.snapshot, original, "snapshot survives the container");

        let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
        assert_eq!(reopened.revision(), Revision(0));
    }

    /// A `.ymodel` open is the archive pipeline, never a connector probe.
    ///
    /// `cad-host` routes on `policy::is_ymodel_target` **before** any importer
    /// runs; the carrier side of that contract is (a) the bytes travel through
    /// `PlatformFiles` unchanged and (b) the installed adapter decodes them. The
    /// same bytes are not a DXF/DWG (they start with `PK`), so a connector probe
    /// would only ever fail on them — the branch this test pins is the archive.
    #[test]
    fn a_ymodel_open_is_decoded_by_the_archive_not_a_connector() {
        use cad_platform::{FileEvent, FileTarget, PlatformFiles};

        let archive = ZipArchive::new();
        let bytes = archive
            .write(&ArchiveWrite {
                snapshot: &snapshot(&sample(), 3),
                dirty: &cad_db::ChangeScope::default(),
                previous: None,
                document_uuid: Some("open-through-archive"),
                open_revision: 0,
                saved_at: None,
            })
            .expect("encode")
            .bytes;
        let path = std::env::temp_dir().join(format!(
            "yacr-windows-ymodel-open-{}.ymodel",
            std::process::id()
        ));
        std::fs::write(&path, &bytes).unwrap();

        let files = crate::platform::ReactFiles::with_chooser(
            "zh-CN",
            std::sync::Arc::new(|_, _| crate::platform::FileChooserOutcome::Cancelled),
        )
        .expect("labels load");
        files
            .begin_open_target(&FileTarget::from_path(&path.to_string_lossy()))
            .unwrap();
        let mut opened = None;
        for _ in 0..300 {
            if let Some(event) = files.poll().into_iter().next() {
                opened = Some(event);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = std::fs::remove_file(&path);

        let FileEvent::Opened { target, bytes } = opened.expect("open event") else {
            panic!("expected Opened");
        };
        assert!(
            cad_host::policy::is_ymodel_target(&target),
            "the shared funnel routes this target to the archive before connectors"
        );
        let loaded = archive
            .read(&bytes)
            .expect("the carrier archive decodes the delivered bytes");
        assert_eq!(loaded.manifest.document_uuid, "open-through-archive");
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
