//! Persisted construction guides (`tables/guides.json`, minor 26): the table
//! write path, undo patch and snapshot round trip.

use cad_domain::guide::{GuideLine, GuideTable, GUIDE_CAP};
use cad_domain::{DatabaseId, DocumentIdentity, Point3, Revision, UnitContext};

use super::*;

fn guide(x: f64) -> GuideLine {
    GuideLine::new(
        Point3 { x, y: 0.0, z: 0.0 },
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
    )
    .expect("a valid guide")
}

fn snapshot(db: &DrawingDatabase) -> DocumentSnapshot {
    db.snapshot(
        UnitContext::drawing_units(),
        DocumentIdentity::Temporary(1),
        Vec::new(),
        db.revision(),
    )
    .expect("snapshot")
}

#[test]
fn a_guide_write_is_one_revision_and_reads_back() {
    let mut db = DrawingDatabaseBuilder::new(DatabaseId(1))
        .finish()
        .expect("empty database");
    let before = db.revision();
    let mut next = GuideTable::new();
    next.push(guide(0.0)).unwrap();
    let changes = db
        .apply_guides(next.clone(), TransactionId(1), "GUIDE")
        .expect("write");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert!(changes.scope.tables.contains(&TableKind::Guides));
    assert_eq!(db.guides(), &next);

    let snapshot = snapshot(&db);
    assert!(snapshot.has_guides());
    let rows = snapshot.tables.get(TableKind::Guides).expect("the table");
    assert_eq!(rows[0]["point"]["x"], serde_json::json!(0.0));
    let loaded = DrawingDatabase::from_snapshot(&snapshot).expect("reopen");
    assert_eq!(loaded.guides(), &next);
}

#[test]
fn the_cap_and_clear_are_named() {
    let mut table = GuideTable::new();
    for index in 0..GUIDE_CAP {
        table.push(guide(index as f64)).unwrap();
    }
    let error = table.push(guide(999.0)).unwrap_err();
    assert_eq!(error.marker(), "guide.cap_exceeded");
    table.clear();
    assert!(table.is_empty());
}

#[test]
fn a_zero_direction_is_refused() {
    let error = GuideLine::through(
        Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        },
        Point3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        },
    );
    assert!(error.is_none());
}
