//! Document texture entries (`textures/<key>`, ADR-0038 D4).
//!
//! What is asserted here, all through real archives:
//!
//! * a snapshot texture becomes one `textures/<key>` entry, stored (not re-deflated)
//!   and described by the manifest with `foreign: false`, `len` and `sha256`;
//! * save → read returns the identical bytes;
//! * a corrupted or truncated texture entry refuses the open through the existing
//!   entry-verification path;
//! * an **unlisted** `textures/...` entry is a foreign entry and is preserved and
//!   re-written verbatim.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use cad_db::{ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, StoreError};
use cad_domain::ymodel::{markers, DocumentHeader, IdCounters};
use cad_domain::{DatabaseId, DocumentIdentity, Revision, UnitContext};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

/// A canonical texture key (32 lowercase hex + extension).
const KEY: &str = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";

/// A small but real-looking PNG payload: signature + a body that is not JSON.
fn png_bytes() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R']);
    bytes.extend((0..64).map(|index| (index * 7) as u8));
    bytes
}

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(9),
        units: UnitContext::drawing_units(),
        identity: DocumentIdentity::Temporary(5),
        resource_keys: Vec::new(),
        embed_resources: false,
        variables: serde_json::json!({}),
        next: IdCounters::default(),
    }
}

fn snapshot_with(textures: &[(&str, &[u8])]) -> DocumentSnapshot {
    let mut map: BTreeMap<String, Arc<[u8]>> = BTreeMap::new();
    for (key, bytes) in textures {
        map.insert((*key).to_string(), Arc::from(*bytes));
    }
    DocumentSnapshot {
        header: header(),
        revision: Revision(4),
        models: Vec::new(),
        shards: Vec::new(),
        tables: Default::default(),
        textures: map,
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    }
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

fn read_archive(bytes: &[u8]) -> Result<cad_db::LoadedDocument, StoreError> {
    let mut source = MemorySource::new(bytes.to_vec());
    ZipDocumentStore::new().read(&mut source)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
}

/// Every entry as `(name, body, compression)`, plus the parsed manifest.
fn explode(bytes: &[u8]) -> (Vec<(String, Vec<u8>, CompressionMethod)>, Value) {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip");
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("entry");
        let name = file.name().expect("name").to_string();
        let method = file.compression();
        let mut body = Vec::new();
        file.read_to_end(&mut body).expect("body");
        if name != MANIFEST_ENTRY {
            entries.push((name, body, method));
        }
    }
    let mut manifest_file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut manifest_bytes = Vec::new();
    manifest_file
        .read_to_end(&mut manifest_bytes)
        .expect("manifest");
    (
        entries,
        serde_json::from_slice(&manifest_bytes).expect("manifest json"),
    )
}

/// Rebuild an archive from `entries` plus `manifest`, manifest last.
///
/// The manifest is passed through unchanged, which is what makes a tampered entry
/// fail the recorded `len`/`sha256` instead of being silently accepted.
fn assemble(entries: &[(String, Vec<u8>)], manifest: &Value) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut sorted: Vec<&(String, Vec<u8>)> = entries.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, body) in sorted {
        writer
            .start_file(name.as_str(), zip_options())
            .expect("start");
        writer.write_all(body).expect("body");
    }
    writer
        .start_file(MANIFEST_ENTRY, zip_options())
        .expect("manifest start");
    serde_json::to_writer(&mut writer, manifest).expect("manifest body");
    writer.finish().expect("finish").into_inner()
}

fn manifest_entry<'a>(manifest: &'a Value, path: &str) -> &'a Value {
    manifest["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .find(|entry| entry["path"] == path)
        .unwrap_or_else(|| panic!("manifest has no entry {path:?}"))
}

// ------------------------------------------------------------- round trip ---

#[test]
fn a_snapshot_texture_is_one_stored_binary_entry_with_a_verified_record() {
    let bytes = png_bytes();
    let snapshot = snapshot_with(&[(KEY, &bytes)]);
    let (archive, report) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    assert!(report.entries_written >= 1);

    let path = format!("textures/{KEY}");
    let (entries, manifest) = explode(&archive);
    let (_, body, method) = entries
        .iter()
        .find(|(name, _, _)| name == &path)
        .unwrap_or_else(|| panic!("no {path:?} entry"));
    assert_eq!(body, &bytes, "the bytes are written verbatim");
    assert_eq!(
        *method,
        CompressionMethod::Stored,
        "an already-compressed image is not deflated again"
    );

    let record = manifest_entry(&manifest, &path);
    assert_eq!(record["foreign"], Value::Bool(false));
    assert_eq!(record["len"], Value::from(bytes.len()));
    assert_eq!(record["sha256"], Value::String(sha256_hex(&bytes)));
    assert!(record["crc"].is_u64());
}

#[test]
fn save_read_returns_the_identical_bytes_and_key() {
    let jpeg: Vec<u8> = {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0];
        bytes.extend((0..32).map(|index| index as u8));
        bytes
    };
    let jpeg_key = "0f1e2d3c4b5a69788796a5b4c3d2e1f0.jpg";
    let png = png_bytes();
    let snapshot = snapshot_with(&[(KEY, &png), (jpeg_key, &jpeg)]);
    let (archive, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);

    let loaded = read_archive(&archive).expect("read");
    assert_eq!(loaded.snapshot.textures.len(), 2);
    assert_eq!(&loaded.snapshot.textures[KEY][..], &png[..]);
    assert_eq!(&loaded.snapshot.textures[jpeg_key][..], &jpeg[..]);
    // The loaded-document mirror is the same decode, so it cannot disagree.
    assert_eq!(loaded.textures, loaded.snapshot.textures);
    // A load → save cycle keeps them byte-identical (the archive is a fixed point
    // for the texture entries too).
    let (again, _) = write_snapshot(&mut ZipDocumentStore::new(), &loaded.snapshot);
    assert_eq!(again, archive);
}

// --------------------------------------------------------- corrupted input ---

#[test]
fn a_texture_entry_with_the_wrong_bytes_is_refused() {
    let bytes = png_bytes();
    let snapshot = snapshot_with(&[(KEY, &bytes)]);
    let (archive, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    let (mut entries, manifest) = explode(&archive);
    let path = format!("textures/{KEY}");

    // Same length, one changed byte: only the recorded hash can catch this.
    for (name, body, _) in entries.iter_mut() {
        if name == &path {
            let last = body.len() - 1;
            body[last] ^= 0xff;
        }
    }
    let rebuilt: Vec<(String, Vec<u8>)> = entries
        .iter()
        .map(|(name, body, _)| (name.clone(), body.clone()))
        .collect();
    let tampered = assemble(&rebuilt, &manifest);
    let error = read_archive(&tampered).expect_err("refused");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(error.to_string().contains(&path), "{error}");
}

#[test]
fn a_truncated_texture_entry_is_refused() {
    let bytes = png_bytes();
    let snapshot = snapshot_with(&[(KEY, &bytes)]);
    let (archive, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    let (mut entries, manifest) = explode(&archive);
    let path = format!("textures/{KEY}");
    for (name, body, _) in entries.iter_mut() {
        if name == &path {
            body.truncate(body.len() / 2);
        }
    }
    let rebuilt: Vec<(String, Vec<u8>)> = entries
        .iter()
        .map(|(name, body, _)| (name.clone(), body.clone()))
        .collect();
    let truncated = assemble(&rebuilt, &manifest);
    let error = read_archive(&truncated).expect_err("refused");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn a_listed_texture_with_a_non_canonical_name_is_refused() {
    // `is_modelled` accepts only `textures/<valid key>`; a listed entry this build
    // cannot name is a newer writer's data, so it is refused rather than dropped on
    // the next save.
    let snapshot = snapshot_with(&[(KEY, &png_bytes())]);
    let (archive, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    let (mut entries, mut manifest) = explode(&archive);
    entries.push((
        "textures/not-a-key.png".to_string(),
        vec![1, 2, 3],
        CompressionMethod::Deflated,
    ));
    let record = serde_json::json!({
        "path": "textures/not-a-key.png",
        "sha256": sha256_hex(&[1, 2, 3]),
        "sha256_c": null,
        "len": 3,
        "crc": crc32(&[1u8, 2, 3]),
        "foreign": false,
    });
    manifest["entries"]
        .as_array_mut()
        .expect("entries")
        .push(record);
    let rebuilt: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .map(|(name, body, _)| (name, body))
        .collect();
    let archive = assemble(&rebuilt, &manifest);
    let error = read_archive(&archive).expect_err("refused");
    assert_eq!(error.marker, markers::UNSUPPORTED_MINOR, "{error}");
}

/// The zip CRC-32 of a byte string (the store's own algorithm, recomputed here so
/// the fixture does not depend on crate internals).
fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let mut value = index as u32;
        for _ in 0..8 {
            value = if value & 1 == 1 {
                0xedb8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
        }
        *entry = value;
    }
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc = table[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

// ------------------------------------------------------- foreign textures ---

#[test]
fn an_unlisted_texture_entry_survives_as_a_foreign_entry() {
    let png = png_bytes();
    let snapshot = snapshot_with(&[(KEY, &png)]);
    let (archive, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    let (mut entries, manifest) = explode(&archive);

    // A blob this build may not model: a `textures/` path with no manifest record.
    let unlisted_path = "textures/00000000000000000000000000000000.png";
    let unlisted_body = b"a blob another writer left in textures/".to_vec();
    entries.push((
        unlisted_path.to_string(),
        unlisted_body.clone(),
        CompressionMethod::Deflated,
    ));
    let rebuilt: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .map(|(name, body, _)| (name, body))
        .collect();
    let with_foreign = assemble(&rebuilt, &manifest);

    let mut store = ZipDocumentStore::new();
    let mut source = MemorySource::new(with_foreign.clone());
    let loaded = store.read(&mut source).expect("read");
    assert!(
        loaded
            .manifest
            .foreign_entries
            .contains(&unlisted_path.to_string()),
        "{:?}",
        loaded.manifest.foreign_entries
    );
    // Not decoded into the document's textures: it is not a modelled entry.
    assert_eq!(loaded.snapshot.textures.len(), 1);
    assert_eq!(&loaded.snapshot.textures[KEY][..], &png[..]);

    // The load kept the archive for foreign preservation, so the next save carries
    // the unlisted blob through unchanged.
    let (resaved, _) = write_snapshot(&mut store, &loaded.snapshot);
    let (entries, manifest) = explode(&resaved);
    let (_, body, _) = entries
        .iter()
        .find(|(name, _, _)| name == unlisted_path)
        .unwrap_or_else(|| panic!("the unlisted texture entry was dropped"));
    assert_eq!(body, &unlisted_body);
    assert_eq!(
        manifest_entry(&manifest, unlisted_path)["foreign"],
        Value::Bool(true)
    );
}
