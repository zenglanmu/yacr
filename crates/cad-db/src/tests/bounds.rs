//! Cached/memoized model-space bounds versus the reference algorithm.

use super::*;

// ---------------------------------------------------------------------------
// Cached / memoized model-space bounds
//
// The optimized `bounds()` is compared against
// `bounds_reference_for_test()`, which re-implements the original
// `model_space` + `block_entities` algorithm, so any semantic drift fails.
// ---------------------------------------------------------------------------

fn insert_at(block: u128, dx: f64) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BlockId(block),
        transform: Transform3::translation(point(dx, 0.0)),
    }
}

#[test]
fn block_entities_are_not_model_space_and_inserts_expand_in_bounds() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Block 0 owns a unit line; two model inserts place it at x=10 and x=20.
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(
        2,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    ))
    .unwrap();
    b.insert_entity(raw_entity(1, SpaceId::Model, insert_at(0, 10.0)))
        .unwrap();
    b.insert_entity(raw_entity(3, SpaceId::Model, insert_at(0, 20.0)))
        .unwrap();
    let db = b.finish().unwrap();

    // The block definition is not drawn top-level (audit B15).
    assert_eq!(db.model_space().len(), 2);
    assert_eq!(db.block_entities(BlockId(0)).len(), 1);
    // Bounds expand both instances: 10 .. 21.
    let (min, max) = db.bounds().unwrap();
    assert_eq!(min.x, 10.0);
    assert_eq!(max.x, 21.0);
}

#[test]
fn cyclic_block_reference_does_not_loop_bounds() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Block 0 inserts itself.
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(2, SpaceId::Block(BlockId(0)), insert_at(0, 1.0)))
        .unwrap();
    b.insert_entity(raw_entity(1, SpaceId::Model, insert_at(0, 0.0)))
        .unwrap();
    let db = b.finish().unwrap();
    // Must terminate; a self-referential block contributes no bounds.
    assert!(db.bounds().is_none());
}

/// Assert the cached/memoized bounds equal the original algorithm's result.
fn assert_bounds_match_reference(db: &DrawingDatabase) {
    assert_eq!(
        db.bounds(),
        db.bounds_reference_for_test(),
        "optimized bounds must equal the reference algorithm"
    );
}

fn rotate_degrees(degrees: f64) -> Transform3 {
    let theta = degrees.to_radians();
    let (sin, cos) = theta.sin_cos();
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = cos;
    matrix[0][1] = -sin;
    matrix[1][0] = sin;
    matrix[1][1] = cos;
    Transform3 { matrix }
}

/// A block whose member entity is placed in block space but not listed as a
/// member yet, plus a model line and a model INSERT of that block.
fn block_with_late_member_db() -> DrawingDatabase {
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
    b.insert_entity(raw_entity(
        10,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    ))
    .unwrap();
    b.insert_entity(raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Line {
            start: point(1000.0, 0.0),
            end: point(1001.0, 0.0),
        },
    ))
    .unwrap();
    b.insert_entity(raw_entity(
        2,
        SpaceId::Model,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::identity(),
        },
    ))
    .unwrap();
    b.finish().unwrap()
}

/// Block 0 owns two members governed by states "A" (member 10) and "B"
/// (member 11), with one model INSERT so the block contributes to bounds.
fn visibility_bounds_db(active: Option<&str>) -> DrawingDatabase {
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
    b.insert_entity(raw_entity(
        10,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    ))
    .unwrap();
    b.insert_entity(raw_entity(
        11,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(100.0, 0.0),
            end: point(101.0, 0.0),
        },
    ))
    .unwrap();
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
            active_state: active.map(str::to_string),
        },
    )
    .unwrap();
    b.insert_entity(raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(point(0.0, 0.0)),
        },
    ))
    .unwrap();
    b.finish().unwrap()
}

/// A chain of `length` nested blocks: block `i` inserts block `i + 1`, and the
/// last block holds a unit line; one model INSERT enters block 0.
fn block_chain_db(length: usize) -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    for i in 0..length {
        b.insert_block(BlockDefinition {
            id: BlockId(i as u128),
            entities: vec![EntityId(1000 + i as u128)],
            dynamic_visibility: None,
        })
        .unwrap();
        let geometry = if i + 1 < length {
            SemanticGeometry::Insert {
                block: BlockId((i + 1) as u128),
                transform: Transform3::translation(point(1.0, 0.0)),
            }
        } else {
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            }
        };
        b.insert_entity(raw_entity(
            1000 + i as u128,
            SpaceId::Block(BlockId(i as u128)),
            geometry,
        ))
        .unwrap();
    }
    b.insert_entity(raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::identity(),
        },
    ))
    .unwrap();
    b.finish().unwrap()
}

#[test]
fn model_bounds_are_cached_and_repeated_calls_are_identical() {
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("line", TransactionId(1))
        .unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(3.0, 4.0)))
        .unwrap();
    tx.commit().unwrap();

    let revision = db.revision();
    let first = db.bounds();
    assert_eq!(first, Some((point(0.0, 0.0), point(3.0, 4.0))));
    // The first call populated the cache at the current revision.
    assert_eq!(db.cached_bounds_revision_for_test(), Some(revision));

    let second = db.bounds();
    assert_eq!(first, second);
    // A repeat call with no mutation leaves the same (still-valid) cache entry.
    assert_eq!(db.cached_bounds_revision_for_test(), Some(revision));
}

#[test]
fn model_bounds_cache_tracks_insert_update_and_delete() {
    let mut db = writable_db();
    assert_eq!(db.bounds(), None);
    assert_bounds_match_reference(&db);

    let mut tx = db
        .begin_drawing_transaction("insert", TransactionId(1))
        .unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    tx.insert_entity(line_entity(2, point(0.0, 0.0), point(0.0, 5.0)))
        .unwrap();
    tx.commit().unwrap();
    assert_bounds_match_reference(&db);
    assert_eq!(db.bounds(), Some((point(0.0, 0.0), point(1.0, 5.0))));

    // Update entity 1: extend the x extent.
    let mut tx = db
        .begin_drawing_transaction("update", TransactionId(2))
        .unwrap();
    tx.update_entity(line_entity(1, point(0.0, 0.0), point(10.0, 0.0)))
        .unwrap();
    tx.commit().unwrap();
    assert_bounds_match_reference(&db);
    assert_eq!(db.bounds(), Some((point(0.0, 0.0), point(10.0, 5.0))));

    // Delete entity 2: the y extent collapses back to the line.
    let mut tx = db
        .begin_drawing_transaction("delete", TransactionId(3))
        .unwrap();
    tx.delete_entity(EntityId(2)).unwrap();
    tx.commit().unwrap();
    assert_bounds_match_reference(&db);
    assert_eq!(db.bounds(), Some((point(0.0, 0.0), point(10.0, 0.0))));
}

#[test]
fn model_bounds_cache_tracks_visibility_switches() {
    let mut db = visibility_bounds_db(Some("A"));
    assert_bounds_match_reference(&db);
    assert_eq!(db.bounds(), Some((point(0.0, 0.0), point(1.0, 0.0))));
    let revision = db.revision();
    assert_eq!(db.cached_bounds_revision_for_test(), Some(revision));

    let changes = db
        .set_block_visibility_state(BlockId(0), "B", TransactionId(1), "switch")
        .unwrap();
    assert_eq!(changes.after, Revision(revision.0 + 1));
    assert_bounds_match_reference(&db);
    assert_eq!(db.bounds(), Some((point(100.0, 0.0), point(101.0, 0.0))));
}

#[test]
fn model_bounds_cache_invalidates_on_display_state_restore() {
    let mut db = block_with_late_member_db();
    assert_bounds_match_reference(&db);
    assert_eq!(db.bounds(), Some((point(1000.0, 0.0), point(1001.0, 0.0))));
    let revision = db.revision();
    assert_eq!(db.cached_bounds_revision_for_test(), Some(revision));

    // `restore_entity_display_state` adds the entity to block 0 without raising
    // the revision: the cache must still be cleared or `bounds()` goes stale.
    db.restore_entity_display_state(
        EntityId(10),
        &EntityDisplayState {
            attributes: EntityRenderAttributes::default(),
            visibility: VisibilityMembership {
                blocks: vec![BlockId(0)],
                visibility: Vec::new(),
            },
        },
    );
    assert_eq!(db.revision(), revision);
    assert_eq!(db.cached_bounds_revision_for_test(), None);
    assert_bounds_match_reference(&db);
    assert_eq!(db.bounds(), Some((point(0.0, 0.0), point(1001.0, 0.0))));
}

#[test]
fn model_bounds_match_reference_for_translated_and_rotated_blocks() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Block 0: a unit line.
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(10)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(
        10,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
    ))
    .unwrap();
    // Block 1: one translated and one rotated INSERT of block 0.
    b.insert_block(BlockDefinition {
        id: BlockId(1),
        entities: vec![EntityId(11), EntityId(12)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(
        11,
        SpaceId::Block(BlockId(1)),
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(point(5.0, 0.0)),
        },
    ))
    .unwrap();
    b.insert_entity(raw_entity(
        12,
        SpaceId::Block(BlockId(1)),
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: rotate_degrees(90.0),
        },
    ))
    .unwrap();
    // Two model instances of block 1: the translation memo applies to one and
    // the rotated nested INSERT must still expand exactly.
    b.insert_entity(raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Insert {
            block: BlockId(1),
            transform: Transform3::translation(point(0.0, 0.0)),
        },
    ))
    .unwrap();
    b.insert_entity(raw_entity(
        2,
        SpaceId::Model,
        SemanticGeometry::Insert {
            block: BlockId(1),
            transform: Transform3::translation(point(100.0, 0.0)),
        },
    ))
    .unwrap();
    let db = b.finish().unwrap();
    assert_bounds_match_reference(&db);
    assert!(db.bounds().is_some());
}

#[test]
fn model_bounds_match_reference_for_deep_block_chains() {
    // Below the depth limit the translation memo is used throughout; at and
    // beyond it both paths must truncate identically.
    for length in [20usize, 40] {
        let db = block_chain_db(length);
        assert_bounds_match_reference(&db);
    }
    // The short chain reaches its terminal line...
    assert!(block_chain_db(20).bounds().is_some());
    // ...and the long one is truncated before reaching the line.
    assert!(block_chain_db(40).bounds().is_none());
}

#[test]
fn model_bounds_match_reference_for_cyclic_blocks() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    // Block 0 inserts itself.
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(raw_entity(
        2,
        SpaceId::Block(BlockId(0)),
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(point(1.0, 0.0)),
        },
    ))
    .unwrap();
    b.insert_entity(raw_entity(
        1,
        SpaceId::Model,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(point(0.0, 0.0)),
        },
    ))
    .unwrap();
    let db = b.finish().unwrap();
    assert_bounds_match_reference(&db);
    assert_eq!(db.bounds(), None);
}

#[test]
fn model_bounds_match_reference_for_every_visibility_state() {
    for active in [Some("A"), Some("B"), None] {
        let db = visibility_bounds_db(active);
        assert_bounds_match_reference(&db);
    }
}
