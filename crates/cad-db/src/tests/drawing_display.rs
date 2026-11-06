//! Display-attribute and display-state staging (the properties write path).

use super::*;

// ---------------------------------------------------------------------------
// Display-attribute staging (the properties write path reads this back)
// ---------------------------------------------------------------------------

/// Insert entity 1 through the sanctioned path and return the database.
fn writable_db_with_line() -> DrawingDatabase {
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create line", TransactionId(1))
        .unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    tx.commit().unwrap();
    db
}

#[test]
fn drawing_display_update_is_one_bump_and_a_style_mask() {
    let mut db = writable_db_with_line();
    let before = db.revision();
    let attributes = EntityRenderAttributes {
        color: EntityColor::Explicit([1, 2, 3]),
        ..EntityRenderAttributes::default()
    };

    let mut tx = db
        .begin_drawing_transaction("set colour", TransactionId(2))
        .unwrap();
    tx.update_entity_display(EntityId(1), attributes.clone())
        .unwrap();
    let changes = tx.commit().unwrap();

    assert_eq!(db.revision(), Revision(before.0 + 1), "one revision bump");
    assert_eq!(
        db.entity_render_attributes(EntityId(1)).color,
        EntityColor::Explicit([1, 2, 3])
    );
    // The entity row itself is unchanged, but the update entry must carry STYLE
    // so the representation is rebuilt.
    let mask = changes
        .changes
        .iter()
        .find_map(|change| match change {
            ObjectChange::Update(object, mask) if object.0 == 1 => Some(*mask),
            _ => None,
        })
        .expect("an update for entity 1");
    assert!(mask.contains(ChangeMask::STYLE), "got {mask:?}");
    assert!(mask.invalidates_representation());
}

#[test]
fn drawing_display_update_rollback_leaves_the_attributes_untouched() {
    let mut db = writable_db_with_line();
    let before = db.entity_render_attributes(EntityId(1));
    {
        let mut tx = db
            .begin_drawing_transaction("set colour", TransactionId(2))
            .unwrap();
        tx.update_entity_display(
            EntityId(1),
            EntityRenderAttributes {
                color: EntityColor::Explicit([9, 9, 9]),
                ..EntityRenderAttributes::default()
            },
        )
        .unwrap();
        tx.rollback();
    }
    assert_eq!(db.entity_render_attributes(EntityId(1)), before);
    assert_eq!(db.revision(), Revision(1));
}

#[test]
fn drawing_display_update_rejects_a_duplicate_stage() {
    let mut db = writable_db_with_line();
    let mut tx = db
        .begin_drawing_transaction("set colour", TransactionId(2))
        .unwrap();
    tx.update_entity_display(EntityId(1), EntityRenderAttributes::default())
        .unwrap();
    let error = tx
        .update_entity_display(EntityId(1), EntityRenderAttributes::default())
        .unwrap_err();
    assert!(matches!(error, CadError::Invariant(_)), "got {error:?}");
}

#[test]
fn drawing_display_update_refuses_a_deleted_entity() {
    let mut db = writable_db_with_line();
    let mut tx = db
        .begin_drawing_transaction("delete and recolour", TransactionId(2))
        .unwrap();
    tx.delete_entity(EntityId(1)).unwrap();
    let error = tx
        .update_entity_display(EntityId(1), EntityRenderAttributes::default())
        .unwrap_err();
    assert!(matches!(error, CadError::Invariant(_)), "got {error:?}");
}

// ---------------------------------------------------------------------------
// Full display-state staging (render attributes + visibility membership)
// ---------------------------------------------------------------------------

/// Layer 0, block 0 whose members 10/11 are governed by visibility states
/// "A"/"B", and one model line (entity 1).
fn display_state_db() -> DrawingDatabase {
    use crate::tables::{DynamicBlockState, DynamicBlockVisibility};
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(10), EntityId(11)],
        dynamic_visibility: None,
    })
    .unwrap();
    for id in [10u128, 11] {
        b.insert_entity(raw_entity(
            id,
            SpaceId::Block(BlockId(0)),
            SemanticGeometry::Line {
                start: point(id as f64, 0.0),
                end: point(id as f64 + 1.0, 0.0),
            },
        ))
        .unwrap();
    }
    b.set_block_dynamic_visibility(
        BlockId(0),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(10), EntityId(11)],
            states: vec![
                DynamicBlockState {
                    name: "A".into(),
                    entities: vec![EntityId(10)],
                },
                DynamicBlockState {
                    name: "B".into(),
                    entities: vec![EntityId(11)],
                },
            ],
            active_state: None,
        },
    )
    .unwrap();
    b.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    b.finish().unwrap()
}

fn captured_display_state() -> EntityDisplayState {
    EntityDisplayState {
        attributes: EntityRenderAttributes {
            color: EntityColor::Explicit([7, 8, 9]),
            ..EntityRenderAttributes::default()
        },
        visibility: VisibilityMembership {
            blocks: vec![BlockId(0)],
            visibility: vec![VisibilityStateMembership {
                block: BlockId(0),
                member: true,
                states: vec!["A".into()],
            }],
        },
    }
}

#[test]
fn drawing_display_state_for_an_inserted_entity_is_one_bump_and_a_style_update() {
    let mut db = display_state_db();
    let before = db.revision();
    let state = captured_display_state();

    let mut tx = db
        .begin_drawing_transaction("paste block member", TransactionId(1))
        .unwrap();
    tx.insert_entity(raw_entity(
        20,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(2.0, 0.0),
            end: point(3.0, 0.0),
        },
    ))
    .unwrap();
    // The row does not exist yet: display state is applied after the insert, in
    // the same revision.
    tx.update_entity_display_state(EntityId(20), &state)
        .unwrap();
    let changes = tx.commit().unwrap();

    assert_eq!(db.revision(), Revision(before.0 + 1), "one revision bump");
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, Revision(before.0 + 1));
    assert!(
        changes
            .changes
            .contains(&ObjectChange::Insert(ObjectId(20))),
        "the insert is still announced: {:?}",
        changes.changes
    );
    let updates: Vec<ChangeMask> = changes
        .changes
        .iter()
        .filter_map(|change| match change {
            ObjectChange::Update(object, mask) if object.0 == 20 => Some(*mask),
            _ => None,
        })
        .collect();
    assert_eq!(updates.len(), 1, "one update for the inserted entity");
    assert!(
        updates[0].contains(ChangeMask::STYLE),
        "got {:?}",
        updates[0]
    );
    assert!(updates[0].invalidates_representation());

    assert_eq!(
        db.entity_render_attributes(EntityId(20)).color,
        EntityColor::Explicit([7, 8, 9]),
        "the captured attributes were applied"
    );
    assert_eq!(
        db.entity_visibility_membership(EntityId(20)),
        state.visibility,
        "the captured block/visibility membership was applied"
    );
}

#[test]
fn drawing_display_state_for_an_updated_entity_merges_into_one_update() {
    let mut db = writable_db_with_line();
    let before = db.revision();
    let state = EntityDisplayState {
        attributes: EntityRenderAttributes {
            color: EntityColor::Explicit([1, 2, 3]),
            ..EntityRenderAttributes::default()
        },
        visibility: VisibilityMembership::default(),
    };

    let mut tx = db
        .begin_drawing_transaction("stretch and recolour", TransactionId(2))
        .unwrap();
    tx.update_entity(line_entity(1, point(0.0, 0.0), point(10.0, 0.0)))
        .unwrap();
    tx.update_entity_display_state(EntityId(1), &state).unwrap();
    let changes = tx.commit().unwrap();

    assert_eq!(db.revision(), Revision(before.0 + 1), "one revision bump");
    assert_eq!(
        changes.changes.len(),
        1,
        "the update and the display state share one entry: {:?}",
        changes.changes
    );
    let mask = match &changes.changes[0] {
        ObjectChange::Update(object, mask) if object.0 == 1 => *mask,
        other => panic!("expected the update for entity 1, got {other:?}"),
    };
    assert!(mask.contains(ChangeMask::GEOMETRY), "got {mask:?}");
    assert!(mask.contains(ChangeMask::STYLE), "got {mask:?}");
    assert_eq!(
        db.entity_render_attributes(EntityId(1)).color,
        EntityColor::Explicit([1, 2, 3])
    );
}

#[test]
fn drawing_display_state_naming_a_missing_block_is_refused_unchanged() {
    let mut db = writable_db_with_line();
    let before_revision = db.revision();
    let state = EntityDisplayState {
        attributes: EntityRenderAttributes {
            color: EntityColor::Explicit([4, 5, 6]),
            ..EntityRenderAttributes::default()
        },
        visibility: VisibilityMembership {
            blocks: vec![BlockId(999)],
            visibility: Vec::new(),
        },
    };

    let mut tx = db
        .begin_drawing_transaction("restore missing block", TransactionId(2))
        .unwrap();
    tx.update_entity_display_state(EntityId(1), &state).unwrap();
    let error = tx.commit().unwrap_err();
    match error {
        CadError::Invariant(message) => {
            assert!(message.contains("EntityId(1)"), "got {message}");
            assert!(message.contains("BlockId(999)"), "got {message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    // Nothing landed: no revision, no entity change, no attributes, no
    // membership.
    assert_eq!(db.revision(), before_revision);
    assert_eq!(db.entity_count(), 1);
    assert_eq!(
        db.entity_render_attributes(EntityId(1)),
        EntityRenderAttributes::default()
    );
    assert!(db
        .entity_visibility_membership(EntityId(1))
        .blocks
        .is_empty());
    assert!(db.block(BlockId(0)).unwrap().entities.is_empty());
}

#[test]
fn drawing_display_state_naming_a_missing_visibility_state_is_refused_unchanged() {
    let mut db = display_state_db();
    let before_revision = db.revision();
    let before_attributes = db.entity_render_attributes(EntityId(1));
    let state = EntityDisplayState {
        attributes: EntityRenderAttributes {
            color: EntityColor::Explicit([4, 5, 6]),
            ..EntityRenderAttributes::default()
        },
        visibility: VisibilityMembership {
            blocks: Vec::new(),
            visibility: vec![VisibilityStateMembership {
                block: BlockId(0),
                member: true,
                states: vec!["Missing".into()],
            }],
        },
    };

    let mut tx = db
        .begin_drawing_transaction("restore missing state", TransactionId(1))
        .unwrap();
    tx.update_entity_display_state(EntityId(1), &state).unwrap();
    let error = tx.commit().unwrap_err();
    match error {
        CadError::Invariant(message) => {
            assert!(message.contains("EntityId(1)"), "got {message}");
            assert!(message.contains("BlockId(0)"), "got {message}");
            assert!(message.contains("'Missing'"), "got {message}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    assert_eq!(db.revision(), before_revision);
    assert_eq!(db.entity_render_attributes(EntityId(1)), before_attributes);
    assert!(db
        .entity_visibility_membership(EntityId(1))
        .visibility
        .is_empty());
    assert_eq!(
        db.block_dynamic_visibility(BlockId(0))
            .unwrap()
            .member_entities,
        vec![EntityId(10), EntityId(11)],
        "the block's governed member list is untouched"
    );
}

#[test]
fn drawing_display_state_with_resolvable_references_is_fully_applied() {
    let mut db = display_state_db();
    let before_revision = db.revision();
    let state = EntityDisplayState {
        attributes: EntityRenderAttributes {
            color: EntityColor::Explicit([3, 2, 1]),
            ..EntityRenderAttributes::default()
        },
        visibility: VisibilityMembership {
            blocks: vec![BlockId(0)],
            visibility: vec![VisibilityStateMembership {
                block: BlockId(0),
                member: true,
                // Entity 10 is already in state "A"; "B" is a state the block
                // does have, so the membership write is sanctioned.
                states: vec!["B".into()],
            }],
        },
    };

    let mut tx = db
        .begin_drawing_transaction("restore display", TransactionId(1))
        .unwrap();
    tx.update_entity_display_state(EntityId(10), &state)
        .unwrap();
    let changes = tx.commit().unwrap();

    assert_eq!(
        db.revision(),
        Revision(before_revision.0 + 1),
        "one revision bump"
    );
    assert_eq!(
        db.entity_render_attributes(EntityId(10)).color,
        EntityColor::Explicit([3, 2, 1])
    );
    assert!(db
        .entity_visibility_membership(EntityId(10))
        .blocks
        .contains(&BlockId(0)));
    let visibility = db.block_dynamic_visibility(BlockId(0)).unwrap();
    assert!(visibility.member_entities.contains(&EntityId(10)));
    assert!(visibility
        .states
        .iter()
        .find(|entry| entry.name == "B")
        .unwrap()
        .entities
        .contains(&EntityId(10)));
    let mask = changes
        .changes
        .iter()
        .find_map(|change| match change {
            ObjectChange::Update(object, mask) if object.0 == 10 => Some(*mask),
            _ => None,
        })
        .expect("the restored id is announced");
    assert!(mask.contains(ChangeMask::STYLE), "got {mask:?}");
}
