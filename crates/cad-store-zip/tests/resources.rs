//! Bundled external resources: `resources/<kind>/<key>` entries and
//! `tables/resources.json` (ADR-0040 D1/D4/D5/D6/D8/D12/D18).
//!
//! Everything here is a real archive round trip. The only hand-built archives are
//! the ones no API can produce: a listed entry with a path this build does not
//! model, and a **foreign** blob sitting at a content-addressed resource path —
//! both of which exist to prove the store refuses or takes over rather than
//! silently losing bytes.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use cad_db::{
    ChangeScope, DocumentSnapshot, DocumentStore, DrawingDatabase, DrawingDatabaseBuilder,
    MemorySink, MemorySource, SaveReport, StoreError, TableKind,
};
use cad_domain::ymodel::{markers, DocumentHeader, IdCounters};
use cad_domain::{BundledKind, DatabaseId, DocumentIdentity, Revision, UnitContext};
use cad_store_zip::ZipDocumentStore;

use common::{assemble, crc32, explode, sha256_hex, v2_record};

/// A `.shx` font payload: small, opaque, not JSON.
const FONT: &[u8] = b"\x01\x00shx font bytes\xff";

// ---------------------------------------------------------------- fixtures ---

/// The content-addressed key of `bytes`: `<sha256 prefix as 32 hex>.<ext>`.
fn key_for(bytes: &[u8], ext: &str) -> String {
    format!("{}.{ext}", &sha256_hex(bytes)[..32])
}

fn header(embed_resources: bool) -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(11),
        units: UnitContext::drawing_units(),
        identity: DocumentIdentity::Temporary(2),
        resource_keys: Vec::new(),
        embed_resources,
        variables: serde_json::json!({}),
        next: IdCounters::default(),
    }
}

/// A document with no elements and no resources.
fn snapshot() -> DocumentSnapshot {
    DocumentSnapshot {
        header: header(false),
        revision: Revision(3),
        models: Vec::new(),
        shards: Vec::new(),
        tables: Default::default(),
        textures: Default::default(),
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    }
}

/// Append the `tables/resources.json` row for one resource.
fn add_row(
    snapshot: &mut DocumentSnapshot,
    kind: BundledKind,
    key: &str,
    name: &str,
    source: Option<&str>,
) {
    let mut row = serde_json::Map::new();
    row.insert("key".to_string(), Value::from(key));
    row.insert("kind".to_string(), Value::from(kind.as_str()));
    row.insert("name".to_string(), Value::from(name));
    row.insert(
        "source".to_string(),
        source.map(Value::from).unwrap_or(Value::Null),
    );
    snapshot
        .tables
        .rows
        .entry(TableKind::Resources)
        .or_default()
        .push(Value::Object(row));
}

/// Add one resource to `snapshot` (bytes, name, source **and** the table row) and
/// return its key.
///
/// `resource_sources` only gains an entry when a path was recorded: `cad-db`'s
/// builder removes the key for "no path", and one spelling for that state is what
/// keeps a load → snapshot a fixed point.
fn add_resource(
    snapshot: &mut DocumentSnapshot,
    kind: BundledKind,
    name: &str,
    ext: &str,
    bytes: &[u8],
    source: Option<&str>,
) -> String {
    let key = key_for(bytes, ext);
    let id = (kind, key.clone());
    snapshot.resources.insert(id.clone(), Arc::from(bytes));
    snapshot.resource_names.insert(id.clone(), name.to_string());
    if let Some(source) = source {
        snapshot
            .resource_sources
            .insert(id, Some(source.to_string()));
    }
    add_row(snapshot, kind, &key, name, source);
    key
}

fn write_snapshot(
    store: &mut ZipDocumentStore,
    snapshot: &DocumentSnapshot,
) -> (Vec<u8>, SaveReport) {
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

// --------------------------------------------------------- archive helpers ---

fn manifest_entry<'a>(manifest: &'a Value, path: &str) -> &'a Value {
    manifest["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .find(|entry| entry["path"] == path)
        .unwrap_or_else(|| panic!("manifest has no entry {path:?}"))
}

/// Remove one entry's manifest record, leaving the blob: the shape an archive
/// written by another tool has (an entry nobody describes).
fn unlist(base: &[u8], path: &str) -> Vec<u8> {
    let (entries, mut manifest) = explode(base);
    manifest["entries"]
        .as_array_mut()
        .expect("entries")
        .retain(|record| record["path"].as_str() != Some(path));
    assemble(&entries, &manifest)
}

/// Replace one entry's bytes and keep it unlisted.
fn replace_unlisted(base: &[u8], path: &str, body: &[u8]) -> Vec<u8> {
    let (mut entries, mut manifest) = explode(base);
    manifest["entries"]
        .as_array_mut()
        .expect("entries")
        .retain(|record| record["path"].as_str() != Some(path));
    for (name, old) in entries.iter_mut() {
        if name == path {
            *old = body.to_vec();
        }
    }
    assemble(&entries, &manifest)
}

/// Move one entry **and** its manifest record to a new path, leaving the body and
/// the hashes untouched: the shape of a writer that computed the wrong key.
fn rename_entry(base: &[u8], from: &str, to: &str) -> Vec<u8> {
    let (mut entries, mut manifest) = explode(base);
    for (name, _) in entries.iter_mut() {
        if name == from {
            *name = to.to_string();
        }
    }
    for record in manifest["entries"].as_array_mut().expect("entries") {
        if record["path"].as_str() == Some(from) {
            record["path"] = Value::from(to);
        }
    }
    assemble(&entries, &manifest)
}

/// Add a **listed, modelled** entry this build may not accept, with a correct
/// record so the version gate (not a hash check) is what refuses the archive.
fn with_listed_entry(base: &[u8], path: &str, body: &[u8]) -> Vec<u8> {
    let (mut entries, mut manifest) = explode(base);
    entries.push((path.to_string(), body.to_vec()));
    // A `raw` record is a complete `Stored` v2 entry; it still names a path this
    // build does not model, so the read gate refuses it as a newer writer's data.
    manifest["entries"]
        .as_array_mut()
        .expect("entries")
        .push(v2_record(path, "raw", body, Some(body)));
    assemble(&entries, &manifest)
}

// ------------------------------------------------------------- round trip ---

#[test]
fn three_resources_round_trip_with_their_names_sources_and_table_body() {
    let jpeg: Vec<u8> = {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0];
        bytes.extend((0..48).map(|index| (index * 5) as u8));
        bytes
    };
    let dxf = b"0\nSECTION\n2\nENTITIES\n0\nENDSEC\n0\nEOF\n".to_vec();

    let mut snapshot = snapshot();
    let font = add_resource(
        &mut snapshot,
        BundledKind::Font,
        "arial.shx",
        "shx",
        FONT,
        Some("C:/proj/fonts/arial.shx"),
    );
    let xref = add_resource(
        &mut snapshot,
        BundledKind::Xref,
        "STRUCT",
        "dxf",
        &dxf,
        None,
    );
    let image = add_resource(
        &mut snapshot,
        BundledKind::Image,
        "logo.jpg",
        "jpg",
        &jpeg,
        Some("C:/proj/images/logo.jpg"),
    );

    let (bytes, report) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    assert!(report.entries_written >= 6, "{report:?}");

    // Each payload is one `raw` entry at its content-addressed path; `explode`
    // returns it verbatim (a raw payload is its own logical form).
    let (entries, manifest) = explode(&bytes);
    for (kind, key, payload) in [
        (BundledKind::Font, &font, FONT),
        (BundledKind::Xref, &xref, &dxf[..]),
        (BundledKind::Image, &image, &jpeg[..]),
    ] {
        let path = format!("resources/{kind}/{key}");
        let body = entries
            .iter()
            .find(|(name, _)| name == &path)
            .unwrap_or_else(|| panic!("no {path:?} entry"))
            .1
            .clone();
        assert_eq!(body, payload, "{path} bytes are written verbatim");
        let record = manifest_entry(&manifest, &path);
        assert_eq!(record["codec"], Value::String("raw".to_string()));
        assert_eq!(record["foreign"], Value::Bool(false));
        assert_eq!(record["sha256"], Value::String(sha256_hex(payload)));
        assert_eq!(
            record["sha256_c"], record["sha256"],
            "a raw entry stores its logical bytes, so both digests agree"
        );
        assert_eq!(record["len"], Value::from(payload.len() as u64));
        assert_eq!(record["len_c"], record["len"]);
        assert_eq!(record["crc"], Value::from(u64::from(crc32(payload))));
    }

    // The table body is the row array the snapshot held, byte for byte.
    let table_body = entries
        .iter()
        .find(|(name, _)| name == "tables/resources.json")
        .expect("the mapping table is written")
        .1
        .clone();
    let decoded: Value = serde_json::from_slice(&table_body).expect("array");
    assert_eq!(
        decoded,
        Value::Array(
            snapshot
                .tables
                .get(TableKind::Resources)
                .expect("rows")
                .to_vec()
        ),
        "tables/resources.json must stay a plain array of the same rows"
    );

    // Read back: payloads, names, sources and the table are identical.
    let loaded = read_archive(&bytes).expect("read");
    assert!(loaded.snapshot.bytes_equal(&snapshot), "payloads survive");
    assert_eq!(loaded.snapshot, snapshot, "the snapshot is a fixed point");
    assert_eq!(
        &loaded.snapshot.resources[&(BundledKind::Font, font.clone())][..],
        FONT
    );
    assert_eq!(
        &loaded.snapshot.resources[&(BundledKind::Xref, xref.clone())][..],
        &dxf[..]
    );
    assert_eq!(
        loaded.snapshot.resource_names[&(BundledKind::Font, font)],
        "arial.shx"
    );
    assert_eq!(
        loaded.snapshot.resource_sources[&(BundledKind::Image, image)],
        Some("C:/proj/images/logo.jpg".to_string())
    );
    // A sourceless row decodes to *no* entry, not to an entry holding `None`.
    assert!(!loaded
        .snapshot
        .resource_sources
        .contains_key(&(BundledKind::Xref, xref)));
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::Resources),
        snapshot.tables.get(TableKind::Resources)
    );
}

// ------------------------------------------------------------------ minor ---

#[test]
fn the_minor_is_per_document_so_a_plain_document_stays_minor_four() {
    let plain = snapshot();
    let (bytes, _) = write_snapshot(&mut ZipDocumentStore::new(), &plain);
    assert_eq!(
        explode(&bytes).1["format"]["minor"],
        Value::from(4),
        "no resources and no flag: an older build must still open it"
    );

    let mut packed = snapshot();
    add_resource(
        &mut packed,
        BundledKind::Font,
        "arial.ttf",
        "ttf",
        FONT,
        None,
    );
    let (bytes, _) = write_snapshot(&mut ZipDocumentStore::new(), &packed);
    assert_eq!(
        explode(&bytes).1["format"]["minor"],
        Value::from(5),
        "a resource payload needs the resource level"
    );

    // The flag alone is enough: it is written into document.json, which a
    // minor-4 reader would refuse as an unknown field.
    let mut flagged = snapshot();
    flagged.header.embed_resources = true;
    let (bytes, _) = write_snapshot(&mut ZipDocumentStore::new(), &flagged);
    assert_eq!(explode(&bytes).1["format"]["minor"], Value::from(5));

    // So is the mapping table on its own, even empty: a pre-0040 reader would
    // refuse the `tables/resources.json` path it does not model.
    let mut mapped = snapshot();
    mapped.tables.set(TableKind::Resources, Vec::new());
    let (bytes, _) = write_snapshot(&mut ZipDocumentStore::new(), &mapped);
    assert_eq!(explode(&bytes).1["format"]["minor"], Value::from(5));
    assert!(explode(&bytes)
        .0
        .iter()
        .any(|(name, _)| name == "tables/resources.json"));
}

// --------------------------------------------------------------- raw copy ---

#[test]
fn an_unchanged_resource_is_raw_copied_and_the_archive_stays_stable() {
    let mut snapshot = snapshot();
    let key = add_resource(
        &mut snapshot,
        BundledKind::Font,
        "arial.ttf",
        "ttf",
        FONT,
        None,
    );
    let path = format!("resources/font/{key}");

    let (first, first_report) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    assert_eq!(first_report.entries_copied, 0, "no previous archive");

    let mut store = ZipDocumentStore::new().with_previous(Some(first.clone()));
    let (second, report) = write_snapshot(&mut store, &snapshot);
    assert_eq!(
        report.entries_copied, 1,
        "only the resource is copyable: {report:?}"
    );
    assert_eq!(
        report.entries_written,
        first_report.entries_written - 1,
        "the copied resource must not be re-encoded"
    );

    // The copy carries the source record's measurements over unchanged.
    let before = explode(&first).1;
    let after = explode(&second).1;
    let (old, new) = (
        manifest_entry(&before, &path),
        manifest_entry(&after, &path),
    );
    for field in ["sha256", "sha256_c", "len", "len_c", "crc"] {
        assert_eq!(old[field], new[field], "{field} must survive the copy");
    }
    let body = explode(&second)
        .0
        .into_iter()
        .find(|(name, _)| name == &path)
        .expect("the resource entry is still there")
        .1;
    assert_eq!(body, FONT, "the copied bytes are identical");

    // A third save from the second archive is byte-identical.
    let mut store = ZipDocumentStore::new().with_previous(Some(second.clone()));
    let (third, _) = write_snapshot(&mut store, &snapshot);
    assert_eq!(third, second, "the archive is a fixed point");
}

// ------------------------------------------------------------- refusals ----

#[test]
fn a_key_that_does_not_name_its_bytes_is_refused_on_write() {
    let mut snapshot = snapshot();
    let wrong = format!("{}.ttf", "0".repeat(32));
    let id = (BundledKind::Font, wrong.clone());
    snapshot.resources.insert(id.clone(), Arc::from(FONT));
    snapshot.resource_names.insert(id, "arial.ttf".to_string());
    // Both representations agree, so the refusal is the key/content invariant and
    // not the mapping check.
    add_row(&mut snapshot, BundledKind::Font, &wrong, "arial.ttf", None);

    let mut sink = MemorySink::new();
    let error = ZipDocumentStore::new()
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect_err("a wrong key must refuse the save");
    // A collector that computed the wrong key is not a corrupt document: F4's
    // distinct write-side marker, not `ymodel.invalid_document`.
    assert_eq!(error.marker, markers::RESOURCE_INVALID_KEY, "{error}");
    assert!(
        error.to_string().contains("does not name its own payload"),
        "{error}"
    );
    assert!(
        sink.committed_bytes().is_none(),
        "a refused save commits nothing"
    );
}

#[test]
fn a_name_edited_in_one_representation_only_refuses_the_save() {
    let mut document = snapshot();
    let key = add_resource(
        &mut document,
        BundledKind::Font,
        "arial.ttf",
        "ttf",
        FONT,
        Some("C:/proj/fonts/arial.ttf"),
    );
    // Edit only `resource_names`: the row still says "arial.ttf".
    document
        .resource_names
        .insert((BundledKind::Font, key.clone()), "renamed.ttf".to_string());
    let mut sink = MemorySink::new();
    let error = ZipDocumentStore::new()
        .write(&document, &ChangeScope::default(), &mut sink)
        .expect_err("a name edited in one place must refuse the save");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(error.to_string().contains("disagree"), "{error}");
    assert!(sink.committed_bytes().is_none());

    // Same for `resource_sources`: a different path in the map than in the row.
    let mut edited = snapshot();
    let key = add_resource(
        &mut edited,
        BundledKind::Font,
        "arial.ttf",
        "ttf",
        FONT,
        Some("C:/proj/fonts/arial.ttf"),
    );
    edited.resource_sources.insert(
        (BundledKind::Font, key),
        Some("D:/elsewhere/arial.ttf".to_string()),
    );
    let mut sink = MemorySink::new();
    let error = ZipDocumentStore::new()
        .write(&edited, &ChangeScope::default(), &mut sink)
        .expect_err("a source edited in one place must refuse the save");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(sink.committed_bytes().is_none());
}

#[test]
fn rows_without_payloads_are_refused_not_written_as_a_lower_minor() {
    // A row names bytes the snapshot does not carry. Writing it would produce an
    // archive this build's own reader refuses (`check_mapping`), so the save is
    // refused instead — the only choice that cannot emit an unopenable file.
    let mut snapshot = snapshot();
    add_row(
        &mut snapshot,
        BundledKind::Font,
        &format!("{}.ttf", "0".repeat(32)),
        "arial.ttf",
        None,
    );
    let mut sink = MemorySink::new();
    let error = ZipDocumentStore::new()
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect_err("a row without a payload must refuse the save");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(
        sink.committed_bytes().is_none(),
        "no archive may be committed at all"
    );
}

#[test]
fn a_key_that_does_not_name_its_bytes_is_refused_on_read_too() {
    let mut snapshot = snapshot();
    let key = add_resource(
        &mut snapshot,
        BundledKind::Font,
        "arial.ttf",
        "ttf",
        FONT,
        None,
    );
    let (base, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);

    // A canonical-looking key whose hex prefix is not this payload's digest. The
    // manifest still describes the bytes, so only the key/content check can catch
    // it — and it must, or the next save would raw-copy the stale bytes.
    let wrong = format!("{}.ttf", "0".repeat(32));
    let archive = rename_entry(
        &base,
        &format!("resources/font/{key}"),
        &format!("resources/font/{wrong}"),
    );
    let error = read_archive(&archive).expect_err("key/content mismatch");
    assert_eq!(error.marker, markers::INVALID_DOCUMENT, "{error}");
    assert!(
        error.to_string().contains("does not name its own payload"),
        "{error}"
    );
}

#[test]
fn illegal_key_forms_are_refused_on_write_and_yield_unsupported_minor_on_read() {
    let hex = &sha256_hex(FONT)[..32];
    for key in [
        "../x.ttf".to_string(),
        "/tmp/x.ttf".to_string(),
        format!("{hex}.TTF"),
        format!("{hex}.docx"),
    ] {
        let mut snapshot = snapshot();
        let id = (BundledKind::Font, key.clone());
        snapshot.resources.insert(id.clone(), Arc::from(FONT));
        snapshot.resource_names.insert(id, "arial.ttf".to_string());
        let mut sink = MemorySink::new();
        let error = ZipDocumentStore::new()
            .write(&snapshot, &ChangeScope::default(), &mut sink)
            .expect_err(&key);
        assert_eq!(
            error.marker,
            markers::RESOURCE_UNSUPPORTED_KIND,
            "{key}: {error}"
        );
    }

    // A **listed** entry with a non-canonical path is a newer writer's data: the
    // version gate refuses it instead of dropping the bytes on the next save.
    let mut snapshot = snapshot();
    add_resource(
        &mut snapshot,
        BundledKind::Font,
        "arial.ttf",
        "ttf",
        FONT,
        None,
    );
    let (base, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    for path in [
        "resources/font/../x.ttf".to_string(),
        format!("resources/font/{}.TTF", "0".repeat(32)),
    ] {
        let archive = with_listed_entry(&base, &path, b"a blob");
        let error = read_archive(&archive).expect_err(&path);
        assert_eq!(error.marker, markers::UNSUPPORTED_MINOR, "{path}: {error}");
    }
}

// ------------------------------------------------------- foreign collision ---

#[test]
fn a_foreign_blob_at_a_resource_path_is_taken_over_or_refused() {
    let mut snapshot = snapshot();
    let key = add_resource(
        &mut snapshot,
        BundledKind::Font,
        "arial.ttf",
        "ttf",
        FONT,
        None,
    );
    let path = format!("resources/font/{key}");
    let (first, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);

    // Same bytes, but the previous archive does not list them: the modelled entry
    // is the same asset, so it takes over without a conflict (D8).
    let previous = unlist(&first, &path);
    let mut store = ZipDocumentStore::new().with_previous(Some(previous));
    let (second, _) = write_snapshot(&mut store, &snapshot);
    let (entries, manifest) = explode(&second);
    assert_eq!(
        entries
            .iter()
            .find(|(name, _)| name == &path)
            .expect("the resource is still written")
            .1,
        FONT
    );
    assert_eq!(
        manifest_entry(&manifest, &path)["foreign"],
        Value::Bool(false)
    );

    // Different bytes at the same content-addressed path are a real conflict.
    let previous = replace_unlisted(&first, &path, b"different bytes entirely");
    let mut store = ZipDocumentStore::new().with_previous(Some(previous));
    let mut sink = MemorySink::new();
    let error = store
        .write(&snapshot, &ChangeScope::default(), &mut sink)
        .expect_err("different bytes must refuse the save");
    assert_eq!(error.marker, markers::RESOURCE_CONFLICT, "{error}");
    assert!(sink.committed_bytes().is_none());
}

// -------------------------------------------------------------- retention ---

#[test]
fn a_resource_only_archive_is_kept_as_the_next_save_s_base() {
    let mut snapshot = snapshot();
    add_resource(
        &mut snapshot,
        BundledKind::Image,
        "logo.png",
        "png",
        b"\x89PNG\r\n\x1a\nbody",
        None,
    );
    let (bytes, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    assert!(
        explode(&bytes).1["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .all(|record| record["foreign"] != Value::Bool(true)),
        "fixture precondition: no foreign entries"
    );

    // D18: read keeps the archive because a modelled resource is a raw-copy base.
    let mut store = ZipDocumentStore::new();
    let mut source = MemorySource::new(bytes.clone());
    let loaded = store.read(&mut source).expect("read");
    assert!(
        store.previous_archive().is_some(),
        "a resource entry must be enough to keep the previous bytes"
    );
    assert_eq!(loaded.manifest.foreign_entries, Vec::<String>::new());

    let (resaved, report) = write_snapshot(&mut store, &loaded.snapshot);
    assert_eq!(report.entries_copied, 1, "{report:?}");
    // The copy proves the base was kept; the payload is what reopens.
    let (entries, _) = explode(&resaved);
    assert!(
        entries
            .iter()
            .any(|(name, _)| name.starts_with("resources/")),
        "the resource survived the load → save cycle"
    );
    let reloaded = read_archive(&resaved).expect("reopen");
    assert_eq!(reloaded.snapshot, loaded.snapshot);
    // Saving again from the copied archive is byte-identical: a raw copy is a
    // fixed point even though it re-writes the local header once.
    let mut store = ZipDocumentStore::new().with_previous(Some(resaved.clone()));
    let (third, _) = write_snapshot(&mut store, &reloaded.snapshot);
    assert_eq!(third, resaved);
}

/// The map the reader hands back is the same `(kind, key)` map the snapshot had.
#[test]
fn the_loaded_resource_map_is_keyed_by_kind_and_key() {
    let mut snapshot = snapshot();
    let key = add_resource(
        &mut snapshot,
        BundledKind::Xref,
        "STRUCT",
        "dwg",
        FONT,
        None,
    );
    let (bytes, _) = write_snapshot(&mut ZipDocumentStore::new(), &snapshot);
    let loaded = read_archive(&bytes).expect("read");
    let expected: BTreeMap<(BundledKind, String), Arc<[u8]>> =
        BTreeMap::from([((BundledKind::Xref, key.clone()), Arc::from(FONT))]);
    assert_eq!(loaded.snapshot.resources, expected);
    assert_eq!(
        loaded.snapshot.resource_names[&(BundledKind::Xref, key)],
        "STRUCT"
    );
}

/// F2: a sourceless resource must survive `snapshot → database → snapshot`.
///
/// `cad-db` has one spelling for "no path" (the absent key, its builder removes
/// the entry). The store used to insert a `None` on read, so the reloaded snapshot
/// disagreed with the one the database produced and every load → save rewrote the
/// archive. This fixture starts from the **producer**, not from a hand-built map,
/// which is why the store-level round-trip tests could not see it.
#[test]
fn a_sourceless_resource_is_a_fixed_point_through_a_database() {
    let key = key_for(FONT, "ttf");
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(11));
    builder
        .insert_resource(BundledKind::Font, &key, "arial.ttf", Arc::from(FONT))
        .expect("insert the resource");
    builder
        .set_resource_source(BundledKind::Font, &key, None)
        .expect("clear the source path");
    let db = builder.finish().expect("build the database");
    let first = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(2),
            Vec::new(),
            Revision(3),
        )
        .expect("encode the snapshot");
    assert!(
        first.resource_sources.is_empty(),
        "fixture precondition: cad-db spells 'no path' by omitting the entry"
    );

    let (bytes, _) = write_snapshot(&mut ZipDocumentStore::new(), &first);
    let loaded = read_archive(&bytes).expect("read");
    // The `source: null` row must not resurrect a `None` entry on read.
    assert!(
        loaded.snapshot.resource_sources.is_empty(),
        "read must not invent a second spelling for 'no path'"
    );
    assert_eq!(
        &loaded.snapshot.resources[&(BundledKind::Font, key.clone())][..],
        FONT
    );
    // The row still carries `source: null`, so nothing was lost on disk.
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Resources)
        .expect("the mapping table");
    assert_eq!(rows[0]["source"], Value::Null);

    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    let second = reopened
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(2),
            Vec::new(),
            Revision(3),
        )
        .expect("encode the snapshot again");
    assert_eq!(
        second, loaded.snapshot,
        "snapshot → database → snapshot is a fixed point"
    );
    assert!(second.bytes_equal(&loaded.snapshot));
}
