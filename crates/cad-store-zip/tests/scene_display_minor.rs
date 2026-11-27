//! The scene display tier (`.ymodel` minor 24): a scene row that captures the
//! document Shadows/Fog record is stamped 24, reads it back, and a display-free
//! scene keeps its lower tier.

mod common;

use std::io::{Cursor, Read};

use serde_json::Value;
use zip::ZipArchive;

use cad_db::{ChangeScope, DocumentSnapshot, DocumentStore, MemorySink, MemorySource, TableKind};
use cad_domain::display::{DisplaySettings, FogSettings, ShadowSettings};
use cad_domain::scene::{Scene, SceneVisualStyle};
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

fn display() -> DisplaySettings {
    DisplaySettings {
        shadows: ShadowSettings {
            enabled: true,
            azimuth: 210.0,
            elevation: 30.0,
        },
        fog: FogSettings {
            enabled: true,
            start: 50.0,
            end: 900.0,
            color: [10, 20, 30],
        },
    }
}

fn scene_row(display: Option<DisplaySettings>) -> Value {
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
        display,
        soften: None,
    }
    .to_json()
}

fn snapshot(display: Option<DisplaySettings>) -> DocumentSnapshot {
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
        .set(TableKind::Scenes, vec![scene_row(display)]);
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
fn a_scene_with_display_is_written_at_minor_twenty_four_and_reads_back() {
    let bytes = write_bytes(&snapshot(Some(display())));
    assert_eq!(
        minor_of(&bytes),
        24,
        "a display snapshot is the minor-24 tier"
    );
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_scene_display());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Scenes)
        .expect("table entry");
    assert_eq!(rows[0]["display"]["shadows"]["enabled"], Value::Bool(true));
    assert_eq!(rows[0]["display"]["fog"]["end"], Value::from(900.0));
    let table = cad_domain::scene::SceneTable::from_json(rows).expect("the row decodes");
    assert_eq!(table.scenes().next().unwrap().display, Some(display()));
}

#[test]
fn a_display_free_scene_keeps_its_lower_minor() {
    let bytes = write_bytes(&snapshot(None));
    assert_eq!(minor_of(&bytes), 20);
    assert!(!read_bytes(&bytes).snapshot.has_scene_display());
}

#[test]
fn an_out_of_range_display_record_is_a_named_refusal() {
    let mut row = scene_row(None);
    row["display"] = serde_json::json!({
        "shadows": { "enabled": false, "azimuth": 135.0, "elevation": 45.0 },
        "fog": { "enabled": true, "start": 900.0, "end": 50.0, "color": [1, 2, 3] }
    });
    let error =
        cad_domain::scene::Scene::from_json(&row).expect_err("fog end before start is refused");
    assert_eq!(error.marker(), "scene.display_invalid");
}

#[test]
fn a_scene_soften_snapshot_is_written_at_minor_twenty_eight() {
    let mut snapshot = snapshot(None);
    let mut row = scene_row(None);
    row["soften"] = serde_json::json!([{ "entity": 1025, "angle": 30.0 }]);
    snapshot.tables.set(TableKind::Scenes, vec![row]);
    let bytes = write_bytes(&snapshot);
    assert_eq!(minor_of(&bytes), 28, "soften is the minor-28 tier");
    let loaded = read_bytes(&bytes);
    assert!(loaded.snapshot.has_scene_soften());
    let rows = loaded
        .snapshot
        .tables
        .get(TableKind::Scenes)
        .expect("the scene table");
    let table = cad_domain::scene::SceneTable::from_json(rows).expect("decode");
    let scene = table.scenes().next().expect("one scene");
    let soften = scene.soften.as_ref().expect("captured");
    assert_eq!(soften.len(), 1);
    assert_eq!(soften[0].entity.0, 1025);
    assert!((soften[0].angle - 30.0).abs() < 1e-12);
}
