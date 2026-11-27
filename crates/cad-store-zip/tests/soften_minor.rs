//! The soften tier (`.ymodel` minor 21): an element with
//! `render_attributes.soften` is stamped 21; an ordinary document keeps its
//! lower minor, and a pre-21 archive still opens.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{
    ChangeScope, DbEntity, DbObject, DocumentSnapshot, DocumentStore, DrawingDatabase,
    DrawingDatabaseBuilder, EntityRenderAttributes, Layer, MemorySink, MemorySource,
};
use cad_domain::{
    DatabaseId, DocumentIdentity, EntityId, LayerId, ObjectId, Point3, Revision, SemanticGeometry,
    SpaceId, UnitContext,
};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

use common::with_manifest;

fn line_entity(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
        draw_order: id as i64,
    }
}

fn database(soften: Option<f64>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.insert_entity(line_entity(7)).unwrap();
    if let Some(angle) = soften {
        builder
            .set_entity_render_attributes(
                EntityId(7),
                EntityRenderAttributes {
                    soften: Some(angle),
                    ..EntityRenderAttributes::default()
                },
            )
            .unwrap();
    }
    builder.finish().unwrap()
}

fn snapshot(soften: Option<f64>) -> DocumentSnapshot {
    let db = database(soften);
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        db.revision(),
    )
    .unwrap()
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
fn a_soften_angle_is_written_at_minor_twenty_one_and_reads_back() {
    let bytes = write_bytes(&snapshot(Some(20.0)));
    assert_eq!(minor_of(&bytes), 21, "a soften angle is the minor-21 tier");
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_soften());
    let reopened = DrawingDatabase::from_snapshot(&loaded.snapshot).expect("reopen");
    assert_eq!(
        reopened.entity_render_attributes(EntityId(7)).soften,
        Some(20.0)
    );
}

#[test]
fn a_document_without_soften_keeps_the_plain_minor_and_omits_the_key() {
    let bytes = write_bytes(&snapshot(None));
    assert_eq!(minor_of(&bytes), 4);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_soften());
}

#[test]
fn a_minor_twenty_archive_still_opens() {
    let bytes = with_minor(&write_bytes(&snapshot(None)), 20);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_soften());
}

#[test]
fn a_minor_above_the_ceiling_is_refused() {
    let bytes = with_minor(
        &write_bytes(&snapshot(Some(20.0))),
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
