//! `tables/vendor.json` and the minor-12 tier (ADR-0030 D7).
//!
//! The table carries host-owned vendor semantics (one row per entity, keyed by
//! entity id). The store treats it as opaque rows, exactly like the report
//! tables: it writes every present table and reads it back without interpreting
//! the shape. These contracts pin the tier: rows add `tables/vendor.json` and
//! stamp `minor 12`; a document without rows keeps the minor its other features
//! imply; an archive stamped above a minor-11 reader's ceiling is refused by the
//! same version gate.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{
    ChangeScope, DocumentSnapshot, DocumentStore, ElementSnapshot, MemorySink, MemorySource,
    ModelKind, ModelRecord, ShardSnapshot, TableKind,
};
use cad_domain::ymodel::{DocumentHeader, IdCounters, ModelKey, PersistentIdOf, Shard};
use cad_domain::{DatabaseId, DocumentIdentity, EntityId, Revision, Unit, UnitContext};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

use common::with_manifest;

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(5),
        units: UnitContext {
            source: Unit::Millimeter,
            display: Unit::Meter,
            display_per_source: Some(0.001),
            decimal_places: 4,
        },
        identity: DocumentIdentity::Temporary(1),
        resource_keys: Vec::new(),
        embed_resources: false,
        variables: serde_json::json!({}),
        next: IdCounters {
            entity: 2,
            block: 1,
            layer: 1,
            layout: 1,
            style: 1,
            dim_style: 1,
            mleader_style: 1,
            linetype: 1,
            scale: 1,
        },
    }
}

fn element(seq: u128) -> Value {
    serde_json::json!({
        "class": "cad:Element",
        "id": EntityId(seq).persistent().raw(),
        "type_key": "AcDbLine",
        "space": {"kind": "model"},
        "layer": 1,
        "source_handle": null,
        "invisible": false,
        "render_attributes": null,
        "geometry": {"class": "cad:Line", "start": [0.0, 0.0, 0.0], "end": [1.0, 0.0, 0.0]},
    })
}

fn snapshot() -> DocumentSnapshot {
    DocumentSnapshot {
        header: header(),
        revision: Revision(2),
        models: vec![ModelRecord {
            key: ModelKey::ModelSpace,
            kind: ModelKind::ModelSpace,
            name: None,
            owner: None,
        }],
        shards: vec![ShardSnapshot {
            model: ModelKey::ModelSpace,
            shard: Shard(0),
            elements: vec![ElementSnapshot {
                id: EntityId(1),
                payload: element(1),
            }],
        }],
        tables: Default::default(),
        textures: Default::default(),
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    }
}

/// The minimal version-independent vendor row (ADR-0030 D7): keyed by entity id,
/// with the vendor class/status and no binary class-body bytes.
fn vendor_row() -> Value {
    serde_json::json!({
        "entity": EntityId(1).persistent().raw(),
        "handle": "1A2F",
        "kind": "tch",
        "class": "TCH_OPENING",
        "schema": 1,
        "status": "ok",
        "source": {"dwg": 1027, "base": 6, "class": 3, "decoder": 1},
        "units": {"scale": 100, "unit": "mm"},
        "attributes": [{"name": "DN", "type": "f64", "value": 100.0}],
        "params": {"width": 900, "height": 2100, "thickness": 240},
        "refs": [],
        "diagnostics": [],
    })
}

fn write_bytes(snapshot: &DocumentSnapshot) -> Vec<u8> {
    let mut sink = MemorySink::new();
    ZipDocumentStore::new()
        .write(snapshot, &ChangeScope::default(), &mut sink)
        .expect("write");
    sink.committed_bytes().expect("committed").to_vec()
}

fn read_bytes(bytes: &[u8]) -> cad_db::LoadedDocument {
    let mut source = MemorySource::new(bytes.to_vec());
    ZipDocumentStore::new().read(&mut source).expect("read")
}

/// `manifest.format.minor` of an archive, read as an untrusted tool would.
fn minor_of(bytes: &[u8]) -> u64 {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("manifest body");
    let manifest: Value = serde_json::from_slice(&body).expect("manifest json");
    manifest["format"]["minor"].as_u64().expect("minor")
}

/// Whether the archive lists `tables/vendor.json` as a live entry.
fn lists_vendor_entry(bytes: &[u8]) -> bool {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("manifest body");
    let manifest: Value = serde_json::from_slice(&body).expect("manifest json");
    manifest["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .any(|entry| entry["path"].as_str() == Some("tables/vendor.json"))
}

#[test]
fn a_document_with_vendor_rows_writes_the_table_and_minor_twelve() {
    let mut document = snapshot();
    document.tables.set(TableKind::Vendor, vec![vendor_row()]);

    let bytes = write_bytes(&document);
    assert_eq!(
        minor_of(&bytes),
        12,
        "vendor semantics alone earn the vendor tier"
    );
    assert!(
        lists_vendor_entry(&bytes),
        "the table is written when the document has rows"
    );

    let loaded = read_bytes(&bytes);
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::Vendor),
        Some(&[vendor_row()][..]),
        "the opaque rows survive the round trip unchanged"
    );
    // A second save keeps the tier (the table is still there).
    assert_eq!(minor_of(&write_bytes(&loaded.snapshot)), 12);
}

#[test]
fn a_document_without_vendor_rows_keeps_its_plain_minor() {
    let bytes = write_bytes(&snapshot());
    assert_eq!(
        minor_of(&bytes),
        4,
        "no feature earns more than plain minor 4"
    );
    assert!(
        !lists_vendor_entry(&bytes),
        "an empty vendor table must not be written just because the build knows it"
    );

    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.tables.get(TableKind::Vendor).is_none());
}

#[test]
fn a_minor_eleven_reader_refuses_the_vendor_tier() {
    // The vendor archive is stamped 12 (first test). A minor-11 build gates on
    // `manifest.format.minor > 11` **before** decoding any entry, so it refuses
    // with `ymodel.unsupported_minor` instead of silently dropping the rows.
    // This build cannot lower its own ceiling, so the same gate is exercised at
    // its boundary: the vendored archive restamped one above this build's minor.
    let mut document = snapshot();
    document.tables.set(TableKind::Vendor, vec![vendor_row()]);
    let bytes = write_bytes(&document);

    const MINOR_ELEVEN_READER_CEILING: u64 = 11;
    assert_eq!(
        minor_of(&bytes),
        MINOR_ELEVEN_READER_CEILING + 1,
        "the writer stamps the vendor tier so an 11 reader's gate fires"
    );
    assert!(
        u64::from(cad_domain::ymodel::FORMAT_MINOR) > MINOR_ELEVEN_READER_CEILING,
        "this build's ceiling is above the older reader's, as the tier assumes"
    );

    let above = with_manifest_minor(&bytes, u64::from(cad_domain::ymodel::FORMAT_MINOR) + 1);
    let mut source = MemorySource::new(above);
    let error = ZipDocumentStore::new()
        .read(&mut source)
        .expect_err("a newer minor is refused");
    assert_eq!(
        error.marker,
        cad_domain::ymodel::markers::UNSUPPORTED_MINOR,
        "{error}"
    );
}

#[test]
fn a_vendor_table_in_a_lower_stamped_archive_is_kept_and_resaved_at_twelve() {
    // The non-conforming direction: a vendor table under an older minor stamp is
    // accepted (nothing is silently dropped) and the next save re-stamps 12, the
    // same way the BIM contracts handle a minor-5 archive that holds a `bim`
    // block.
    let mut document = snapshot();
    document.tables.set(TableKind::Vendor, vec![vendor_row()]);
    let twelve = write_bytes(&document);

    let loaded = read_bytes(&with_manifest_minor(&twelve, 11));
    assert_eq!(
        loaded.snapshot.tables.get(TableKind::Vendor),
        Some(&[vendor_row()][..])
    );
    assert_eq!(minor_of(&write_bytes(&loaded.snapshot)), 12);
}

/// Restamp `manifest.format` to `read: 2, minor: <minor>`, leaving every entry
/// payload untouched (every fixture here is a v2 archive).
fn with_manifest_minor(bytes: &[u8], minor: u64) -> Vec<u8> {
    with_manifest(bytes, |manifest| {
        manifest["format"]["read"] = Value::from(2);
        manifest["format"]["minor"] = Value::from(minor);
    })
}
