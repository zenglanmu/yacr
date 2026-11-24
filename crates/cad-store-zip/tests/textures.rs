//! Document texture entries (`textures/<key>`, ADR-0038 D4).
//!
//! What is asserted here, all through real archives:
//!
//! * a snapshot texture becomes one `textures/<key>` entry, stored (not re-deflated)
//!   and described by the manifest with `codec: "raw"`, `foreign: false`, the two
//!   digests and the stored-payload length;
//! * save → read returns the identical bytes;
//! * a corrupted or truncated texture entry refuses the open through the existing
//!   entry-verification path;
//! * an **unlisted** `textures/...` entry is a foreign entry and is preserved and
//!   re-written verbatim.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;
use zip::CompressionMethod;

use cad_db::{ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, StoreError};
use cad_domain::ymodel::{markers, DocumentHeader, IdCounters};
use cad_domain::{DatabaseId, DocumentIdentity, Revision, UnitContext};
use cad_store_zip::ZipDocumentStore;

use common::{assemble, assemble_raw, explode, explode_raw, sha256_hex, v2_record};

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
        application_domain: None,
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
    let (entries, manifest, methods) = explode_raw(&archive);
    let index = entries
        .iter()
        .position(|(name, _)| name == &path)
        .unwrap_or_else(|| panic!("no {path:?} entry"));
    assert_eq!(
        entries[index].1, bytes,
        "the raw codec stores the bytes verbatim"
    );
    assert_eq!(
        methods[index],
        CompressionMethod::Stored,
        "an already-compressed image is not deflated again"
    );

    let record = manifest_entry(&manifest, &path);
    assert_eq!(record["codec"], Value::String("raw".to_string()));
    assert_eq!(record["foreign"], Value::Bool(false));
    assert_eq!(record["len"], Value::from(bytes.len() as u64));
    assert_eq!(record["len_c"], Value::from(bytes.len() as u64));
    assert_eq!(record["sha256"], Value::String(sha256_hex(&bytes)));
    assert_eq!(
        record["sha256_c"], record["sha256"],
        "a raw entry stores its logical bytes, so both digests agree"
    );
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
    for (name, body) in entries.iter_mut() {
        if name == &path {
            let last = body.len() - 1;
            body[last] ^= 0xff;
        }
    }
    let tampered = assemble(&entries, &manifest);
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
    for (name, body) in entries.iter_mut() {
        if name == &path {
            body.truncate(body.len() / 2);
        }
    }
    let truncated = assemble(&entries, &manifest);
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
    let body = vec![1u8, 2, 3];
    let path = "textures/not-a-key.png";
    entries.push((path.to_string(), body.clone()));
    manifest["entries"]
        .as_array_mut()
        .expect("entries")
        .push(v2_record(path, "raw", &body, Some(&body)));
    let archive = assemble(&entries, &manifest);
    let error = read_archive(&archive).expect_err("refused");
    assert_eq!(error.marker, markers::UNSUPPORTED_MINOR, "{error}");
}

// ------------------------------------------------------- foreign textures ---

#[test]
fn an_unlisted_texture_entry_survives_as_a_foreign_entry() {
    let png = png_bytes();
    let snapshot = snapshot_with(&[(KEY, &png)]);
    let (archive, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    let (mut entries, manifest, mut methods) = explode_raw(&archive);

    // A blob this build may not model: a `textures/` path with no manifest record.
    // Written `Deflated` on purpose: a foreign entry keeps another writer's method.
    let unlisted_path = "textures/00000000000000000000000000000000.png";
    let unlisted_body = b"a blob another writer left in textures/".to_vec();
    entries.push((unlisted_path.to_string(), unlisted_body.clone()));
    methods.push(CompressionMethod::Deflated);
    let with_methods: Vec<(String, Vec<u8>, CompressionMethod)> = entries
        .into_iter()
        .zip(methods)
        .map(|((name, body), method)| (name, body, method))
        .collect();
    let with_foreign = assemble_raw(&with_methods, &manifest);

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
    let (_, body) = entries
        .iter()
        .find(|(name, _)| name == unlisted_path)
        .unwrap_or_else(|| panic!("the unlisted texture entry was dropped"));
    assert_eq!(body, &unlisted_body);
    assert_eq!(
        manifest_entry(&manifest, unlisted_path)["foreign"],
        Value::Bool(true)
    );
    // The carried entry is opaque and records no stored-payload digest.
    assert_eq!(
        manifest_entry(&manifest, unlisted_path)["codec"],
        Value::String("opaque".to_string())
    );
    assert_eq!(
        manifest_entry(&manifest, unlisted_path)["sha256_c"],
        Value::Null
    );
}
