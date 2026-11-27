//! The guide tier (`.ymodel` minor 26): a document with `tables/guides.json` is
//! stamped 26; an ordinary document keeps its lower minor.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, TableKind};
use cad_domain::ymodel::{DocumentHeader, IdCounters};
use cad_domain::{DatabaseId, DocumentIdentity, Revision, Unit, UnitContext};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

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

fn snapshot(with_guides: bool) -> DocumentSnapshot {
    points_snapshot(with_guides, false)
}

fn points_snapshot(with_guides: bool, points: bool) -> DocumentSnapshot {
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
    if with_guides {
        let row = if points {
            serde_json::json!({
                "point": { "x": 0.0, "y": 0.0, "z": 0.0 }
            })
        } else {
            serde_json::json!({
                "point": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "direction": { "x": 1.0, "y": 0.0, "z": 0.0 }
            })
        };
        document.tables.set(TableKind::Guides, vec![row]);
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

fn minor_of(bytes: &[u8]) -> u64 {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("manifest body");
    let manifest: Value = serde_json::from_slice(&body).expect("manifest json");
    manifest["format"]["minor"].as_u64().expect("minor")
}

#[test]
fn a_guide_table_is_written_at_minor_twenty_six_and_reads_back() {
    let bytes = write_bytes(&snapshot(true));
    assert_eq!(minor_of(&bytes), 26, "a guide row is the minor-26 tier");
    let mut source = MemorySource::new(bytes);
    let loaded = ZipDocumentStore::new().read(&mut source).expect("read");
    assert!(loaded.snapshot.has_guides());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Guides)
        .expect("the guide table");
    assert_eq!(rows[0]["direction"]["x"], Value::from(1.0));
}

#[test]
fn a_guide_point_is_written_at_minor_twenty_seven() {
    let bytes = write_bytes(&points_snapshot(true, true));
    assert_eq!(minor_of(&bytes), 27, "a point row is the minor-27 tier");
    let mut source = MemorySource::new(bytes);
    let loaded = ZipDocumentStore::new().read(&mut source).expect("read");
    assert!(loaded.snapshot.has_guide_points());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Guides)
        .expect("the guide table");
    assert!(rows[0].get("direction").is_none());
}

#[test]
fn a_document_without_guides_keeps_its_lower_minor() {
    let bytes = write_bytes(&snapshot(false));
    assert_eq!(minor_of(&bytes), 4);
}
