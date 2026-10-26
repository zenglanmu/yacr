//! The post-import POINT display style write
//! (`DrawingDatabase::set_point_display_style`).
//!
//! The importer's builder path is covered by
//! [`crate::tests::builder`]; this module pins the **user** write that the typed
//! `PDMODE`/`PDSIZE` commands use: the stored raw values, exactly one revision
//! bump per real change, an authoritative no-op for an unchanged pair, a refusal
//! for a non-finite size, and **no** entity change in the published change set.

use super::*;

fn db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn a_write_stores_both_values_raw_and_bumps_the_revision_once() {
    let mut db = db();
    let before = db.revision();
    let changeset = db
        .set_point_display_style(35, -20.0, TransactionId(7), "set point display style")
        .expect("a finite size is a legal write");
    assert_eq!(db.point_display_mode(), 35, "the mode is stored raw");
    assert_eq!(db.point_display_size(), -20.0, "the sign is preserved");
    assert_eq!(db.revision(), Revision(before.0 + 1), "one revision bump");
    assert_eq!(changeset.before, before);
    assert_eq!(changeset.after, db.revision());
    assert!(
        changeset.changes.is_empty(),
        "a header write changes no entity"
    );
}

#[test]
fn an_unchanged_pair_is_an_authoritative_no_op() {
    let mut db = db();
    db.set_point_display_style(3, 2.5, TransactionId(1), "seed")
        .unwrap();
    let settled = db.revision();
    let changeset = db
        .set_point_display_style(3, 2.5, TransactionId(2), "same values")
        .expect("writing the stored pair is admitted");
    assert_eq!(
        db.revision(),
        settled,
        "nothing observable changed, so no new revision"
    );
    assert_eq!(changeset.before, changeset.after);
    assert!(changeset.changes.is_empty());
}

#[test]
fn a_non_finite_size_is_refused_before_anything_is_written() {
    let mut db = db();
    db.set_point_display_style(4, 1.0, TransactionId(1), "seed")
        .unwrap();
    let settled = db.revision();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = db
            .set_point_display_style(2, bad, TransactionId(2), "bad size")
            .expect_err("a non-finite size is not a display size");
        assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    }
    assert_eq!(
        db.point_display_mode(),
        4,
        "the refused write changed nothing"
    );
    assert_eq!(db.point_display_size(), 1.0);
    assert_eq!(db.revision(), settled);
}

#[test]
fn any_i16_mode_is_stored_without_interpretation() {
    // The db never interprets a mode: base glyphs, the 32/64/96 shape bits and a
    // value this build cannot express all round-trip verbatim, so the display
    // layer stays the single interpreter (and reports what it cannot draw).
    let mut db = db();
    for mode in [0, 1, 2, 3, 4, 35, 96, -1, i16::MIN, i16::MAX] {
        db.set_point_display_style(mode, 0.0, TransactionId(1), "mode sweep")
            .unwrap();
        assert_eq!(db.point_display_mode(), mode);
    }
}
