//! The scene tier (`.ymodel` minor 20): a document with a `tables/scenes.json`
//! row is stamped 20; an ordinary document keeps its lower minor, and a pre-20
//! archive still opens with no scenes.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, TableKind};
use cad_domain::scene::{Scene, SceneVisualStyle};
use cad_domain::view::{NamedView, ViewCamera, ViewProjection};
use cad_domain::ymodel::{DocumentHeader, IdCounters};
use cad_domain::{
    DatabaseId, DocumentIdentity, Point3, Revision, SpaceId, Unit, UnitContext, WorkPlane,
};
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

fn scene_row() -> Value {
    let camera = ViewCamera {
        eye: Point3 {
            x: 0.0,
            y: 0.0,
            z: 10.0,
        },
        target: Point3 {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        },
        up: Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        projection: ViewProjection::Orthographic { scale: 2.0 },
    };
    Scene {
        view: NamedView {
            name: "Overview".into(),
            camera,
            is_3d: true,
            saved_2d_camera: camera,
            work_plane: WorkPlane {
                origin: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                u: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                v: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            },
            space: SpaceId::Model,
        },
        style: SceneVisualStyle::Shaded,
        tags: None,
        display: None,
        soften: None,
    }
    .to_json()
}

fn snapshot(with_scenes: bool) -> DocumentSnapshot {
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
    if with_scenes {
        document.tables.set(TableKind::Scenes, vec![scene_row()]);
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
fn a_scene_table_is_written_at_minor_twenty_and_reads_back() {
    let bytes = write_bytes(&snapshot(true));
    assert_eq!(minor_of(&bytes), 20, "a scene row is the minor-20 tier");
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_scenes());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Scenes)
        .expect("table entry");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], Value::from("Overview"));
    assert_eq!(rows[0]["style"], Value::from("SHADED"));
}

#[test]
fn a_document_without_scenes_keeps_the_plain_minor() {
    let bytes = write_bytes(&snapshot(false));
    assert_eq!(minor_of(&bytes), 4);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_scenes());
    assert!(loaded.snapshot.tables.get(TableKind::Scenes).is_none());
}

#[test]
fn a_minor_nineteen_archive_still_opens() {
    let bytes = with_minor(&write_bytes(&snapshot(false)), 19);
    let loaded = read_bytes(&bytes);
    assert!(!loaded.snapshot.has_scenes());
    assert_eq!(minor_of(&bytes), 19);
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
