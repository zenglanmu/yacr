//! The dirty scope of one commit (`crate::scope`).
//!
//! These tests pin the two facts a saver relies on and that must coexist:
//! `ChangeSet::changes` may be **empty** (a layer/layout/style/header write is
//! not an entity change) while `ChangeSet::scope` names exactly what must be
//! rewritten. Each test fails if a write path stops filling its scope.

use std::collections::BTreeSet;

use super::*;

/// Layer `0`, one paper layout and one model-space line on it.
fn scope_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    let mut entity = raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    );
    entity.layer = LayerId(0);
    b.insert_entity(entity).unwrap();
    b.finish().unwrap()
}

fn shard_of(seq: u128) -> cad_domain::ymodel::Shard {
    cad_domain::ymodel::shard_of(seq)
}

fn model_key(space: SpaceId) -> cad_domain::ymodel::ModelKey {
    cad_domain::ymodel::ModelKey::from_space(space)
}

// ---------------------------------------------------------------------------
// Table rows: no object change, but a dirty table and (on create) the header
// ---------------------------------------------------------------------------

#[test]
fn create_and_rename_layer_publish_no_entity_change_but_dirty_the_table() {
    let mut db = scope_db();

    let (id, created) = db
        .create_layer("WALLS", TransactionId(1), "create layer")
        .expect("a fresh layer is accepted");
    // The historical fact the rest of the system depends on: no object change.
    assert!(created.changes.is_empty());
    assert_eq!(
        created.scope.tables,
        BTreeSet::from([TableKind::Layers]),
        "the layer row is dirty even though no entity changed"
    );
    assert!(
        created.scope.header,
        "a created row mints an id, so the persisted high-water mark moves"
    );
    assert!(created.scope.shards.is_empty());

    let renamed = db
        .rename_layer(id, "WALLS-2", TransactionId(2), "rename layer")
        .expect("a fresh spelling is accepted");
    assert!(renamed.changes.is_empty());
    assert_eq!(renamed.scope.tables, BTreeSet::from([TableKind::Layers]));
    assert!(
        !renamed.scope.header,
        "a rename mints no id, so document.json is unchanged"
    );
    assert!(renamed.scope.shards.is_empty());
}

#[test]
fn renaming_a_layer_to_its_stored_name_is_an_empty_scope_and_change_set() {
    let mut db = scope_db();
    let no_op = db
        .rename_layer(LayerId(0), "0", TransactionId(1), "rename layer")
        .expect("the stored spelling is accepted");
    assert!(no_op.changes.is_empty());
    assert!(
        no_op.scope.is_empty(),
        "a no-op must not claim any row is dirty: {:?}",
        no_op.scope
    );
}

// ---------------------------------------------------------------------------
// Element rows: new shard on insert, old shard on delete, both on a space move
// ---------------------------------------------------------------------------

#[test]
fn inserting_an_entity_dirties_the_shard_of_its_space_and_deleting_it_the_old_one() {
    let mut db = scope_db();
    let inserted = db
        .apply_drawing_changes(
            "insert",
            TransactionId(1),
            vec![(
                EntityId(5),
                Some(raw_entity(
                    5,
                    SpaceId::Model,
                    SemanticGeometry::Line {
                        start: point(2.0, 0.0),
                        end: point(3.0, 0.0),
                    },
                )),
            )],
        )
        .expect("a plain model-space insert is accepted");
    assert_eq!(
        inserted.scope.shards,
        BTreeSet::from([(model_key(SpaceId::Model), shard_of(5))])
    );
    assert!(
        inserted.scope.prev_shards.is_empty(),
        "a new id has no old row"
    );
    assert!(
        inserted.scope.header,
        "the new row's id must be reflected by the persisted watermark"
    );

    let deleted = db
        .apply_drawing_changes("delete", TransactionId(2), vec![(EntityId(5), None)])
        .expect("the row exists");
    assert!(
        deleted.scope.shards.is_empty(),
        "a delete writes no new row"
    );
    assert!(
        deleted
            .scope
            .prev_shards
            .contains(&(model_key(SpaceId::Model), shard_of(5))),
        "the shard the old row occupied must be rewritten: {:?}",
        deleted.scope
    );
    assert!(!deleted.scope.header);
}

#[test]
fn moving_an_entity_into_a_paper_layout_dirties_both_shards() {
    let mut db = scope_db();
    let mut moved = db.entity(EntityId(1)).expect("the line exists").clone();
    moved.space = SpaceId::Paper(LayoutId(1));
    let changeset = db
        .apply_drawing_changes(
            "move to paper",
            TransactionId(1),
            vec![(EntityId(1), Some(moved))],
        )
        .expect("layout 1 exists");

    assert!(
        changeset
            .scope
            .shards
            .contains(&(model_key(SpaceId::Paper(LayoutId(1))), shard_of(1))),
        "the new space's shard must be rewritten: {:?}",
        changeset.scope
    );
    assert!(
        changeset
            .scope
            .prev_shards
            .contains(&(model_key(SpaceId::Model), shard_of(1))),
        "the old model-space shard loses the row: {:?}",
        changeset.scope
    );
}

// ---------------------------------------------------------------------------
// Block definition rows are a table; their members are Block(id) elements
// ---------------------------------------------------------------------------

#[test]
fn block_definition_rows_and_members_dirty_different_models() {
    let mut db = scope_db();
    let block = db.allocate_block_id();

    let created = db
        .apply_drawing_changes_with_blocks(
            "create block",
            TransactionId(1),
            &[(
                block,
                Some(crate::drawing::BlockRow {
                    definition: BlockDefinition {
                        id: block,
                        entities: Vec::new(),
                        dynamic_visibility: None,
                    },
                    metadata: Some(BlockMetadata {
                        name: "B1".into(),
                        base_point: point(0.0, 0.0),
                    }),
                }),
            )],
            Vec::new(),
        )
        .expect("a clean empty block creates");
    assert!(
        created.scope.tables.contains(&TableKind::Blocks),
        "the definition row is a table in v1: {:?}",
        created.scope
    );
    assert!(
        created.scope.shards.is_empty(),
        "a definition row is not an element shard: {:?}",
        created.scope
    );
    assert!(
        created.scope.header,
        "a created block id moves the watermark"
    );

    let member = raw_entity(
        9,
        SpaceId::Block(block),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    );
    let added = db
        .apply_drawing_changes(
            "add member",
            TransactionId(2),
            vec![(EntityId(9), Some(member))],
        )
        .expect("the block exists");
    assert!(
        added
            .scope
            .shards
            .contains(&(model_key(SpaceId::Block(block)), shard_of(9))),
        "a member is an element of the block model: {:?}",
        added.scope
    );
    assert!(
        !added
            .scope
            .prev_shards
            .contains(&(model_key(SpaceId::Block(block)), shard_of(9))),
        "adding a member must not mark the old side: {:?}",
        added.scope
    );
}

// ---------------------------------------------------------------------------
// Header-only writes: document.json dirty, nothing else
// ---------------------------------------------------------------------------

#[test]
fn header_writes_dirty_only_the_header() {
    let mut db = scope_db();
    let scale = db
        .set_linetype_scale(2.0, TransactionId(1), "ltscale")
        .expect("a positive scale is accepted");
    assert!(scale.scope.header);
    assert!(scale.scope.shards.is_empty());
    assert!(scale.scope.prev_shards.is_empty());
    assert!(scale.scope.tables.is_empty());

    let base = db
        .set_insertion_base(
            Point3 {
                x: 3.0,
                y: 4.0,
                z: 0.0,
            },
            TransactionId(2),
            "base",
        )
        .expect("a finite base is accepted");
    assert!(base.scope.header);
    assert!(base.scope.shards.is_empty());
    assert!(base.scope.tables.is_empty());

    // Storing the value that is already there is not a write.
    let no_op = db
        .set_insertion_base(
            Point3 {
                x: 3.0,
                y: 4.0,
                z: 0.0,
            },
            TransactionId(3),
            "base",
        )
        .expect("the same base is accepted");
    assert!(no_op.scope.is_empty());
    assert!(no_op.changes.is_empty());
}

// ---------------------------------------------------------------------------
// Scale rows: table + the id watermark on create
// ---------------------------------------------------------------------------

#[test]
fn creating_a_scale_dirties_the_scale_table_and_the_header() {
    let mut db = scope_db();
    let (created, _row) = db
        .create_scale("1:200", 1.0, 200.0, TransactionId(1), "scalelist add")
        .expect("a fresh, finite scale is accepted");
    assert!(created.changes.is_empty(), "a table row is not an entity");
    assert_eq!(created.scope.tables, BTreeSet::from([TableKind::Scales]));
    assert!(created.scope.header);
    assert!(created.scope.shards.is_empty());
}
