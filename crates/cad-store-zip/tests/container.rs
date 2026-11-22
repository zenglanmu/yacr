//! Container-level contract tests for the `.ymodel` v2 entry codec.
//!
//! These assert the bytes on disk, not only that a snapshot round-trips:
//! every entry this build writes is a zip `Stored` entry, a modelled JSON payload
//! is one zstd frame, `manifest.json` is plain JSON, and each integrity gate in
//! `src/read.rs` refuses the archive it is meant to refuse.
//!
//! The fixture is the smallest document that still exercises the modelled entry
//! set (`document.json`, `schema/classes.json`, `tables/models.json`); the
//! hand-built archives exist only for failures no writer can produce.

mod common;

use std::io::{Cursor, Read, Write};

use serde_json::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use cad_db::{
    ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, StoreError,
    DOCUMENT_ENTRY,
};
use cad_domain::ymodel::{markers, DocumentHeader, IdCounters};
use cad_domain::{DatabaseId, DocumentIdentity, Revision, UnitContext};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

use common::{assemble_raw, crc32, entry_methods, explode_raw, frame, sha256_hex, with_manifest};

/// `MAX_ENTRY_BYTES` in `src/read.rs`: 1 GiB.
const MAX_ENTRY_BYTES: u64 = 1 << 30;

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(3),
        units: UnitContext::drawing_units(),
        identity: DocumentIdentity::Temporary(1),
        resource_keys: Vec::new(),
        embed_resources: false,
        variables: serde_json::json!({}),
        next: IdCounters::default(),
    }
}

fn snapshot() -> DocumentSnapshot {
    DocumentSnapshot {
        header: header(),
        revision: Revision(2),
        models: Vec::new(),
        shards: Vec::new(),
        tables: Default::default(),
        textures: Default::default(),
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    }
}

fn save(snapshot: &DocumentSnapshot) -> Vec<u8> {
    let mut sink = MemorySink::new();
    ZipDocumentStore::new()
        .write(snapshot, &ChangeScope::default(), &mut sink)
        .expect("write");
    sink.committed_bytes().expect("committed").to_vec()
}

fn read_archive(bytes: &[u8]) -> Result<cad_db::LoadedDocument, StoreError> {
    let mut source = MemorySource::new(bytes.to_vec());
    ZipDocumentStore::new().read(&mut source)
}

/// The exact zstd magic the format's frames start with.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

fn is_zstd_frame(payload: &[u8]) -> bool {
    payload.starts_with(&ZSTD_MAGIC)
}

fn deflated_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
}

/// Replace one modelled entry's **stored payload** and refresh the record's
/// stored-payload fields (`sha256_c`, `len_c`, `crc`), leaving the logical
/// `sha256`/`len`/`codec` alone.
///
/// This is how a corrupt frame is injected while every structural check before
/// the decode still passes.
fn replace_stored(base: &[u8], path: &str, payload: Vec<u8>) -> Vec<u8> {
    let (entries, mut manifest, methods) = explode_raw(base);
    let entries: Vec<(String, Vec<u8>, CompressionMethod)> = entries
        .into_iter()
        .zip(methods)
        .map(|((name, body), method)| {
            let body = if name == path { payload.clone() } else { body };
            (name, body, method)
        })
        .collect();
    for record in manifest["entries"].as_array_mut().expect("entries") {
        if record["path"] == path {
            record["sha256_c"] = Value::String(sha256_hex(&payload));
            record["len_c"] = Value::from(payload.len() as u64);
            record["crc"] = Value::from(u64::from(crc32(&payload)));
        }
    }
    assemble_raw(&entries, &manifest)
}

fn record_mut<'a>(manifest: &'a mut Value, path: &str) -> &'a mut Value {
    manifest["entries"]
        .as_array_mut()
        .expect("entries")
        .iter_mut()
        .find(|record| record["path"] == path)
        .unwrap_or_else(|| panic!("manifest has no record for {path}"))
}

#[test]
fn every_written_entry_is_stored_and_json_payloads_are_zstd_frames() {
    let bytes = save(&snapshot());

    for (name, method) in entry_methods(&bytes) {
        assert_eq!(
            method,
            CompressionMethod::Stored,
            "{name} must be a zip Stored entry"
        );
    }

    let (entries, _manifest, _methods) = explode_raw(&bytes);
    assert!(!entries.is_empty());
    let mut saw_json = false;
    for (name, payload) in &entries {
        if name.ends_with(".json") {
            assert!(
                is_zstd_frame(payload),
                "{name} must be one zstd frame, not {:02x?}",
                &payload[..payload.len().min(4)]
            );
            saw_json = true;
        }
    }
    assert!(saw_json, "the fixture must contain a modelled JSON entry");
}

#[test]
fn manifest_is_stored_plain_json() {
    let bytes = save(&snapshot());
    let methods = entry_methods(&bytes);
    assert!(
        methods
            .iter()
            .any(|(name, method)| name == MANIFEST_ENTRY && *method == CompressionMethod::Stored),
        "manifest.json must be a Stored entry: {methods:?}"
    );

    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip");
    let mut body = Vec::new();
    archive
        .by_name(MANIFEST_ENTRY)
        .expect("manifest")
        .read_to_end(&mut body)
        .expect("manifest body");
    serde_json::from_slice::<Value>(&body).expect("manifest is plain JSON");
    assert_eq!(body.first(), Some(&b'{'), "manifest starts with plain JSON");
    assert!(!is_zstd_frame(&body), "manifest.json is not a frame");
}

#[test]
fn a_wrong_stored_digest_is_refused_before_decoding() {
    let bytes = save(&snapshot());
    // The payload is untouched and its logical digest is still right; only the
    // stored-payload digest lies. The check must run before the frame is decoded.
    let archive = with_manifest(&bytes, |manifest| {
        record_mut(manifest, DOCUMENT_ENTRY)["sha256_c"] = Value::String("f".repeat(64));
    });
    let error = read_archive(&archive).expect_err("the stored digest is wrong");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(
        error.to_string().contains("stored payload"),
        "the pre-decompression digest check must be what refused it: {error}"
    );
}

#[test]
fn a_stored_payload_that_is_not_a_frame_is_refused() {
    let bytes = save(&snapshot());
    let archive = replace_stored(
        &bytes,
        DOCUMENT_ENTRY,
        b"definitely not a zstd frame".to_vec(),
    );
    let error = read_archive(&archive).expect_err("not a frame");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn a_frame_that_declares_a_different_size_is_refused() {
    let bytes = save(&snapshot());
    // The frame is well formed and hashes correctly, but it declares 5 bytes while
    // manifest.json still records the real logical length: the header check must
    // refuse it before decoding.
    let archive = replace_stored(&bytes, DOCUMENT_ENTRY, frame(b"short"));
    let error = read_archive(&archive).expect_err("pledged size mismatch");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(
        error.to_string().contains("declares"),
        "the frame header, not a later hash, must be what refused it: {error}"
    );
}

#[test]
fn a_truncated_frame_is_refused() {
    let bytes = save(&snapshot());
    let (entries, _manifest, _methods) = explode_raw(&bytes);
    // The document entry's Stored payload is its zstd frame; cut the frame short.
    let mut payload = entries
        .iter()
        .find(|(name, _)| name == DOCUMENT_ENTRY)
        .map(|(_, body)| body.clone())
        .expect("document.json");
    payload.truncate(payload.len() - 4);
    let archive = replace_stored(&bytes, DOCUMENT_ENTRY, payload);
    let error = read_archive(&archive).expect_err("truncated frame");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
}

#[test]
fn a_modelled_entry_written_deflated_is_refused() {
    let bytes = save(&snapshot());
    let (entries, manifest, mut methods) = explode_raw(&bytes);
    let index = entries
        .iter()
        .position(|(name, _)| name == DOCUMENT_ENTRY)
        .expect("document.json");
    methods[index] = CompressionMethod::Deflated;
    let entries: Vec<(String, Vec<u8>, CompressionMethod)> = entries
        .into_iter()
        .zip(methods)
        .map(|((name, body), method)| (name, body, method))
        .collect();

    // First write the Deflated container, then record the compressed size the zip
    // central directory actually holds. Every earlier check must pass so the
    // refusal is specifically "a modelled entry must be Stored".
    let probe = assemble_raw(&entries, &manifest);
    let (crc, compressed_size) = {
        let mut archive = ZipArchive::new(Cursor::new(probe)).expect("zip");
        let file = archive.by_name(DOCUMENT_ENTRY).expect("document.json");
        (file.crc32(), file.compressed_size())
    };
    let mut manifest = manifest;
    {
        let record = record_mut(&mut manifest, DOCUMENT_ENTRY);
        record["crc"] = Value::from(u64::from(crc));
        record["len_c"] = Value::from(compressed_size);
    }
    let archive = assemble_raw(&entries, &manifest);
    let error = read_archive(&archive).expect_err("Deflated modelled entry");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(
        error.to_string().contains("Stored"),
        "the compression method, not a hash, must be what refused it: {error}"
    );
}

#[test]
fn a_v1_archive_is_refused_as_an_unsupported_version() {
    // A version-1 archive: Deflated JSON entries and `format.read: 1`. The gate
    // runs on the manifest before any entry is decoded, so this is
    // UNSUPPORTED_VERSION — not the INVALID_DOCUMENT a decode attempt would raise.
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(DOCUMENT_ENTRY, deflated_options())
        .expect("document start");
    // Deliberately not a zstd frame: a late gate would fail decoding this.
    writer
        .write_all(br#"{"not":"a v2 frame"}"#)
        .expect("document body");
    writer
        .start_file("schema/classes.json", deflated_options())
        .expect("classes start");
    writer.write_all(b"[]").expect("classes body");
    let manifest = serde_json::json!({
        "format": {"read": 1, "write": 1, "minor": 4},
    });
    writer
        .start_file(MANIFEST_ENTRY, deflated_options())
        .expect("manifest start");
    serde_json::to_writer(&mut writer, &manifest).expect("manifest body");
    let bytes = writer.finish().expect("finish").into_inner();

    let error = read_archive(&bytes).expect_err("a v1 archive is refused");
    assert_eq!(
        error.marker,
        markers::UNSUPPORTED_VERSION,
        "the read gate must run before any entry decode: {error}"
    );
}

#[test]
fn an_entry_over_the_entry_limit_is_refused() {
    let bytes = save(&snapshot());
    // The manifest declares a logical length over MAX_ENTRY_BYTES; the payload is
    // untouched, so only the pre-allocation cap can refuse this.
    let archive = with_manifest(&bytes, |manifest| {
        record_mut(manifest, DOCUMENT_ENTRY)["len"] = Value::from(MAX_ENTRY_BYTES + 1);
    });
    let error = read_archive(&archive).expect_err("over the entry limit");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(
        error.to_string().contains("entry limit"),
        "the size cap, not a hash, must be what refused it: {error}"
    );
}
