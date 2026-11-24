//! Encoding a [`cad_db::DocumentSnapshot`] into `.ymodel` bytes (ADR-0037 D11).
//!
//! The save is **incremental at the entry level**: an element shard the commit did
//! not touch (`dirty.shards ∪ dirty.prev_shards`, Phase 2) is copied from the
//! previous archive at the compressed-byte level with
//! [`zip::ZipWriter::raw_copy_file`], while a shard the commit touched is
//! re-encoded from the snapshot. An entry that is no longer live simply is not
//! written and cannot reappear on the next open. The other entries carried over
//! are the **foreign** ones, also raw-copied — never parsed, never re-compressed,
//! never dropped.
//!
//! Everything else is small and always re-encoded: `document.json`,
//! `schema/classes.json`, `tables/models.json` and every `tables/*.json` row set.
//! Raw-copying a modelled entry needs the previous archive's manifest to describe
//! it; a first save has no previous archive, so every entry is encoded then.
//!
//! # Entry payloads
//!
//! Every entry this save encodes is a `Stored` zip entry (a copied foreign entry
//! keeps its original compression method). A modelled JSON entry's payload is one
//! zstd frame ([`crate::codec`]) whose header pledges the logical JSON length; a
//! texture or resource entry's payload is its bytes verbatim. The manifest records
//! which (`codec`) plus the logical and stored lengths and digests. Because the
//! frame is a buffer, it is hashed (`sha256_c`) before it is written, and a
//! raw-copied entry inherits the source record verbatim — the copied payload is
//! unchanged.
//!
//! The write order is the format's: every entry in sorted path order, then
//! `manifest.json` **last** (its entry table can only be built once every other
//! entry has been hashed), then the zip central directory via `finish`.
//!
//! # Streaming, and why `begin` gets no size hint
//!
//! Entries are fed to the sink one at a time through [`SinkWriter`]: at most one
//! entry's JSON and its compressed frame exist as buffers, so a 200 MB document
//! does not need a 400 MB spike. The consequence is that the archive's total size
//! is genuinely unknown
//! when `sink.begin` runs — a hint would be a guess, and a wrong hint is worse
//! than `None` (a carrier could reserve the wrong amount or refuse a save that
//! would have fit). The budget is therefore enforced per chunk, which is what
//! ADR-0037 D16 asks for anyway.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};

use serde_json::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use cad_db::{
    table_entry, ByteSink, ChangeScope, DocumentSnapshot, ModelRecord, SaveReport, ShardSnapshot,
    StoreError, CLASSES_ENTRY, DOCUMENT_ENTRY, MODELS_ENTRY,
};
use cad_domain::ymodel::{markers, DocumentHeader, ModelKey, Shard};
use cad_domain::{BundledKind, DocumentIdentity};

use crate::codec::FrameEncoder;
use crate::crc32::crc32;
use crate::json::{hex_lower, malformed, sha256_hex, Sha256Sum};
use crate::manifest::{Codec, EntryRecord, Manifest};
use crate::models::{models_value, storable_model_id};
use crate::previous::{self, PreviousArchive};
use crate::resources::{self, check_write_key_matches_digest};
use crate::schema::classes_value;
use crate::sink::{io_error, zip_error, SinkWriter, WriteMetrics};
use crate::MANIFEST_ENTRY;

/// The `open_revision` written when the store was never told the import baseline.
///
/// `u64::MAX` never equals a real revision, so the L0 passthrough predicate
/// (`revision == manifest.open_revision`, ADR-0037 D2) stays false. That is the
/// safe direction: the alternative — defaulting to the snapshot's own revision —
/// would make an edited document claim to still be its source file.
pub(crate) const NO_IMPORT_BASELINE: u64 = u64::MAX;

/// Raw model id (decimal) → the shard entries of that model, sorted.
type ShardMap = BTreeMap<String, Vec<String>>;

/// What one archive entry is encoded from.
enum Body<'a> {
    Header(&'a DocumentHeader),
    Classes,
    Models(&'a [ModelRecord]),
    Table(&'a [Value]),
    Shard(&'a ShardSnapshot),
    /// A document texture's raw bytes (ADR-0038 D4).
    ///
    /// Written verbatim: PNG/JPEG are already compressed, so a second deflate pass
    /// only costs CPU and blocks, and the format's `len`/`sha256` describe exactly
    /// the image the importer registered. Streamed, never buffered, and never
    /// raw-copied (ADR-0040 D14).
    Binary(&'a [u8]),
    /// A bundled external resource's raw bytes (`resources/<kind>/<key>`,
    /// ADR-0040 D1).
    ///
    /// The key travels with the bytes because the write path asserts that the key
    /// names this payload (`sha256(bytes)[..32]`). Streamed like a texture, but
    /// eligible for the raw copy when the previous archive holds it unchanged.
    Resource {
        kind: BundledKind,
        key: &'a str,
        bytes: &'a [u8],
    },
}

impl Body<'_> {
    /// The entry's uncompressed JSON bytes.
    ///
    /// Binary bodies are *streamed* by the writer (ADR-0040 D15), so they have no
    /// buffer to return; reaching here with one is an internal invariant failure,
    /// not a document error.
    fn encode(&self) -> Result<Vec<u8>, StoreError> {
        // `serde_json::to_vec` per arm rather than a generic helper: the crate has
        // no direct `serde` dependency (the workspace's `serde_json` is enough),
        // so a `T: Serialize` bound cannot be written without adding one.
        match self {
            Body::Header(header) => {
                serde_json::to_vec(&header.to_json_value()).map_err(encode_error)
            }
            Body::Classes => serde_json::to_vec(&classes_value()).map_err(encode_error),
            Body::Models(models) => serde_json::to_vec(&models_value(models)).map_err(encode_error),
            Body::Table(rows) => serde_json::to_vec(rows).map_err(encode_error),
            Body::Shard(shard) => {
                // Sorted by id: the format contract for
                // `elements/<model>-<shard>.json`, independent of the order the
                // database collected the rows in.
                let mut rows: Vec<&cad_db::ElementSnapshot> = shard.elements.iter().collect();
                rows.sort_by_key(|element| element.id.0);
                let payloads: Vec<&Value> = rows.iter().map(|element| &element.payload).collect();
                serde_json::to_vec(&payloads).map_err(encode_error)
            }
            Body::Binary(_) | Body::Resource { .. } => Err(malformed(
                "entry",
                "a binary entry is streamed to the zip, not buffered",
            )),
        }
    }
}

fn encode_error(error: serde_json::Error) -> StoreError {
    malformed("entry", error)
}

/// Where a planned entry's bytes come from when the previous archive holds them.
///
/// A `Shard` is copyable only when the commit left it untouched (`dirty`); a
/// `Resource` is content-addressed, so "the key did not change" already means "the
/// bytes did not change" and no dirty scope is needed (ADR-0040 D7/D8).
pub(crate) enum CopySource {
    Shard(ModelKey, Shard),
    Resource(BundledKind, String),
}

/// One entry the save will write: re-encoded, or copied from the previous
/// archive.
struct Planned<'a> {
    path: String,
    body: Option<Body<'a>>,
    /// The previous-archive entry a raw copy may take from, when one is eligible.
    ///
    /// `None` means "always re-encoded". A texture is deliberately `None`: its
    /// bytes are already in memory and ADR-0040 D14 keeps `textures/<key>` out of
    /// the raw copy.
    copy: Option<CopySource>,
}

type ZipSink<'a> = ZipWriter<zip::write::StreamWriter<SinkWriter<'a>>>;

/// Encode `snapshot` and hand the bytes to `sink`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode(
    max_bytes: Option<u64>,
    previous_bytes: Option<&[u8]>,
    dirty: &ChangeScope,
    open_revision: Option<u64>,
    document_uuid: Option<&str>,
    saved_at: Option<&str>,
    snapshot: &DocumentSnapshot,
    sink: &mut dyn ByteSink,
) -> Result<SaveReport, StoreError> {
    let (mut planned, shard_map) = plan_entries(snapshot)?;

    let previous = match previous_bytes {
        Some(bytes) => Some(PreviousArchive::scan(bytes)?),
        None => None,
    };

    // One map for every raw copy, whether it is a shard this commit did not touch
    // or a foreign entry. A copied entry's `body` is `None`; the record is what the
    // next manifest lists.
    let mut copy_records = BTreeMap::new();

    // Shards the commit did not touch, and resources the previous archive already
    // holds, keep their compressed bytes. The source manifest must describe the
    // entry as modelled (`!foreign`); otherwise it is re-encoded rather than
    // trusted.
    if let Some(previous) = previous.as_ref() {
        for entry in &mut planned {
            let Some(source) = entry.copy.as_ref() else {
                continue;
            };
            match source {
                CopySource::Shard(model, shard) => {
                    if !copyable_shard(previous, dirty, &entry.path, *model, *shard) {
                        continue;
                    }
                }
                // D8: a resource is copyable exactly when the previous manifest
                // recorded this path as a modelled entry. The planned path, the
                // copy source and the body must all name the same `(kind, key)`;
                // a planner that split them would copy a blob under a name that
                // does not describe it.
                CopySource::Resource(kind, key) => {
                    let same_resource = previous.record(&entry.path).is_some()
                        && matches!(
                            &entry.body,
                            Some(Body::Resource { kind: body_kind, key: body_key, .. })
                                if body_kind == kind && *body_key == key.as_str()
                        );
                    if !same_resource {
                        continue;
                    }
                }
            }
            let source = previous
                .record(&entry.path)
                .expect("a copyable entry has a source record");
            // Inherited verbatim: the source record already describes the very
            // bytes `raw_copy_file` carries over (including a true `sha256_c` for
            // a stored payload this build wrote). Re-deriving it would either
            // recompute the same values or — for a field the source measured
            // differently — make the copy unverifiable.
            copy_records.insert(entry.path.clone(), source.clone());
            entry.body = None;
        }
    }

    // Foreign entries join the plan at the compressed-byte level. A collision with
    // a live entry is an explicit failure, never an overwrite (D11). The one
    // exception is a content-addressed resource path: identical bytes are the same
    // asset, so the modelled entry takes over (ADR-0040 D8).
    let mut foreign_paths = Vec::new();
    if let (Some(previous), Some(bytes)) = (previous.as_ref(), previous_bytes) {
        let live: BTreeSet<&str> = planned.iter().map(|entry| entry.path.as_str()).collect();
        for path in previous.foreign() {
            if path == MANIFEST_ENTRY {
                continue;
            }
            if live.contains(path.as_str()) {
                // A content-addressed resource path can legitimately reappear
                // (ADR-0040 D8): identical bytes are the same asset, so the
                // modelled entry takes over; different bytes are a real conflict,
                // never an overwrite. Everything else is a plain collision.
                let new_bytes = planned
                    .iter()
                    .find(|entry| entry.path == path.as_str())
                    .and_then(|entry| match entry.body {
                        Some(Body::Resource {
                            bytes: planned_bytes,
                            ..
                        }) => Some(planned_bytes),
                        _ => None,
                    });
                if let Some(new_bytes) = new_bytes {
                    if resources::foreign_has_same_content(bytes, path, new_bytes)? {
                        continue;
                    }
                    return Err(resources::conflict(path));
                }
                return Err(previous::conflict(path));
            }
            let record = match previous.record_for_copy(path) {
                // Copied verbatim: an unmodelled entry's record already describes
                // the bytes raw-copied below, and it was `foreign: true` in the
                // source manifest (that is why it is in `foreign()` at all).
                Some(record) => record.clone(),
                // Not described by the source manifest: measure it from the bytes.
                None => previous::measure(bytes, path)?,
            };
            copy_records.insert(path.clone(), record);
            foreign_paths.push(path.clone());
        }
    }
    for path in foreign_paths {
        planned.push(Planned {
            path,
            body: None,
            copy: None,
        });
    }
    planned.sort_by(|left, right| left.path.cmp(&right.path));

    sink.begin(None)?;
    let metrics = RefCell::new(WriteMetrics {
        written: 0,
        failure: None,
    });
    let outcome = write_entries(
        sink,
        &metrics,
        max_bytes,
        &planned,
        &copy_records,
        previous_bytes,
        snapshot,
        open_revision,
        document_uuid,
        saved_at,
        shard_map,
    );
    match outcome {
        Ok((bytes_written, entries_written, entries_copied)) => {
            if let Err(error) = sink.commit() {
                sink.abort();
                return Err(error);
            }
            Ok(SaveReport {
                saved_revision: snapshot.revision,
                bytes_written,
                entries_written,
                entries_copied,
                // The sink does not expose the committed bytes, so the archive hash
                // is not knowable here; a carrier that needs it hashes the file.
                sha256: None,
            })
        }
        Err(error) => {
            sink.abort();
            Err(error)
        }
    }
}

/// The live entry set, in sorted path order, plus the shard map it implies.
fn plan_entries(snapshot: &DocumentSnapshot) -> Result<(Vec<Planned<'_>>, ShardMap), StoreError> {
    let mut planned = Vec::new();
    planned.push(Planned {
        path: DOCUMENT_ENTRY.to_string(),
        body: Some(Body::Header(&snapshot.header)),
        copy: None,
    });
    planned.push(Planned {
        path: CLASSES_ENTRY.to_string(),
        body: Some(Body::Classes),
        copy: None,
    });
    // A model key the format cannot represent is refused before anything is
    // written. This is defense in depth for the sequence-0 reservation
    // (`ModelKey::from_persistent`): the allocators never issue it, so only a
    // malformed snapshot reaches here, and it must fail the save rather than
    // produce a `tables/models.json` row no reader can decode.
    for model in &snapshot.models {
        if storable_model_id(model.key).is_none() {
            return Err(unrepresentable_model(MODELS_ENTRY, model.key));
        }
    }
    planned.push(Planned {
        path: MODELS_ENTRY.to_string(),
        body: Some(Body::Models(&snapshot.models)),
        copy: None,
    });
    for (kind, rows) in &snapshot.tables.rows {
        planned.push(Planned {
            path: table_entry(*kind).to_string(),
            body: Some(Body::Table(rows)),
            copy: None,
        });
    }
    // Document textures (ADR-0038 D4/D5). The snapshot is the authority: exactly
    // one entry per texture it holds, and never one it does not — an orphan image
    // must not appear in the archive. Texture entries are never raw-copied (their
    // bytes are already in memory), so `copy: None` also means "always re-encoded".
    for (key, bytes) in &snapshot.textures {
        planned.push(Planned {
            path: texture_entry(key)?,
            body: Some(Body::Binary(&bytes[..])),
            copy: None,
        });
    }
    // Bundled resources (ADR-0040 D1/D4/D5). Content-addressed, one entry per
    // snapshot resource, and again the snapshot is the authority: a key it does
    // not hold is never written, so a dropped resource leaves no orphan entry.
    for resource in resources::plan(snapshot)? {
        planned.push(Planned {
            path: resource.path,
            body: Some(Body::Resource {
                kind: resource.kind,
                key: resource.key,
                bytes: resource.bytes,
            }),
            copy: Some(CopySource::Resource(
                resource.kind,
                resource.key.to_string(),
            )),
        });
    }

    let mut shard_map = ShardMap::new();
    let mut seen = BTreeSet::new();
    for shard in &snapshot.shards {
        // Same reservation check as the model list above, applied where the
        // entry path is built: `elements/<model>-<shard>.json` is only readable
        // if the model id round-trips back to this key.
        let Some(id) = storable_model_id(shard.model) else {
            return Err(unrepresentable_model(
                &format!("elements/{:?}-{}.json", shard.model, shard.shard.0),
                shard.model,
            ));
        };
        let path = shard_entry(id.raw(), shard.shard);
        if !seen.insert(path.clone()) {
            return Err(malformed(
                "elements",
                format!("two shards encode to the same entry {path:?}"),
            ));
        }
        shard_map
            .entry(id.raw().to_string())
            .or_default()
            .push(path.clone());
        planned.push(Planned {
            path,
            body: Some(Body::Shard(shard)),
            copy: Some(CopySource::Shard(shard.model, shard.shard)),
        });
    }
    for paths in shard_map.values_mut() {
        paths.sort();
    }
    Ok((planned, shard_map))
}

/// A refused [`ModelKey`]: the entry it would have named, and the key itself.
///
/// Stable marker [`markers::UNREPRESENTABLE_MODEL`] — never `invalid_document`,
/// which describes a malformed *archive*; here the snapshot is malformed and the
/// save is refused before an unreadable archive can be produced.
fn unrepresentable_model(path: &str, model: ModelKey) -> StoreError {
    StoreError::new(
        markers::UNREPRESENTABLE_MODEL,
        format!("{path} names {model:?}, which has no model id this format can read back"),
    )
}

/// Whether a shard entry may be copied from the previous archive instead of
/// re-encoded.
///
/// Every condition is required: the source manifest must list the path as a shard,
/// describe it as a modelled entry, and the commit must not have touched either
/// side of the row (a move dirties the old shard through `prev_shards`).
fn copyable_shard(
    previous: &PreviousArchive,
    dirty: &ChangeScope,
    path: &str,
    model: ModelKey,
    shard: Shard,
) -> bool {
    previous.lists_shard(path)
        && previous.record(path).is_some_and(|record| !record.foreign)
        && !dirty.shards.contains(&(model, shard))
        && !dirty.prev_shards.contains(&(model, shard))
}

/// `elements/<model raw>-<shard>.json`.
fn shard_entry(model_raw: i64, shard: cad_domain::ymodel::Shard) -> String {
    format!("elements/{model_raw}-{}.json", shard.0)
}

/// The archive path of one document texture: `textures/<key>` (ADR-0038 D4).
///
/// The key is re-validated here rather than trusted: it becomes a path inside an
/// untrusted container, so a `/`, a `..` or an uppercase spelling would either
/// escape `textures/` or name an entry no reader could resolve. Refusing it at
/// write time means no archive this build produces can carry such a name.
fn texture_entry(key: &str) -> Result<String, StoreError> {
    if !cad_domain::is_valid_texture_key(key) {
        return Err(malformed(
            "textures",
            format!("{key:?} is not a valid texture key (expected <32 lowercase hex>.png|.jpg)"),
        ));
    }
    Ok(format!("textures/{key}"))
}

#[allow(clippy::too_many_arguments)]
fn write_entries(
    sink: &mut dyn ByteSink,
    metrics: &RefCell<WriteMetrics>,
    max_bytes: Option<u64>,
    planned: &[Planned<'_>],
    copy_records: &BTreeMap<String, EntryRecord>,
    previous_bytes: Option<&[u8]>,
    snapshot: &DocumentSnapshot,
    open_revision: Option<u64>,
    document_uuid: Option<&str>,
    saved_at: Option<&str>,
    shard_map: ShardMap,
) -> Result<(u64, usize, usize), StoreError> {
    let writer = SinkWriter {
        sink,
        metrics,
        budget: max_bytes,
        poisoned: false,
    };
    let mut zip = ZipWriter::new_stream(writer);
    // One compression context for the whole save: the match-finder tables are
    // built once, and every frame is still independent (each pledges its own
    // content size and is verified on its own).
    let mut encoder = FrameEncoder::new();

    let mut records: Vec<EntryRecord> = Vec::with_capacity(planned.len() + 1);
    let mut entries_written = 0usize;
    let mut entries_copied = 0usize;
    let mut previous_archive: Option<ZipArchive<Cursor<&[u8]>>> = None;

    for entry in planned {
        match &entry.body {
            // Binary payloads are measured and streamed, never buffered
            // (ADR-0040 D15). A resource key is asserted against the measured
            // digest before the entry is started, so a wrong key cannot leave a
            // partial entry behind.
            Some(Body::Binary(bytes)) => {
                let record = write_binary(&mut zip, metrics, &entry.path, bytes, None)?;
                records.push(record);
                entries_written += 1;
            }
            Some(Body::Resource { kind, key, bytes }) => {
                let record =
                    write_binary(&mut zip, metrics, &entry.path, bytes, Some((*kind, key)))?;
                records.push(record);
                entries_written += 1;
            }
            Some(body) => {
                // The JSON is already a buffer (`Body::encode`), so the frame is
                // compressed from memory and can pledge its exact logical size.
                let json = body.encode()?;
                let frame = encoder.frame(&json)?;
                let record = EntryRecord {
                    path: entry.path.clone(),
                    codec: Codec::Zstd,
                    sha256: sha256_hex(&json),
                    sha256_c: Some(sha256_hex(&frame)),
                    len: json.len() as u64,
                    len_c: frame.len() as u64,
                    // The CRC is of the STORED payload, matching what the zip
                    // central directory records for this entry.
                    crc: crc32(&frame),
                    foreign: false,
                };
                // Drop the uncompressed JSON before the frame reaches the sink:
                // the peak is one entry's `json + frame`, not the whole archive's.
                drop(json);
                start_entry(&mut zip, metrics, &entry.path)?;
                // Chunked so `SinkWriter`'s per-chunk budget check still aborts
                // mid-entry instead of after the whole frame is handed over.
                for chunk in frame.chunks(resources::BINARY_CHUNK) {
                    zip.write_all(chunk)
                        .map_err(|error| io_error(metrics, error, &entry.path))?;
                }
                records.push(record);
                entries_written += 1;
            }
            None => {
                let record = copy_records.get(&entry.path).ok_or_else(|| {
                    malformed(
                        "copied entry",
                        format!(
                            "{:?} was planned as a copied entry without a record",
                            entry.path
                        ),
                    )
                })?;
                let bytes = previous_bytes.ok_or_else(|| {
                    previous::unreadable(format!("{:?} needs a previous archive", entry.path))
                })?;
                if previous_archive.is_none() {
                    previous_archive = Some(previous::open(bytes)?);
                }
                let archive = previous_archive.as_mut().expect("opened just above");
                let file = archive
                    .by_name(&entry.path)
                    .map_err(|error| zip_error(metrics, error, &entry.path))?;
                zip.raw_copy_file(file)
                    .map_err(|error| zip_error(metrics, error, &entry.path))?;
                records.push(record.clone());
                entries_copied += 1;
            }
        }
    }

    let manifest = Manifest {
        format: cad_db::FormatVersion {
            read: cad_domain::ymodel::FORMAT_READ,
            write: cad_domain::ymodel::FORMAT_WRITE,
            minor: crate::minor::document_minor(snapshot),
        },
        generator: crate::GENERATOR.to_string(),
        document_uuid: document_uuid
            .map(str::to_string)
            .unwrap_or_else(|| derive_document_uuid(&snapshot.header)),
        open_revision: open_revision.unwrap_or(NO_IMPORT_BASELINE),
        saved_revision: snapshot.revision.0,
        saved_at: saved_at.map(str::to_string),
        shard_map,
        entries: records,
    };
    let bytes = serde_json::to_vec(&manifest.to_json_value()).map_err(encode_error)?;
    start_entry(&mut zip, metrics, MANIFEST_ENTRY)?;
    zip.write_all(&bytes)
        .map_err(|error| io_error(metrics, error, MANIFEST_ENTRY))?;
    zip.finish()
        .map_err(|error| zip_error(metrics, error, "archive"))?;

    let written = metrics.borrow().written;
    Ok((written, entries_written, entries_copied))
}

/// Stream one opaque entry straight into the zip, measuring it on the way.
///
/// No buffer is built: a 64 MiB resource is hashed, CRC'd and written in the
/// chunk sequence it already lives in (ADR-0040 D15). `expected_key` is the
/// resource key the entry must name; it is checked **before** the entry is
/// started so a mismatched key cannot leave a partial entry in the archive.
fn write_binary(
    zip: &mut ZipSink<'_>,
    metrics: &RefCell<WriteMetrics>,
    path: &str,
    bytes: &[u8],
    expected_key: Option<(BundledKind, &str)>,
) -> Result<EntryRecord, StoreError> {
    let (sha256, len, crc) = resources::measure(bytes);
    if let Some((kind, key)) = expected_key {
        check_write_key_matches_digest(kind, key, &sha256, path)?;
    }
    start_entry(zip, metrics, path)?;
    for chunk in bytes.chunks(resources::BINARY_CHUNK) {
        zip.write_all(chunk)
            .map_err(|error| io_error(metrics, error, path))?;
    }
    Ok(EntryRecord {
        path: path.to_string(),
        codec: Codec::Raw,
        // The stored payload is the logical payload, so both digests are the
        // same value and both lengths are the same number.
        sha256_c: Some(sha256.clone()),
        sha256,
        len,
        len_c: len,
        crc,
        foreign: false,
    })
}

/// The options every entry is written with.
///
/// `Stored`, never deflated: the payload of a JSON entry is already a zstd frame
/// (the format's own codec, see [`crate::codec`]) and a binary payload is already
/// compressed. Deflating either again would spend CPU to produce a different
/// compressed byte string for the same logical bytes — and `Stored` is what makes
/// the manifest's `len_c`/`sha256_c` describe exactly the bytes the zip holds.
/// The timestamp is fixed so two saves stay byte-deterministic.
pub(crate) fn entry_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
}

fn start_entry(
    zip: &mut ZipSink<'_>,
    metrics: &RefCell<WriteMetrics>,
    path: &str,
) -> Result<(), StoreError> {
    zip.start_file(path, entry_options())
        .map_err(|error| zip_error(metrics, error, path))
}

/// A UUID-shaped, deterministic document identifier derived from the header's
/// identity (ADR-0037 D5: one identity per document, stable across saves).
///
/// The version nibble is `8` (RFC 4122 "custom"), not `4`: this is not a random
/// UUID and must not pretend to be one. Save-As mints a new identity anyway, and
/// a carrier can pin the value with
/// [`crate::ZipDocumentStore::with_document_uuid`].
fn derive_document_uuid(header: &DocumentHeader) -> String {
    let mut hasher = Sha256Sum::new();
    hasher.update(b"yacr.ymodel.document_uuid.v1\0");
    match &header.identity {
        DocumentIdentity::Temporary(serial) => {
            hasher.update(b"temporary:");
            hasher.update(serial.to_string().as_bytes());
        }
        DocumentIdentity::Sha256(hash) => {
            hasher.update(b"sha256:");
            hasher.update(hex_lower(hash).as_bytes());
        }
    }
    let hex = hasher.hex();
    format!(
        "{}-{}-8{}-a{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use cad_db::ModelKind;
    use cad_domain::ymodel::markers;
    use cad_domain::ymodel::ModelKey;
    use cad_domain::ymodel::{IdCounters, Shard};
    use cad_domain::{DatabaseId, Revision, UnitContext};

    fn header(serial: u128) -> DocumentHeader {
        DocumentHeader {
            database: DatabaseId(1),
            units: UnitContext::drawing_units(),
            identity: DocumentIdentity::Temporary(serial),
            resource_keys: Vec::new(),
            embed_resources: false,
            application_domain: None,
            variables: serde_json::json!({}),
            next: IdCounters::default(),
        }
    }

    #[test]
    fn a_saved_archive_reads_back_through_the_zstd_codec() {
        use cad_db::{ChangeScope, MemorySink};

        let snapshot = DocumentSnapshot {
            header: header(1),
            revision: Revision(3),
            models: Vec::new(),
            shards: Vec::new(),
            tables: Default::default(),
            textures: Default::default(),
            resources: Default::default(),
            resource_names: Default::default(),
            resource_sources: Default::default(),
        };
        let mut sink = MemorySink::new();
        encode(
            None,
            None,
            &ChangeScope::default(),
            None,
            None,
            None,
            &snapshot,
            &mut sink,
        )
        .expect("save");
        let bytes = sink.into_committed_bytes().expect("committed");
        let parsed = crate::read::parse(&bytes).expect("parse");
        assert_eq!(parsed.loaded.snapshot.revision, Revision(3));

        // Every modelled JSON entry is a zstd frame in a Stored zip entry, and the
        // manifest records the stored payload's digest and length for it.
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("zip");
        let mut manifest = Vec::new();
        {
            use std::io::Read as _;
            archive
                .by_name(MANIFEST_ENTRY)
                .expect("manifest")
                .read_to_end(&mut manifest)
                .expect("read manifest");
        }
        let manifest: Value = serde_json::from_slice(&manifest).expect("manifest json");
        let entries = manifest["entries"].as_array().expect("entries");
        assert!(!entries.is_empty());
        let mut saw_zstd = false;
        for entry in entries {
            assert_eq!(entry["codec"], Value::from("zstd"), "{entry}");
            assert!(entry["sha256_c"].is_string(), "a modelled entry records it");
            let record_len_c = entry["len_c"].as_u64().expect("len_c");
            let file = archive
                .by_name(entry["path"].as_str().expect("path"))
                .expect("entry");
            assert_eq!(
                file.compression(),
                zip::CompressionMethod::Stored,
                "{} must be Stored",
                entry["path"]
            );
            assert_eq!(file.compressed_size(), record_len_c);
            saw_zstd = true;
        }
        assert!(saw_zstd);
    }

    #[test]
    fn texture_entry_accepts_only_the_canonical_key_form() {
        let key = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";
        assert_eq!(
            texture_entry(key).expect("a canonical png key"),
            format!("textures/{key}")
        );
        assert_eq!(
            texture_entry("0f1e2d3c4b5a69788796a5b4c3d2e1f0.jpg").expect("a canonical jpg key"),
            "textures/0f1e2d3c4b5a69788796a5b4c3d2e1f0.jpg"
        );
        for bad in [
            "",
            "/tmp/tex.png",
            "../tex.png",
            "textures/4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png",
            "4C2F2E0A5F3D9B7C1E8A6D4B2C0F9E71.png",
            "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.jpeg",
            "tex.png",
        ] {
            let error = texture_entry(bad).expect_err(bad);
            assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{bad}: {error}");
        }
    }

    #[test]
    fn the_plan_emits_one_binary_entry_per_snapshot_texture() {
        let key = "4c2f2e0a5f3d9b7c1e8a6d4b2c0f9e71.png";
        let mut snapshot = DocumentSnapshot {
            header: header(1),
            revision: Revision(1),
            models: Vec::new(),
            shards: Vec::new(),
            tables: Default::default(),
            textures: BTreeMap::from([(
                key.to_string(),
                Arc::from(vec![0x89u8, b'P', b'N', b'G']),
            )]),
            resources: Default::default(),
            resource_names: Default::default(),
            resource_sources: Default::default(),
        };
        let (planned, _) = plan_entries(&snapshot).expect("plan");
        let entry = planned
            .iter()
            .find(|entry| entry.path == format!("textures/{key}"))
            .expect("the texture entry is planned");
        assert!(
            matches!(entry.body, Some(Body::Binary(_))),
            "a texture is a binary body"
        );
        assert!(entry.copy.is_none(), "a texture is never raw-copied");

        // The snapshot is the authority: dropping the texture drops the entry.
        snapshot.textures.clear();
        let (planned, _) = plan_entries(&snapshot).expect("plan");
        assert!(
            planned
                .iter()
                .all(|entry| !entry.path.starts_with("textures/")),
            "no orphan texture entry may be written"
        );

        // An invalid key is refused before any byte is written.
        snapshot
            .textures
            .insert("../escape.png".to_string(), Arc::from(vec![1u8]));
        assert!(plan_entries(&snapshot).is_err());
    }

    #[test]
    fn shard_entries_name_the_model_and_shard() {
        assert_eq!(shard_entry(1024, Shard(0)), "elements/1024-0.json");
        assert_eq!(shard_entry(1024, Shard(7)), "elements/1024-7.json");
    }

    #[test]
    fn the_derived_uuid_is_stable_and_uuid_shaped() {
        let first = derive_document_uuid(&header(1));
        assert_eq!(first, derive_document_uuid(&header(1)));
        assert_ne!(first, derive_document_uuid(&header(2)));
        assert_eq!(first.len(), 36);
        let groups: Vec<&str> = first.split('-').collect();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(groups[2].starts_with('8'), "{first}");
        assert!(groups[3].starts_with('a'), "{first}");
        assert!(first.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn the_plan_covers_the_header_registry_models_tables_and_shards() {
        let mut snapshot = DocumentSnapshot {
            header: header(1),
            revision: Revision(4),
            models: vec![ModelRecord {
                key: ModelKey::ModelSpace,
                kind: ModelKind::ModelSpace,
                name: None,
                owner: None,
            }],
            shards: vec![
                ShardSnapshot {
                    model: ModelKey::ModelSpace,
                    shard: Shard(1),
                    elements: Vec::new(),
                },
                ShardSnapshot {
                    model: ModelKey::ModelSpace,
                    shard: Shard(0),
                    elements: Vec::new(),
                },
            ],
            tables: Default::default(),
            textures: Default::default(),
            resources: Default::default(),
            resource_names: Default::default(),
            resource_sources: Default::default(),
        };
        snapshot.tables.set(cad_db::TableKind::Layers, Vec::new());
        let (planned, shard_map) = plan_entries(&snapshot).expect("plan");
        let paths: Vec<&str> = planned.iter().map(|entry| entry.path.as_str()).collect();
        assert!(paths.contains(&DOCUMENT_ENTRY));
        assert!(paths.contains(&CLASSES_ENTRY));
        assert!(paths.contains(&MODELS_ENTRY));
        assert!(paths.contains(&"tables/layers.json"));
        let key = ModelKey::ModelSpace.persistent().raw().to_string();
        assert_eq!(
            shard_map.get(&key).map(Vec::as_slice),
            Some(
                &[
                    format!("elements/{key}-0.json"),
                    format!("elements/{key}-1.json"),
                ][..]
            )
        );
        // Only shard entries are marked copyable; the header, registry, models and
        // tables are always re-encoded.
        let marked: BTreeSet<(ModelKey, Shard)> = planned
            .iter()
            .filter_map(|entry| match entry.copy {
                Some(CopySource::Shard(model, shard)) => Some((model, shard)),
                _ => None,
            })
            .collect();
        assert_eq!(
            marked,
            BTreeSet::from([
                (ModelKey::ModelSpace, Shard(0)),
                (ModelKey::ModelSpace, Shard(1))
            ])
        );
    }

    #[test]
    fn two_shards_cannot_share_an_entry() {
        let snapshot = DocumentSnapshot {
            header: header(1),
            revision: Revision(1),
            models: Vec::new(),
            shards: vec![
                ShardSnapshot {
                    model: ModelKey::ModelSpace,
                    shard: Shard(0),
                    elements: Vec::new(),
                },
                ShardSnapshot {
                    model: ModelKey::ModelSpace,
                    shard: Shard(0),
                    elements: Vec::new(),
                },
            ],
            tables: Default::default(),
            textures: Default::default(),
            resources: Default::default(),
            resource_names: Default::default(),
            resource_sources: Default::default(),
        };
        assert!(plan_entries(&snapshot).is_err());
    }
}
