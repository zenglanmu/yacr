//! The scene tag tier (`.ymodel` minor 23): a scene row that captures visible
//! tags is stamped 23, reads its names/visibility back, and a tag-free scene
//! keeps the minor-20 tier.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, TableKind};
use cad_domain::scene::{Scene, SceneTag, SceneVisualStyle};
use cad_domain::view::{NamedView, ViewCamera, ViewProjection};
use cad_domain::ymodel::{DocumentHeader, IdCounters};
use cad_domain::{
    DatabaseId, DocumentIdentity, Point3, Revision, SpaceId, Unit, UnitContext, WorkPlane,
};
use cad_store_zip::{ZipDocumentStore, MANIFEST_ENTRY};

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

fn scene_row(tags: Option<Vec<SceneTag>>) -> Value {
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
        tags,
        display: None,
    }
    .to_json()
}

fn snapshot(tags: Option<Vec<SceneTag>>) -> DocumentSnapshot {
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
    document
        .tables
        .set(TableKind::Scenes, vec![scene_row(tags)]);
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
fn a_scene_with_tags_is_written_at_minor_twenty_three_and_reads_back() {
    let tags = vec![
        SceneTag {
            name: "Layer 0".into(),
            visible: true,
        },
        SceneTag {
            name: "Walls".into(),
            visible: false,
        },
    ];
    let bytes = write_bytes(&snapshot(Some(tags.clone())));
    assert_eq!(minor_of(&bytes), 23, "a tag snapshot is the minor-23 tier");
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_scenes());
    assert!(loaded.snapshot.has_scene_tags());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Scenes)
        .expect("table entry");
    assert_eq!(rows[0]["tags"][0]["name"], Value::from("Layer 0"));
    assert_eq!(rows[0]["tags"][0]["visible"], Value::Bool(true));
    assert_eq!(rows[0]["tags"][1]["name"], Value::from("Walls"));
    assert_eq!(rows[0]["tags"][1]["visible"], Value::Bool(false));
    // Decode through the domain codec, not just the raw JSON.
    let table = cad_domain::scene::SceneTable::from_json(rows).expect("the row decodes");
    assert_eq!(table.scenes().next().unwrap().tags.as_ref(), Some(&tags));
}

#[test]
fn a_tag_free_scene_keeps_the_minor_twenty_tier() {
    let bytes = write_bytes(&snapshot(None));
    assert_eq!(minor_of(&bytes), 20);
    assert!(!read_bytes(&bytes).snapshot.has_scene_tags());
}

#[test]
fn a_malformed_tag_row_is_a_named_refusal() {
    let mut row = scene_row(None);
    row["tags"] = serde_json::json!([{ "name": "Walls" }]);
    let error =
        cad_domain::scene::Scene::from_json(&row).expect_err("a tag without visibility is refused");
    assert_eq!(error.marker(), "scene.tags_invalid");
}
