//! The material tier (`.ymodel` minor 19): a document with a
//! `tables/materials.json` row is stamped 19; an ordinary document keeps its
//! lower minor, and a pre-19 archive still opens with no paint.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, TableKind};
use cad_domain::ymodel::{DocumentHeader, IdCounters};
use cad_domain::{DatabaseId, DocumentIdentity, Revision, Unit, UnitContext};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

use common::with_manifest;

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(9),
        units: UnitContext {
            source: Unit::Millimeter,
            display: Unit::Millimeter,
            display_per_source: Some(1.0),
            decimal_places: 2,
        },
        identity: DocumentIdentity::Temporary(4),
        resource_keys: Vec::new(),
        embed_resources: false,
        application_domain: None,
        variables: serde_json::json!({}),
        next: IdCounters {
            entity: 1,
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

fn snapshot(with_materials: bool) -> DocumentSnapshot {
    let mut document = DocumentSnapshot {
        header: header(),
        revision: Revision(1),
        models: Vec::new(),
        shards: Vec::new(),
        tables: Default::default(),
        textures: Default::default(),
        resources: Default::default(),
        resource_names: Default::default(),
        resource_sources: Default::default(),
    };
    if with_materials {
        document.tables.set(
            TableKind::Materials,
            vec![serde_json::json!({
                "seq": 1,
                "name": "Red",
                "color": [178, 34, 34],
                "entities": [1025]
            })],
        );
    }
    document
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

fn minor_of(bytes: &[u8]) -> u64 {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("manifest body");
    let manifest: Value = serde_json::from_slice(&body).expect("manifest json");
    manifest["format"]["minor"].as_u64().expect("minor")
}

fn with_minor(bytes: &[u8], minor: u64) -> Vec<u8> {
    with_manifest(bytes, |manifest| {
        manifest["format"]["read"] = Value::from(2);
        manifest["format"]["minor"] = Value::from(minor);
    })
}

#[test]
fn a_material_table_is_written_at_minor_nineteen_and_reads_back() {
    let bytes = write_bytes(&snapshot(true));
    assert_eq!(minor_of(&bytes), 19, "a material row is the minor-19 tier");
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_materials());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Materials)
        .expect("table entry");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], Value::from("Red"));
    assert_eq!(rows[0]["color"], serde_json::json!([178, 34, 34]));
}

#[test]
fn a_document_without_materials_keeps_the_plain_minor() {
    let bytes = write_bytes(&snapshot(false));
    assert_eq!(minor_of(&bytes), 4);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_materials());
    assert!(loaded.snapshot.tables.get(TableKind::Materials).is_none());
}

#[test]
fn a_minor_eighteen_archive_still_opens() {
    let bytes = with_minor(&write_bytes(&snapshot(false)), 18);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_materials());
    assert_eq!(minor_of(&bytes), 18);
}

#[test]
fn a_minor_above_the_ceiling_is_refused() {
    let bytes = with_minor(
        &write_bytes(&snapshot(true)),
        u64::from(cad_domain::ymodel::FORMAT_MINOR) + 1,
    );
    let mut source = MemorySource::new(bytes);
    let error = ZipDocumentStore::new()
        .read(&mut source)
        .expect_err("a newer minor is refused");
    assert_eq!(
        error.marker,
        cad_domain::ymodel::markers::UNSUPPORTED_MINOR,
        "{error}"
    );
}
