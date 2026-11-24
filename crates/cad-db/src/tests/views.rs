//! Persisted named views (`tables/views.json`, minor 18): the table write path
//! and its snapshot round trip.
//!
//! The contract: one `VIEW SAVE`/`DELETE` is one revision with only
//! `TableKind::Views` dirty; an identical save is a no-op; a malformed or
//! over-cap table is never partially applied; the snapshot encodes the table only
//! when it holds a row; and undo/redo is the same patch with its sides swapped.

use cad_domain::view::{markers, NamedView, NamedViewTable, ViewCamera, ViewProjection};
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

fn view(name: &str) -> NamedView {
    NamedView {
        name: name.into(),
        camera: camera(1.5),
        is_3d: false,
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
    }
}

fn empty_db() -> DrawingDatabase {
    DrawingDatabaseBuilder::new(DatabaseId(1))
        .finish()
        .expect("an empty database")
}

#[test]
fn a_save_is_one_revision_and_one_view_table_dirty_mark() {
    let mut db = empty_db();
    let before = db.revision();
    let planned = db
        .plan_save_named_view(view("Plan A"))
        .expect("the row is valid")
        .expect("the table did not hold it");
    assert!(!planned.replaced);
    let changes = db
        .apply_named_view_patches(
            std::slice::from_ref(&planned.patch),
            TransactionId(1),
            "VIEW SAVE Plan A",
        )
        .expect("apply");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, db.revision());
    assert!(changes.changes.is_empty(), "no geometry row moved");
    assert!(changes.scope.tables.contains(&TableKind::Views));
    assert!(!changes.scope.header);
    assert_eq!(
        db.named_views()
            .resolve("plan a")
            .map(|view| view.name.as_str()),
        Some("Plan A"),
        "resolution is case-insensitive"
    );
}

#[test]
fn an_identical_save_is_a_no_op_and_a_delete_restores_through_undo() {
    let mut db = empty_db();
    let planned = db.plan_save_named_view(view("A")).unwrap().unwrap();
    db.apply_named_view_patches(
        std::slice::from_ref(&planned.patch),
        TransactionId(1),
        "save",
    )
    .unwrap();
    let revision = db.revision();
    assert_eq!(db.plan_save_named_view(view("A")).unwrap(), None);
    assert_eq!(db.revision(), revision, "an identical save writes nothing");

    let delete = db.plan_delete_named_view("a").unwrap();
    db.apply_named_view_patches(
        std::slice::from_ref(&delete.patch),
        TransactionId(2),
        "delete",
    )
    .unwrap();
    assert!(db.named_views().is_empty());

    // Undo passes the swapped sides; the row comes back verbatim.
    let undo = ViewPatch {
        name: delete.patch.name.clone(),
        before: delete.patch.after.clone(),
        after: delete.patch.before.clone(),
    };
    db.apply_named_view_patches(std::slice::from_ref(&undo), TransactionId(3), "undo")
        .unwrap();
    assert_eq!(db.named_views().resolve("A").unwrap(), &view("A"));
}

#[test]
fn a_refused_delete_or_an_over_cap_save_changes_nothing() {
    let db = empty_db();
    let start = db.revision();
    let error = db.plan_delete_named_view("missing").unwrap_err();
    assert!(format!("{error:?}").contains("view.unknown"));

    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(2));
    let mut table = NamedViewTable::new();
    for index in 0..cad_domain::NAMED_VIEW_CAP {
        table.save(view(&format!("V{index}"))).unwrap();
    }
    builder.set_named_views(table).unwrap();
    let full = builder.finish().unwrap();
    let full_revision = full.revision();
    let error = full.plan_save_named_view(view("OVER")).unwrap_err();
    assert!(format!("{error:?}").contains(markers::VIEW_CAP_EXCEEDED));
    assert_eq!(full.revision(), full_revision);
    assert_eq!(full.named_views().len(), cad_domain::NAMED_VIEW_CAP);

    assert_eq!(db.revision(), start, "the failed delete wrote nothing");
}

#[test]
fn the_snapshot_writes_the_table_only_when_it_holds_a_row_and_reads_it_back() {
    let mut db = empty_db();
    let planned = db.plan_save_named_view(view("Plan A")).unwrap().unwrap();
    db.apply_named_view_patches(
        std::slice::from_ref(&planned.patch),
        TransactionId(1),
        "save",
    )
    .unwrap();
    let mut paper = view("Paper");
    paper.space = SpaceId::Paper(LayoutId(3));
    paper.is_3d = true;
    let planned = db.plan_save_named_view(paper).unwrap().unwrap();
    db.apply_named_view_patches(
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
    assert!(snapshot.has_named_views());
    let rows = snapshot.tables.get(TableKind::Views).expect("table entry");
    assert_eq!(rows.len(), 2);

    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    assert_eq!(loaded.named_views(), db.named_views());
    assert_eq!(
        loaded.named_views().resolve("paper").unwrap().space,
        SpaceId::Paper(LayoutId(3))
    );
}

#[test]
fn a_document_without_the_table_loads_with_no_views() {
    let db = empty_db();
    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            Revision(7),
        )
        .unwrap();
    assert!(!snapshot.has_named_views());
    assert!(snapshot.tables.get(TableKind::Views).is_none());
    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    assert!(loaded.named_views().is_empty());
}

#[test]
fn a_duplicate_name_in_the_table_refuses_the_load() {
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
        TableKind::Views,
        vec![view("A").to_json(), view("a").to_json()],
    );
    let error = DrawingDatabase::from_snapshot(&snapshot).expect_err("duplicate names");
    assert!(format!("{error:?}").contains(markers::VIEW_DUPLICATE_NAME));
}
