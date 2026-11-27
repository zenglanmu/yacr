//! The display tier (`.ymodel` minor 22): a document with a
//! `tables/display.json` record is stamped 22; an ordinary document keeps its
//! lower minor, and a pre-22 archive still opens with the default record.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, TableKind};
use cad_domain::display::{DisplaySettings, FogSettings, ShadowSettings};
use cad_domain::ymodel::{DocumentHeader, IdCounters};
use cad_domain::{DatabaseId, DocumentIdentity, Revision, Unit, UnitContext};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

use common::with_manifest;

fn header() -> DocumentHeader {
    DocumentHeader {
        database: DatabaseId(10),
        units: UnitContext {
            source: Unit::Millimeter,
            display: Unit::Millimeter,
            display_per_source: Some(1.0),
            decimal_places: 2,
        },
        identity: DocumentIdentity::Temporary(5),
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

fn display_row() -> Value {
    DisplaySettings {
        shadows: ShadowSettings {
            enabled: true,
            azimuth: 90.0,
            elevation: 30.0,
        },
        fog: FogSettings {
            enabled: true,
            start: 5.0,
            end: 50.0,
            color: [9, 8, 7],
        },
    }
    .to_json()
}

fn snapshot(with_display: bool) -> DocumentSnapshot {
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
    if with_display {
        document.tables.set(TableKind::Display, vec![display_row()]);
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
fn a_display_record_is_written_at_minor_twenty_two_and_reads_back() {
    let bytes = write_bytes(&snapshot(true));
    assert_eq!(
        minor_of(&bytes),
        22,
        "a display record is the minor-22 tier"
    );
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_display_settings());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Display)
        .expect("table entry");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["shadows"]["enabled"], Value::from(true));
    assert_eq!(rows[0]["fog"]["color"], serde_json::json!([9, 8, 7]));
}

#[test]
fn a_document_without_display_settings_keeps_the_plain_minor() {
    let bytes = write_bytes(&snapshot(false));
    assert_eq!(minor_of(&bytes), 4);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_display_settings());
    assert!(loaded.snapshot.tables.get(TableKind::Display).is_none());
}

#[test]
fn a_minor_twenty_one_archive_still_opens() {
    let bytes = with_minor(&write_bytes(&snapshot(false)), 21);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_display_settings());
    assert_eq!(minor_of(&bytes), 21);
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
