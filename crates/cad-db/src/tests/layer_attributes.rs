//! Layer attribute storage contract tests.
//!
//! The attributes are **storage only** in this phase: nothing reads them to
//! change behaviour, so these tests assert the data shape, the default
//! resolution and the documented `visible == !(off || frozen)` invariant — never
//! a rendering effect.

use super::*;
use crate::{LayerAttributes, LayerColor, LayerLineWeight};

/// A database with layer 0 and layout 1; no block is needed here.
fn layer_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(crate::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn layer_attributes_default_to_on_plottable_and_opaque() {
    // The documented default is a live, plottable, opaque ACI 7 white layer
    // using the solid linetype and ByLayer weight. Every boolean is false
    // *except* `plottable`.
    let attributes = LayerAttributes::default();
    assert!(!attributes.off);
    assert!(!attributes.frozen);
    assert!(!attributes.frozen_in_new_viewport);
    assert!(!attributes.locked);
    assert!(!attributes.xref_dependent);
    assert!(attributes.plottable);
    assert_eq!(attributes.color, LayerColor::default());
    assert_eq!(attributes.color.aci, Some(7));
    assert_eq!(attributes.color.rgb, [255, 255, 255]);
    assert_eq!(attributes.linetype, "Continuous");
    assert_eq!(attributes.lineweight, LayerLineWeight::ByLayer);
    assert_eq!(attributes.opacity, 1.0);
    assert_eq!(attributes.plot_style, None);
    // The default set is visible under the documented predicate.
    assert!(attributes.is_visible());
}

#[test]
fn layer_attributes_round_trip_through_the_builder() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "Walls".into(),
        visible: false,
    })
    .unwrap();
    b.insert_layout(crate::Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    // Off and frozen are recorded *separately*, and both make the row invisible.
    let stored = LayerAttributes {
        off: true,
        frozen: true,
        frozen_in_new_viewport: true,
        locked: true,
        xref_dependent: true,
        plottable: false,
        color: LayerColor {
            aci: Some(1),
            rgb: [255, 0, 0],
        },
        linetype: "DASHED".into(),
        lineweight: LayerLineWeight::Hundredths(35),
        opacity: 0.5,
        plot_style: Some("Normal".into()),
    };
    b.set_layer_attributes(LayerId(0), stored.clone()).unwrap();
    let db = b.finish().unwrap();
    assert_eq!(db.layer_attributes(LayerId(0)), stored);
}

#[test]
fn layer_attributes_fall_back_to_the_default_for_an_unrecorded_row() {
    // Hand-built fixtures insert a `Layer` row without attributes; the cloned
    // accessor must answer the documented default rather than a zeroed value.
    let db = layer_db();
    let attributes = db.layer_attributes(LayerId(0));
    assert_eq!(attributes, LayerAttributes::default());
    // The borrowed variant is honest: it reports "nothing recorded" instead of
    // fabricating a row.
    assert!(db.layer_attributes_ref(LayerId(0)).is_none());
    // An unknown layer id answers the default too (never a panic).
    assert_eq!(db.layer_attributes(LayerId(99)), LayerAttributes::default());
}

#[test]
fn set_layer_attributes_rejects_an_unknown_layer() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    let error = b
        .set_layer_attributes(LayerId(7), LayerAttributes::default())
        .expect_err("attributes for an unknown layer are a caller bug");
    assert!(
        error.to_string().contains("unknown layer"),
        "unexpected error: {error}"
    );
}

#[test]
fn delete_layer_drops_the_attributes_row() {
    let mut db = layer_db();
    let (id, _) = db
        .create_layer("Roof", TransactionId(1), "create")
        .expect("create");
    // `create_layer` records the default attributes, so the row is present.
    assert_eq!(
        db.layer_attributes_ref(id),
        Some(&LayerAttributes::default())
    );
    db.delete_layer(id, TransactionId(2), "delete")
        .expect("delete unused layer");
    assert!(db.layer(LayerId(id.0)).is_none());
    assert!(
        db.layer_attributes_ref(id).is_none(),
        "the attributes row outlived its layer"
    );
    assert_eq!(db.layer_attributes(id), LayerAttributes::default());
}

#[test]
fn created_layers_keep_the_visible_flags_invariant() {
    let mut db = layer_db();
    let (id, _) = db
        .create_layer("Roof", TransactionId(1), "create")
        .expect("create");
    let layer = db.layer(id).expect("layer exists");
    let attributes = db.layer_attributes(id);
    // Invariant: `visible == !(off || frozen)`. A created layer is visible and
    // carries the default (on, thawed) flags.
    assert_eq!(layer.visible, !(attributes.off || attributes.frozen));
    assert!(layer.visible);
    assert!(attributes.is_visible());
}

#[test]
fn layer_ids_are_not_reused_after_delete() {
    // Id invariance: deleting a layer must not let a new layer inherit its id
    // (and therefore its attributes row). The attributes map is keyed by id, so
    // this is the same guarantee `next_layer_id` documents.
    let mut db = layer_db();
    let (first, _) = db.create_layer("A", TransactionId(1), "create").unwrap();
    db.delete_layer(first, TransactionId(2), "delete").unwrap();
    let (second, _) = db.create_layer("B", TransactionId(3), "create").unwrap();
    assert_ne!(first, second);
    assert!(db.layer_attributes_ref(first).is_none());
    assert_eq!(db.layer_attributes(second), LayerAttributes::default());
}

// ---------------------------------------------------------------------------
// The post-import attribute write (`DrawingDatabase::set_layer_attributes`)
// ---------------------------------------------------------------------------

/// The whole row as comparable state: attributes plus the derived `visible` flag.
fn row(db: &DrawingDatabase, id: LayerId) -> (LayerAttributes, bool) {
    (
        db.layer_attributes(id),
        db.layer(id).expect("layer row").visible,
    )
}

#[test]
fn setting_attributes_raises_the_revision_once_and_keeps_changes_empty() {
    let mut db = layer_db();
    let before = db.revision();
    let mut attributes = db.layer_attributes(LayerId(0));
    attributes.color = LayerColor {
        aci: Some(1),
        rgb: [255, 0, 0],
    };

    let changes = db
        .set_layer_attributes(LayerId(0), attributes.clone(), TransactionId(1), "colour")
        .expect("the layer exists");

    assert_eq!(
        db.layer_attributes(LayerId(0)).color,
        LayerColor {
            aci: Some(1),
            rgb: [255, 0, 0]
        }
    );
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, db.revision());
    // No entity changed: the revision is the staleness tag, nothing else.
    assert!(changes.changes.is_empty());
}

#[test]
fn off_and_frozen_drive_visible_and_unfreezing_restores_it() {
    let mut db = layer_db();
    let base = db.layer_attributes(LayerId(0));
    assert!(db.layer(LayerId(0)).expect("layer").visible);

    // off => invisible.
    let mut off = base.clone();
    off.off = true;
    db.set_layer_attributes(LayerId(0), off.clone(), TransactionId(1), "off")
        .expect("write");
    assert!(!db.layer(LayerId(0)).expect("layer").visible);

    // still off, now also frozen: still invisible, and the invariant holds.
    let mut both = off.clone();
    both.frozen = true;
    db.set_layer_attributes(LayerId(0), both.clone(), TransactionId(2), "freeze")
        .expect("write");
    assert_eq!(row(&db, LayerId(0)), (both.clone(), false));
    let stored = db.layer_attributes(LayerId(0));
    assert_eq!(
        db.layer(LayerId(0)).expect("layer").visible,
        !(stored.off || stored.frozen)
    );

    // unfreezing alone does not restore visibility: `off` is still set.
    let mut off_only = both.clone();
    off_only.frozen = false;
    db.set_layer_attributes(LayerId(0), off_only.clone(), TransactionId(3), "thaw")
        .expect("write");
    assert!(!db.layer(LayerId(0)).expect("layer").visible);

    // clearing the last flag restores the row.
    db.set_layer_attributes(LayerId(0), base.clone(), TransactionId(4), "on")
        .expect("write");
    assert_eq!(row(&db, LayerId(0)), (base, true));
}

#[test]
fn an_unchanged_attribute_write_is_a_no_op_without_a_revision_bump() {
    let mut db = layer_db();
    let before = db.revision();
    let attributes = db.layer_attributes(LayerId(0));

    let changes = db
        .set_layer_attributes(LayerId(0), attributes, TransactionId(1), "same")
        .expect("the layer exists");

    assert_eq!(db.revision(), before, "nothing changed, so no new revision");
    assert_eq!(changes.before, changes.after);
    assert!(changes.changes.is_empty());
}

#[test]
fn a_write_that_only_fixes_a_stale_visible_flag_still_counts_as_a_change() {
    // A hand-built row can disagree with its attributes (the builder never
    // recomputes `visible`). Rewriting the same attributes must repair the flag
    // and publish that as a real change, not as a no-op.
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let attributes = LayerAttributes {
        off: true,
        ..LayerAttributes::default()
    };
    b.set_layer_attributes(LayerId(0), attributes.clone())
        .unwrap();
    let mut db = b.finish().unwrap();
    assert!(
        db.layer(LayerId(0)).expect("layer").visible,
        "the fixture deliberately starts inconsistent"
    );
    let before = db.revision();

    db.set_layer_attributes(LayerId(0), attributes, TransactionId(1), "repair")
        .expect("write");

    assert!(!db.layer(LayerId(0)).expect("layer").visible);
    assert_eq!(db.revision(), Revision(before.0 + 1));
}

#[test]
fn setting_attributes_on_an_unknown_layer_is_refused_and_changes_nothing() {
    let mut db = layer_db();
    let before = db.revision();

    let error = db
        .set_layer_attributes(
            LayerId(7),
            LayerAttributes::default(),
            TransactionId(1),
            "unknown",
        )
        .expect_err("no such layer");

    assert!(error.to_string().contains("layer.unknown_id"), "{error}");
    assert_eq!(db.revision(), before);
}
