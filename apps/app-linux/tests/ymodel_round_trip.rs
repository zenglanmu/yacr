//! End-to-end persistence through the Linux carrier's real archive adapter.
//!
//! `cad-db`'s fixed-point test and `cad-store-zip`'s archive tests each prove one
//! half. This one proves the composition the carrier actually ships:
//!
//! ```text
//! DrawingDatabase::snapshot -> ZipArchive::write -> bytes
//!   -> ZipArchive::read -> DrawingDatabase::from_snapshot -> snapshot
//! ```
//!
//! It also pins the two properties the host layer depends on and cannot check
//! itself: the store round-trips the *document identity* it was given, and a
//! `.ymodel` written by this adapter can be read back by the same adapter
//! without a live session (that is what a reopen is).

#![cfg(target_os = "linux")]

use app_linux::ymodel_archive::ZipArchive;
use cad_db::{DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};
use cad_domain::DocumentIdentity;
use cad_domain::{
    DatabaseId, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry, SpaceId, Unit,
    UnitContext,
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

#[test]
fn the_carrier_adapter_round_trips_a_document() {
    let archive = ZipArchive::new();
    let original = snapshot(&sample(), 0);

    let written = archive
        .write(&ArchiveWrite {
            snapshot: &original,
            dirty: &cad_db::ChangeScope::default(),
            previous: None,
            document_uuid: Some("carrier-test-document"),
            open_revision: 0,
            saved_at: Some("2026-11-18T00:00:00Z"),
        })
        .expect("the adapter writes an archive");

    let loaded = archive
        .read(&written.bytes)
        .expect("the adapter reads it back");
    assert_eq!(
        loaded.manifest.document_uuid, "carrier-test-document",
        "the archive keeps the identity it was given, so a reopen can reuse it"
    );
    assert_eq!(loaded.manifest.open_revision, Revision(0));
    assert_eq!(loaded.snapshot, original, "snapshot survives the container");

    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(reopened.revision(), Revision(0));
    assert_eq!(reopened.id_counters(), sample().id_counters());

    // A second save of the reopened document must be byte-identical: the carrier
    // gives the store no clock and no randomness, so a save is reproducible.
    let again = snapshot(&reopened, loaded.manifest.saved_revision.0);
    let second = archive
        .write(&ArchiveWrite {
            snapshot: &again,
            dirty: &cad_db::ChangeScope::default(),
            previous: None,
            document_uuid: Some("carrier-test-document"),
            open_revision: 0,
            saved_at: Some("2026-11-18T00:00:00Z"),
        })
        .expect("second write");
    assert_eq!(
        written.bytes, second.bytes,
        "saving is byte-for-byte stable"
    );

    // Touch the database so an unused-value lint cannot hide a missing assert.
    assert_eq!(reopened.entities().count(), 3);
}

// Foreign-entry preservation is covered where it lives: `cad-store-zip`'s
// archive tests build an unlisted entry and assert it survives a save with
// `previous` bytes. This adapter forwards `ArchiveWrite::previous` unchanged, so
// re-testing it here would only re-run the store.
