//! The per-face material tier (`.ymodel` minor 25): a material row with a
//! non-empty `faces` list is stamped 25; a whole-entity-only table keeps 19.

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

fn row(faces: Option<Value>) -> Value {
    let mut row = serde_json::json!({
        "seq": 1,
        "name": "Red",
        "color": [178, 34, 34],
        "entities": []
    });
    if let Some(faces) = faces {
        row["faces"] = faces;
    }
    row
}

fn snapshot(faces: Option<Value>) -> DocumentSnapshot {
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
    document.tables.set(TableKind::Materials, vec![row(faces)]);
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

#[test]
fn a_face_paint_is_written_at_minor_twenty_five_and_reads_back() {
    let faces = serde_json::json!([{ "entity": 1025, "face": "skp:face:0" }]);
    let bytes = write_bytes(&snapshot(Some(faces)));
    assert_eq!(minor_of(&bytes), 25, "a face paint is the minor-25 tier");
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_face_materials());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Materials)
        .expect("the material table");
    assert_eq!(rows[0]["faces"][0]["face"], Value::from("skp:face:0"));
}

#[test]
fn an_entity_only_material_row_keeps_the_minor_nineteen_tier() {
    let bytes = write_bytes(&snapshot(None));
    assert_eq!(minor_of(&bytes), 19);
    assert!(!read_bytes(&bytes).snapshot.has_face_materials());
}
