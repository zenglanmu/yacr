//! The document section plane (`tables/sections.json`, minor 32).

use cad_domain::section::SectionPlane;
use cad_domain::{DatabaseId, DocumentIdentity, Point3, Revision, TransactionId, UnitContext};

use super::*;

fn plane(active: bool) -> SectionPlane {
    SectionPlane::new(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 2.0,
        },
    )
    .unwrap()
    .with_active(active)
}

#[test]
fn a_section_write_is_one_revision_and_one_table_dirty_mark() {
    let mut db = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
    let before = db.revision();
    let changes = db
        .apply_section_plane(Some(plane(true)), TransactionId(1), "SECTIONPLANE SET")
        .expect("apply");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert!(changes.scope.tables.contains(&TableKind::Sections));
    assert!(!changes.scope.header);
    let stored = db.section_plane().expect("stored");
    assert!(stored.active);
    assert!((stored.normal.z - 1.0).abs() < 1.0e-9);
}

#[test]
fn an_identical_write_is_refused_and_undo_swaps_the_slot() {
    let mut db = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
    let patch = crate::SectionPatch {
        before: None,
        after: Some(plane(false)),
    };
    db.apply_section_patches(std::slice::from_ref(&patch), TransactionId(1), "set")
        .unwrap();
    assert_eq!(db.section_plane(), Some(plane(false)));
    let revision = db.revision();
    db.apply_section_patches(std::slice::from_ref(&patch), TransactionId(2), "same")
        .expect_err("the slot is already this plane");
    assert_eq!(db.revision(), revision);
    db.apply_section_patches(
        std::slice::from_ref(&patch.inverse()),
        TransactionId(3),
        "undo",
    )
    .unwrap();
    assert_eq!(db.section_plane(), None);
}

#[test]
fn the_snapshot_round_trips_the_plane_and_an_empty_document_has_no_table() {
    let mut db = DrawingDatabaseBuilder::new(DatabaseId(1)).finish().unwrap();
    let empty = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            db.revision(),
        )
        .unwrap();
    assert!(!empty.has_sections());
    assert!(empty.tables.get(TableKind::Sections).is_none());

    db.apply_section_plane(Some(plane(true)), TransactionId(1), "set")
        .unwrap();
    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            db.revision(),
        )
        .unwrap();
    assert!(snapshot.has_sections());
    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    assert_eq!(loaded.section_plane(), db.section_plane());
}
