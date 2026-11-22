//! Dirty-shard raw copy (ADR-0037 D11 Phase 2).
//!
//! Every case is a real save through [`ZipDocumentStore`] with a real
//! [`DrawingDatabase`] snapshot; nothing inspects the store's private state. The
//! only hand-built archive is the corrupted previous one, which no API can
//! produce and which exists to prove a copied shard is still verified on open.
//!
//! The fixture keeps one entity in shard 0 (sequence 1) and one in shard 1
//! (sequence 2048): two shards, so "copy the untouched one" is observable.

mod common;

use serde_json::Value;

use cad_db::{
    ChangeScope, DbEntity, DbObject, DocumentSnapshot, DocumentStore, DrawingDatabase,
    DrawingDatabaseBuilder, Layer, Layout, MemorySink, MemorySource, SaveReport,
};
use cad_domain::ymodel::{markers, ModelKey};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, LayoutId, Point3, Revision, SemanticGeometry,
    SpaceId, UnitContext,
};
use cad_store_zip::ZipDocumentStore;

use common::{assemble, explode};

// ---------------------------------------------------------------- fixtures ---

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn line(id: u128, end_x: f64) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: cad_domain::ObjectId(id),
            type_key: "AcDbLine".to_string(),
            revision: Revision(0),
            source_handle: Some(format!("{id:X}")),
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(end_x, 0.0, 0.0),
        },
        draw_order: 0,
    }
}

/// One model-space entity per `(id, end_x)`; sequence 1 is shard 0, sequence
/// 2048 is shard 1 (`Shard::of` divides by 2048).
fn database(entities: &[(u128, f64)]) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(7));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".to_string(),
            visible: true,
        })
        .expect("insert layer");
    builder
        .insert_layout(Layout {
            id: LayoutId(0),
            name: "Model".to_string(),
            viewports: vec![],
        })
        .expect("insert layout");
    for (id, end_x) in entities {
        builder
            .insert_entity(line(*id, *end_x))
            .expect("insert entity");
    }
    builder.finish().expect("build the fixture database")
}

fn snapshot_of(db: &DrawingDatabase, revision: Revision) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(3),
        Vec::new(),
        revision,
    )
    .expect("encode the snapshot")
}

// --------------------------------------------------------- archive helpers ---

fn write_snapshot(
    store: &mut ZipDocumentStore,
    snapshot: &DocumentSnapshot,
    dirty: &ChangeScope,
) -> (Vec<u8>, SaveReport) {
    let mut sink = MemorySink::new();
    let report = store
        .write(snapshot, dirty, &mut sink)
        .expect("write the archive");
    let bytes = sink.committed_bytes().expect("the save committed").to_vec();
    (bytes, report)
}

fn read_archive(bytes: &[u8]) -> Result<cad_db::LoadedDocument, cad_db::StoreError> {
    let mut source = MemorySource::new(bytes.to_vec());
    ZipDocumentStore::new().read(&mut source)
}

/// `elements/<model space>-<shard>.json`.
fn shard_path(shard: u64) -> String {
    format!(
        "elements/{}-{shard}.json",
        ModelKey::ModelSpace.persistent().raw()
    )
}

fn manifest_of(bytes: &[u8]) -> Value {
    explode(bytes).1
}

fn entry_record<'a>(manifest: &'a Value, path: &str) -> &'a Value {
    manifest["entries"]
        .as_array()
        .expect("manifest entries")
        .iter()
        .find(|record| record["path"] == path)
        .unwrap_or_else(|| panic!("no manifest record for {path}"))
}

/// The verified fields of one entry's manifest record.
///
/// Both digests and both lengths are included: a v2 modelled entry records the
/// logical pair (`sha256`/`len`) and the stored-payload pair (`sha256_c`/`len_c`),
/// and a raw copy must inherit all of them unchanged.
fn checksum(record: &Value) -> (Value, Value, Value, Value, Value) {
    (
        record["sha256"].clone(),
        record["sha256_c"].clone(),
        record["len"].clone(),
        record["len_c"].clone(),
        record["crc"].clone(),
    )
}

/// Replace one entry's recorded `sha256` with a valid-looking but wrong hash.
fn corrupt_record(bytes: &[u8], path: &str) -> Vec<u8> {
    let (entries, mut manifest) = explode(bytes);
    let record = manifest["entries"]
        .as_array_mut()
        .expect("manifest entries")
        .iter_mut()
        .find(|record| record["path"] == path)
        .unwrap_or_else(|| panic!("no manifest record for {path}"));
    record["sha256"] = Value::String("f".repeat(64));
    assemble(&entries, &manifest)
}

/// Break only the **stored-payload** digest of one entry.
///
/// The frame and every other record field stay valid, so a reader that checked
/// the logical hash first (or decoded before hashing the payload) would accept
/// the archive. Refusing it proves `sha256_c` is verified before decompression.
fn corrupt_stored_record(bytes: &[u8], path: &str) -> Vec<u8> {
    let (entries, mut manifest) = explode(bytes);
    let record = manifest["entries"]
        .as_array_mut()
        .expect("manifest entries")
        .iter_mut()
        .find(|record| record["path"] == path)
        .unwrap_or_else(|| panic!("no manifest record for {path}"));
    record["sha256_c"] = Value::String("f".repeat(64));
    assemble(&entries, &manifest)
}

fn end_x(db: &DrawingDatabase, id: u128) -> f64 {
    match db.entity(EntityId(id)).expect("entity").geometry {
        SemanticGeometry::Line { end, .. } => end.x,
        ref other => panic!("fixture entity is not a line: {other:?}"),
    }
}

// ------------------------------------------------------------------ tests ---

/// A first save has no previous archive, so every entry is encoded.
#[test]
fn the_first_save_encodes_every_shard() {
    let db = database(&[(1, 10.0), (2048, 20.0)]);
    let snapshot = snapshot_of(&db, Revision(7));
    let (bytes, report) = write_snapshot(
        &mut ZipDocumentStore::new(),
        &snapshot,
        &ChangeScope::default(),
    );

    assert_eq!(report.entries_copied, 0, "{report:?}");
    let (entries, manifest) = explode(&bytes);
    for shard in [0, 1] {
        let path = shard_path(shard);
        assert!(
            entries.iter().any(|(name, _)| name == &path),
            "missing {path}"
        );
        assert!(entry_record(&manifest, &path)["foreign"] == Value::Bool(false));
    }
}

/// Saving with only shard 0 dirty copies shard 1's compressed bytes unchanged.
#[test]
fn an_untouched_shard_is_copied_and_its_record_is_unchanged() {
    let (first, first_report) = {
        let db = database(&[(1, 10.0), (2048, 20.0)]);
        let snapshot = snapshot_of(&db, Revision(7));
        write_snapshot(
            &mut ZipDocumentStore::new(),
            &snapshot,
            &ChangeScope::default(),
        )
    };

    let db = database(&[(1, 30.0), (2048, 20.0)]);
    let snapshot = snapshot_of(&db, Revision(8));
    let mut dirty = ChangeScope::default();
    dirty.element_in(ModelKey::ModelSpace, 1);

    let mut store = ZipDocumentStore::new().with_previous(Some(first.clone()));
    let (second, report) = write_snapshot(&mut store, &snapshot, &dirty);
    assert!(report.entries_copied >= 1, "{report:?}");
    assert_eq!(report.entries_copied, 1, "only shard 1 was untouched");
    assert!(
        report.entries_written < first_report.entries_written,
        "the copied shard must not be re-encoded: {report:?}"
    );

    // Shard 1's record is the first archive's record, field for field.
    let before = manifest_of(&first);
    let after = manifest_of(&second);
    assert_eq!(
        checksum(entry_record(&before, &shard_path(1))),
        checksum(entry_record(&after, &shard_path(1))),
        "a raw copy must not re-hash or re-measure its bytes"
    );
    // Shard 0 changed, so its record cannot be the old one.
    assert_ne!(
        entry_record(&before, &shard_path(0))["sha256"],
        entry_record(&after, &shard_path(0))["sha256"]
    );

    // The dirty shard's new content is what reopens; the copied shard is intact.
    let loaded = read_archive(&second).expect("the raw-copy archive reopens");
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(end_x(&reopened, 1), 30.0);
    assert_eq!(end_x(&reopened, 2048), 20.0);
}

/// A shard named only by `prev_shards` is re-encoded, not copied.
#[test]
fn a_shard_only_in_prev_shards_is_reencoded() {
    let (first, first_report) = {
        let db = database(&[(1, 10.0), (2048, 20.0)]);
        let snapshot = snapshot_of(&db, Revision(7));
        write_snapshot(
            &mut ZipDocumentStore::new(),
            &snapshot,
            &ChangeScope::default(),
        )
    };

    // Same content; only the "old row" side of the commit names shard 1.
    let db = database(&[(1, 10.0), (2048, 20.0)]);
    let snapshot = snapshot_of(&db, Revision(8));
    let mut dirty = ChangeScope::default();
    dirty.prev_element_in(ModelKey::ModelSpace, 2048);

    let mut store = ZipDocumentStore::new().with_previous(Some(first.clone()));
    let (second, report) = write_snapshot(&mut store, &snapshot, &dirty);
    assert_eq!(
        report.entries_copied, 1,
        "shard 0 may be copied, shard 1 must not: {report:?}"
    );
    assert_eq!(
        report.entries_written,
        first_report.entries_written - 1,
        "exactly one shard was re-encoded: {report:?}"
    );

    let loaded = read_archive(&second).expect("reopen");
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(reopened.entity_count(), 2);
}

/// A shard deleted from the snapshot is not written back, copied or not.
#[test]
fn a_deleted_shard_is_not_resurrected() {
    let (first, _) = {
        let db = database(&[(1, 10.0), (2048, 20.0)]);
        let snapshot = snapshot_of(&db, Revision(7));
        write_snapshot(
            &mut ZipDocumentStore::new(),
            &snapshot,
            &ChangeScope::default(),
        )
    };

    let db = database(&[(1, 10.0)]);
    let snapshot = snapshot_of(&db, Revision(8));
    let mut dirty = ChangeScope::default();
    dirty.prev_element_in(ModelKey::ModelSpace, 2048);

    let mut store = ZipDocumentStore::new().with_previous(Some(first.clone()));
    let (second, report) = write_snapshot(&mut store, &snapshot, &dirty);

    let (entries, manifest) = explode(&second);
    assert!(
        !entries.iter().any(|(name, _)| name == &shard_path(1)),
        "the deleted shard must not survive as a raw copy"
    );
    assert_eq!(
        manifest["shard_map"].as_object().expect("shard map").len(),
        1,
        "shard_map must describe only the shards still live"
    );
    assert!(report.entries_written >= 1, "{report:?}");

    let loaded = read_archive(&second).expect("reopen");
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(reopened.entity_count(), 1);
    assert_eq!(reopened.entity(EntityId(2048)), None);
}

/// A shard the previous manifest does not list is encoded from the snapshot.
#[test]
fn a_shard_missing_from_the_previous_archive_is_reencoded() {
    // The previous archive holds only shard 0.
    let (previous, _) = {
        let db = database(&[(1, 10.0)]);
        let snapshot = snapshot_of(&db, Revision(7));
        write_snapshot(
            &mut ZipDocumentStore::new(),
            &snapshot,
            &ChangeScope::default(),
        )
    };
    let previous_manifest = manifest_of(&previous);
    let listed = previous_manifest["entries"]
        .as_array()
        .expect("manifest entries")
        .iter()
        .any(|record| record["path"] == shard_path(1));
    assert!(!listed, "fixture precondition: shard 1 is absent");

    // The new snapshot adds shard 1 while shard 0 is dirty.
    let db = database(&[(1, 30.0), (2048, 20.0)]);
    let snapshot = snapshot_of(&db, Revision(8));
    let mut dirty = ChangeScope::default();
    dirty.element_in(ModelKey::ModelSpace, 1);

    let store = ZipDocumentStore::new().with_previous(Some(previous));
    let (second, report) = write_snapshot(&mut store.clone(), &snapshot, &dirty);
    assert_eq!(
        report.entries_copied, 0,
        "nothing is copyable: shard 0 is dirty, shard 1 was never recorded"
    );

    let (entries, manifest) = explode(&second);
    assert!(entries.iter().any(|(name, _)| name == &shard_path(1)));
    assert_eq!(
        entry_record(&manifest, &shard_path(1))["foreign"],
        Value::Bool(false)
    );

    let loaded = read_archive(&second).expect("reopen");
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    assert_eq!(end_x(&reopened, 1), 30.0);
    assert_eq!(end_x(&reopened, 2048), 20.0);
}

/// A copied shard is verified on open; a corrupt source record is not trusted.
#[test]
fn a_corrupt_previous_record_fails_the_reopened_archive() {
    let (first, _) = {
        let db = database(&[(1, 10.0), (2048, 20.0)]);
        let snapshot = snapshot_of(&db, Revision(7));
        write_snapshot(
            &mut ZipDocumentStore::new(),
            &snapshot,
            &ChangeScope::default(),
        )
    };
    let corrupted = corrupt_record(&first, &shard_path(1));

    let db = database(&[(1, 30.0), (2048, 20.0)]);
    let snapshot = snapshot_of(&db, Revision(8));
    let mut dirty = ChangeScope::default();
    dirty.element_in(ModelKey::ModelSpace, 1);

    let mut store = ZipDocumentStore::new().with_previous(Some(corrupted));
    let (second, report) = write_snapshot(&mut store, &snapshot, &dirty);
    assert_eq!(report.entries_copied, 1, "{report:?}");

    // The shard's compressed bytes are fine, but the record this save carried over
    // does not describe them; the reader verifies instead of trusting the copy.
    let error = read_archive(&second).expect_err("a wrong recorded hash must fail the open");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

/// Only the stored-payload digest is wrong: the pre-decompression check refuses it.
#[test]
fn a_wrong_stored_digest_is_refused_before_decompression() {
    let (first, _) = {
        let db = database(&[(1, 10.0), (2048, 20.0)]);
        let snapshot = snapshot_of(&db, Revision(7));
        write_snapshot(
            &mut ZipDocumentStore::new(),
            &snapshot,
            &ChangeScope::default(),
        )
    };
    let corrupted = corrupt_stored_record(&first, &shard_path(1));

    // Save with the untouched shard 1 copied from the corrupt source record.
    let db = database(&[(1, 30.0), (2048, 20.0)]);
    let snapshot = snapshot_of(&db, Revision(8));
    let mut dirty = ChangeScope::default();
    dirty.element_in(ModelKey::ModelSpace, 1);

    let mut store = ZipDocumentStore::new().with_previous(Some(corrupted));
    let (second, report) = write_snapshot(&mut store, &snapshot, &dirty);
    assert_eq!(report.entries_copied, 1, "{report:?}");

    // The frame still decodes to bytes whose logical `sha256`/`len` match the
    // record; only `sha256_c` is a lie, so the stored-payload check must be what
    // refuses the open (before any frame is decompressed).
    let error = read_archive(&second).expect_err("a wrong stored digest must fail the open");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(
        error.to_string().contains("stored payload"),
        "the pre-decompression check must be what refused it: {error}"
    );
}
