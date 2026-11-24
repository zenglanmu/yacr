//! Persisted entity groups: the `tables/groups.json` tier and round trip.
//!
//! The table is document data, so the format contract is two-fold:
//!
//! * a document that carries a group stamps minor **17** (15 is mirrored text,
//!   16 is the 2D constraint table), and a document without one stays at its
//!   lower tier;
//! * the row (name, description, selectable flag and ordered member ids) comes
//!   back byte-faithfully through a real archive and `from_snapshot`.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{
    ChangeScope, DocumentSnapshot, DocumentStore, DrawingDatabaseBuilder, Layer, MemorySink,
    MemorySource, TableKind,
};
use cad_domain::ymodel::PersistentIdOf;
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, Point3, Revision, SemanticGeometry,
    UnitContext,
};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

fn snapshot() -> DocumentSnapshot {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(9));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_entity(cad_db::DbEntity {
            object: cad_db::DbObject {
                id: cad_domain::ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: cad_domain::SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3::default(),
                end: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
    let database = builder.finish().unwrap();
    database
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(2),
        )
        .expect("encode the snapshot")
}

fn group_row(seq: u64, name: &str, members: &[u128]) -> Value {
    serde_json::json!({
        "seq": seq,
        "name": name,
        "description": "exterior walls",
        "selectable": true,
        "members": members.iter().map(|id| EntityId(*id).persistent().raw()).collect::<Vec<_>>(),
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

fn minor_of(bytes: &[u8]) -> u64 {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).expect("zip");
    let mut file = archive.by_name(MANIFEST_ENTRY).expect("manifest");
    let mut body = Vec::new();
    file.read_to_end(&mut body).expect("manifest body");
    let manifest: Value = serde_json::from_slice(&body).expect("manifest json");
    manifest["format"]["minor"].as_u64().expect("minor")
}

#[test]
fn a_document_without_groups_keeps_its_lower_minor() {
    let bytes = write_bytes(&snapshot());
    assert_eq!(minor_of(&bytes), 4);
    assert!(!read_bytes(&bytes).snapshot.has_groups());
}

#[test]
fn a_group_document_is_minor_sixteen_and_round_trips() {
    let mut document = snapshot();
    document
        .tables
        .set(TableKind::Groups, vec![group_row(1, "Walls", &[1])]);

    let bytes = write_bytes(&document);
    assert_eq!(
        minor_of(&bytes),
        17,
        "15 is mirrored text, 16 is constraints"
    );
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_groups());

    let database = cad_db::DrawingDatabase::from_snapshot(&loaded.snapshot).expect("hydrate");
    let groups: Vec<_> = database.groups().collect();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "Walls");
    assert_eq!(groups[0].description, "exterior walls");
    assert!(groups[0].selectable);
    assert_eq!(groups[0].members, vec![EntityId(1)]);
}

#[test]
fn a_dangling_member_refuses_the_load() {
    let mut document = snapshot();
    document
        .tables
        .set(TableKind::Groups, vec![group_row(1, "Walls", &[1, 77])]);
    let bytes = write_bytes(&document);
    let loaded = read_bytes(&bytes);
    let error = cad_db::DrawingDatabase::from_snapshot(&loaded.snapshot)
        .expect_err("a dangling group member is a malformed document");
    assert!(format!("{error}").contains("group.stale_ref"), "{error}");
}
