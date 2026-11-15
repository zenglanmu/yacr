//! Post-import writes for `$INSBASE` / `$LIMMIN` / `$LIMMAX`.
//!
//! The database stores the values raw and reports `None` until something writes
//! them (the importer does not read these variables — see
//! `drawing::header_geometry`), so a read either returns the stored value or an
//! explicit absence, never an invented default.

use super::*;

fn empty_db() -> DrawingDatabase {
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
fn the_header_points_start_unstored_and_report_none() {
    let db = empty_db();
    assert_eq!(db.insertion_base(), None);
    assert_eq!(db.limits_min(), None);
    assert_eq!(db.limits_max(), None);
    assert_eq!(db.model_limits(), None);
}

#[test]
fn the_insertion_base_is_stored_raw_in_one_revision() {
    let mut db = empty_db();
    let before = db.revision();
    let base = Point3 {
        x: 12.5,
        y: -3.0,
        z: 2.0,
    };
    let changeset = db
        .set_insertion_base(base, TransactionId(1), "base")
        .expect("a finite point is accepted");
    assert_eq!(db.insertion_base(), Some(base));
    assert_eq!(db.revision().0, before.0 + 1);
    assert_eq!(changeset.before, before);
    assert!(
        changeset.changes.is_empty(),
        "a header value is not an entity"
    );

    // Storing the same point again is an authoritative no-op.
    let again = db
        .set_insertion_base(base, TransactionId(2), "base")
        .expect("the same point is not an error");
    assert_eq!(again.before, again.after);
    assert_eq!(db.revision().0, before.0 + 1);
}

#[test]
fn a_non_finite_base_is_refused_and_nothing_changes() {
    let mut db = empty_db();
    let revision = db.revision();
    for base in [
        Point3 {
            x: f64::NAN,
            y: 0.0,
            z: 0.0,
        },
        Point3 {
            x: 0.0,
            y: f64::INFINITY,
            z: 0.0,
        },
    ] {
        let error = db
            .set_insertion_base(base, TransactionId(1), "base")
            .unwrap_err();
        assert!(error.to_string().contains("base.invalid_point"), "{error}");
    }
    assert_eq!(db.revision(), revision);
    assert_eq!(db.insertion_base(), None);
}

#[test]
fn both_limit_corners_are_written_in_one_revision() {
    let mut db = empty_db();
    let before = db.revision();
    db.set_model_limits(
        Some([0.0, 0.0]),
        Some([420.0, 297.0]),
        TransactionId(1),
        "limits",
    )
    .expect("finite corners are accepted");
    assert_eq!(db.limits_min(), Some([0.0, 0.0]));
    assert_eq!(db.limits_max(), Some([420.0, 297.0]));
    assert_eq!(db.model_limits(), Some(([0.0, 0.0], [420.0, 297.0])));
    assert_eq!(db.revision().0, before.0 + 1);
}

#[test]
fn one_limit_corner_can_be_written_alone_and_leaves_the_other_untouched() {
    let mut db = empty_db();
    // `SETVAR LIMMIN` writes one corner: the other stays unset, so `model_limits`
    // is still `None` rather than a rectangle with an invented corner.
    db.set_model_limits(Some([-5.0, -5.0]), None, TransactionId(1), "limits")
        .expect("one corner is enough for the database");
    assert_eq!(db.limits_min(), Some([-5.0, -5.0]));
    assert_eq!(db.limits_max(), None);
    assert_eq!(db.model_limits(), None);

    let before = db.revision();
    db.set_model_limits(None, Some([10.0, 10.0]), TransactionId(2), "limits")
        .expect("the second corner completes the pair");
    assert_eq!(
        db.limits_min(),
        Some([-5.0, -5.0]),
        "the first corner survives"
    );
    assert_eq!(db.model_limits(), Some(([-5.0, -5.0], [10.0, 10.0])));
    assert_eq!(db.revision().0, before.0 + 1);
}

#[test]
fn a_non_finite_limit_corner_is_refused_and_the_pair_is_untouched() {
    let mut db = empty_db();
    db.set_model_limits(
        Some([0.0, 0.0]),
        Some([10.0, 10.0]),
        TransactionId(1),
        "limits",
    )
    .unwrap();
    let revision = db.revision();
    let error = db
        .set_model_limits(Some([f64::NAN, 0.0]), None, TransactionId(2), "limits")
        .unwrap_err();
    assert!(
        error.to_string().contains("limits.invalid_point"),
        "{error}"
    );
    let error = db
        .set_model_limits(None, None, TransactionId(2), "limits")
        .unwrap_err();
    assert!(error.to_string().contains("limits.no_corners"), "{error}");
    assert_eq!(db.revision(), revision);
    assert_eq!(db.model_limits(), Some(([0.0, 0.0], [10.0, 10.0])));
}

#[test]
fn rewriting_the_stored_limits_is_an_authoritative_no_op() {
    let mut db = empty_db();
    db.set_model_limits(
        Some([0.0, 0.0]),
        Some([420.0, 297.0]),
        TransactionId(1),
        "limits",
    )
    .unwrap();
    let before = db.revision();
    let changeset = db
        .set_model_limits(
            Some([0.0, 0.0]),
            Some([420.0, 297.0]),
            TransactionId(2),
            "limits",
        )
        .expect("the same corners are not an error");
    assert_eq!(changeset.before, changeset.after);
    assert_eq!(db.revision(), before);
}
