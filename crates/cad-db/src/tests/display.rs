//! Per-document display settings (`tables/display.json`, minor 22): the write
//! path and its snapshot round trip.
//!
//! One write replaces the whole record as **one** revision with only
//! `TableKind::Display` dirty; an identical write is refused as a no-op; the
//! snapshot writes the table only when the record differs from the default; and
//! undo replays the same patch with its sides swapped.

use cad_domain::display::{DisplaySettings, FogSettings, ShadowSettings};
use cad_domain::{DatabaseId, DocumentIdentity, Revision, TransactionId, UnitContext};

use super::*;

fn empty_db() -> DrawingDatabase {
    DrawingDatabaseBuilder::new(DatabaseId(1))
        .finish()
        .expect("an empty database")
}

fn shadows_on() -> DisplaySettings {
    DisplaySettings {
        shadows: ShadowSettings {
            enabled: true,
            azimuth: 90.0,
            elevation: 30.0,
        },
        ..DisplaySettings::default()
    }
}

fn fog_on() -> DisplaySettings {
    DisplaySettings {
        fog: FogSettings {
            enabled: true,
            start: 5.0,
            end: 50.0,
            color: [10, 20, 30],
        },
        ..DisplaySettings::default()
    }
}

#[test]
fn a_display_write_is_one_revision_and_one_table_dirty_mark() {
    let mut db = empty_db();
    let before = db.revision();
    let changes = db
        .apply_display_settings(shadows_on(), TransactionId(1), "SHADOWS ON")
        .expect("apply");
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, db.revision());
    assert!(changes.changes.is_empty(), "no geometry row moved");
    assert!(changes.scope.tables.contains(&TableKind::Display));
    assert!(!changes.scope.header);
    assert_eq!(db.display_settings(), shadows_on());
}

#[test]
fn an_identical_write_is_refused_and_writes_nothing() {
    let mut db = empty_db();
    let error = db
        .apply_display_settings(DisplaySettings::default(), TransactionId(1), "no-op")
        .expect_err("the record is already the default");
    assert!(format!("{error:?}").contains("display.no_change"));
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn undo_replays_the_patch_with_its_sides_swapped() {
    let mut db = empty_db();
    let patch = crate::DisplayPatch {
        before: DisplaySettings::default(),
        after: fog_on(),
    };
    db.apply_display_patches(std::slice::from_ref(&patch), TransactionId(1), "FOG ON")
        .unwrap();
    assert_eq!(db.display_settings(), fog_on());
    let inverse = patch.inverse();
    db.apply_display_patches(std::slice::from_ref(&inverse), TransactionId(2), "undo")
        .unwrap();
    assert_eq!(db.display_settings(), DisplaySettings::default());
    // A stale patch (its before side is not the current record) writes nothing.
    let revision = db.revision();
    let error = db
        .apply_display_patches(std::slice::from_ref(&inverse), TransactionId(3), "stale")
        .expect_err("the patch's before side is the fog record, not the default");
    assert!(format!("{error:?}").contains("display.stale_patch"));
    assert_eq!(db.revision(), revision);
    // A display step carries exactly one whole-record patch.
    db.apply_display_patches(&[], TransactionId(4), "empty")
        .expect_err("no patch");
}

#[test]
fn the_snapshot_writes_the_table_only_when_it_differs_from_the_default() {
    let db = empty_db();
    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            db.revision(),
        )
        .unwrap();
    assert!(!snapshot.has_display_settings());
    assert!(snapshot.tables.get(TableKind::Display).is_none());

    let mut db = empty_db();
    db.apply_display_settings(shadows_on(), TransactionId(1), "SHADOWS ON")
        .unwrap();
    let snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            db.revision(),
        )
        .unwrap();
    assert!(snapshot.has_display_settings());
    let rows = snapshot
        .tables
        .get(TableKind::Display)
        .expect("table entry");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["shadows"]["enabled"], serde_json::Value::from(true));
    assert_eq!(rows[0]["fog"]["enabled"], serde_json::Value::from(false));

    let loaded = DrawingDatabase::from_snapshot(&snapshot).unwrap();
    assert_eq!(loaded.display_settings(), db.display_settings());
}

#[test]
fn a_malformed_or_extra_row_is_refused_with_the_stable_marker() {
    let mut db = empty_db();
    db.apply_display_settings(shadows_on(), TransactionId(1), "SHADOWS ON")
        .unwrap();
    let mut snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            db.revision(),
        )
        .unwrap();
    let row = snapshot.tables.get(TableKind::Display).unwrap()[0].clone();
    snapshot
        .tables
        .set(TableKind::Display, vec![row.clone(), row]);
    let error = DrawingDatabase::from_snapshot(&snapshot).expect_err("two rows");
    // The snapshot layer names the document marker; the display codec's own
    // reason rides in the detail.
    assert!(
        format!("{error:?}").contains("tables/display.json"),
        "{error:?}"
    );

    let mut snapshot = db
        .snapshot(
            UnitContext::drawing_units(),
            DocumentIdentity::Temporary(1),
            Vec::new(),
            db.revision(),
        )
        .unwrap();
    snapshot.tables.set(
        TableKind::Display,
        vec![serde_json::json!({
            "shadows": {"enabled": true, "azimuth": 10.0, "elevation": 10.0},
            "fog": {"enabled": false, "start": 1.0, "end": 2.0, "color": [0, 0, 0]},
            "extra": 1
        })],
    );
    let error = DrawingDatabase::from_snapshot(&snapshot).expect_err("unknown field");
    assert!(
        format!("{error:?}").contains("tables/display.json"),
        "{error:?}"
    );
}
