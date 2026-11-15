//! History replay restores stored rows; new writes stay geometry-validated.
//!
//! The importer is deliberately permissive (it records whatever the file holds)
//! and the write path is not, so a real database can hold a row the write
//! validator refuses. Undo/redo reinstalls rows the database itself held, which is
//! why replay has its own entry point: geometry is restored byte-for-byte, while
//! every reference rule still runs. These tests pin both halves — otherwise a
//! future change could either re-validate replays (breaking one undo step for any
//! drawing with an imported zero-length line) or relax new writes (letting a
//! command create one).

use super::*;

fn zero_length_line(id: u128) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: Some(format!("{id:X}")),
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: point(3.0, 7.0),
            end: point(3.0, 7.0),
        },
        draw_order: id as i64,
    }
}

/// Layer 0, layout 1, block 0 with one zero-length member (entity 2) and a
/// normal model line (entity 1). Built through the **import** builder, i.e. the
/// path that stores what the file holds without geometry validation.
fn imported_db() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "L1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(2)],
        dynamic_visibility: None,
    })
    .unwrap();
    b.insert_entity(line_entity(1, point(0.0, 0.0), point(4.0, 0.0)))
        .unwrap();
    b.insert_entity(DbEntity {
        space: SpaceId::Block(BlockId(0)),
        ..zero_length_line(2)
    })
    .unwrap();
    b.finish().unwrap()
}

/// Delete through the combined validated path, restore through the replay entry
/// point: the row comes back unchanged in one revision.
#[test]
fn replay_restores_an_imported_degenerate_row_verbatim() {
    let mut db = imported_db();
    let stored = db
        .entity(EntityId(2))
        .expect("the imported member exists")
        .clone();
    assert!(
        crate::validate_geometry(&stored.geometry).is_err(),
        "the fixture must hold geometry the write validator rejects"
    );

    // 1. The delete is an ordinary validated write: a delete has no geometry.
    let deleted_at = db.revision();
    db.apply_drawing_changes_with_blocks("erase", TransactionId(1), &[], vec![(EntityId(2), None)])
        .expect("deleting the stored row is a normal write");
    assert!(db.entity(EntityId(2)).is_none());
    assert_eq!(
        db.block(BlockId(0)).map(|block| block.entities.clone()),
        Some(Vec::new()),
        "the delete prunes the membership"
    );
    assert_eq!(db.revision(), Revision(deleted_at.0 + 1));

    // 2. The restore is a replay: the same row the database held goes back.
    let changes = db
        .replay_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes(
            "undo",
            TransactionId(2),
            &[],
            vec![(EntityId(2), Some(stored.clone()))],
            &[],
            &[],
            &[],
            crate::StyleTableChanges::default(),
            &[],
        )
        .expect("replaying a row the database already held must not be refused");
    assert_eq!(
        db.entity(EntityId(2)),
        Some(&stored),
        "the row is restored byte-for-byte (geometry, layer, space, ordering)"
    );
    assert_eq!(
        db.block(BlockId(0)).map(|block| block.entities.clone()),
        Some(vec![EntityId(2)]),
        "the membership comes back with the row"
    );
    assert_eq!(
        changes.before,
        Revision(deleted_at.0 + 1),
        "the replay is one revision on top of the delete"
    );
    assert_eq!(changes.after, Revision(deleted_at.0 + 2));
}

/// The same row through the **normal** write path is still refused with the
/// stable reason: the replay mode did not weaken new writes.
#[test]
fn a_new_write_of_the_same_degenerate_row_is_still_refused() {
    let mut db = imported_db();
    let stored = db.entity(EntityId(2)).expect("member").clone();
    let degenerate = CadError::Invariant("invalid entity geometry: line is zero length".into());

    // Re-writing the stored row (an identical update) is a new write.
    assert_eq!(
        db.apply_drawing_changes(
            "update",
            TransactionId(1),
            vec![(EntityId(2), Some(stored.clone()))],
        )
        .unwrap_err(),
        degenerate
    );

    // And a command cannot re-create the row it just deleted.
    db.apply_drawing_changes("erase", TransactionId(2), vec![(EntityId(2), None)])
        .expect("delete");
    assert_eq!(
        db.apply_drawing_changes(
            "retype",
            TransactionId(3),
            vec![(EntityId(2), Some(stored.clone()))],
        )
        .unwrap_err(),
        degenerate
    );
    assert!(
        db.entity(EntityId(2)).is_none(),
        "the refused write mutated nothing"
    );

    // The transaction path (what every command uses) is the same rule, not a
    // second implementation.
    let mut transaction = db
        .begin_drawing_transaction("insert", TransactionId(4))
        .expect("transaction");
    transaction
        .insert_entity(zero_length_line(9))
        .expect("staging is not a commit");
    assert_eq!(transaction.commit().unwrap_err(), degenerate);
    assert!(db.entity(EntityId(9)).is_none());
}

/// Replay skips geometry validation **only**. A dangling reference is still
/// refused, before any mutation, so the relaxed mode cannot store a broken row.
#[test]
fn replay_still_enforces_references_and_identity() {
    let mut db = imported_db();
    let stored = db.entity(EntityId(2)).expect("member").clone();
    let before = db.revision();

    let replay = |db: &mut DrawingDatabase, id: u128, row: DbEntity, transaction: u128| {
        db.replay_drawing_changes_with_blocks_display_layers_layouts_styles_and_linetypes(
            "undo",
            TransactionId(transaction),
            &[],
            vec![(EntityId(id), Some(row))],
            &[],
            &[],
            &[],
            crate::StyleTableChanges::default(),
            &[],
        )
    };

    // A layer the database does not have (and the batch does not stage).
    let error = replay(
        &mut db,
        2,
        DbEntity {
            layer: LayerId(77),
            ..stored.clone()
        },
        1,
    )
    .unwrap_err();
    assert_eq!(
        error,
        CadError::Invariant(format!(
            "entity {:?} references missing layer {:?}",
            EntityId(2),
            LayerId(77)
        ))
    );

    // A space that does not exist.
    let error = replay(
        &mut db,
        2,
        DbEntity {
            space: SpaceId::Block(BlockId(42)),
            ..stored.clone()
        },
        2,
    )
    .unwrap_err();
    assert_eq!(
        error,
        CadError::Invariant(format!(
            "entity {:?} references missing block {:?}",
            EntityId(2),
            BlockId(42)
        ))
    );

    // A paper layout that does not exist.
    let error = replay(
        &mut db,
        2,
        DbEntity {
            space: SpaceId::Paper(LayoutId(9)),
            ..stored.clone()
        },
        3,
    )
    .unwrap_err();
    assert_eq!(
        error,
        CadError::Invariant(format!(
            "entity {:?} references missing layout {:?}",
            EntityId(2),
            LayoutId(9)
        ))
    );

    // The identity relationship is not waived either.
    let wrong_object_id = DbEntity {
        object: DbObject {
            id: ObjectId(99),
            ..stored.object.clone()
        },
        ..stored.clone()
    };
    assert!(replay(&mut db, 2, wrong_object_id, 4).is_err());

    assert_eq!(db.revision(), before, "every refusal left the db untouched");
    assert_eq!(db.entity(EntityId(2)), Some(&stored));
}
