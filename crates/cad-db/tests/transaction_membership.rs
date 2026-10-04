//! Contract regressions for space ownership and dynamic visibility changes.

use cad_db::*;
use cad_domain::*;

fn entity(id: u128, space: SpaceId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space,
        geometry: SemanticGeometry::Line {
            start: Point3::default(),
            end: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        },
        draw_order: 0,
    }
}

fn database(active_state: Option<&str>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for id in 1..=3 {
        builder
            .insert_entity(entity(id, SpaceId::Block(BlockId(10))))
            .unwrap();
    }
    builder
        .insert_block(BlockDefinition {
            id: BlockId(10),
            entities: vec![EntityId(1), EntityId(2), EntityId(3)],
            dynamic_visibility: Some(DynamicBlockVisibility {
                member_entities: vec![EntityId(1), EntityId(2)],
                states: vec![
                    DynamicBlockState {
                        name: "first".into(),
                        entities: vec![EntityId(1)],
                    },
                    DynamicBlockState {
                        name: "second".into(),
                        entities: vec![EntityId(2)],
                    },
                ],
                active_state: active_state.map(str::to_string),
            }),
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(20),
            entities: Vec::new(),
            dynamic_visibility: None,
        })
        .unwrap();
    builder.finish().unwrap()
}

#[test]
fn moving_entity_between_blocks_prunes_old_visibility_references() {
    let mut db = database(Some("first"));
    let mut moved = db.entity(EntityId(1)).unwrap().clone();
    moved.space = SpaceId::Block(BlockId(20));
    let mut tx = db
        .begin_drawing_transaction("move to another block", TransactionId(1))
        .unwrap();
    tx.update_entity(moved).unwrap();
    let changes = tx.commit().unwrap();

    assert_eq!(changes.after, Revision(1));
    assert!(!db
        .block(BlockId(10))
        .unwrap()
        .entities
        .contains(&EntityId(1)));
    assert_eq!(db.block(BlockId(20)).unwrap().entities, vec![EntityId(1)]);
    let visibility = db.block_dynamic_visibility(BlockId(10)).unwrap();
    assert!(!visibility.member_entities.contains(&EntityId(1)));
    assert!(visibility
        .states
        .iter()
        .all(|state| !state.entities.contains(&EntityId(1))));
    assert!(db
        .block_entities(BlockId(10))
        .iter()
        .all(|entity| entity.id != EntityId(1)));

    let changes = db
        .set_block_visibility_state(BlockId(10), "second", TransactionId(2), "switch")
        .unwrap();
    assert_eq!(
        changes.changes,
        vec![ObjectChange::Update(ObjectId(2), ChangeMask::GEOMETRY)]
    );
}

#[test]
fn moving_entity_to_model_space_removes_old_block_membership() {
    let mut db = database(Some("first"));
    let mut moved = db.entity(EntityId(1)).unwrap().clone();
    moved.space = SpaceId::Model;
    db.apply_drawing_changes(
        "move to model space",
        TransactionId(1),
        vec![(EntityId(1), Some(moved))],
    )
    .unwrap();

    assert_eq!(db.model_space()[0].id, EntityId(1));
    assert!(!db
        .block(BlockId(10))
        .unwrap()
        .entities
        .contains(&EntityId(1)));
    assert!(db
        .block_entities(BlockId(10))
        .iter()
        .all(|entity| entity.id != EntityId(1)));
}

#[test]
fn failed_batch_leaves_space_membership_and_visibility_unchanged() {
    let mut db = database(Some("first"));
    let before_block = db.block(BlockId(10)).unwrap().clone();
    let before_entity = db.entity(EntityId(1)).unwrap().clone();
    let before_allocator = db.next_entity_id();
    let mut moved = before_entity.clone();
    moved.space = SpaceId::Block(BlockId(20));
    let mut invalid = entity(4, SpaceId::Model);
    invalid.layer = LayerId(99);
    let result = db.apply_drawing_changes(
        "invalid mixed batch",
        TransactionId(1),
        vec![(EntityId(1), Some(moved)), (EntityId(4), Some(invalid))],
    );

    assert!(matches!(result, Err(CadError::Invariant(_))));
    assert_eq!(db.revision(), Revision(0));
    assert_eq!(db.next_entity_id(), before_allocator);
    assert_eq!(db.block(BlockId(10)), Some(&before_block));
    assert!(db.block(BlockId(20)).unwrap().entities.is_empty());
    assert_eq!(db.entity(EntityId(1)), Some(&before_entity));
    assert!(db.entity(EntityId(4)).is_none());
}

#[test]
fn visibility_switch_does_not_report_ungoverned_entities_as_changed() {
    for active in [Some("first"), None] {
        let mut db = database(active);
        let changes = db
            .set_block_visibility_state(BlockId(10), "second", TransactionId(1), "switch")
            .unwrap();
        let mut expected = vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)];
        if active.is_some() {
            expected.push(ObjectChange::Update(ObjectId(2), ChangeMask::GEOMETRY));
        }
        assert_eq!(changes.changes, expected);
        assert_eq!(changes.before, Revision(0));
        assert_eq!(changes.after, Revision(1));
        assert_eq!(
            db.block_visible_entities(BlockId(10)).unwrap(),
            vec![EntityId(2), EntityId(3)]
        );
    }
}

#[test]
fn equivalent_visibility_sets_change_state_without_geometry_delta() {
    let mut db = database(Some("first"));
    let mut moved = db.entity(EntityId(1)).unwrap().clone();
    moved.space = SpaceId::Model;
    let mut moved_second = db.entity(EntityId(2)).unwrap().clone();
    moved_second.space = SpaceId::Model;
    db.apply_drawing_changes(
        "move governed entities out",
        TransactionId(1),
        vec![
            (EntityId(1), Some(moved)),
            (EntityId(2), Some(moved_second)),
        ],
    )
    .unwrap();
    let changes = db
        .set_block_visibility_state(BlockId(10), "second", TransactionId(2), "switch")
        .unwrap();

    assert!(changes.changes.is_empty());
    assert_eq!(changes.before, Revision(1));
    assert_eq!(changes.after, Revision(2));
    assert_eq!(
        db.block_dynamic_visibility(BlockId(10))
            .unwrap()
            .active_state
            .as_deref(),
        Some("second")
    );
    assert_eq!(
        db.block_visible_entities(BlockId(10)).unwrap(),
        vec![EntityId(3)]
    );
}
