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
