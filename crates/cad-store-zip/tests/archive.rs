//! Contract tests for the `.ymodel` container (ADR-0037 D11/D12/D13).
//!
//! Every case is a real round trip through `MemorySink`/`MemorySource`: no test
//! inspects private state, and no test asserts a value it did not get back out of
//! an archive. The one exception is the fixture builder below, which hand-writes
//! archives that contain entries this build does not model — the situation the
//! store must preserve.

mod common;

use std::io::{Cursor, Write};

use serde_json::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use cad_db::{
    table_entry, ChangeScope, DocumentSnapshot, DocumentStore, ElementSnapshot, MemorySink,
    MemorySource, ModelKind, ModelRecord, ShardSnapshot, StoreError, TableKind, CLASSES_ENTRY,
    DOCUMENT_ENTRY, MODELS_ENTRY,
};
use cad_domain::ymodel::{markers, DocumentHeader, IdCounters, ModelKey, PersistentIdOf, Shard};
use cad_domain::{DatabaseId, DocumentIdentity, EntityId, LayoutId, Revision, Unit, UnitContext};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

use common::{assemble, explode, frame, sha256_hex, v2_record, with_manifest};

const FOREIGN_PATH: &str = "raw/source-17.bin";
const FOREIGN_BODY: &[u8] = b"\x00\x01\x02opaque source bytes\xff";

// ---------------------------------------------------------------- fixtures ---

fn header(serial: u128) -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(7),
        units: UnitContext {
            source: Unit::Millimeter,
            display: Unit::Meter,
            display_per_source: Some(0.001),
            decimal_places: 4,
        },
        identity: DocumentIdentity::Temporary(serial),
        resource_keys: vec!["arial.ttf".to_string(), "logo.png".to_string()],
        embed_resources: false,
        application_domain: None,
        // Header variables are owned by `cad-db`; the store only carries them.
        variables: serde_json::json!({}),
        next: IdCounters {
            entity: 41,
            block: 2,
            layer: 4,
            layout: 2,
            style: 3,
            dim_style: 1,
            mleader_style: 1,
            linetype: 3,
            scale: 2,
        },
    }
}

fn element(seq: u128, class: &str) -> Value {
    serde_json::json!({
        "id": EntityId(seq).persistent().raw(),
        "class": class,
        "layer": 1,
        "geometry": {"start": [0.0, 0.0, 0.0], "end": [1.0, 2.0, 0.0]},
    })
}

/// Two models, two shards, three tables — the smallest document that exercises
/// every kind of entry the format has.
fn snapshot() -> DocumentSnapshot {
    let models = vec![
        ModelRecord {
            key: ModelKey::ModelSpace,
            kind: ModelKind::ModelSpace,
            name: Some("Model".to_string()),
            owner: None,
        },
        ModelRecord {
            key: ModelKey::Paper(LayoutId(1)),
            kind: ModelKind::PaperSpace,
            name: Some("Layout 1".to_string()),
            owner: Some(EntityId(9).persistent()),
        },
    ];
    // Order matters for equality: the reader returns models sorted by model id and
    // shards sorted by (model id, shard index).
    let shards = vec![
        ShardSnapshot {
            model: ModelKey::ModelSpace,
            shard: Shard(0),
            elements: vec![
                ElementSnapshot {
                    id: EntityId(1),
                    payload: element(1, "cad:Line"),
                },
                ElementSnapshot {
                    id: EntityId(2),
                    payload: element(2, "cad:Circle"),
                },
            ],
        },
        ShardSnapshot {
            model: ModelKey::Paper(LayoutId(1)),
            shard: Shard(0),
            elements: vec![ElementSnapshot {
                id: EntityId(9),
                payload: element(9, "cad:Text"),
            }],
        },
    ];
    let mut document = DocumentSnapshot {
        header: header(3),
        revision: Revision(12),
        models,
        shards,
        tables: Default::default(),
        textures: Default::default(),
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    };
    document.tables.set(
        TableKind::Layers,
        vec![
            serde_json::json!({"id": 1, "name": "0", "color": 7}),
            serde_json::json!({"id": 2, "name": "Walls", "color": 1}),
        ],
    );
    document.tables.set(
        TableKind::Styles,
        vec![serde_json::json!({"id": 1, "name": "Standard", "height": 2.5})],
    );
    document.tables.set(
        TableKind::Blocks,
        vec![serde_json::json!({"id": 1, "name": "Door"})],
    );
    document
}

fn write_snapshot(
    store: &mut ZipDocumentStore,
    snapshot: &DocumentSnapshot,
) -> (Vec<u8>, cad_db::SaveReport) {
    let mut sink = MemorySink::new();
    let report = store
        .write(snapshot, &ChangeScope::default(), &mut sink)
        .expect("write");
    let bytes = sink.committed_bytes().expect("committed").to_vec();
    (bytes, report)
}

fn archive_bytes(snapshot: &DocumentSnapshot) -> Vec<u8> {
    write_snapshot(&mut ZipDocumentStore::new(), snapshot).0
}

fn read_archive(bytes: &[u8]) -> Result<cad_db::LoadedDocument, StoreError> {
    let mut source = MemorySource::new(bytes.to_vec());
    ZipDocumentStore::new().read(&mut source)
}

// ------------------------------------------------------- archive surgery ----

/// Add entries this build does not model to `base`.
///
/// `listed` decides whether the entry also appears in `manifest.entries` as an
/// `opaque` foreign record — the two shapes a foreign entry really takes (a
/// conformant writer's unlisted blob, and this store's own record of one it
/// carried over).
fn with_foreign(base: &[u8], path: &str, body: &[u8], listed: bool) -> Vec<u8> {
    let (mut entries, mut manifest) = explode(base);
    entries.push((path.to_string(), body.to_vec()));
    if listed {
        manifest["entries"]
            .as_array_mut()
            .expect("entries")
            .push(v2_record(path, "opaque", body, Some(body)));
        manifest["entries"]
            .as_array_mut()
            .expect("entries")
            .sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    }
    // `assemble` writes the unlisted entry `Stored` verbatim and frames the
    // modelled JSON entries, so the archive is a valid v2 container.
    assemble(&entries, &manifest)
}

/// Replace one entry's bytes, leaving `manifest.json` untouched (tampering).
fn tamper(base: &[u8], path: &str, body: &[u8]) -> Vec<u8> {
    let (entries, manifest) = explode(base);
    let entries: Vec<(String, Vec<u8>)> = if body.is_empty() {
        entries
            .into_iter()
            .filter(|(name, _)| name != path)
            .collect()
    } else {
        entries
            .into_iter()
            .map(|(name, old)| {
                if name == path {
                    (name, body.to_vec())
                } else {
                    (name, old)
                }
            })
            .collect()
    };
    assemble(&entries, &manifest)
}

/// Patch `manifest.json` and leave every entry payload alone.
fn patch_manifest(base: &[u8], patch: impl FnOnce(&mut Value)) -> Vec<u8> {
    with_manifest(base, patch)
}

/// Write entries with no `manifest.json` at all: not an archive this store opens.
fn write_entries_without_manifest(entries: &[(String, Vec<u8>, CompressionMethod)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut sorted = entries.to_vec();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, body, method) in &sorted {
        let options = SimpleFileOptions::default()
            .compression_method(*method)
            .last_modified_time(DateTime::default());
        writer.start_file(name.as_str(), options).expect("start");
        writer.write_all(body).expect("body");
    }
    writer.finish().expect("finish").into_inner()
}

// ------------------------------------------------------------------ tests ---

#[test]
fn a_snapshot_survives_a_full_round_trip_with_a_foreign_entry() {
    let snapshot = snapshot();
    let previous = with_foreign(&archive_bytes(&snapshot), FOREIGN_PATH, FOREIGN_BODY, true);

    let mut store = ZipDocumentStore::new().with_previous(Some(previous));
    let (bytes, report) = write_snapshot(&mut store, &snapshot);

    // The save encoded the document entries and copied the foreign one through.
    // Untouched shards are raw-copied too now (Phase 2), so the count is an
    // aggregate; the structural checks below prove which entry was preserved.
    assert_eq!(report.saved_revision, Revision(12));
    assert!(report.entries_written >= 6, "{report:?}");
    assert!(report.entries_copied >= 1, "{report:?}");
    assert_eq!(
        report.sha256, None,
        "the sink cannot report the archive hash"
    );
    assert_eq!(report.bytes_written, bytes.len() as u64);

    let loaded = read_archive(&bytes).expect("read back");
    assert_eq!(loaded.snapshot, snapshot, "the document is a fixed point");
    // `PartialEq` compares only keys and lengths for binary payloads (ADR-0040
    // D16), so the shaping assertion above cannot catch corrupted bytes.
    assert!(
        loaded.snapshot.bytes_equal(&snapshot),
        "textures/resources must survive byte for byte"
    );
    assert_eq!(loaded.snapshot.revision, loaded.manifest.saved_revision);
    assert_eq!(
        loaded.manifest.foreign_entries,
        vec![FOREIGN_PATH.to_string()]
    );

    // The foreign bytes are byte-identical, and still outside the model.
    let (entries, manifest) = explode(&bytes);
    let carried = entries
        .iter()
        .find(|(name, _)| name == FOREIGN_PATH)
        .expect("foreign entry preserved");
    assert_eq!(carried.1, FOREIGN_BODY);
    let record = manifest["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|record| record["path"] == FOREIGN_PATH)
        .expect("recorded");
    assert_eq!(record["foreign"], Value::Bool(true));
    assert_eq!(record["codec"], Value::String("opaque".to_string()));
    assert_eq!(record["sha256"], Value::String(sha256_hex(FOREIGN_BODY)));
    // An opaque entry is never decoded, so this contract records no stored
    // payload digest for it (the copied source record is inherited verbatim).
    assert_eq!(record["sha256_c"], Value::Null);
}

#[test]
fn an_unlisted_foreign_entry_is_preserved_too() {
    // The shape another writer's archive takes: an entry nothing records.
    let snapshot = snapshot();
    let previous = with_foreign(&archive_bytes(&snapshot), FOREIGN_PATH, FOREIGN_BODY, false);

    let mut store = ZipDocumentStore::new().with_previous(Some(previous));
    let (bytes, report) = write_snapshot(&mut store, &snapshot);
    // The foreign entry plus the shards this scope leaves untouched.
    assert!(report.entries_copied >= 1, "{report:?}");

    let (entries, manifest) = explode(&bytes);
    assert_eq!(
        entries
            .iter()
            .find(|(name, _)| name == FOREIGN_PATH)
            .map(|(_, body)| body.as_slice()),
        Some(FOREIGN_BODY)
    );
    // It is now recorded, with hashes measured from the bytes (no `sha256_c`: the
    // compressed bytes are not reachable through the zip reader's public API).
    let record = manifest["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|record| record["path"] == FOREIGN_PATH)
        .expect("recorded");
    assert_eq!(record["sha256"], Value::String(sha256_hex(FOREIGN_BODY)));
    assert_eq!(record["len"], Value::from(FOREIGN_BODY.len() as u64));
    assert_eq!(record["sha256_c"], Value::Null);

    // And a second load → save cycle still keeps it, now via the recorded hash.
    let mut store = ZipDocumentStore::new();
    let mut source = MemorySource::new(bytes);
    let loaded = store.read(&mut source).expect("read");
    assert_eq!(
        loaded.manifest.foreign_entries,
        vec![FOREIGN_PATH.to_string()]
    );
    let (bytes, report) = write_snapshot(&mut store, &loaded.snapshot);
    assert!(report.entries_copied >= 1, "{report:?}");
    let (entries, _) = explode(&bytes);
    assert!(entries.iter().any(|(name, _)| name == FOREIGN_PATH));
}

#[test]
fn a_newer_read_version_is_refused_before_anything_else_is_parsed() {
    let base = archive_bytes(&snapshot());
    // Break a modelled entry as well: if the gate ran late, this would surface as
    // INVALID_DOCUMENT instead of UNSUPPORTED_VERSION.
    let broken = tamper(&base, DOCUMENT_ENTRY, b"not json at all");
    let archive = patch_manifest(&broken, |manifest| {
        // `2` is this build's current read version; `3` is newer.
        manifest["format"]["read"] = Value::from(3);
    });
    let error = read_archive(&archive).expect_err("refused");
    assert_eq!(error.marker, markers::UNSUPPORTED_VERSION, "{error}");
}

#[test]
fn a_newer_minor_is_refused_before_anything_else_is_parsed() {
    let base = archive_bytes(&snapshot());
    let broken = tamper(&base, DOCUMENT_ENTRY, b"not json at all");
    let archive = patch_manifest(&broken, |manifest| {
        manifest["format"]["minor"] = Value::from(cad_domain::ymodel::FORMAT_MINOR + 1);
    });
    let error = read_archive(&archive).expect_err("refused");
    assert_eq!(error.marker, markers::UNSUPPORTED_MINOR, "{error}");

    // At the ceiling the archive is still opened.
    let same_minor = tamper(&base, DOCUMENT_ENTRY, b"not json at all");
    let error = read_archive(&same_minor).expect_err("refused");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn a_tampered_entry_is_an_invalid_document() {
    let base = archive_bytes(&snapshot());

    // Same length, different content: only the hash can catch this.
    let (entries, manifest) = explode(&base);
    let layers = entries
        .iter()
        .find(|(name, _)| name == table_entry(TableKind::Layers))
        .expect("layers");
    let mut bytes = layers.1.clone();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x20;
    assert_eq!(bytes.len(), layers.1.len());
    let tampered_entries: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .map(|(name, body)| {
            if name == table_entry(TableKind::Layers) {
                (name, bytes.clone())
            } else {
                (name, body)
            }
        })
        .collect();
    let flipped = assemble(&tampered_entries, &manifest);
    let error = read_archive(&flipped).expect_err("hash mismatch");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");

    // Truncation is caught by the length check (a prefix would still decompress).
    let truncated = tamper(&base, table_entry(TableKind::Layers), b"[]");
    let error = read_archive(&truncated).expect_err("length mismatch");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");

    // A missing entry is refused, not skipped.
    let missing = tamper(&base, table_entry(TableKind::Layers), b"");
    let error = read_archive(&missing).expect_err("missing entry");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn the_manifest_never_lists_itself() {
    let bytes = archive_bytes(&snapshot());
    let (entries, manifest) = explode(&bytes);
    let paths: Vec<&str> = manifest["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|record| record["path"].as_str().expect("path"))
        .collect();
    assert!(!paths.contains(&MANIFEST_ENTRY), "{paths:?}");
    // Manifest last, so a streaming writer never has to seek back to it.
    let mut archive = ZipArchive::new(Cursor::new(bytes.clone())).expect("zip");
    let names: Vec<String> = (0..archive.len())
        .map(|index| {
            let file = archive.by_index(index).expect("entry");
            file.name().expect("name").to_string()
        })
        .collect();
    assert_eq!(names.last().map(String::as_str), Some(MANIFEST_ENTRY));
    assert_eq!(names.len(), entries.len() + 1);

    // A forged manifest that lists itself is refused on read.
    let forged = patch_manifest(&bytes, |manifest| {
        let record = manifest["entries"]
            .as_array()
            .expect("entries")
            .first()
            .expect("one entry")
            .clone();
        let mut record = record;
        record["path"] = Value::String(MANIFEST_ENTRY.to_string());
        manifest["entries"]
            .as_array_mut()
            .expect("entries")
            .push(record);
    });
    let error = read_archive(&forged).expect_err("self-listing");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn two_writes_of_one_snapshot_are_byte_identical() {
    let snapshot = snapshot();
    let first = archive_bytes(&snapshot);
    let second = archive_bytes(&snapshot);
    assert_eq!(first, second);

    // …and the same holds through a load → save cycle, which is where a clock or
    // an insertion-ordered map would show up.
    let loaded = read_archive(&first).expect("read");
    let mut store = ZipDocumentStore::new();
    let mut source = MemorySource::new(first.clone());
    let loaded_via_store = store.read(&mut source).expect("read again");
    assert_eq!(loaded_via_store.snapshot, loaded.snapshot);
    assert_eq!(archive_bytes(&loaded.snapshot), first);
}

#[test]
fn a_budget_below_the_archive_size_aborts_the_sink() {
    let snapshot = snapshot();
    let reference = archive_bytes(&snapshot);
    let mut sink = MemorySink::new();
    let mut store = ZipDocumentStore::new().with_max_bytes(Some(32));
    let error = store
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect_err("budget");
    assert_eq!(error.marker, markers::PERSISTENCE_TOO_LARGE, "{error}");
    assert!(sink.was_aborted());
    assert_eq!(sink.committed_bytes(), None);
    assert!(
        sink.buffered().is_empty(),
        "abort leaves no partial archive"
    );
    assert!(reference.len() as u64 > 32);

    // A budget above the archive size still succeeds: the cap is not a target.
    let mut sink = MemorySink::new();
    let mut store = ZipDocumentStore::new().with_max_bytes(Some(1 << 20));
    store
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect("within budget");
    assert!(!sink.was_aborted());
    assert!(sink.committed_bytes().is_some());
}

#[test]
fn the_manifest_pins_the_document_and_the_import_baseline() {
    let snapshot = snapshot();
    let mut store = ZipDocumentStore::new()
        .with_document_uuid(Some("8f0c1a9e-1d4a-4f9b-8f1e-6d3a1c2b3c4d".to_string()))
        .with_open_revision(Some(4))
        .with_saved_at(Some("2026-11-18T00:00:00Z".to_string()));
    let (bytes, _) = write_snapshot(&mut store, &snapshot);
    let (_, manifest) = explode(&bytes);
    assert_eq!(
        manifest["document_uuid"],
        "8f0c1a9e-1d4a-4f9b-8f1e-6d3a1c2b3c4d"
    );
    assert_eq!(manifest["open_revision"], Value::from(4));
    assert_eq!(manifest["saved_revision"], Value::from(12));
    assert_eq!(manifest["saved_at"], "2026-11-18T00:00:00Z");
    assert_eq!(
        manifest["format"]["read"],
        Value::from(2),
        "this build reads only v2 zstd-frame archives"
    );
    assert_eq!(manifest["format"]["write"], Value::from(2));
    assert_eq!(
        manifest["format"]["minor"],
        Value::from(4),
        "a document that packs nothing stays a minor-4 archive (ADR-0040 D12)"
    );
    assert_eq!(
        cad_domain::ymodel::FORMAT_MINOR,
        32,
        "ADR-0043: P1 expressions/configurations moved the tier to 9, P2 features to 10 and \
         P3 relations to 11; ADR-0030 D7 vendor semantics moved it to 12; ADR-0053 D2 \
         application_domain moved it to 13; ADR-0055 block attributes moved it to 14; \
         Text.mirrored (15), 2D constraints (16) and GROUP (17) are reserved; persisted \
         named views moved the top tier to 18; document materials to 19; scenes to 20; soften to 21; display settings to 22; scene visible-tag snapshots to 23; scene Shadows/Fog snapshots to 24; per-face material paints to 25; construction guides to 26; guide point markers to 27; scene soften snapshots to 28; soften-clear lists to 29; guide styling to 30; scene hidden snapshots to 31; section planes to 32"
    );

    let loaded = read_archive(&bytes).expect("read");
    assert_eq!(loaded.manifest.open_revision, Revision(4));
    assert_eq!(
        loaded.manifest.document_uuid,
        "8f0c1a9e-1d4a-4f9b-8f1e-6d3a1c2b3c4d"
    );

    // Without a pinned baseline the store refuses to claim one (never the
    // snapshot's own revision: that is the L0 passthrough bug).
    let (bytes, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    let (_, manifest) = explode(&bytes);
    assert_eq!(manifest["open_revision"], Value::from(u64::MAX));
    assert_ne!(manifest["open_revision"], manifest["saved_revision"]);
}

#[test]
fn every_modelled_entry_is_in_the_manifest_and_the_shard_map_agrees() {
    let snapshot = snapshot();
    let bytes = archive_bytes(&snapshot);
    let (entries, manifest) = explode(&bytes);
    let paths: Vec<&str> = manifest["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|record| record["path"].as_str().expect("path"))
        .collect();
    for (name, _) in &entries {
        assert!(paths.contains(&name.as_str()), "{name} is not listed");
    }
    for expected in [
        DOCUMENT_ENTRY,
        CLASSES_ENTRY,
        MODELS_ENTRY,
        "tables/layers.json",
        "tables/styles.json",
        "tables/blocks.json",
    ] {
        assert!(paths.contains(&expected), "{expected} is missing");
    }
    // `entries` is sorted by path, and no hash is missing.
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted);
    for record in manifest["entries"].as_array().expect("entries") {
        assert_eq!(
            record["sha256"].as_str().map(str::len),
            Some(64),
            "a modelled entry records the logical digest: {record}"
        );
        assert_eq!(
            record["sha256_c"].as_str().map(str::len),
            Some(64),
            "a modelled entry records the stored-payload digest: {record}"
        );
        assert_eq!(
            record["codec"],
            Value::String("zstd".to_string()),
            "every entry of this snapshot is a modelled JSON entry: {record}"
        );
    }

    let model_space = ModelKey::ModelSpace.persistent().raw().to_string();
    let paper = ModelKey::Paper(LayoutId(1)).persistent().raw().to_string();
    assert_eq!(
        manifest["shard_map"][model_space.as_str()],
        serde_json::json!(["elements/".to_string() + &model_space + "-0.json"])
    );
    assert_eq!(
        manifest["shard_map"][paper.as_str()],
        serde_json::json!(["elements/".to_string() + &paper + "-0.json"])
    );
}

#[test]
fn elements_are_sorted_by_id_and_a_duplicate_is_refused() {
    let mut snapshot = snapshot();
    snapshot.shards[0].elements.reverse();
    let bytes = archive_bytes(&snapshot);
    let loaded = read_archive(&bytes).expect("read");
    let ids: Vec<u128> = loaded.snapshot.shards[0]
        .elements
        .iter()
        .map(|element| element.id.0)
        .collect();
    assert_eq!(ids, [1, 2], "the entry is written in id order");

    // A payload whose id repeats inside one shard is an invalid document.
    let (entries, manifest) = explode(&bytes);
    let shard_path = format!(
        "elements/{}-0.json",
        ModelKey::ModelSpace.persistent().raw()
    );
    let shard = entries
        .iter()
        .find(|(name, _)| *name == shard_path)
        .expect("shard");
    let mut rows: Value = serde_json::from_slice(&shard.1).expect("rows");
    let first = rows[0].clone();
    rows.as_array_mut().expect("array").push(first);
    let entries: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .map(|(name, body)| {
            if name == shard_path {
                (name, serde_json::to_vec(&rows).expect("encode"))
            } else {
                (name, body)
            }
        })
        .collect();
    // The manifest record would no longer describe the re-framed payload, so
    // rebuild it for this entry only: the duplicate must be caught by the
    // decoder, not by a stale hash.
    let mut manifest = manifest;
    let body = serde_json::to_vec(&rows).expect("encode");
    let record = v2_record(&shard_path, "zstd", &frame(&body), Some(&body));
    for entry in manifest["entries"].as_array_mut().expect("entries") {
        if entry["path"] == shard_path.as_str() {
            *entry = record.clone();
        }
    }
    let duplicate = assemble(&entries, &manifest);
    let error = read_archive(&duplicate).expect_err("duplicate id");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn a_shard_map_that_disagrees_with_the_entries_is_refused() {
    let bytes = archive_bytes(&snapshot());
    let archive = patch_manifest(&bytes, |manifest| {
        manifest["shard_map"] = serde_json::json!({});
    });
    let error = read_archive(&archive).expect_err("shard_map mismatch");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn an_archive_without_a_manifest_is_refused() {
    let (entries, _) = explode(&archive_bytes(&snapshot()));
    let entries: Vec<(String, Vec<u8>, CompressionMethod)> = entries
        .into_iter()
        .map(|(name, body)| (name, body, CompressionMethod::Deflated))
        .collect();
    // No manifest at all: the entries alone are not an archive.
    let bytes = write_entries_without_manifest(&entries);
    let error = read_archive(&bytes).expect_err("no manifest");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");

    // And bytes that are not a zip at all are the same failure, not a panic.
    let error = read_archive(b"definitely not a zip").expect_err("not a zip");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn a_listed_entry_this_build_does_not_model_is_refused_not_dropped() {
    let base = archive_bytes(&snapshot());
    // Listed, non-foreign, and unknown to this build: a newer writer's entry.
    let (mut entries, mut manifest) = explode(&base);
    let body = b"{}".to_vec();
    entries.push(("raw/source-99.bin".to_string(), body.clone()));
    // A complete, non-foreign v2 record: the refusal must be "this build does
    // not model the path", not a hash or version error.
    manifest["entries"]
        .as_array_mut()
        .expect("entries")
        .push(v2_record("raw/source-99.bin", "raw", &body, Some(&body)));
    let archive = assemble(&entries, &manifest);
    let error = read_archive(&archive).expect_err("unmodelled entry");
    assert_eq!(error.marker, markers::UNSUPPORTED_MINOR, "{error}");
}

#[test]
fn a_foreign_entry_cannot_overwrite_a_live_one() {
    let snapshot = snapshot();
    // The previous archive claims `tables/layers.json` is foreign; the save writes
    // it from the snapshot, and the collision must fail instead of losing either.
    let base = archive_bytes(&snapshot);
    let previous = patch_manifest(&base, |manifest| {
        for record in manifest["entries"].as_array_mut().expect("entries") {
            if record["path"] == table_entry(TableKind::Layers) {
                record["foreign"] = Value::Bool(true);
            }
        }
    });
    let mut store = ZipDocumentStore::new().with_previous(Some(previous));
    let mut sink = MemorySink::new();
    let error = store
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect_err("collision");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(sink.committed_bytes().is_none());
}

#[test]
fn a_previous_archive_that_cannot_be_read_fails_the_save() {
    let snapshot = snapshot();
    let mut store = ZipDocumentStore::new().with_previous(Some(b"truncated".to_vec()));
    let mut sink = MemorySink::new();
    let error = store
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect_err("unreadable previous");
    assert_eq!(error.marker, markers::FOREIGN_ENTRIES_UNREADABLE, "{error}");
    assert!(sink.committed_bytes().is_none());
}

#[test]
fn deleting_a_table_or_shard_from_the_snapshot_drops_its_entry() {
    let full = snapshot();
    let bytes = archive_bytes(&full);
    let mut smaller = full.clone();
    smaller.tables.rows.remove(&TableKind::Blocks);
    smaller.shards.pop();
    let second = archive_bytes(&smaller);
    let (entries, manifest) = explode(&second);
    let paths: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert!(!paths.contains(&"tables/blocks.json"));
    let paper = ModelKey::Paper(LayoutId(1)).persistent().raw().to_string();
    assert!(!paths.contains(&format!("elements/{paper}-0.json").as_str()));
    // No stale shard_map row survives the deletion either.
    assert_eq!(manifest["shard_map"].as_object().expect("map").len(), 1);
    assert!(read_archive(&bytes).is_ok());
}

#[test]
fn host_metadata_tables_round_trip_as_opaque_rows() {
    // ADR-0037 D11/D14: import provenance and compatibility reports live in
    // `tables/sources.json` / `tables/compat.json`. The store never interprets
    // them, so a decoded `Value` is the contract (the bytes may be re-encoded).
    let mut full = snapshot();
    let compat = serde_json::json!({
        "connector": "dxf",
        "direction": "import",
        "level": "L1",
        "counts": null,
        "completeness": {"kind": "partial", "reasons": ["spline flags dropped"]},
        "capabilities": [{
            "type_key": "AcDbSpline",
            "read": "verified",
            "semantic": "partial",
            "render": "verified",
            "pick": "verified",
            "measure": "unsupported",
        }],
        "entries": [{
            "object": 7,
            "type_key": null,
            "status": "unverified",
            "code": "import.spline.flags_dropped",
            "message": "flags dropped",
        }],
    });
    let source = serde_json::json!({
        "source_id": "0123456789abcdef",
        "identifier": "plan.dxf",
        "kind": "drawing",
        "checksum": "0123456789abcdef",
        "retained": false,
        "connector": "dxf",
        "open_revision": null,
    });
    full.tables.set(TableKind::Compat, vec![compat.clone()]);
    full.tables.set(TableKind::Sources, vec![source.clone()]);

    let bytes = archive_bytes(&full);
    let (entries, _) = explode(&bytes);
    for path in ["tables/compat.json", "tables/sources.json"] {
        assert!(
            entries.iter().any(|(name, _)| name == path),
            "{path} must be a live entry"
        );
    }

    let loaded = read_archive(&bytes).expect("read");
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::Compat),
        Some(&[compat][..])
    );
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::Sources),
        Some(&[source][..])
    );
    // A second write of the decoded snapshot produces the same table rows.
    let again = read_archive(&archive_bytes(&loaded.snapshot)).expect("re-read");
    assert_eq!(
        again.snapshot.tables.get(TableKind::Compat),
        loaded.snapshot.tables.get(TableKind::Compat)
    );
    assert_eq!(
        again.snapshot.tables.get(TableKind::Sources),
        loaded.snapshot.tables.get(TableKind::Sources)
    );
}
