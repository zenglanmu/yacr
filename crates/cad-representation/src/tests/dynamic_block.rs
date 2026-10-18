use super::*;

#[test]
fn only_the_active_state_members_are_expanded() {
    // Active state A: only the state-A lines are emitted (member 52 absent).
    let db = dynamic_block_db("A");
    assert_eq!(emitted_member_xs(&db), vec![0.0, 1.0]);

    // Active state B: only state-B lines (member 50 absent).
    let db = dynamic_block_db("B");
    assert_eq!(emitted_member_xs(&db), vec![1.0, 2.0]);
}

#[test]
fn switching_state_rederives_from_the_same_database() {
    // Switching the authoritative active state must change the derived
    // geometry without any re-import of the base drawing.
    let mut db = dynamic_block_db("A");
    assert_eq!(emitted_member_xs(&db), vec![0.0, 1.0]);
    db.set_block_visibility_state(BlockId(5), "B", TransactionId(1), "switch")
        .unwrap();
    assert_eq!(emitted_member_xs(&db), vec![1.0, 2.0]);
}

#[test]
fn an_unresolved_active_state_expands_every_member() {
    use cad_db::{DynamicBlockState, DynamicBlockVisibility};
    let mut b = empty_db();
    b.insert_block(BlockDefinition {
        id: BlockId(5),
        entities: vec![EntityId(50), EntityId(51)],
        dynamic_visibility: None,
    })
    .unwrap();
    for (i, id) in [50u128, 51].into_iter().enumerate() {
        b.insert_entity(line_entity(
            id,
            SpaceId::Block(BlockId(5)),
            p(i as f64, 0.0),
            p(i as f64, 1.0),
        ))
        .unwrap();
    }
    b.set_block_dynamic_visibility(
        BlockId(5),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(50), EntityId(51)],
            states: vec![DynamicBlockState {
                name: "A".into(),
                entities: vec![EntityId(50)],
            }],
            active_state: None,
        },
    )
    .unwrap();
    b.insert_entity(insert_entity(1, SpaceId::Model, 5, 0.0))
        .unwrap();
    let db = b.finish().unwrap();
    // No resolved state: both members are emitted (never hide on a guess).
    assert_eq!(emitted_member_xs(&db), vec![0.0, 1.0]);
}
