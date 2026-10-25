//! Layer locking on the entity write path.
//!
//! `LayerAttributes::locked` used to be storage only. These tests pin the one
//! behaviour it now has (`drawing::lock`): an entity on a locked layer is
//! read-only — it cannot be modified, created or deleted — while display-only
//! writes, layer-table edits and unlocked layers keep working.
//!
//! Every refusal is asserted to leave the transaction empty (nothing staged),
//! the revision unchanged and the tables/entities untouched, because a refused
//! write must never leave partial state.

use super::*;

/// Layer 0 (unlocked) plus `WALLS` (**locked**), with entity 1 on `WALLS` and
/// entity 2 on layer 0. Both are ordinary model-space lines.
fn lock_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    let walls = LayerAttributes {
        locked: true,
        ..LayerAttributes::default()
    };
    b.set_layer_attributes(LayerId(3), walls).unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(entity_on(1, LayerId(3))).unwrap();
    b.insert_entity(entity_on(2, LayerId(0))).unwrap();
    b.finish().unwrap()
}

fn entity_on(id: u128, layer: LayerId) -> DbEntity {
    let mut entity = line_entity(id, point(0.0, 0.0), point(10.0, 0.0));
    entity.layer = layer;
    entity
}

fn moved(entity: &DbEntity) -> DbEntity {
    let mut changed = entity.clone();
    changed.geometry = SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(20.0, 0.0),
    };
    changed
}

/// The existing post-import attribute write; `set_layer_attributes` is the only
/// way to unlock (or relock) a layer, and it is a layer-table edit, so it is
/// never itself refused by the lock rule.
fn set_locked(db: &mut DrawingDatabase, layer: LayerId, locked: bool, transaction: u128) {
    let mut attributes = db.layer_attributes(layer);
    attributes.locked = locked;
    db.set_layer_attributes(layer, attributes, TransactionId(transaction), "layer lock")
        .expect("a layer-table attribute write is always allowed");
}

fn assert_locked_refusal(error: &CadError, entity: u128, layer: &str) {
    let text = error.to_string();
    assert!(text.contains("layer.locked"), "stable code missing: {text}");
    assert!(
        text.contains(&format!("entity {entity}")),
        "the refusal names the entity: {text}"
    );
    assert!(text.contains(layer), "the refusal names the layer: {text}");
}

#[test]
fn a_locked_layer_refuses_update_transform_and_delete_without_staging() {
    let mut db = lock_db();
    let before = db.revision();
    let stored = db.entity(EntityId(1)).cloned().expect("the line on WALLS");
    let layer_rows: Vec<Layer> = db.layers().cloned().collect();

    let mut tx = db
        .begin_drawing_transaction("locked edit", TransactionId(1))
        .unwrap();
    let error = tx.update_entity(moved(&stored)).unwrap_err();
    assert_locked_refusal(&error, 1, "WALLS");
    assert_eq!(tx.staged_len(), 0, "a refused update stages nothing");

    let error = tx
        .transform_entity(EntityId(1), &Transform3::translation(point(5.0, 0.0)))
        .unwrap_err();
    assert_locked_refusal(&error, 1, "WALLS");
    assert_eq!(tx.staged_len(), 0, "a refused move stages nothing");

    let error = tx.delete_entity(EntityId(1)).unwrap_err();
    assert_locked_refusal(&error, 1, "WALLS");
    assert_eq!(tx.staged_len(), 0, "a refused delete stages nothing");

    // The transaction is still empty, so committing publishes no change at all:
    // no revision bump, no entity/table mutation.
    let changes = tx
        .commit()
        .expect("an empty transaction commits as a no-op");
    assert!(changes.is_empty());
    assert_eq!(changes.before, changes.after);
    assert_eq!(db.revision(), before);
    assert_eq!(db.entity(EntityId(1)), Some(&stored));
    assert_eq!(db.layers().cloned().collect::<Vec<_>>(), layer_rows);
    assert!(
        db.layer_attributes(LayerId(3)).locked,
        "the lock is untouched"
    );
    // The entity on the unlocked layer was never involved.
    assert!(db.entity(EntityId(2)).is_some());
}

#[test]
fn inserting_on_a_locked_layer_is_refused_and_nothing_is_inserted() {
    let mut db = lock_db();
    let before = db.revision();
    let count = db.entity_count();
    let next_id = db.next_entity_id();

    let mut tx = db
        .begin_drawing_transaction("locked insert", TransactionId(1))
        .unwrap();
    let error = tx.insert_entity(entity_on(99, LayerId(3))).unwrap_err();
    assert_locked_refusal(&error, 99, "WALLS");
    assert_eq!(tx.staged_len(), 0);

    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(db.entity_count(), count);
    assert!(db.entity(EntityId(99)).is_none());
    assert_eq!(db.revision(), before);
    assert_eq!(
        db.next_entity_id(),
        next_id,
        "a refused insert does not consume an id"
    );
}

#[test]
fn moving_onto_or_off_a_locked_layer_is_refused() {
    let mut db = lock_db();
    let on_unlocked = db.entity(EntityId(2)).cloned().unwrap();
    let on_locked = db.entity(EntityId(1)).cloned().unwrap();

    // Onto the locked layer: the write would create an entity on it.
    let mut tx = db
        .begin_drawing_transaction("move onto locked", TransactionId(1))
        .unwrap();
    let mut target = on_unlocked.clone();
    target.layer = LayerId(3);
    let error = tx.update_entity(target).unwrap_err();
    assert_locked_refusal(&error, 2, "WALLS");
    assert!(
        error.to_string().contains("cannot move entity 2 onto"),
        "{error}"
    );
    assert_eq!(tx.staged_len(), 0);
    tx.commit().unwrap();
    assert_eq!(db.entity(EntityId(2)), Some(&on_unlocked));

    // Off the locked layer: still a modification of an entity on it.
    let mut tx = db
        .begin_drawing_transaction("move off locked", TransactionId(2))
        .unwrap();
    let mut source = on_locked.clone();
    source.layer = LayerId(0);
    let error = tx.update_entity(source).unwrap_err();
    assert_locked_refusal(&error, 1, "WALLS");
    assert!(
        error.to_string().contains("cannot modify entity 1 on"),
        "{error}"
    );
    assert_eq!(tx.staged_len(), 0);
    tx.commit().unwrap();
    assert_eq!(db.entity(EntityId(1)), Some(&on_locked));
}

#[test]
fn unlocked_layers_still_accept_every_write() {
    let mut db = lock_db();
    let start = db.revision();

    // Insert on the unlocked layer.
    let mut tx = db
        .begin_drawing_transaction("insert", TransactionId(1))
        .unwrap();
    tx.insert_entity(entity_on(10, LayerId(0))).unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(changes.changes, vec![ObjectChange::Insert(ObjectId(10))]);
    assert_eq!(db.revision(), Revision(start.0 + 1));

    // Update (geometry) on the unlocked layer.
    let stored = db.entity(EntityId(2)).cloned().unwrap();
    let mut tx = db
        .begin_drawing_transaction("update", TransactionId(2))
        .unwrap();
    tx.update_entity(moved(&stored)).unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(
        changes.changes,
        vec![ObjectChange::Update(ObjectId(2), ChangeMask::GEOMETRY)]
    );
    assert_eq!(db.revision(), Revision(start.0 + 2));

    // Move on the unlocked layer: GEOMETRY plus the MOVE-only TRANSFORM bit.
    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(3))
        .unwrap();
    tx.transform_entity(EntityId(2), &Transform3::translation(point(0.0, 5.0)))
        .unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(
        changes.changes,
        vec![ObjectChange::Update(
            ObjectId(2),
            ChangeMask::GEOMETRY.union(ChangeMask::TRANSFORM)
        )]
    );
    assert_eq!(db.revision(), Revision(start.0 + 3));

    // Delete on the unlocked layer.
    let mut tx = db
        .begin_drawing_transaction("delete", TransactionId(4))
        .unwrap();
    tx.delete_entity(EntityId(2)).unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(changes.changes, vec![ObjectChange::Delete(ObjectId(2))]);
    assert!(db.entity(EntityId(2)).is_none());
    assert_eq!(db.revision(), Revision(start.0 + 4));

    // The locked layer was never touched by any of it.
    assert!(db.entity(EntityId(1)).is_some());
}

#[test]
fn display_attribute_writes_keep_working_on_a_locked_layer() {
    // `update_entity_display` is a display-representation rebuild, not a drawing
    // edit: it stages the stored entity row *unchanged* and only replaces the
    // attributes beside it, so the lock rule (which refuses effective row
    // changes) lets it through — exactly like AutoCAD regenerating a locked
    // layer.
    let mut db = lock_db();
    let before = db.revision();
    let stored = db.entity(EntityId(1)).cloned().unwrap();
    let mut attributes = db.entity_render_attributes(EntityId(1));
    attributes.color = EntityColor::Explicit([255, 0, 0]);

    let mut tx = db
        .begin_drawing_transaction("recolour", TransactionId(1))
        .unwrap();
    tx.update_entity_display(EntityId(1), attributes)
        .expect("a display-only write is allowed on a locked layer");
    assert_eq!(tx.staged_len(), 1);
    let changes = tx.commit().unwrap();

    assert_eq!(changes.changes.len(), 1);
    match &changes.changes[0] {
        ObjectChange::Update(object, mask) => {
            assert_eq!(*object, ObjectId(1));
            assert!(
                mask.contains(ChangeMask::STYLE),
                "a display write must invalidate the representation: {mask:?}"
            );
        }
        other => panic!("expected an update, got {other:?}"),
    }
    assert_eq!(db.revision(), Revision(before.0 + 1));
    // The entity row itself (geometry/layer/draw order) is untouched; only the
    // display attributes changed.
    assert_eq!(db.entity(EntityId(1)), Some(&stored));
    assert_eq!(
        db.entity_render_attributes(EntityId(1)).color,
        EntityColor::Explicit([255, 0, 0])
    );
}

#[test]
fn unlocking_the_layer_lets_the_same_edit_through() {
    let mut db = lock_db();
    let stored = db.entity(EntityId(1)).cloned().unwrap();

    // Refused while locked …
    let mut tx = db
        .begin_drawing_transaction("locked", TransactionId(1))
        .unwrap();
    assert!(tx.update_entity(moved(&stored)).is_err());
    tx.commit().unwrap();

    // … accepted after the layer-table attribute write clears the lock.
    set_locked(&mut db, LayerId(3), false, 2);
    assert!(!db.layer_attributes(LayerId(3)).locked);
    let unlocked_revision = db.revision();
    let mut tx = db
        .begin_drawing_transaction("unlocked", TransactionId(3))
        .unwrap();
    tx.update_entity(moved(&stored))
        .expect("the layer is unlocked now");
    tx.commit().unwrap();
    assert_eq!(db.revision(), Revision(unlocked_revision.0 + 1));
    assert_ne!(db.entity(EntityId(1)), Some(&stored));

    // Relocking refuses the next edit again: the rule reads live state, there is
    // no cached lock decision.
    set_locked(&mut db, LayerId(3), true, 4);
    let mut tx = db
        .begin_drawing_transaction("relocked", TransactionId(5))
        .unwrap();
    let error = tx.delete_entity(EntityId(1)).unwrap_err();
    assert_locked_refusal(&error, 1, "WALLS");
}

#[test]
fn history_replay_that_would_touch_a_now_locked_layer_is_refused() {
    // History replays a recorded patch through the same validated apply path, so
    // the current lock state wins: a step that was legal when it was recorded is
    // refused once its layer is locked, and nothing is muted. This is the
    // documented rule (`drawing::lock`), not an accident.
    let mut db = lock_db();
    set_locked(&mut db, LayerId(3), false, 1);
    let before = db.entity(EntityId(1)).cloned().unwrap();
    let after = moved(&before);
    db.apply_drawing_changes(
        "recorded move",
        TransactionId(2),
        vec![(EntityId(1), Some(after.clone()))],
    )
    .expect("the layer was unlocked when the edit was made");
    set_locked(&mut db, LayerId(3), true, 3);
    let locked_revision = db.revision();

    // Undo: replay the `before` side.
    let error = db
        .apply_drawing_changes(
            "undo: recorded move",
            TransactionId(2),
            vec![(EntityId(1), Some(before.clone()))],
        )
        .unwrap_err();
    assert_locked_refusal(&error, 1, "WALLS");
    assert_eq!(
        db.revision(),
        locked_revision,
        "a refused replay does not bump the revision"
    );
    assert_eq!(
        db.entity(EntityId(1)),
        Some(&after),
        "a refused replay leaves the drawing exactly as it was"
    );

    // Redo/undo of a create-or-delete: None is refused the same way, and the
    // entity is still there.
    let error = db
        .apply_drawing_changes("undo: erase", TransactionId(4), vec![(EntityId(1), None)])
        .unwrap_err();
    assert_locked_refusal(&error, 1, "WALLS");
    assert_eq!(db.entity(EntityId(1)), Some(&after));
    assert_eq!(db.revision(), locked_revision);
}

#[test]
fn freezing_is_still_only_visibility_and_not_a_write_rule() {
    // This lane adds no freeze enforcement: a frozen layer is hidden
    // (`visible == !(off || frozen)`), which keeps its entities out of the scene
    // and the pick funnel, but the write path itself does not refuse an edit.
    let mut db = lock_db();
    set_locked(&mut db, LayerId(3), false, 1);
    let mut attributes = db.layer_attributes(LayerId(3));
    attributes.frozen = true;
    db.set_layer_attributes(LayerId(3), attributes, TransactionId(2), "freeze")
        .unwrap();
    assert!(
        !db.layer(LayerId(3)).expect("the WALLS row").visible,
        "a frozen layer is invisible"
    );

    let mut tx = db
        .begin_drawing_transaction("edit a frozen layer", TransactionId(3))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::translation(point(1.0, 0.0)))
        .expect("freeze is not enforced on the write path");
    let changes = tx.commit().unwrap();
    assert_eq!(changes.changes.len(), 1);
}
