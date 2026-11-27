//! Persisted scenes (`tables/scenes.json`, minor 20): the table write path and
//! its snapshot round trip.
//!
//! The contract mirrors the named-view table: one `SCENE SAVE`/`DELETE` is one
//! revision with only `TableKind::Scenes` dirty; an identical save is a no-op;
//! a malformed or over-cap table never partially applies; the snapshot writes the
//! table only when it holds a row; and undo/redo is the same patch with its sides
//! swapped.

use cad_domain::scene::{markers, Scene, SceneTable, SceneVisualStyle, SCENE_CAP};
use cad_domain::view::{NamedView, ViewCamera, ViewProjection};
use cad_domain::{
    DatabaseId, DocumentIdentity, LayoutId, Point3, Revision, SpaceId, TransactionId, UnitContext,
    WorkPlane,
};

use super::*;

fn camera(scale: f64) -> ViewCamera {
    ViewCamera {
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
        projection: ViewProjection::Orthographic { scale },
    }
}

fn scene(name: &str) -> Scene {
    Scene {
        view: NamedView {
            name: name.into(),
            camera: camera(1.5),
            is_3d: true,
            saved_2d_camera: camera(1.5),
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
        style: SceneVisualStyle::Wireframe,
        tags: None,
        display: None,
        soften: None,
    }
}

fn empty_db() -> DrawingDatabase {
    DrawingDatabaseBuilder::new(DatabaseId(1))
        .finish()
        .expect("an empty database")
}

#[test]
fn a_save_is_one_revision_and_one_scene_table_dirty_mark() {
    let mut db = empty_db();
    let before = db.revision();
    let planned = db
        .plan_save_scene(scene("Overview"))
        .expect("the row is valid")
        .expect("the table did not hold it");
    assert!(!planned.replaced);
    let changes = db
        .apply_scene_patches(
            std::slice::from_ref(&planned.patch),
            TransactionId(1),
            "SCENE SAVE Overview",
        )
        .expect("apply");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, db.revision());
    assert!(changes.changes.is_empty(), "no geometry row moved");
    assert!(changes.scope.tables.contains(&TableKind::Scenes));
    assert!(!changes.scope.header);
    assert_eq!(
        db.scenes().resolve("overview").map(|scene| scene.name()),
        Some("Overview"),
        "resolution is case-insensitive"
    );
    assert_eq!(
        db.scenes().resolve("Overview").unwrap().style,
        SceneVisualStyle::Wireframe
    );
}

#[test]
fn an_identical_save_is_a_no_op_and_a_delete_restores_through_undo() {
    let mut db = empty_db();
    let planned = db.plan_save_scene(scene("A")).unwrap().unwrap();
    db.apply_scene_patches(
        std::slice::from_ref(&planned.patch),
        TransactionId(1),
        "save",
    )
    .unwrap();
    let revision = db.revision();
    assert_eq!(db.plan_save_scene(scene("A")).unwrap(), None);
    assert_eq!(db.revision(), revision, "an identical save writes nothing");

    let delete = db.plan_delete_scene("a").unwrap();
    db.apply_scene_patches(
        std::slice::from_ref(&delete.patch),
        TransactionId(2),
        "delete",
    )
    .unwrap();
    assert!(db.scenes().is_empty());

    let undo = delete.patch.inverse();
    db.apply_scene_patches(std::slice::from_ref(&undo), TransactionId(3), "undo")
        .unwrap();
    assert_eq!(db.scenes().resolve("A").unwrap(), &scene("A"));
}

#[test]
fn a_refused_delete_or_an_over_cap_save_changes_nothing() {
    let db = empty_db();
    let start = db.revision();
    let error = db.plan_delete_scene("missing").unwrap_err();
    assert!(format!("{error:?}").contains(markers::SCENE_UNKNOWN));

    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(2));
    let mut table = SceneTable::new();
    for index in 0..SCENE_CAP {
        table.save(scene(&format!("S{index}"))).unwrap();
    }
    builder.set_scenes(table).unwrap();
    let full = builder.finish().unwrap();
    let full_revision = full.revision();
    let error = full.plan_save_scene(scene("OVER")).unwrap_err();
    assert!(format!("{error:?}").contains(markers::SCENE_CAP_EXCEEDED));
    assert_eq!(full.revision(), full_revision);
    assert_eq!(full.scenes().len(), SCENE_CAP);
    assert_eq!(db.revision(), start, "the failed delete wrote nothing");
}

#[test]
fn the_snapshot_writes_the_table_only_when_it_holds_a_row_and_reads_it_back() {
    let mut db = empty_db();
    let planned = db.plan_save_scene(scene("Overview")).unwrap().unwrap();
    db.apply_scene_patches(
        std::slice::from_ref(&planned.patch),
        TransactionId(1),
        "save",
    )
    .unwrap();
    let mut paper = scene("Sheet");
    paper.view.space = SpaceId::Paper(LayoutId(3));
    paper.style = SceneVisualStyle::Shaded;
    let planned = db.plan_save_scene(paper).unwrap().unwrap();
    db.apply_scene_patches(
        std::slice::from_ref(&planned.patch),
        TransactionId(2),
        "save paper",
    )
    .unwrap();

    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            db.revision(),
        )
        .unwrap();
    assert!(snapshot.has_scenes());
    let rows = snapshot.tables.get(TableKind::Scenes).expect("table entry");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["style"], serde_json::Value::from("WIREFRAME"));
    assert_eq!(rows[1]["style"], serde_json::Value::from("SHADED"));

    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    assert_eq!(loaded.scenes(), db.scenes());
    assert_eq!(
        loaded.scenes().resolve("sheet").unwrap().view.space,
        SpaceId::Paper(LayoutId(3))
    );
}

#[test]
fn a_document_without_the_table_loads_with_no_scenes() {
    let db = empty_db();
    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(7),
        )
        .unwrap();
    assert!(!snapshot.has_scenes());
    assert!(snapshot.tables.get(TableKind::Scenes).is_none());
    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    assert!(loaded.scenes().is_empty());
}

#[test]
fn a_duplicate_name_or_unknown_style_refuses_the_load() {
    let db = empty_db();
    let mut snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(1),
        )
        .unwrap();
    snapshot.tables.set(
        TableKind::Scenes,
        vec![scene("A").to_json(), scene("a").to_json()],
    );
    let error = DrawingDatabase::from_snapshot(&snapshot).unwrap_err();
    assert!(
        format!("{error:?}").contains(markers::SCENE_DUPLICATE_NAME),
        "{error:?}"
    );

    let mut row = scene("A").to_json();
    row["style"] = serde_json::Value::from("REALISTIC");
    snapshot.tables.set(TableKind::Scenes, vec![row]);
    let error = DrawingDatabase::from_snapshot(&snapshot).unwrap_err();
    assert!(
        format!("{error:?}").contains(markers::SCENE_STYLE_INVALID),
        "{error:?}"
    );
}
