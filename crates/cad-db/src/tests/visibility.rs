//! Dynamic-block visibility states and deltas.

use super::*;

// ---- Dynamic-block visibility (spec §3.2) ----

/// A block definition + members for the dynamic-visibility tests.
///
/// Block 0 owns three lines (entities 10, 11, 12) governed by a two-state
/// visibility parameter: "A" shows 10 and 11, "B" shows 11 and 12.
fn dynamic_block_db(active: Option<&str>) -> DrawingDatabase {
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
        entities: vec![EntityId(10), EntityId(11), EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    for id in [10u128, 11, 12] {
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
            member_entities: vec![EntityId(10), EntityId(11), EntityId(12)],
            states: vec![
                DynamicBlockState {
                    name: "A".into(),
                    entities: vec![EntityId(10), EntityId(11)],
                },
                DynamicBlockState {
                    name: "B".into(),
                    entities: vec![EntityId(11), EntityId(12)],
                },
            ],
            active_state: active.map(str::to_string),
        },
    )
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn dynamic_block_emits_only_the_active_state_entities() {
    let db = dynamic_block_db(Some("A"));
    let visible: Vec<EntityId> = db.block_entities(BlockId(0)).iter().map(|e| e.id).collect();
    assert_eq!(visible, vec![EntityId(10), EntityId(11)]);
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(10), EntityId(11)])
    );

    let db = dynamic_block_db(Some("B"));
    let visible: Vec<EntityId> = db.block_entities(BlockId(0)).iter().map(|e| e.id).collect();
    assert_eq!(visible, vec![EntityId(11), EntityId(12)]);
}

#[test]
fn unknown_active_state_draws_every_member_never_a_guess() {
    let db = dynamic_block_db(None);
    assert_eq!(db.block_entities(BlockId(0)).len(), 3);
    // Entities outside the governed set are always visible.
    assert!(db
        .block_dynamic_visibility(BlockId(0))
        .unwrap()
        .is_visible(EntityId(999)));
}

#[test]
fn visibility_switch_emits_the_expected_delta_and_raises_revision() {
    let mut db = dynamic_block_db(Some("A"));
    let before_revision = db.revision();
    let changeset = db
        .set_block_visibility_state(BlockId(0), "B", TransactionId(7), "switch to B")
        .unwrap();
    assert_eq!(changeset.before, before_revision);
    assert_eq!(changeset.after, Revision(before_revision.0 + 1));
    assert_eq!(db.revision(), changeset.after);
    // Only the entities that entered/left the visible set are reported:
    // 10 leaves, 12 enters, 11 is common to both states.
    let mut reported: Vec<u128> = changeset
        .changes
        .iter()
        .map(|c| match c {
            ObjectChange::Update(id, mask) => {
                assert!(mask.contains(ChangeMask::GEOMETRY));
                id.0
            }
            other => panic!("unexpected change {other:?}"),
        })
        .collect();
    reported.sort();
    assert_eq!(reported, vec![10, 12]);
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(11), EntityId(12)])
    );
}

#[test]
fn switching_to_the_already_active_state_is_a_no_op() {
    let mut db = dynamic_block_db(Some("A"));
    let before = db.revision();
    let changeset = db
        .set_block_visibility_state(BlockId(0), "A", TransactionId(1), "same state")
        .unwrap();
    assert!(changeset.is_empty());
    assert_eq!(changeset.after, before);
    assert_eq!(db.revision(), before);
}

#[test]
fn unknown_visibility_state_is_rejected() {
    let mut db = dynamic_block_db(Some("A"));
    let before = db.revision();
    let result = db.set_block_visibility_state(BlockId(0), "NOPE", TransactionId(1), "bad state");
    assert!(matches!(result, Err(CadError::InvalidInput(_))));
    assert_eq!(db.revision(), before, "a rejected switch changes nothing");
    assert_eq!(
        db.block_visible_entities(BlockId(0)),
        Some(vec![EntityId(10), EntityId(11)])
    );
}

#[test]
fn visibility_for_a_block_without_a_parameter_is_rejected() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    let mut db = b.finish().unwrap();
    assert!(db.block_visible_entities(BlockId(0)).is_none());
    assert!(matches!(
        db.set_block_visibility_state(BlockId(0), "A", TransactionId(1), "no parameter"),
        Err(CadError::InvalidInput(_))
    ));
}

#[test]
fn builder_rejects_a_visibility_state_for_a_non_member_entity() {
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
        entities: vec![EntityId(1)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(
        1,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Point(point(0.0, 0.0)),
    ))
    .unwrap();
    // Entity 2 is not a member of the block.
    b.set_block_dynamic_visibility(
        BlockId(0),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(1)],
            states: vec![DynamicBlockState {
                name: "A".into(),
                entities: vec![EntityId(2)],
            }],
            active_state: Some("A".into()),
        },
    )
    .unwrap();
    assert!(b.finish().is_err(), "a non-member entity must be rejected");
}

#[test]
fn builder_rejects_duplicate_state_names_and_an_undefined_active_state() {
    use crate::tables::{DynamicBlockState, DynamicBlockVisibility};
    let make = |states: Vec<DynamicBlockState>, active: Option<&str>| {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(1)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(raw_entity(
            1,
            SpaceId::Block(BlockId(0)),
            SemanticGeometry::Point(point(0.0, 0.0)),
        ))
        .unwrap();
        b.set_block_dynamic_visibility(
            BlockId(0),
            DynamicBlockVisibility {
                member_entities: vec![EntityId(1)],
                states,
                active_state: active.map(str::to_string),
            },
        )
        .unwrap();
        b.finish()
    };
    let state = |name: &str| DynamicBlockState {
        name: name.into(),
        entities: vec![EntityId(1)],
    };
    assert!(make(vec![state("A"), state("A")], Some("A")).is_err());
    assert!(make(vec![state("A")], Some("B")).is_err());
    assert!(make(vec![state("A")], Some("A")).is_ok());
}
