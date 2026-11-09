//! Contract tests for the combined layer-table write core
//! (`drawing::layer_patch`).
//!
//! A layer row is [`Layer`] plus [`LayerAttributes`]; the combined apply path is
//! what lets an undo record restore a layer and the entities that sit on it as
//! one revision. Everything below pins the single revision bump, the derived
//! `visible` flag, the delete-safety rules (post-batch entity view) and the
//! all-or-nothing refusal shape.

use super::*;

/// Layer 0 (`0`), layer 1 (`WALLS`, one model line) and layer 2 (`UNUSED`, no
/// entities).
fn patch_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [
        (LayerId(0), "0"),
        (LayerId(1), "WALLS"),
        (LayerId(2), "UNUSED"),
    ] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let mut line = line_entity(1, point(0.0, 0.0), point(4.0, 0.0));
    line.layer = LayerId(1);
    b.insert_entity(line).unwrap();
    b.finish().unwrap()
}

/// A layer row; `visible` is deliberately caller-supplied so a test can prove
/// the apply path re-derives it instead of trusting the copy.
fn row(id: u128, name: &str, attributes: LayerAttributes, visible: bool) -> LayerRow {
    LayerRow {
        layer: Layer {
            id: LayerId(id),
            name: name.into(),
            visible,
        },
        attributes,
    }
}

#[test]
fn upsert_and_delete_commit_one_revision_and_derive_visible_from_attributes() {
    let mut db = patch_db();
    let start = db.revision();
    let attributes_before = db.layer_attributes(LayerId(1));
    assert!(!attributes_before.frozen);

    let mut frozen = attributes_before.clone();
    frozen.frozen = true;
    // The copied flag claims `true`; the apply path must ignore it and store
    // `is_visible()` of the attributes (`false`).
    let upsert = row(1, "WALLS", frozen.clone(), true);
    let changes = db
        .apply_drawing_changes_with_blocks_display_and_layers(
            "freeze layer and remove an unused one",
            TransactionId(7),
            &[],
            Vec::new(),
            &[],
            &[(LayerId(1), Some(upsert)), (LayerId(2), None)],
        )
        .expect("the combined batch is valid");

    assert_eq!(changes.before, start);
    assert_eq!(changes.after, Revision(start.0 + 1));
    assert_eq!(db.revision(), Revision(start.0 + 1), "exactly one revision");
    assert!(
        changes.changes.is_empty(),
        "a layer-table edit publishes no entity object changes"
    );

    let layer = db.layer(LayerId(1)).expect("updated row").clone();
    assert_eq!(layer.name, "WALLS");
    assert!(!layer.visible, "visible is derived from the attributes");
    assert_eq!(db.layer_attributes(LayerId(1)), frozen);
    assert!(db.layer(LayerId(2)).is_none(), "the unused layer is gone");
    assert!(
        !db.layer_attributes.contains_key(&LayerId(2)),
        "the attribute row never outlives its layer"
    );
}

#[test]
fn deleting_an_in_use_layer_is_refused_and_changes_nothing() {
    let mut db = patch_db();
    let start = db.revision();
    let snapshot = format!("{db:?}");
    let error = db
        .apply_drawing_changes_with_blocks_display_and_layers(
            "delete an in-use layer",
            TransactionId(8),
            &[],
            Vec::new(),
            &[],
            &[(LayerId(1), None)],
        )
        .expect_err("the surviving entity still references the layer");
    assert!(
        error.to_string().contains("layer.in_use"),
        "expected the stable in-use refusal, got {error}"
    );
    assert_eq!(db.revision(), start, "a refusal bumps no revision");
    assert!(db.layer(LayerId(1)).is_some(), "the layer still exists");
    assert_eq!(format!("{db:?}"), snapshot, "nothing was mutated");
}

#[test]
fn deleting_a_layer_together_with_its_only_entity_is_accepted() {
    let mut db = patch_db();
    let start = db.revision();
    let changes = db
        .apply_drawing_changes_with_blocks_display_and_layers(
            "delete a layer and its entity",
            TransactionId(9),
            &[],
            vec![(EntityId(1), None)],
            &[],
            &[(LayerId(1), None)],
        )
        .expect("the post-batch entity view has no reference left");
    assert_eq!(changes.after, Revision(start.0 + 1));
    assert_eq!(db.revision(), Revision(start.0 + 1), "exactly one revision");
    assert!(db.entity(EntityId(1)).is_none());
    assert!(db.layer(LayerId(1)).is_none());
    assert!(!db.layer_attributes.contains_key(&LayerId(1)));
}

#[test]
fn deleting_the_default_layer_is_refused() {
    let mut db = patch_db();
    let start = db.revision();
    let error = db
        .apply_drawing_changes_with_blocks_display_and_layers(
            "delete the default layer",
            TransactionId(10),
            &[],
            Vec::new(),
            &[],
            &[(LayerId(0), None)],
        )
        .expect_err("layer 0 is required");
    assert!(
        error.to_string().contains("layer.default_undeletable"),
        "expected the default-layer refusal, got {error}"
    );
    assert_eq!(db.revision(), start);
    assert!(db.layer(LayerId(0)).is_some());
}

#[test]
fn an_entity_may_be_restored_onto_a_layer_restored_in_the_same_batch() {
    let mut db = patch_db();
    let start = db.revision();
    let mut line = line_entity(1, point(0.0, 0.0), point(4.0, 0.0));
    line.layer = LayerId(9);
    let restore = row(9, "RESTORED", LayerAttributes::default(), false);
    db.apply_drawing_changes_with_blocks_display_and_layers(
        "restore a layer and its entity",
        TransactionId(11),
        &[],
        vec![(EntityId(1), Some(line))],
        &[],
        &[(LayerId(9), Some(restore))],
    )
    .expect("the staged layer is a valid reference");
    assert_eq!(db.revision(), Revision(start.0 + 1));
    assert_eq!(
        db.entity(EntityId(1)).map(|entity| entity.layer),
        Some(LayerId(9))
    );
    assert_eq!(
        db.layer(LayerId(9)).map(|layer| layer.name.clone()),
        Some("RESTORED".to_string())
    );
}
