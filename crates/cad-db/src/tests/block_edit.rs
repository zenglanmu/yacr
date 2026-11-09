//! Contract tests for the atomic block-table write core (`drawing::block_edit`).
//!
//! A `BLOCK`-shaped edit is one transaction: create the definition row, move the
//! selected entities into `SpaceId::Block(new)` with their geometry translated
//! by `T(-base)`, and insert one model-space reference carrying `T(base)`.
//! Everything below pins the all-or-nothing contract, the allocator rules and
//! the member-list invariant.

use std::collections::BTreeSet;

use super::*;

/// Layer 0, `WALLS` (unlocked), layout 1, an unnamed writable block `0`, a named
/// empty block `EXIST` (id 7), and two model lines (entities 1 and 2).
fn block_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(5), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
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
    b.insert_block(BlockDefinition {
        id: BlockId(7),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.set_block_metadata(BlockId(7), "EXIST", point(0.0, 0.0))
        .unwrap();
    let mut first = line_entity(1, point(0.0, 0.0), point(4.0, 0.0));
    first.layer = LayerId(0);
    b.insert_entity(first).unwrap();
    let mut second = line_entity(2, point(4.0, 0.0), point(4.0, 3.0));
    second.layer = LayerId(5);
    b.insert_entity(second).unwrap();
    b.finish().unwrap()
}

fn row(name: &str, id: BlockId, members: Vec<EntityId>, base: Point3) -> BlockRow {
    BlockRow {
        definition: BlockDefinition {
            id,
            entities: members,
            dynamic_visibility: None,
        },
        metadata: Some(BlockMetadata {
            name: name.into(),
            base_point: base,
        }),
    }
}

fn reference_entity(id: u128, block: BlockId, base: Point3) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block,
            transform: Transform3::translation(base),
        },
        draw_order: id as i64,
    }
}

/// The `BLOCK`-command member shape: same id/layer/draw order, translated by
/// `T(-base)`, owned by the new block.
fn moved_into(db: &DrawingDatabase, id: EntityId, block: BlockId, base: Point3) -> DbEntity {
    let mut entity = db.entity(id).expect("stored member").clone();
    entity.geometry = crate::transform_geometry(
        &entity.geometry,
        &Transform3::translation(point(-base.x, -base.y)),
    )
    .expect("a line translates exactly");
    entity.space = SpaceId::Block(block);
    entity
}

/// Run a combined batch and assert it is refused with `code`, leaving the
/// revision, the allocators and every stored table exactly as they were.
fn assert_refused_unchanged(
    mut db: DrawingDatabase,
    block_changes: &[(BlockId, Option<BlockRow>)],
    entities: Vec<(EntityId, Option<DbEntity>)>,
    code: &str,
) {
    let revision = db.revision();
    let next_entity = db.next_entity_id();
    let next_block = db.next_block_id();
    let snapshot = format!("{db:?}");
    let error = db
        .apply_drawing_changes_with_blocks(
            "refused combined batch",
            TransactionId(1),
            block_changes,
            entities,
        )
        .expect_err("the batch must be refused");
    assert!(
        error.to_string().contains(code),
        "expected stable code {code:?}, got {error}"
    );
    assert_eq!(
        db.revision(),
        revision,
        "a refusal does not bump the revision"
    );
    assert_eq!(
        db.next_entity_id(),
        next_entity,
        "a refusal does not advance the entity allocator"
    );
    assert_eq!(
        db.next_block_id(),
        next_block,
        "a refusal does not advance the block allocator"
    );
    assert_eq!(
        format!("{db:?}"),
        snapshot,
        "a refusal leaves the database unchanged (tables, allocators, rows)"
    );
}

fn create(db: &mut DrawingDatabase, block: BlockId, name: &str, members: Vec<EntityId>) {
    db.apply_drawing_changes_with_blocks(
        "create block",
        TransactionId(100),
        &[(block, Some(row(name, block, members, point(0.0, 0.0))))],
        Vec::new(),
    )
    .expect("a clean empty block creates");
}

#[test]
fn atomic_create_moves_members_and_inserts_the_reference_in_one_revision() {
    let mut db = block_db();
    let before = db.revision();
    let block = db.allocate_block_id();
    assert_eq!(block, BlockId(8), "the builder initialises max(id)+1");
    let base = point(4.0, 0.0);

    let member1 = moved_into(&db, EntityId(1), block, base);
    let member2 = moved_into(&db, EntityId(2), block, base);
    let reference = reference_entity(3, block, base);

    let mut tx = db
        .begin_drawing_transaction("BLOCK", TransactionId(1))
        .unwrap();
    assert_eq!(
        tx.stage_create_block(row("B1", block, vec![EntityId(1), EntityId(2)], base)),
        Ok(block)
    );
    tx.update_entity(member1).unwrap();
    tx.update_entity(member2).unwrap();
    tx.insert_entity(reference).unwrap();
    assert_eq!(tx.staged_len(), 4, "block row + two members + reference");
    let changes = tx.commit().expect("one atomic commit");

    // Exactly one revision, one change set, caller order preserved.
    assert_eq!(changes.before, before);
    assert_eq!(changes.after, Revision(before.0 + 1));
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert!(changes.follows(db.id(), before));
    assert_eq!(
        changes.changes,
        vec![
            ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY.union(ChangeMask::STYLE)),
            ObjectChange::Update(ObjectId(2), ChangeMask::GEOMETRY.union(ChangeMask::STYLE)),
            ObjectChange::Insert(ObjectId(3)),
        ],
        "member updates then the reference insert, in staging order"
    );

    // The row, its metadata and both name resolutions exist.
    assert_eq!(
        db.block(block).expect("the created row").entities,
        vec![EntityId(1), EntityId(2)]
    );
    assert_eq!(db.block_name(block), Some("B1"));
    assert_eq!(
        db.block_base_point(block),
        Some(base),
        "the stored base point is the one the transform uses"
    );
    assert!(db.block_by_name("B1").is_some());
    assert!(
        db.block_by_name("b1").is_some(),
        "name resolution stays exact-then-case-insensitive"
    );

    // Member identity/placement is preserved; only space and geometry moved.
    for (id, layer, order) in [
        (EntityId(1), LayerId(0), 1i64),
        (EntityId(2), LayerId(5), 2i64),
    ] {
        let member = db.entity(id).expect("member survives the conversion");
        assert_eq!(member.layer, layer, "the member keeps its layer");
        assert_eq!(member.draw_order, order, "the member keeps its draw order");
        assert_eq!(member.space, SpaceId::Block(block));
    }
    assert_eq!(
        db.entity(EntityId(1)).unwrap().geometry,
        SemanticGeometry::Line {
            start: point(-4.0, 0.0),
            end: point(0.0, 0.0),
        },
        "geometry is translated by T(-base)"
    );

    // The reference is a normal model-space INSERT carrying T(base).
    let reference = db.entity(EntityId(3)).expect("the reference row");
    assert_eq!(reference.space, SpaceId::Model);
    match &reference.geometry {
        SemanticGeometry::Insert {
            block: referenced,
            transform,
        } => {
            assert_eq!(*referenced, block);
            assert_eq!(transform.apply_point(point(0.0, 0.0)), base);
        }
        other => panic!("expected an INSERT, got {other:?}"),
    }

    // Both allocators stay ahead of everything the batch applied.
    assert_eq!(db.next_block_id(), BlockId(9));
    assert_eq!(db.next_entity_id(), EntityId(4));
}

#[test]
fn a_refused_combined_batch_changes_nothing() {
    let base = point(1.0, 0.0);
    let block = BlockId(8);

    // Duplicate name, exact …
    assert_refused_unchanged(
        block_db(),
        &[(block, Some(row("EXIST", block, Vec::new(), base)))],
        Vec::new(),
        "block.duplicate_name",
    );
    // … and case-insensitive.
    assert_refused_unchanged(
        block_db(),
        &[(block, Some(row("exist", block, Vec::new(), base)))],
        Vec::new(),
        "block.duplicate_name",
    );

    // A member that does not exist post-batch is a phantom member.
    assert_refused_unchanged(
        block_db(),
        &[(block, Some(row("B1", block, vec![EntityId(99)], base)))],
        Vec::new(),
        "block.member_list_mismatch",
    );

    // A non-finite base point would poison every reference transform.
    let mut non_finite = row("B1", block, Vec::new(), base);
    non_finite.metadata.as_mut().unwrap().base_point = point(f64::NAN, 0.0);
    assert_refused_unchanged(
        block_db(),
        &[(block, Some(non_finite))],
        Vec::new(),
        "block.base_point_non_finite",
    );

    // A runtime create cannot carry a dynamic-visibility descriptor.
    let mut dynamic = row("B1", block, Vec::new(), base);
    dynamic.definition.dynamic_visibility = Some(DynamicBlockVisibility {
        member_entities: Vec::new(),
        states: Vec::new(),
        active_state: None,
    });
    assert_refused_unchanged(
        block_db(),
        &[(block, Some(dynamic))],
        Vec::new(),
        "block.dynamic_visibility_unsupported",
    );

    // A locked member layer is refused by the existing lock chokepoint, before
    // any block-row mutation.
    let mut locked = block_db();
    let mut attributes = locked.layer_attributes(LayerId(5));
    attributes.locked = true;
    locked
        .set_layer_attributes(LayerId(5), attributes, TransactionId(2), "lock")
        .unwrap();
    let member = moved_into(&locked, EntityId(2), block, base);
    assert_refused_unchanged(
        locked,
        &[(block, Some(row("B1", block, vec![EntityId(2)], base)))],
        vec![(EntityId(2), Some(member))],
        "layer.locked",
    );

    // A reference to a block that is neither stored nor staged is dangling.
    let db = block_db();
    let mut dangling = db.entity(EntityId(1)).unwrap().clone();
    dangling.space = SpaceId::Block(BlockId(77));
    assert_refused_unchanged(
        db,
        &[],
        vec![(EntityId(1), Some(dangling))],
        "references missing block",
    );
}

#[test]
fn an_entity_only_batch_inserts_updates_and_deletes_in_one_revision() {
    let mut db = block_db();
    let before = db.revision();
    let mut updated = db.entity(EntityId(1)).unwrap().clone();
    updated.geometry = SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(9.0, 0.0),
    };
    // Block 7 is stored, so this INSERT names a valid reference without staging
    // any block row.
    let inserted = reference_entity(3, BlockId(7), point(1.0, 2.0));

    let changes = db
        .apply_drawing_changes_with_blocks(
            "entity-only batch",
            TransactionId(1),
            &[],
            vec![
                (EntityId(1), Some(updated.clone())),
                (EntityId(3), Some(inserted)),
                (EntityId(2), None),
            ],
        )
        .expect("an entity-only batch must not need the block-batch view");

    assert_eq!(changes.before, before);
    assert_eq!(changes.after, Revision(before.0 + 1));
    assert_eq!(db.revision(), Revision(before.0 + 1));
    assert_eq!(
        changes.changes,
        vec![
            ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY),
            ObjectChange::Insert(ObjectId(3)),
            ObjectChange::Delete(ObjectId(2)),
        ],
        "caller order preserved, exactly like a plain entity batch"
    );
    assert_eq!(db.entity(EntityId(1)).unwrap().geometry, updated.geometry);
    assert!(db.entity(EntityId(2)).is_none());
    assert!(db.entity(EntityId(3)).is_some());
    assert!(db.block(BlockId(8)).is_none(), "no block row was invented");
    assert_eq!(db.next_block_id(), BlockId(8));
    assert_eq!(db.next_entity_id(), EntityId(4));
}

#[test]
fn an_invalid_entity_only_batch_rolls_back_without_revision_or_allocator_moves() {
    let db = block_db();

    // Deleting a row that does not exist is refused by the entity phase.
    assert_refused_unchanged(
        db.clone(),
        &[],
        vec![(EntityId(99), None)],
        "does not exist",
    );

    // The same entity twice is a transaction-shape error.
    let mut first = db.entity(EntityId(1)).unwrap().clone();
    first.geometry = SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(1.0, 0.0),
    };
    assert_refused_unchanged(
        db.clone(),
        &[],
        vec![(EntityId(1), Some(first)), (EntityId(1), None)],
        "modifies entity",
    );

    // A space naming a missing block is still refused with no block row staged,
    // so skipping the block-batch view cannot admit a dangling reference.
    let mut orphan = db.entity(EntityId(1)).unwrap().clone();
    orphan.space = SpaceId::Block(BlockId(77));
    assert_refused_unchanged(
        db,
        &[],
        vec![(EntityId(1), Some(orphan))],
        "references missing block",
    );
}

#[test]
fn a_combined_block_batch_is_still_validated_when_entities_are_staged() {
    let base = point(4.0, 0.0);
    let block = BlockId(8);

    // The block row claims no members, but the same batch moves entity 1 into
    // its space. The post-batch member list — built only because block rows
    // exist — must still catch the mismatch.
    let db = block_db();
    let member = moved_into(&db, EntityId(1), block, base);
    assert_refused_unchanged(
        db,
        &[(block, Some(row("B1", block, Vec::new(), base)))],
        vec![(EntityId(1), Some(member))],
        "block.member_list_mismatch",
    );

    // The same combined shape with a matching member list commits once.
    let mut db = block_db();
    let member = moved_into(&db, EntityId(1), block, base);
    let before = db.revision();
    let changes = db
        .apply_drawing_changes_with_blocks(
            "combined create",
            TransactionId(1),
            &[(block, Some(row("B1", block, vec![EntityId(1)], base)))],
            vec![(EntityId(1), Some(member))],
        )
        .expect("the member list matches the post-batch view");
    assert_eq!(changes.after, Revision(before.0 + 1));
    assert_eq!(db.block(block).unwrap().entities, vec![EntityId(1)]);
    assert_eq!(db.entity(EntityId(1)).unwrap().space, SpaceId::Block(block));
}

#[test]
fn the_block_allocator_is_monotonic_across_deletes_and_replays() {
    let mut db = block_db();
    assert_eq!(db.next_block_id(), BlockId(8));

    let first = db.allocate_block_id();
    assert_eq!(first, BlockId(8));
    create(&mut db, first, "A", Vec::new());
    assert_eq!(db.next_block_id(), BlockId(9));

    // Deleting the block does not rewind the allocator: an id carries a
    // generation-like meaning.
    db.apply_drawing_changes_with_blocks(
        "delete A",
        TransactionId(101),
        &[(first, None)],
        Vec::new(),
    )
    .expect("a clean block deletes");
    assert_eq!(db.next_block_id(), BlockId(9));

    let second = db.allocate_block_id();
    assert_eq!(second, BlockId(9));
    assert_ne!(second, first);

    // A replayed recorded id (history/undo shape) bumps the allocator past it.
    let replayed = BlockId(100);
    create(&mut db, replayed, "REPLAY", Vec::new());
    assert_eq!(db.next_block_id(), BlockId(101));
    assert_eq!(db.allocate_block_id(), BlockId(101));
}

#[test]
fn block_delete_is_refused_while_referenced_and_cleans_up_once_free() {
    let mut db = block_db();
    let block = db.allocate_block_id();
    let base = point(0.0, 0.0);
    let member = moved_into(&db, EntityId(1), block, base);
    let reference = reference_entity(3, block, base);
    db.apply_drawing_changes_with_blocks(
        "BLOCK",
        TransactionId(1),
        &[(block, Some(row("B1", block, vec![EntityId(1)], base)))],
        vec![
            (EntityId(1), Some(member.clone())),
            (EntityId(3), Some(reference)),
        ],
    )
    .expect("one atomic create");
    let revision = db.revision();

    // The member still owns the old block space.
    assert_refused_unchanged(db.clone(), &[(block, None)], Vec::new(), "block.in_use");
    // Moving the member out is not enough while the INSERT survives.
    let mut back_to_model = member;
    back_to_model.space = SpaceId::Model;
    assert_refused_unchanged(
        db.clone(),
        &[(block, None)],
        vec![(EntityId(1), Some(back_to_model))],
        "block.in_use",
    );

    // Move the member out and delete the reference: the block is free.
    let mut back_to_model = db.entity(EntityId(1)).unwrap().clone();
    back_to_model.space = SpaceId::Model;
    let changes = db
        .apply_drawing_changes_with_blocks(
            "undo BLOCK",
            TransactionId(2),
            &[(block, None)],
            vec![(EntityId(1), Some(back_to_model)), (EntityId(3), None)],
        )
        .expect("nothing references the block any more");
    assert_eq!(changes.before, revision);
    assert_eq!(changes.after, Revision(revision.0 + 1));

    assert!(db.block(block).is_none(), "the row is gone");
    assert!(db.block_metadata(block).is_none(), "the metadata is gone");
    assert!(
        db.block_name(block).is_none(),
        "the name index entry is gone"
    );
    assert!(
        db.block_by_name("B1").is_none(),
        "the name no longer resolves"
    );
    assert!(
        db.block_entities(block).is_empty(),
        "no member is reachable through the deleted block"
    );

    // The name is free again, and the allocator is still past the deleted id.
    assert_eq!(db.next_block_id(), BlockId(9));
    let replacement = db.allocate_block_id();
    create(&mut db, replacement, "B1", Vec::new());
    assert!(db.block_by_name("B1").is_some());
}

#[test]
fn a_staged_block_is_a_valid_reference_target_only_inside_the_batch() {
    let db = block_db();
    let new = BlockId(8);
    let mut member = db.entity(EntityId(1)).unwrap().clone();
    member.space = SpaceId::Block(new);
    let insert = reference_entity(3, new, point(0.0, 0.0));

    // Standalone validation refuses the not-yet-existing definition.
    assert!(
        crate::validate::validate_entity(&db, EntityId(1), &member).is_err(),
        "SpaceId::Block(new) is dangling outside the batch"
    );
    assert!(
        crate::validate::validate_entity(&db, EntityId(3), &insert).is_err(),
        "Insert {{ block: new }} is dangling outside the batch"
    );

    // The same rows are valid with the batch's block id staged.
    let staged: BTreeSet<BlockId> = [new].into_iter().collect();
    let no_layers: BTreeSet<LayerId> = BTreeSet::new();
    let no_layouts: BTreeSet<LayoutId> = BTreeSet::new();
    crate::validate::validate_entity_in(
        &db,
        EntityId(1),
        &member,
        &staged,
        &no_layers,
        &no_layouts,
    )
    .expect("the staged block owns this member");
    crate::validate::validate_entity_in(
        &db,
        EntityId(3),
        &insert,
        &staged,
        &no_layers,
        &no_layouts,
    )
    .expect("the staged block is an insert target");
}

#[test]
fn converting_to_a_block_and_back_preserves_bounds_and_membership() {
    let mut db = block_db();
    let original_bounds = db.bounds().expect("the two model lines have bounds");
    let block = db.allocate_block_id();
    let base = point(4.0, 0.0);

    let member1 = moved_into(&db, EntityId(1), block, base);
    let member2 = moved_into(&db, EntityId(2), block, base);
    let mut tx = db
        .begin_drawing_transaction("BLOCK", TransactionId(1))
        .unwrap();
    tx.stage_create_block(row("B1", block, vec![EntityId(1), EntityId(2)], base))
        .unwrap();
    tx.update_entity(member1).unwrap();
    tx.update_entity(member2).unwrap();
    tx.insert_entity(reference_entity(3, block, base)).unwrap();
    tx.commit().unwrap();

    assert_close(
        db.bounds().expect("the INSERT expands the block"),
        original_bounds,
    );
    assert_eq!(
        db.block_entities(block)
            .iter()
            .map(|entity| entity.id)
            .collect::<Vec<_>>(),
        vec![EntityId(1), EntityId(2)]
    );

    // Undo shape: members back to model space, reference deleted, row deleted —
    // still one revision.
    let mut model1 = db.entity(EntityId(1)).unwrap().clone();
    model1.space = SpaceId::Model;
    model1.geometry = SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(4.0, 0.0),
    };
    let mut model2 = db.entity(EntityId(2)).unwrap().clone();
    model2.space = SpaceId::Model;
    model2.geometry = SemanticGeometry::Line {
        start: point(4.0, 0.0),
        end: point(4.0, 3.0),
    };
    db.apply_drawing_changes_with_blocks(
        "undo BLOCK",
        TransactionId(2),
        &[(block, None)],
        vec![
            (EntityId(1), Some(model1)),
            (EntityId(2), Some(model2)),
            (EntityId(3), None),
        ],
    )
    .expect("the undo-shaped batch");

    assert!(db.block(block).is_none());
    assert!(
        !db.block(BlockId(0))
            .unwrap()
            .entities
            .contains(&EntityId(1)),
        "the member list was cleaned by the detach step"
    );
    assert_eq!(
        db.model_space()
            .iter()
            .map(|entity| entity.id)
            .collect::<Vec<_>>(),
        vec![EntityId(1), EntityId(2)]
    );
    assert_close(
        db.bounds().expect("model-space lines again"),
        original_bounds,
    );
}

#[test]
fn block_patch_records_the_before_and_after_rows() {
    let db = block_db();
    let before = BlockRow {
        definition: db.block(BlockId(7)).unwrap().clone(),
        metadata: db.block_metadata(BlockId(7)).cloned(),
    };
    let after = BlockRow {
        definition: BlockDefinition {
            id: BlockId(7),
            entities: Vec::new(),
            dynamic_visibility: None,
        },
        metadata: Some(BlockMetadata {
            name: "RENAMED".into(),
            base_point: point(1.0, 2.0),
        }),
    };
    let patch = BlockPatch {
        id: BlockId(7),
        before: Some(before.clone()),
        after: Some(after.clone()),
    };
    assert_eq!(patch.id, BlockId(7));
    assert_eq!(patch.before.as_ref(), Some(&before));
    assert_eq!(patch.after.as_ref(), Some(&after));
    // Revision bumps are the change signal; a block patch carries no ChangeSet.
    assert_eq!(db.revision(), Revision(0));
}

fn assert_close(actual: (Point3, Point3), expected: (Point3, Point3)) {
    for (got, want) in [(actual.0, expected.0), (actual.1, expected.1)] {
        assert!(
            (got.x - want.x).abs() < 1e-9
                && (got.y - want.y).abs() < 1e-9
                && (got.z - want.z).abs() < 1e-9,
            "bounds differ: got {got:?}, want {want:?}"
        );
    }
}
