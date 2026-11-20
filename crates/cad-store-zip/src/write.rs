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
//! The write order is the format's: every entry in sorted path order, then
//! `manifest.json` **last** (its entry table can only be built once every other
//! entry has been hashed), then the zip central directory via `finish`.
//!
//! # Streaming, and why `begin` gets no size hint
//!
//! Entries are fed to the sink one at a time through [`SinkWriter`]: at most one
//! shard's JSON exists as a buffer, so a 200 MB document does not need a 400 MB
//! spike. The consequence is that the archive's total size is genuinely unknown
//! when `sink.begin` runs — a hint would be a guess, and a wrong hint is worse
//! than `None` (a carrier could reserve the wrong amount or refuse a save that
//! would have fit). The budget is therefore enforced per chunk, which is what
//! ADR-0037 D16 asks for anyway.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Write};

use serde_json::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use cad_db::{
    table_entry, ByteSink, ChangeScope, DocumentSnapshot, ModelRecord, SaveReport, ShardSnapshot,
    StoreError, CLASSES_ENTRY, DOCUMENT_ENTRY, MODELS_ENTRY,
};
use cad_domain::ymodel::{markers, DocumentHeader, ModelKey, Shard};
use cad_domain::DocumentIdentity;

use crate::crc32::crc32;
use crate::json::{hex_lower, malformed, sha256_hex, Sha256Sum};
use crate::manifest::{EntryRecord, Manifest};
use crate::models::models_value;
use crate::previous::{self, PreviousArchive};
use crate::schema::classes_value;
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
    /// A document resource's raw bytes (a base-colour texture, ADR-0038 D4).
    ///
    /// Written verbatim: PNG/JPEG are already compressed, so a second deflate pass
    /// only costs CPU and blocks, and the format's `len`/`sha256` describe exactly
    /// the image the importer registered.
    Binary(&'a [u8]),
}

impl Body<'_> {
    /// The entry's uncompressed bytes.
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
            Body::Binary(bytes) => Ok(bytes.to_vec()),
        }
    }

    /// Whether this entry holds JSON (deflated) or opaque bytes (stored).
    fn is_json(&self) -> bool {
        !matches!(self, Body::Binary(_))
    }
}

fn encode_error(error: serde_json::Error) -> StoreError {
    malformed("entry", error)
}

/// One entry the save will write: re-encoded, or copied from the previous
/// archive.
struct Planned<'a> {
    path: String,
    body: Option<Body<'a>>,
    /// The shard this entry holds, when it is a `Body::Shard`.
    ///
    /// Only shard entries are eligible for the raw copy: the rest are small and
    /// are always re-encoded, so `None` here also means "never copy this".
    shard: Option<(ModelKey, Shard)>,
}

/// Everything that must survive the borrow of the sink.
struct WriteMetrics {
    written: u64,
    failure: Option<StoreError>,
}

/// A zip sink that streams straight into a [`ByteSink`] and enforces the budget.
///
/// The budget check happens **before** the chunk reaches the sink, so the failure
/// is `PERSISTENCE_TOO_LARGE` rather than a partially written archive. After a
/// failure the writer goes quiet (every later `write` returns `Ok` and discards),
/// which lets the zip writer's `Drop` finalize without touching an aborted sink
/// and without printing to stderr.
struct SinkWriter<'a> {
    sink: &'a mut dyn ByteSink,
    metrics: &'a RefCell<WriteMetrics>,
    budget: Option<u64>,
    poisoned: bool,
}

impl SinkWriter<'_> {
    fn poison(&mut self, error: StoreError) {
        self.metrics.borrow_mut().failure = Some(error);
        self.poisoned = true;
    }
}

impl Write for SinkWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.poisoned {
            return Ok(buffer.len());
        }
        let written = self.metrics.borrow().written;
        if let Some(limit) = self.budget {
            if written.saturating_add(buffer.len() as u64) > limit {
                self.poison(StoreError::new(
                    markers::PERSISTENCE_TOO_LARGE,
                    format!("writing this document would exceed the {limit}-byte budget"),
                ));
                return Err(io::Error::other("ymodel serialization budget exceeded"));
            }
        }
        match self.sink.write_chunk(buffer) {
            Ok(()) => {
                self.metrics.borrow_mut().written = written + buffer.len() as u64;
                Ok(buffer.len())
            }
            Err(error) => {
                let detail = error.to_string();
                self.poison(error);
                Err(io::Error::other(detail))
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        // The sink has no flush: `commit` is the durability point (D16).
        Ok(())
    }
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

    // Shards the commit did not touch keep their compressed bytes. The source
    // manifest must describe the entry as modelled (`!foreign`) and list it in
    // `shard_map`; otherwise it is re-encoded rather than trusted.
    if let Some(previous) = previous.as_ref() {
        for entry in &mut planned {
            let Some((model, shard)) = entry.shard else {
                continue;
            };
            if !copyable_shard(previous, dirty, &entry.path, model, shard) {
                continue;
            }
            let source = previous
                .record(&entry.path)
                .expect("copyable_shard requires a source record");
            copy_records.insert(
                entry.path.clone(),
                EntryRecord {
                    path: entry.path.clone(),
                    // The compressed bytes are copied unchanged, so the source's
                    // uncompressed hashes still describe them.
                    sha256: source.sha256.clone(),
                    // This build never holds the compressed bytes as a buffer
                    // (`zip`'s raw reader is crate-private), so it cannot claim a
                    // value for them.
                    sha256_c: None,
                    len: source.len,
                    crc: source.crc,
                    foreign: false,
                },
            );
            entry.body = None;
        }
    }

    // Foreign entries join the plan at the compressed-byte level. A collision with
    // a live entry is an explicit failure, never an overwrite (D11).
    let mut foreign_paths = Vec::new();
    if let (Some(previous), Some(bytes)) = (previous.as_ref(), previous_bytes) {
        let live: BTreeSet<&str> = planned.iter().map(|entry| entry.path.as_str()).collect();
        for path in previous.foreign() {
            if path == MANIFEST_ENTRY {
                continue;
            }
            if live.contains(path.as_str()) {
                return Err(previous::conflict(path));
            }
            let record = match previous.reusable(path) {
                Some(record) => {
                    let mut record = record.clone();
                    record.foreign = true;
                    record
                }
                // Not described by the source manifest: hash it from the bytes.
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
            shard: None,
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
        shard: None,
    });
    planned.push(Planned {
        path: CLASSES_ENTRY.to_string(),
        body: Some(Body::Classes),
        shard: None,
    });
    planned.push(Planned {
        path: MODELS_ENTRY.to_string(),
        body: Some(Body::Models(&snapshot.models)),
        shard: None,
    });
    for (kind, rows) in &snapshot.tables.rows {
        planned.push(Planned {
            path: table_entry(*kind).to_string(),
            body: Some(Body::Table(rows)),
            shard: None,
        });
    }
    // Document resources (ADR-0038 D4/D5). The snapshot is the authority: exactly
    // one entry per texture it holds, and never one it does not — an orphan image
    // must not appear in the archive. Texture entries are never raw-copied (their
    // bytes are already in memory), so `shard: None` also means "always re-encoded".
    for (key, bytes) in &snapshot.textures {
        planned.push(Planned {
            path: texture_entry(key)?,
            body: Some(Body::Binary(&bytes[..])),
            shard: None,
        });
    }

    let mut shard_map = ShardMap::new();
    let mut seen = BTreeSet::new();
    for shard in &snapshot.shards {
        let path = shard_entry(shard.model.persistent().raw(), shard.shard);
        if !seen.insert(path.clone()) {
            return Err(malformed(
                "elements",
                format!("two shards encode to the same entry {path:?}"),
            ));
        }
        shard_map
            .entry(shard.model.persistent().raw().to_string())
            .or_default()
            .push(path.clone());
        planned.push(Planned {
            path,
            body: Some(Body::Shard(shard)),
            shard: Some((shard.model, shard.shard)),
        });
    }
    for paths in shard_map.values_mut() {
        paths.sort();
    }
    Ok((planned, shard_map))
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

    let mut records: Vec<EntryRecord> = Vec::with_capacity(planned.len() + 1);
    let mut entries_written = 0usize;
    let mut entries_copied = 0usize;
    let mut previous_archive: Option<ZipArchive<Cursor<&[u8]>>> = None;

    for entry in planned {
        match &entry.body {
            Some(body) => {
                let bytes = body.encode()?;
                let record = EntryRecord {
                    path: entry.path.clone(),
                    sha256: sha256_hex(&bytes),
                    // See the crate docs: encoding streams through the zip writer, so
                    // the compressed bytes are never a buffer this store can hash.
                    sha256_c: None,
                    len: bytes.len() as u64,
                    crc: crc32(&bytes),
                    foreign: false,
                };
                start_entry(&mut zip, metrics, &entry.path, body.is_json())?;
                zip.write_all(&bytes)
                    .map_err(|error| io_error(metrics, error, &entry.path))?;
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
            minor: cad_domain::ymodel::FORMAT_MINOR,
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
    start_json(&mut zip, metrics, MANIFEST_ENTRY)?;
    zip.write_all(&bytes)
        .map_err(|error| io_error(metrics, error, MANIFEST_ENTRY))?;
    zip.finish()
        .map_err(|error| zip_error(metrics, error, "archive"))?;

    let written = metrics.borrow().written;
    Ok((written, entries_written, entries_copied))
}

/// The options every JSON entry is written with: one compression method, one
/// timestamp, so two saves produce the same bytes.
pub(crate) fn json_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
}

/// The options a binary resource entry is written with.
///
/// `Stored`, not `Deflated`: PNG/JPEG are already compressed, so deflating them
/// again spends CPU to produce slightly larger bytes. The timestamp is the same
/// fixed value as every other entry, so a save stays byte-deterministic.
pub(crate) fn binary_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
}

fn start_entry(
    zip: &mut ZipSink<'_>,
    metrics: &RefCell<WriteMetrics>,
    path: &str,
    is_json: bool,
) -> Result<(), StoreError> {
    let options = if is_json {
        json_options()
    } else {
        binary_options()
    };
    zip.start_file(path, options)
        .map_err(|error| zip_error(metrics, error, path))
}

fn start_json(
    zip: &mut ZipSink<'_>,
    metrics: &RefCell<WriteMetrics>,
    path: &str,
) -> Result<(), StoreError> {
    zip.start_file(path, json_options())
        .map_err(|error| zip_error(metrics, error, path))
}

/// A zip failure: the carrier's own error if the sink refused a chunk, otherwise
/// a malformed-output diagnostic.
fn zip_error(
    metrics: &RefCell<WriteMetrics>,
    error: zip::result::ZipError,
    context: &str,
) -> StoreError {
    take_failure(metrics).unwrap_or_else(|| malformed(context, error))
}

fn io_error(metrics: &RefCell<WriteMetrics>, error: io::Error, context: &str) -> StoreError {
    take_failure(metrics).unwrap_or_else(|| malformed(context, error))
}

fn take_failure(metrics: &RefCell<WriteMetrics>) -> Option<StoreError> {
    metrics.borrow_mut().failure.take()
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
    use cad_domain::ymodel::ModelKey;
    use cad_domain::ymodel::{IdCounters, Shard};
    use cad_domain::{DatabaseId, Revision, UnitContext};

    fn header(serial: u128) -> DocumentHeader {
        DocumentHeader {
            database: DatabaseId(1),
            units: UnitContext::drawing_units(),
            identity: DocumentIdentity::Temporary(serial),
            resource_keys: Vec::new(),
            variables: serde_json::json!({}),
            next: IdCounters::default(),
        }
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
        assert!(entry.shard.is_none(), "a texture is never raw-copied");

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
        // Only the shard entries are marked copyable; the header, registry, models
        // and tables are always re-encoded.
        let marked: BTreeSet<(ModelKey, Shard)> =
            planned.iter().filter_map(|entry| entry.shard).collect();
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
        };
        assert!(plan_entries(&snapshot).is_err());
    }
}
