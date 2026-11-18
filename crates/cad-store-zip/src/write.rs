//! Encoding a [`cad_db::DocumentSnapshot`] into `.ymodel` bytes (ADR-0037 D11).
//!
//! Phase 1 is a **full rewrite**: every entry the document holds is re-encoded, so
//! an entry that is no longer live simply is not written and cannot reappear on
//! the next open. The only entries carried over are the **foreign** ones, and
//! they are carried over at the compressed-byte level with
//! [`zip::ZipWriter::raw_copy_file`] — never parsed, never re-compressed, never
//! dropped.
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
    table_entry, ByteSink, DocumentSnapshot, ModelRecord, SaveReport, ShardSnapshot, StoreError,
    CLASSES_ENTRY, DOCUMENT_ENTRY, MODELS_ENTRY,
};
use cad_domain::ymodel::{markers, DocumentHeader};
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
        }
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
pub(crate) fn encode(
    max_bytes: Option<u64>,
    previous_bytes: Option<&[u8]>,
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

    // Foreign entries join the plan at the compressed-byte level. A collision with
    // a live entry is an explicit failure, never an overwrite (D11).
    let mut foreign_records = BTreeMap::new();
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
            foreign_records.insert(path.clone(), record);
            foreign_paths.push(path.clone());
        }
    }
    for path in foreign_paths {
        planned.push(Planned { path, body: None });
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
        &foreign_records,
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
    });
    planned.push(Planned {
        path: CLASSES_ENTRY.to_string(),
        body: Some(Body::Classes),
    });
    planned.push(Planned {
        path: MODELS_ENTRY.to_string(),
        body: Some(Body::Models(&snapshot.models)),
    });
    for (kind, rows) in &snapshot.tables.rows {
        planned.push(Planned {
            path: table_entry(*kind).to_string(),
            body: Some(Body::Table(rows)),
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
        });
    }
    for paths in shard_map.values_mut() {
        paths.sort();
    }
    Ok((planned, shard_map))
}

/// `elements/<model raw>-<shard>.json`.
fn shard_entry(model_raw: i64, shard: cad_domain::ymodel::Shard) -> String {
    format!("elements/{model_raw}-{}.json", shard.0)
}

#[allow(clippy::too_many_arguments)]
fn write_entries(
    sink: &mut dyn ByteSink,
    metrics: &RefCell<WriteMetrics>,
    max_bytes: Option<u64>,
    planned: &[Planned<'_>],
    foreign_records: &BTreeMap<String, EntryRecord>,
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
                start_json(&mut zip, metrics, &entry.path)?;
                zip.write_all(&bytes)
                    .map_err(|error| io_error(metrics, error, &entry.path))?;
                records.push(record);
                entries_written += 1;
            }
            None => {
                let record = foreign_records.get(&entry.path).ok_or_else(|| {
                    malformed(
                        "foreign entry",
                        format!("{:?} was planned without a record", entry.path),
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
        };
        assert!(plan_entries(&snapshot).is_err());
    }
}
