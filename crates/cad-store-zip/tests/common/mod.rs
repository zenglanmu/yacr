//! Shared fixtures for the `.ymodel` v2 contract tests.
//!
//! The container is still zip, but every entry this build models is a zip
//! **`Stored`** entry whose payload is either a zstd frame (`codec: "zstd"`), its
//! bytes verbatim (`codec: "raw"`) or a preserved foreign blob (`codec:
//! "opaque"`). These helpers mirror the writer/reader so a fixture can build a
//! valid v2 archive and inspect one:
//!
//! * [`frame`] / [`decode`] — the exact zstd path `src/codec.rs` uses;
//! * [`v2_record`] — a complete `manifest.entries[]` row with the two digests and
//!   the stored-payload CRC;
//! * [`explode`] — every entry's **logical** bytes (zstd decoded, raw/opaque
//!   verbatim) plus the manifest;
//! * [`explode_raw`] / [`assemble_raw`] — the container-level view, keeping each
//!   entry's compression method and pre-decode payload;
//! * [`assemble`] — a v2 archive from logical payloads, framing what the manifest
//!   marks `zstd` and writing everything `Stored`;
//! * [`with_manifest`] — rewrite only `manifest.json`, preserving every entry
//!   payload (used by the read/minor version fixtures).
//!
//! `#![allow(dead_code)]` is intentional: each integration test binary compiles
//! this module on its own and uses a different subset.
#![allow(dead_code)]
#![allow(clippy::type_complexity)]

use std::io::{Cursor, Read, Write};

use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use structured_zstd::decoding::{read_frame_content_size, FrameContentSize, FrameDecoder};
use structured_zstd::encoding::{CompressionContext, CompressionLevel, StreamingEncoder};

use cad_store_zip::MANIFEST_ENTRY;

/// The level the writer fixes for every entry frame (`src/codec.rs`).
const ZSTD_LEVEL: i32 = 6;

/// Lowercase hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// CRC-32/ISO-HDLC of a byte string, computed with `flate2` (the same algorithm
/// `zip` records in the central directory).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}

/// One zstd frame at the format's fixed level, with a pledged content size —
/// exactly what `src/codec.rs::FrameEncoder` writes.
pub fn frame(json: &[u8]) -> Vec<u8> {
    let mut context = CompressionContext::new(CompressionLevel::from_level(ZSTD_LEVEL));
    let mut encoder = StreamingEncoder::with_context(Vec::new(), &mut context);
    encoder
        .set_pledged_content_size(json.len() as u64)
        .expect("pledge content size");
    encoder.write_all(json).expect("compress");
    encoder.finish().expect("finish frame")
}

/// Decode one `codec: "zstd"` payload back to its logical bytes.
pub fn decode(payload: &[u8]) -> Vec<u8> {
    let size = match read_frame_content_size(payload).expect("frame header") {
        FrameContentSize::Known(size) => usize::try_from(size).expect("addressable size"),
        FrameContentSize::Unknown => panic!("the writer always pledges a content size"),
    };
    let mut decoded = vec![0u8; size];
    let written = FrameDecoder::new()
        .decode_all(payload, &mut decoded)
        .expect("decode frame");
    decoded.truncate(written);
    decoded
}

/// A complete v2 `manifest.entries[]` record.
///
/// `payload` is the **stored** bytes (the frame for `zstd`, the bytes themselves
/// for `raw`/`opaque`); `logical` is the decoded JSON when the caller knows it
/// (it is decoded from `payload` otherwise).
pub fn v2_record(path: &str, codec: &str, payload: &[u8], logical: Option<&[u8]>) -> Value {
    let logical: Vec<u8> = match logical {
        Some(logical) => logical.to_vec(),
        None => match codec {
            "zstd" => decode(payload),
            _ => payload.to_vec(),
        },
    };
    let sha256 = sha256_hex(&logical);
    let sha256_c = match codec {
        "opaque" => Value::Null,
        _ => Value::String(sha256_hex(payload)),
    };
    serde_json::json!({
        "path": path,
        "codec": codec,
        "sha256": sha256,
        "sha256_c": sha256_c,
        "len": logical.len(),
        "len_c": payload.len(),
        "crc": crc32(payload),
        "foreign": codec == "opaque",
    })
}

fn zip_options(method: CompressionMethod) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(method)
        .last_modified_time(DateTime::default())
}

fn read_manifest(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Value {
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest entry");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).expect("manifest body");
    serde_json::from_slice(&bytes).expect("manifest json")
}

/// Every entry **except** `manifest.json` as `(name, zip body, method)`, plus the
/// parsed manifest.
///
/// The bodies are what the zip reader yields (the uncompressed data); for a
/// `Stored` entry that is exactly the stored payload.
pub fn explode_raw(bytes: &[u8]) -> (Vec<(String, Vec<u8>)>, Value, Vec<CompressionMethod>) {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip archive");
    let manifest = read_manifest(&mut archive);
    let mut entries = Vec::new();
    let mut methods = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("entry");
        let name = file.name().expect("entry name").to_string();
        if name == MANIFEST_ENTRY {
            continue;
        }
        let method = file.compression();
        let mut body = Vec::new();
        file.read_to_end(&mut body).expect("entry body");
        entries.push((name, body));
        methods.push(method);
    }
    (entries, manifest, methods)
}

/// Every entry's **logical** bytes (zstd frames decoded, raw/opaque verbatim),
/// plus the parsed manifest.
pub fn explode(bytes: &[u8]) -> (Vec<(String, Vec<u8>)>, Value) {
    let (entries, manifest, _) = explode_raw(bytes);
    let records = manifest["entries"].as_array().cloned().unwrap_or_default();
    let entries = entries
        .into_iter()
        .map(|(name, payload)| {
            let codec = records
                .iter()
                .find(|record| record["path"] == name.as_str())
                .and_then(|record| record["codec"].as_str());
            let logical = if codec == Some("zstd") {
                decode(&payload)
            } else {
                payload
            };
            (name, logical)
        })
        .collect();
    (entries, manifest)
}

/// Every zip entry, `manifest.json` included, with its compression method.
pub fn entry_methods(bytes: &[u8]) -> Vec<(String, CompressionMethod)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("zip archive");
    (0..archive.len())
        .map(|index| {
            let file = archive.by_index(index).expect("entry");
            (
                file.name().expect("entry name").to_string(),
                file.compression(),
            )
        })
        .collect()
}

/// Write `entries` verbatim with the given methods, then `manifest.json` as
/// `Stored` plain JSON last.
pub fn assemble_raw(entries: &[(String, Vec<u8>, CompressionMethod)], manifest: &Value) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut sorted: Vec<&(String, Vec<u8>, CompressionMethod)> = entries.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, body, method) in sorted {
        writer
            .start_file(name.as_str(), zip_options(*method))
            .expect("start entry");
        writer.write_all(body).expect("entry body");
    }
    writer
        .start_file(MANIFEST_ENTRY, zip_options(CompressionMethod::Stored))
        .expect("start manifest");
    serde_json::to_writer(&mut writer, manifest).expect("manifest body");
    writer.finish().expect("finish").into_inner()
}

/// Build a valid v2 archive from **logical** payloads.
///
/// A path the manifest marks `codec: "zstd"` is framed; every other path
/// (including an unlisted foreign entry) is written verbatim. Every entry is
/// `Stored`, so the records `v2_record` produced stay true.
pub fn assemble(entries: &[(String, Vec<u8>)], manifest: &Value) -> Vec<u8> {
    let records = manifest["entries"].as_array().cloned().unwrap_or_default();
    let framed: Vec<(String, Vec<u8>, CompressionMethod)> = entries
        .iter()
        .map(|(name, logical)| {
            let codec = records
                .iter()
                .find(|record| record["path"] == name.as_str())
                .and_then(|record| record["codec"].as_str());
            let payload = if codec == Some("zstd") {
                frame(logical)
            } else {
                logical.clone()
            };
            (name.clone(), payload, CompressionMethod::Stored)
        })
        .collect();
    assemble_raw(&framed, manifest)
}

/// Rewrite only `manifest.json`, preserving every entry's payload and method.
pub fn with_manifest(bytes: &[u8], mutate: impl FnOnce(&mut Value)) -> Vec<u8> {
    let (entries, mut manifest, methods) = explode_raw(bytes);
    mutate(&mut manifest);
    let with_methods: Vec<(String, Vec<u8>, CompressionMethod)> = entries
        .into_iter()
        .zip(methods)
        .map(|((name, body), method)| (name, body, method))
        .collect();
    assemble_raw(&with_methods, &manifest)
}
