//! Parsing `.ymodel` bytes into a [`cad_db::DocumentSnapshot`] (ADR-0037 D11/D13).
//!
//! An archive is untrusted input, so the order is fixed and deliberate:
//!
//! 1. `manifest.json` is read first — nothing else can be interpreted without it
//!    — and the **version gate** runs immediately: a newer `format.read` is
//!    [`markers::UNSUPPORTED_VERSION`] and a newer `format.minor` is
//!    [`markers::UNSUPPORTED_MINOR`], both before any other entry is touched.
//!    Opening a newer minor and re-saving it would silently drop the fields this
//!    build does not model (D11).
//! 2. Every entry the manifest lists is checked against the archive: it must
//!    exist, its zip CRC and stored length must equal the recorded ones, and a
//!    modelled entry must be a `Stored` zip entry (O(1), from the central
//!    directory).
//! 3. Every entry this build models is then decoded: the stored payload's
//!    `sha256_c` and `len_c` are verified first, the frame's declared size is
//!    compared with `len`, and the logical bytes are checked against `sha256`.
//! 4. Entries that are **not** listed, or listed with `foreign: true`, are never
//!    opened. They are reported as `foreign_entries` and preserved by the next
//!    save.
//!
//! # Document textures
//!
//! A listed `textures/<key>` entry is a modelled binary resource (ADR-0038 D4): the
//! key must be the canonical `<32 lowercase hex>.png|.jpg` form, and the bytes go
//! through the same `len`/SHA-256/CRC verification as every other entry before
//! they enter [`cad_db::DocumentSnapshot::textures`]. An **unlisted** `textures/...`
//! entry is not special-cased: it is a foreign entry and is preserved verbatim, so
//! a document written by another tool keeps its blobs.
//!
//! # `sha256_c` and what this build verifies
//!
//! Every entry this build **encodes** is a zip `Stored` entry, so its stored
//! payload **is** a buffer the writer holds and hashes: `sha256_c` is recorded for
//! every modelled entry and verified here **before** the payload is decoded. For a
//! `codec: "zstd"` entry the check is: stored bytes hash to `sha256_c` and their
//! length is `len_c`, the frame declares exactly `len` (`manifest.json`'s logical
//! length), decoding into a `len`-sized slice writes exactly `len`, and those
//! logical bytes hash to `sha256`. A `codec: "raw"` entry skips the frame (its
//! stored bytes are its logical bytes) but runs the same `sha256`/`len` checks.
//! `codec: "opaque"` entries are foreign and never decoded; the central directory
//! checks in step 2 (`crc`, `len_c`, and for foreign entries `len`) still refuse a
//! corrupted blob.
//!
//! The frame's declared content size is compared with `len` **before any block is
//! decoded**, and the output buffer is allocated at exactly `len`, so a header
//! that lies about its size cannot drive the allocation. The reader never uses a
//! streaming `read_to_end` here: that is what would let a lying frame grow the
//! buffer.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::sync::Arc;

use serde_json::Value;
use zip::{CompressionMethod, ZipArchive};

use cad_db::{
    table_entry, DocumentSnapshot, ElementSnapshot, FormatVersion, LoadedDocument, ModelRecord,
    ShardSnapshot, StoreError, TableKind, TableSnapshot, CLASSES_ENTRY, DOCUMENT_ENTRY,
    MODELS_ENTRY,
};
use cad_domain::ymodel::{
    markers, DocumentHeader, IdKind, ModelKey, PersistentId, PersistentIdOf, Shard, FORMAT_MINOR,
    FORMAT_READ,
};
use cad_domain::{BundledKind, EntityId, Revision};

use crate::codec;
use crate::json::{malformed, sha256_hex, Reader};
use crate::manifest::{probe_format, Codec, EntryRecord, Manifest};
use crate::models::models_from_value;
use crate::resources::{self, resource_entry};
use crate::MANIFEST_ENTRY;

/// The largest logical or stored entry this build will decode: 1 GiB.
///
/// The archive is untrusted input, so a manifest-declared length is refused before
/// a buffer of that size is allocated. A modelled entry larger than this is not a
/// document this build can represent; the limit is well above any real shard (the
/// web budget is 256 MB for the whole archive, ADR-0037 D16).
const MAX_ENTRY_BYTES: u64 = 1 << 30;

/// The element shards plus the raw-model → paths map they imply.
type ShardEntries = (Vec<ShardSnapshot>, BTreeMap<i64, Vec<String>>);

/// The three resource views one decode fills, kept together so they cannot drift.
type ResourceEntries = (
    BTreeMap<(BundledKind, String), Arc<[u8]>>,
    BTreeMap<(BundledKind, String), String>,
    BTreeMap<(BundledKind, String), Option<String>>,
);

/// One successfully parsed archive.
pub(crate) struct Parsed {
    pub loaded: LoadedDocument,
    /// `manifest.saved_at`, kept so a load → save cycle does not rewrite it.
    pub saved_at: Option<String>,
}

/// The parse failures the format does not attribute to one entry.
const ARCHIVE: &str = "archive";

/// Parse `bytes` into a snapshot.
pub(crate) fn parse(bytes: &[u8]) -> Result<Parsed, StoreError> {
    let mut archive = open(bytes)?;

    // 1. The manifest, then the version gate — before anything else is read. The
    // gate only needs `format.{read,write,minor}`, so it runs on a probe of the
    // decoded JSON: an archive with a different entry codec is refused as an
    // unsupported version, not as a malformed manifest.
    let manifest = {
        let raw = read_all(&mut archive, MANIFEST_ENTRY)?;
        let value: Value =
            serde_json::from_slice(&raw).map_err(|error| malformed(MANIFEST_ENTRY, error))?;
        let format = probe_format(&value)?;
        gate(&format)?;
        Manifest::from_json_value(&value)?
    };

    // 2. Structural checks that need only the central directory.
    let names = entry_names(&mut archive)?;
    check_listed_entries(&mut archive, &manifest)?;
    let foreign_entries = foreign_entries(&names, &manifest);
    check_known_entries(&manifest)?;

    // 3. Decode what this build models.
    let header = parse_header(&mut archive, &manifest)?;
    parse_classes(&mut archive, &manifest)?;
    let models = parse_models(&mut archive, &manifest)?;
    let tables = parse_tables(&mut archive, &manifest)?;
    let (resources, resource_names, resource_sources) =
        parse_resources(&mut archive, &manifest, &tables)?;
    let textures = parse_textures(&mut archive, &manifest)?;
    let (shards, shard_paths) = parse_shards(&mut archive, &manifest, &models)?;
    check_shard_map(&manifest, &shard_paths)?;

    let manifest_summary = manifest.summary(foreign_entries);
    let snapshot = DocumentSnapshot {
        header,
        // D2: the archive's saved revision seeds the in-memory revision, so
        // an edited-then-reopened document does not satisfy the L0 predicate.
        revision: Revision(manifest.saved_revision),
        models,
        shards,
        tables,
        textures,
        resources,
        resource_names,
        resource_sources,
    };
    Ok(Parsed {
        // The two texture views are filled from one decode, so they cannot drift.
        loaded: LoadedDocument {
            textures: snapshot.textures.clone(),
            snapshot,
            manifest: manifest_summary,
        },
        saved_at: manifest.saved_at.clone(),
    })
}

fn open(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, StoreError> {
    ZipArchive::new(Cursor::new(bytes)).map_err(|error| malformed(ARCHIVE, error))
}

/// D13: refuse what this build cannot represent faithfully.
///
/// `read` is an **equality** gate, not a minimum: version 2 changed the per-entry
/// payload codec (JSON is now a zstd frame in a `Stored` zip entry, with
/// `sha256_c`/`len_c` describing it). A version-1 archive is not unreadable in
/// principle, but this build has no codec for its deflated payloads, and opening
/// it only to refuse an entry mid-way would be worse than a clear version error.
/// Re-importing the source document is the supported path.
fn gate(format: &FormatVersion) -> Result<(), StoreError> {
    if format.read != FORMAT_READ {
        return Err(StoreError::new(
            markers::UNSUPPORTED_VERSION,
            format!(
                "the archive requires format.read {}, but this build reads only \
                 v{FORMAT_READ} zstd-frame archives; a document written before that \
                 codec change must be re-imported",
                format.read
            ),
        ));
    }
    if format.minor > FORMAT_MINOR {
        return Err(StoreError::new(
            markers::UNSUPPORTED_MINOR,
            format!(
                "the archive was written at minor {}, this build writes up to {} \
                 (refusing rather than dropping the fields it does not model)",
                format.minor, FORMAT_MINOR
            ),
        ));
    }
    Ok(())
}

/// Every entry name, refused if the archive repeats one.
fn entry_names(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Result<Vec<String>, StoreError> {
    let mut names = Vec::with_capacity(archive.len());
    let mut seen = BTreeSet::new();
    for name in archive.file_names() {
        let name = name
            .map_err(|error| malformed(ARCHIVE, error))?
            .into_owned();
        if !seen.insert(name.clone()) {
            return Err(malformed(
                ARCHIVE,
                format!("{name:?} appears more than once in the archive"),
            ));
        }
        names.push(name);
    }
    Ok(names)
}

/// Every listed entry exists and matches what the manifest records about its
/// stored and logical bytes.
fn check_listed_entries(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &Manifest,
) -> Result<(), StoreError> {
    for record in &manifest.entries {
        // The declared lengths are the allocation bounds a decode would use, so
        // they are capped before anything is read.
        if record.len > MAX_ENTRY_BYTES || record.len_c > MAX_ENTRY_BYTES {
            return Err(malformed(
                &record.path,
                format!(
                    "manifest.json declares {} logical / {} stored bytes, over the \
                     {MAX_ENTRY_BYTES}-byte entry limit",
                    record.len, record.len_c
                ),
            ));
        }
        let file = archive
            .by_name(&record.path)
            .map_err(|error| malformed(&record.path, error))?;
        if file.crc32() != record.crc {
            return Err(malformed(
                &record.path,
                "the archive's zip CRC does not match manifest.json",
            ));
        }
        if file.compressed_size() != record.len_c {
            return Err(malformed(
                &record.path,
                format!(
                    "the archive stores {} bytes, manifest.json records {}",
                    file.compressed_size(),
                    record.len_c
                ),
            ));
        }
        match record.codec {
            // A modelled payload is Stored, so the zip entry's bytes are exactly
            // the payload `sha256_c`/`len_c` describe and `len_c == size`.
            Codec::Zstd | Codec::Raw => {
                if file.compression() != CompressionMethod::Stored {
                    return Err(malformed(
                        &record.path,
                        format!(
                            "a {} entry must be zip Stored, the archive has {:?}",
                            record.codec.as_str(),
                            file.compression()
                        ),
                    ));
                }
                if file.size() != record.len_c {
                    return Err(malformed(
                        &record.path,
                        format!(
                            "the stored entry is {} bytes, manifest.json records {}",
                            file.size(),
                            record.len_c
                        ),
                    ));
                }
            }
            // A foreign entry may be deflated by another writer and is never
            // decoded here, but its logical length is still a recorded claim.
            Codec::Opaque => {
                if file.size() != record.len {
                    return Err(malformed(
                        &record.path,
                        format!(
                            "the archive holds {} logical bytes, manifest.json records {}",
                            file.size(),
                            record.len
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Entries this build must carry over untouched, sorted.
fn foreign_entries(names: &[String], manifest: &Manifest) -> Vec<String> {
    let mut foreign = BTreeSet::new();
    for name in names {
        if name == MANIFEST_ENTRY {
            continue;
        }
        match manifest.entry(name) {
            Some(record) if !record.foreign => {}
            _ => {
                foreign.insert(name.clone());
            }
        }
    }
    foreign.into_iter().collect()
}

fn parse_header(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &Manifest,
) -> Result<DocumentHeader, StoreError> {
    let record = required_entry(manifest, DOCUMENT_ENTRY)?;
    let value = read_json(archive, record)?;
    DocumentHeader::from_json_value(&value).map_err(|error| malformed(DOCUMENT_ENTRY, error))
}

/// `schema/classes.json` must be present and structurally a class registry.
///
/// It is not compared field by field with this build's registry on purpose: the
/// version gate already guarantees the same `minor`, and a byte-for-byte
/// comparison would reject a re-serialization that is semantically identical. The
/// shape check catches the failures that matter — a truncated registry, or one
/// whose classes have no `type_key` — so a caller can trust the entry to describe
/// the payloads it reads.
fn parse_classes(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &Manifest,
) -> Result<(), StoreError> {
    let record = required_entry(manifest, CLASSES_ENTRY)?;
    let value = read_json(archive, record)?;
    let mut reader = Reader::new(&value, CLASSES_ENTRY)?;
    let classes = reader.array("classes")?;
    reader.finish()?;
    if classes.is_empty() {
        return Err(malformed(CLASSES_ENTRY, "the class registry is empty"));
    }
    for class in &classes {
        let mut class_reader = Reader::new(class, CLASSES_ENTRY)?;
        let _ = class_reader.object("id")?;
        let type_key = class_reader.string("type_key")?;
        if type_key.is_empty() {
            return Err(malformed(CLASSES_ENTRY, "a class has an empty type_key"));
        }
        let fields = class_reader.array("fields")?;
        class_reader.finish()?;
        for field in &fields {
            let mut field_reader = Reader::new(field, CLASSES_ENTRY)?;
            let _ = field_reader.string("name")?;
            let _ = field_reader.string("kind")?;
            field_reader.finish()?;
        }
    }
    Ok(())
}

fn parse_models(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &Manifest,
) -> Result<Vec<ModelRecord>, StoreError> {
    let record = required_entry(manifest, MODELS_ENTRY)?;
    let value = read_json(archive, record)?;
    let models = models_from_value(&value)?;
    let mut seen = BTreeSet::new();
    for model in &models {
        if !seen.insert(model.key.persistent().raw()) {
            return Err(malformed(
                MODELS_ENTRY,
                format!("model {} is listed twice", model.key.persistent().raw()),
            ));
        }
    }
    Ok(models)
}

fn parse_tables(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &Manifest,
) -> Result<TableSnapshot, StoreError> {
    let mut tables = TableSnapshot::default();
    for record in listed_live(manifest) {
        let Some(kind) = table_kind_for_entry(&record.path) else {
            continue;
        };
        let value = read_json(archive, record)?;
        let Value::Array(rows) = value else {
            return Err(malformed(
                &record.path,
                "a table entry must be a JSON array",
            ));
        };
        tables.set(kind, rows);
    }
    Ok(tables)
}

/// Every table path this build models, and the kind it holds.
///
/// The list is a mirror of [`cad_db::table_entry`] and must be updated with it:
/// `TableKind` has no `ALL` constant here on purpose (the enumeration belongs to
/// `cad-db`, and `check-architecture.py` forbids this crate from editing it), so
/// the reverse map is written out and a table missing from it is refused by
/// [`unknown_live_entry`] rather than silently dropped.
const TABLE_ENTRIES: [(TableKind, &str); 26] = [
    (TableKind::Layers, "tables/layers.json"),
    (TableKind::Layouts, "tables/layouts.json"),
    (TableKind::Styles, "tables/styles.json"),
    (TableKind::DimStyles, "tables/dimstyles.json"),
    (TableKind::MLeaderStyles, "tables/mleaderstyles.json"),
    (TableKind::Linetypes, "tables/linetypes.json"),
    (TableKind::Scales, "tables/scales.json"),
    (TableKind::PlotSettings, "tables/plot-settings.json"),
    (TableKind::ExternalReferences, "tables/xrefs.json"),
    (TableKind::Blocks, "tables/blocks.json"),
    // The resource name ⇄ key mapping (ADR-0040 D4). A row array like every
    // other table; the payloads are the `resources/<kind>/<key>` entries.
    (TableKind::Resources, "tables/resources.json"),
    // Host-owned document metadata (ADR-0037 D11). Decoded as opaque rows: the
    // store never interprets a report, it only carries it across a save.
    (TableKind::Sources, "tables/sources.json"),
    (TableKind::Compat, "tables/compat.json"),
    // Shared BIM definitions (ADR-0042 D3). Opaque rows here too: `cad-db`'s
    // snapshot codec owns their shape and their budget.
    (TableKind::BimDefinitions, "tables/bim-definitions.json"),
    // BIM relation edges (ADR-0042 D3 tail/D12). Opaque rows: the endpoints use
    // the element `bim.source` key shape and the snapshot codec owns budgets.
    (TableKind::BimRelations, "tables/bim-relations.json"),
    // Document parameters (ADR-0043 D4). Opaque rows here: `cad-db`'s snapshot
    // codec owns the expression shape and its budgets.
    (TableKind::Parameters, "tables/parameters.json"),
    // Named parameter configurations (ADR-0043 P1). Opaque rows here: the
    // snapshot codec owns the overlay shape and its budgets.
    (TableKind::Configurations, "tables/configurations.json"),
    // P2 feature definitions and P3 in-context relations (opaque rows; the
    // snapshot codec owns their shapes and budgets).
    (TableKind::Features, "tables/features.json"),
    (TableKind::Relations, "tables/relations.json"),
    // Host-owned vendor semantics (ADR-0030 D7). Opaque rows here too: the host
    // owns the row shape, the store only carries it across a save.
    (TableKind::Vendor, "tables/vendor.json"),
    // Persisted entity groups. Opaque rows: the snapshot codec owns the row
    // shape and its member-identity rules.
    (TableKind::Groups, "tables/groups.json"),
    // Persisted named views (minor 18). Opaque rows like the other side tables:
    // `cad-db`'s snapshot codec owns the row shape and its strictness.
    (TableKind::Views, "tables/views.json"),
    // Document materials (minor 19). Opaque rows: the snapshot codec owns the
    // name/colour/assignment rules.
    (TableKind::Materials, "tables/materials.json"),
    (TableKind::Scenes, "tables/scenes.json"),
    (TableKind::Display, "tables/display.json"),
    // Persisted construction guides (minor 26). Opaque rows: the snapshot codec
    // owns the point/direction shape and the cap.
    (TableKind::Guides, "tables/guides.json"),
];

/// The table kind one entry path holds, if this build models it.
fn table_kind_for_entry(path: &str) -> Option<TableKind> {
    TABLE_ENTRIES
        .iter()
        .find(|(kind, entry)| {
            // `table_entry` is the authority; the literal above is only a fast path.
            *entry == path && table_entry(*kind) == path
        })
        .map(|(kind, _)| *kind)
}

fn parse_shards(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &Manifest,
    models: &[ModelRecord],
) -> Result<ShardEntries, StoreError> {
    let model_keys: BTreeMap<i64, ModelKey> = models
        .iter()
        .map(|model| (model.key.persistent().raw(), model.key))
        .collect();

    let mut shards = Vec::new();
    let mut shard_paths: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for record in listed_live(manifest) {
        if !record.path.starts_with("elements/") {
            continue;
        }
        let (model_raw, shard) = parse_shard_path(&record.path)?;
        let model = *model_keys.get(&model_raw).ok_or_else(|| {
            malformed(
                &record.path,
                format!("model {model_raw} is not listed in {MODELS_ENTRY}"),
            )
        })?;
        let value = read_json(archive, record)?;
        let Value::Array(items) = value else {
            return Err(malformed(
                &record.path,
                "a shard entry must be a JSON array of element payloads",
            ));
        };
        let mut elements = Vec::with_capacity(items.len());
        for item in items {
            elements.push(element_snapshot(&record.path, item)?);
        }
        elements.sort_by_key(|element| element.id.0);
        for pair in elements.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(malformed(
                    &record.path,
                    format!("element id {} appears twice in one shard", pair[0].id.0),
                ));
            }
        }
        shards.push(ShardSnapshot {
            model,
            shard,
            elements,
        });
        shard_paths
            .entry(model_raw)
            .or_default()
            .push(record.path.clone());
    }
    Ok((shards, shard_paths))
}

/// The prefix every document texture entry carries (ADR-0038 D4).
const TEXTURES_PREFIX: &str = "textures/";

/// The document texture key one entry path names, when the path really is a
/// `textures/<valid key>` entry.
///
/// One helper for both the "do we model this entry?" check and the decode, so the
/// two can never disagree about what a texture entry is.
fn texture_key(path: &str) -> Option<&str> {
    path.strip_prefix(TEXTURES_PREFIX)
        .filter(|key| cad_domain::is_valid_texture_key(key))
}

/// Every listed `textures/<key>` entry, verified and decoded into the snapshot's
/// texture table (ADR-0038 D3/D4).
///
/// Verification is the existing entry path — [`check_listed_entries`] already
/// compared the zip CRC and `len_c`, and [`read_entry`] checks the stored payload's
/// `sha256_c` and the SHA-256/length of the bytes it yields — so a corrupted or
/// truncated image refuses the open instead of entering the document. An
/// **unlisted** `textures/...` entry never reaches
/// here: [`foreign_entries`] keeps it as a preserved foreign entry, exactly like
/// any other unmodelled blob.
fn parse_textures(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &Manifest,
) -> Result<BTreeMap<String, Arc<[u8]>>, StoreError> {
    let mut textures = BTreeMap::new();
    for record in listed_live(manifest) {
        let Some(key) = texture_key(&record.path) else {
            continue;
        };
        let bytes = read_entry(archive, record)?;
        if bytes.is_empty() {
            return Err(malformed(
                &record.path,
                "a texture entry must not be empty (an empty image cannot decode)",
            ));
        }
        textures.insert(key.to_string(), Arc::from(bytes));
    }
    Ok(textures)
}

/// Every listed `resources/<kind>/<key>` entry, decoded into the snapshot
/// (ADR-0040 D1/D4/D5/D6).
///
/// The key is content-addressed, so the manifest's `sha256` — already verified
/// against the payload by [`read_entry`] — must start with the key's hex prefix.
/// Without that check an archive whose key disagrees with its bytes would be
/// re-saved under the wrong name and then raw-copied forever (D1).
///
/// The mapping comes from `tables/resources.json`, which is a plain row array
/// (D4): its rows name the payloads, and the two sets must be equal. An entry
/// whose name is not in the canonical `resources/<kind>/<32 hex>.<ext>` form never
/// reaches here — [`is_modelled`] refuses it as [`markers::UNSUPPORTED_MINOR`].
fn parse_resources(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    manifest: &Manifest,
    tables: &TableSnapshot,
) -> Result<ResourceEntries, StoreError> {
    let mapping = resources::decode_rows(tables.get(TableKind::Resources).unwrap_or(&[]))?;
    let mut payloads: BTreeMap<(BundledKind, String), Arc<[u8]>> = BTreeMap::new();
    for record in listed_live(manifest) {
        let Some((kind, key)) = resource_entry(&record.path) else {
            continue;
        };
        // D10: the archive is untrusted input, so the declared length is capped
        // before the entry is decompressed, not after it is in memory.
        resources::check_budget(&record.path, record.len)?;
        resources::check_read_key_matches_digest(kind, key, &record.sha256, &record.path)?;
        let bytes = read_entry(archive, record)?;
        payloads.insert((kind, key.to_string()), Arc::from(bytes));
    }
    resources::check_mapping(&payloads, &mapping)?;
    let (names, sources) = mapping;
    Ok((payloads, names, sources))
}

/// Every element row: the payload plus the identity the payload carries.
fn element_snapshot(path: &str, value: Value) -> Result<ElementSnapshot, StoreError> {
    let Some(raw) = value.get("id").and_then(Value::as_i64) else {
        return Err(malformed(
            path,
            "an element payload must carry a top-level \"id\" integer",
        ));
    };
    let id = PersistentId::from_raw(raw)
        .ok_or_else(|| malformed(path, format!("element id {raw} is not storable")))?;
    if id.kind() != IdKind::Entity {
        return Err(malformed(
            path,
            format!("element id {raw} is not an Entity identifier"),
        ));
    }
    Ok(ElementSnapshot {
        id: EntityId::from_seq(id.seq()),
        payload: value,
    })
}

/// `elements/<model raw>-<shard>.json`.
fn parse_shard_path(path: &str) -> Result<(i64, Shard), StoreError> {
    let bad = || malformed(path, "not of the form elements/<model>-<shard>.json");
    let rest = path.strip_prefix("elements/").ok_or_else(bad)?;
    let stem = rest.strip_suffix(".json").ok_or_else(bad)?;
    let (model, shard) = stem.split_once('-').ok_or_else(bad)?;
    let model: i64 = model.parse().map_err(|_| bad())?;
    let shard: u64 = shard.parse().map_err(|_| bad())?;
    let id = PersistentId::from_raw(model)
        .ok_or_else(|| malformed(path, format!("model id {model} is not storable")))?;
    if ModelKey::from_persistent(id).is_none() {
        return Err(malformed(
            path,
            format!("model id {model} is not a Model identifier"),
        ));
    }
    Ok((model, Shard(shard)))
}

/// `shard_map` must describe exactly the shard entries the archive holds.
fn check_shard_map(
    manifest: &Manifest,
    shard_paths: &BTreeMap<i64, Vec<String>>,
) -> Result<(), StoreError> {
    let mut declared: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for (model, paths) in &manifest.shard_map {
        let raw: i64 = model
            .parse()
            .map_err(|_| malformed(MANIFEST_ENTRY, format!("bad shard_map key {model:?}")))?;
        declared.insert(raw, paths.clone());
    }
    if &declared != shard_paths {
        return Err(malformed(
            MANIFEST_ENTRY,
            "shard_map does not match the elements/ entries in the archive",
        ));
    }
    Ok(())
}

/// The listed entries this build re-encodes.
fn listed_live(manifest: &Manifest) -> impl Iterator<Item = &EntryRecord> {
    manifest.entries.iter().filter(|record| !record.foreign)
}

/// A required entry that must be listed, present and non-foreign.
fn required_entry<'a>(
    manifest: &'a Manifest,
    path: &'static str,
) -> Result<&'a EntryRecord, StoreError> {
    match manifest.entry(path) {
        Some(record) if !record.foreign => Ok(record),
        _ => Err(malformed(
            MANIFEST_ENTRY,
            format!("{path} must be a listed, non-foreign entry"),
        )),
    }
}

/// The stored payload of one entry, bounded by `len_c` and verified against
/// `sha256_c` when the manifest records one.
///
/// The read is capped at `len_c + 1` so a truncated or padded entry fails the
/// length comparison instead of growing the buffer; for a modelled entry the
/// digest then matches the bytes the manifest describes.
fn read_payload(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    record: &EntryRecord,
) -> Result<Vec<u8>, StoreError> {
    let file = archive
        .by_name(&record.path)
        .map_err(|error| malformed(&record.path, error))?;
    let mut bytes = Vec::new();
    let read = file
        .take(record.len_c.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| malformed(&record.path, error))?;
    if read as u64 != record.len_c {
        return Err(malformed(
            &record.path,
            format!(
                "read {read} stored bytes, manifest.json records {}",
                record.len_c
            ),
        ));
    }
    if let Some(expected) = &record.sha256_c {
        if sha256_hex(&bytes) != *expected {
            return Err(malformed(
                &record.path,
                "SHA-256 of the stored payload does not match manifest.json",
            ));
        }
    }
    Ok(bytes)
}

/// Read one entry's logical bytes, by codec, and verify them.
///
/// The order is integrity first: the **stored** payload is hashed before it is
/// decoded, so a corrupt frame is refused without spending decode work on it; the
/// decoded bytes are then hashed again (a decode to a different byte string must
/// fail even if the stored bytes were intact).
fn read_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    record: &EntryRecord,
) -> Result<Vec<u8>, StoreError> {
    let bytes = match record.codec {
        Codec::Zstd => {
            codec::decode_frame(&read_payload(archive, record)?, record.len, &record.path)?
        }
        Codec::Raw => read_payload(archive, record)?,
        // Every caller is a listed, non-foreign entry; an opaque record there is a
        // manifest that describes bytes no codec can produce.
        Codec::Opaque => {
            return Err(malformed(
                &record.path,
                "an opaque entry is copied, never decoded as a modelled entry",
            ))
        }
    };
    let digest = sha256_hex(&bytes);
    if digest != record.sha256 {
        return Err(malformed(
            &record.path,
            "SHA-256 of the logical bytes does not match manifest.json",
        ));
    }
    if bytes.len() as u64 != record.len {
        return Err(malformed(
            &record.path,
            format!(
                "decoded {} bytes, manifest.json records {}",
                bytes.len(),
                record.len
            ),
        ));
    }
    Ok(bytes)
}

fn read_json(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    record: &EntryRecord,
) -> Result<Value, StoreError> {
    let bytes = read_entry(archive, record)?;
    serde_json::from_slice(&bytes).map_err(|error| malformed(&record.path, error))
}

/// Raw bytes of one entry, unverified — only `manifest.json` itself is read this
/// way, because its hash is not listed anywhere by design.
fn read_all(archive: &mut ZipArchive<Cursor<&[u8]>>, path: &str) -> Result<Vec<u8>, StoreError> {
    let mut file = archive
        .by_name(path)
        .map_err(|error| malformed(path, error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| malformed(path, error))?;
    Ok(bytes)
}

/// Every listed, non-foreign entry must be one this build can re-encode.
fn check_known_entries(manifest: &Manifest) -> Result<(), StoreError> {
    for record in listed_live(manifest) {
        if is_modelled(&record.path) {
            continue;
        }
        return Err(unknown_live_entry(&record.path));
    }
    Ok(())
}

fn is_modelled(path: &str) -> bool {
    path == DOCUMENT_ENTRY
        || path == CLASSES_ENTRY
        || path == MODELS_ENTRY
        || table_kind_for_entry(path).is_some()
        || texture_key(path).is_some()
        || resource_entry(path).is_some()
        || parse_shard_path(path).is_ok()
}

/// Refuse a listed entry this build does not model (a newer writer's data): the
/// next save would drop it, which D11 forbids.
pub(crate) fn unknown_live_entry(path: &str) -> StoreError {
    StoreError::new(
        markers::UNSUPPORTED_MINOR,
        format!(
            "{path} is a listed entry this build does not model; refusing to open \
             rather than dropping it on the next save"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::LayoutId;

    #[test]
    fn the_table_reverse_map_covers_every_kind_it_names() {
        for (kind, path) in TABLE_ENTRIES {
            assert_eq!(table_entry(kind), path);
            assert_eq!(table_kind_for_entry(path), Some(kind));
        }
        assert_eq!(table_kind_for_entry("tables/unknown.json"), None);
    }

    /// The mirror is checked against an **explicit** list of every `TableKind`.
    ///
    /// `the_table_reverse_map_covers_every_kind_it_names` derives its cases from
    /// `TABLE_ENTRIES` itself, so a new `TableKind` with no mirror row would be
    /// invisible to it. This test names the closed set by hand: adding a variant
    /// without a `TABLE_ENTRIES` row (and therefore without a reader entry) fails
    /// here instead of being silently refused at load time.
    #[test]
    fn every_table_kind_variant_has_a_reader_mirror_entry() {
        let kinds = [
            TableKind::Layers,
            TableKind::Layouts,
            TableKind::Styles,
            TableKind::DimStyles,
            TableKind::MLeaderStyles,
            TableKind::Linetypes,
            TableKind::Scales,
            TableKind::PlotSettings,
            TableKind::ExternalReferences,
            TableKind::Blocks,
            TableKind::Resources,
            TableKind::Sources,
            TableKind::Compat,
            TableKind::BimDefinitions,
            TableKind::BimRelations,
            TableKind::Parameters,
            TableKind::Configurations,
            TableKind::Features,
            TableKind::Relations,
            TableKind::Vendor,
            TableKind::Groups,
            TableKind::Views,
            TableKind::Materials,
            TableKind::Scenes,
            TableKind::Display,
            TableKind::Guides,
        ];
        let mut paths = std::collections::BTreeSet::new();
        for kind in kinds {
            // Exhaustive match: a new `TableKind` variant is a *compile* error
            // here, so the author must add it to this list (and then the length
            // assertion below demands the matching `TABLE_ENTRIES` row).
            let name = match kind {
                TableKind::Layers => "Layers",
                TableKind::Layouts => "Layouts",
                TableKind::Styles => "Styles",
                TableKind::DimStyles => "DimStyles",
                TableKind::MLeaderStyles => "MLeaderStyles",
                TableKind::Linetypes => "Linetypes",
                TableKind::Scales => "Scales",
                TableKind::PlotSettings => "PlotSettings",
                TableKind::ExternalReferences => "ExternalReferences",
                TableKind::Blocks => "Blocks",
                TableKind::Resources => "Resources",
                TableKind::Sources => "Sources",
                TableKind::Compat => "Compat",
                TableKind::BimDefinitions => "BimDefinitions",
                TableKind::BimRelations => "BimRelations",
                TableKind::Parameters => "Parameters",
                TableKind::Configurations => "Configurations",
                TableKind::Features => "Features",
                TableKind::Relations => "Relations",
                TableKind::Vendor => "Vendor",
                TableKind::Groups => "Groups",
                TableKind::Views => "Views",
                TableKind::Materials => "Materials",
                TableKind::Scenes => "Scenes",
                TableKind::Display => "Display",
                TableKind::Guides => "Guides",
            };
            assert_eq!(name, format!("{kind:?}"));
            let path = table_entry(kind);
            assert_eq!(
                table_kind_for_entry(path),
                Some(kind),
                "{kind:?} has no reader mirror entry for {path}"
            );
            assert!(paths.insert(path), "duplicate mirror path {path}");
        }
        assert_eq!(
            kinds.len(),
            TABLE_ENTRIES.len(),
            "every TableKind variant must have exactly one TABLE_ENTRIES row"
        );
        assert_eq!(paths.len(), kinds.len());
    }

    #[test]
    fn shard_paths_must_name_a_model() {
        let model = ModelKey::Paper(LayoutId(3)).persistent().raw();
        let valid = format!("elements/{model}-7.json");
        assert_eq!(parse_shard_path(&valid).expect("valid"), (model, Shard(7)));

        for path in [
            "elements/1024-7".to_string(),
            "elements/1024-7.json".to_string(),
            "elements/model-7.json".to_string(),
            "elements/1024-seven.json".to_string(),
            "elements/1024.json".to_string(),
            "tables/1024-7.json".to_string(),
            "elements/-1-7.json".to_string(),
            // An Entity id is not a Model id.
            format!("elements/{}-0.json", EntityId(1).persistent().raw()),
        ] {
            assert!(parse_shard_path(&path).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn element_ids_are_entity_ids() {
        let value = serde_json::json!({
            "id": EntityId(1).persistent().raw(),
            "class": "cad:Line"
        });
        let element = element_snapshot("elements/1024-0.json", value.clone()).expect("entity id");
        assert_eq!(element.id, EntityId(1));
        assert_eq!(element.payload, value);

        for bad in [
            serde_json::json!({"class": "cad:Line"}),
            serde_json::json!({"id": "1"}),
            // A Layer id: `(2 << 45) | 0`.
            serde_json::json!({"id": 70368744177664i64}),
            serde_json::json!({"id": -5}),
            serde_json::json!([1]),
        ] {
            assert!(
                element_snapshot("elements/1024-0.json", bad.clone()).is_err(),
                "accepted {bad}"
            );
        }
    }
}
