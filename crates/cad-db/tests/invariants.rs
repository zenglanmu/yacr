//! Contract tests for database invariants (audit B12, plus B28 overlap).
//!
//! Locks atomicity, revision monotonicity, ChangeSet ordering and the drawing
//! change masks.

use cad_db::*;
use cad_domain::*;

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn layer() -> Layer {
    Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    }
}

fn line_entity(id: u128, layer: LayerId) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbLine".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer,
        space: SpaceId::Model,
        geometry: SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        draw_order: 0,
    }
}

// ---------------------------------------------------------------------------
// Builder validation (B12 reference checks already present; lock them).
// ---------------------------------------------------------------------------

#[test]
fn builder_accepts_nested_blocks_and_rejects_dangling_block_entity() {
    let mut ok = DrawingDatabaseBuilder::new(DatabaseId(1));
    ok.insert_layer(layer()).unwrap();
    ok.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(1)],
        dynamic_visibility: None,
    })
    .unwrap();
    ok.insert_entity(line_entity(1, LayerId(0))).unwrap();
    assert!(ok.finish().is_ok());

    let mut bad = DrawingDatabaseBuilder::new(DatabaseId(1));
    bad.insert_layer(layer()).unwrap();
    bad.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(99)],
        dynamic_visibility: None,
    })
    .unwrap();
    assert!(matches!(bad.finish(), Err(CadError::Invariant(_))));
}

#[test]
fn layout_entities_require_an_existing_layout() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(layer()).unwrap();
    let mut e = line_entity(1, LayerId(0));
    e.space = SpaceId::Paper(LayoutId(7));
    b.insert_entity(e).unwrap();
    assert!(matches!(b.finish(), Err(CadError::Invariant(_))));
}

// ---------------------------------------------------------------------------
// Controlled drawing write path (docs/drawing-edit.md §1).
// ---------------------------------------------------------------------------

/// A drawing database with layer 0 and block 0 (no members).
fn writable_drawing() -> DrawingDatabase {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(layer()).unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: Vec::new(),
        dynamic_visibility: None,
    })
    .unwrap();
    b.finish().unwrap()
}

fn line_at(id: u128, end_x: f64) -> DbEntity {
    let mut e = line_entity(id, LayerId(0));
    e.geometry = SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(end_x, 0.0),
    };
    e
}

#[test]
fn drawing_transaction_is_atomic_on_invalid_input() {
    let mut db = writable_drawing();
    let revision = db.revision();
    let identity = db.scene_identity();

    let mut tx = db
        .begin_drawing_transaction("draw", TransactionId(1))
        .unwrap();
    tx.insert_entity(line_at(1, 1.0)).unwrap();
    // Entity 2 has a zero-length (degenerate) line.
    tx.insert_entity(line_at(2, 0.0)).unwrap();
    assert!(matches!(tx.commit(), Err(CadError::Invariant(_))));

    assert_eq!(db.entity_count(), 0);
    assert_eq!(db.revision(), revision);
    assert_eq!(db.scene_identity(), identity);
}

#[test]
fn drawing_empty_transaction_is_not_a_change() {
    let mut db = writable_drawing();
    let cs = db
        .begin_drawing_transaction("noop", TransactionId(1))
        .unwrap()
        .commit()
        .unwrap();
    assert!(cs.is_empty());
    assert_eq!(cs.before, cs.after);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn drawing_commit_is_ordered_and_bumps_revision_once() {
    let mut db = writable_drawing();
    let mut tx = db
        .begin_drawing_transaction("draw", TransactionId(9))
        .unwrap();
    tx.insert_entity(line_at(2, 2.0)).unwrap();
    tx.insert_entity(line_at(1, 1.0)).unwrap();
    let cs = tx.commit().unwrap();
    assert_eq!(cs.reason, "draw");
    assert_eq!(cs.transaction, TransactionId(9));
    assert_eq!(cs.before, Revision(0));
    assert_eq!(cs.after, Revision(1));
    assert!(cs.follows(DatabaseId(1), Revision(0)));
    // Staging order, not id order.
    assert_eq!(
        cs.changes,
        vec![
            ObjectChange::Insert(ObjectId(2)),
            ObjectChange::Insert(ObjectId(1)),
        ]
    );
}

#[test]
fn drawing_update_uses_precise_masks() {
    let mut db = writable_drawing();
    db.apply_drawing_changes(
        "seed",
        TransactionId(1),
        vec![(EntityId(1), Some(line_at(1, 1.0)))],
    )
    .unwrap();

    let mut moved_geometry = line_at(1, 1.0);
    moved_geometry.geometry = SemanticGeometry::Line {
        start: point(0.0, 0.0),
        end: point(4.0, 0.0),
    };
    let cs = db
        .apply_drawing_changes(
            "geometry",
            TransactionId(2),
            vec![(EntityId(1), Some(moved_geometry))],
        )
        .unwrap();
    assert_eq!(
        cs.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)]
    );

    let mut restyled = line_at(1, 4.0);
    restyled.draw_order = 77;
    let cs = db
        .apply_drawing_changes(
            "style",
            TransactionId(3),
            vec![(EntityId(1), Some(restyled))],
        )
        .unwrap();
    assert_eq!(
        cs.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::STYLE)]
    );
}

#[test]
fn drawing_transform_reports_transform_plus_geometry_and_bakes_it() {
    let mut db = writable_drawing();
    db.apply_drawing_changes(
        "seed",
        TransactionId(1),
        vec![(EntityId(1), Some(line_at(1, 1.0)))],
    )
    .unwrap();
    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::translation(point(5.0, 3.0)))
        .unwrap();
    let cs = tx.commit().unwrap();
    assert_eq!(
        cs.changes,
        vec![ObjectChange::Update(
            ObjectId(1),
            ChangeMask::TRANSFORM.union(ChangeMask::GEOMETRY)
        )]
    );
    assert_eq!(
        db.entity(EntityId(1)).unwrap().geometry,
        SemanticGeometry::Line {
            start: point(5.0, 3.0),
            end: point(6.0, 3.0),
        }
    );
}

#[test]
fn drawing_delete_removes_every_reference() {
    let mut db = writable_drawing();
    let mut member = line_at(10, 1.0);
    member.space = SpaceId::Block(BlockId(0));
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(10), Some(member))])
        .unwrap();
    assert!(db
        .block(BlockId(0))
        .unwrap()
        .entities
        .contains(&EntityId(10)));

    let mut tx = db
        .begin_drawing_transaction("delete", TransactionId(2))
        .unwrap();
    tx.delete_entity(EntityId(10)).unwrap();
    let cs = tx.commit().unwrap();
    assert_eq!(cs.changes, vec![ObjectChange::Delete(ObjectId(10))]);
    assert!(db.entity(EntityId(10)).is_none());
    assert!(!db
        .block(BlockId(0))
        .unwrap()
        .entities
        .contains(&EntityId(10)));
}

#[test]
fn drawing_id_allocation_never_reuses_deleted_ids() {
    let mut db = writable_drawing();
    db.apply_drawing_changes(
        "seed",
        TransactionId(1),
        vec![(EntityId(5), Some(line_at(5, 1.0)))],
    )
    .unwrap();
    assert_eq!(db.allocate_entity_id(), EntityId(6));
    assert_eq!(db.allocate_entity_id(), EntityId(7));
    db.apply_drawing_changes("del", TransactionId(2), vec![(EntityId(5), None)])
        .unwrap();
    // 5 is gone; the allocator must not reissue it.
    assert_eq!(db.allocate_entity_id(), EntityId(8));
}

#[test]
fn drawing_scene_identity_changes_on_writes_only() {
    let mut db = writable_drawing();
    let base = db.scene_identity();
    // Allocation alone is not a content write.
    let _ = db.allocate_entity_id();
    assert_eq!(db.scene_identity(), base);

    db.apply_drawing_changes(
        "insert",
        TransactionId(1),
        vec![(EntityId(1), Some(line_at(1, 1.0)))],
    )
    .unwrap();
    assert_ne!(db.scene_identity(), base);
}

#[test]
fn drawing_rejects_missing_layer_and_bad_object_identity() {
    let mut db = writable_drawing();
    let mut bad_layer = line_at(1, 1.0);
    bad_layer.layer = LayerId(9);
    assert!(db
        .apply_drawing_changes("x", TransactionId(1), vec![(EntityId(1), Some(bad_layer))])
        .is_err());

    let mut bad_object = line_at(2, 1.0);
    bad_object.object.id = ObjectId(200);
    assert!(db
        .apply_drawing_changes("x", TransactionId(2), vec![(EntityId(2), Some(bad_object))])
        .is_err());
    assert_eq!(db.entity_count(), 0);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn drawing_arc_and_ellipse_translate_exactly() {
    let mut db = writable_drawing();
    let arc = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbArc".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Arc {
            center: point(1.0, 1.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 2.0,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(arc))])
        .unwrap();

    let ellipse = DbEntity {
        object: DbObject {
            id: ObjectId(2),
            type_key: "AcDbEllipse".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(2),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            major_axis: point(3.0, 0.0),
            ratio: 0.5,
            start: 0.25,
            sweep: 1.5,
        },
        draw_order: 2,
    };
    db.apply_drawing_changes(
        "seed2",
        TransactionId(2),
        vec![(EntityId(2), Some(ellipse))],
    )
    .unwrap();

    let delta = point(10.0, -4.0);
    let mut tx = db
        .begin_drawing_transaction("move all", TransactionId(3))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::translation(delta))
        .unwrap();
    tx.transform_entity(EntityId(2), &Transform3::translation(delta))
        .unwrap();
    tx.commit().unwrap();

    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Arc {
            center,
            radius,
            start,
            sweep,
            ..
        } => {
            assert_eq!(*center, point(11.0, -3.0));
            assert!((radius - 2.0).abs() < 1e-12);
            // A pure translation must not change the start angle or sweep.
            assert!(start.abs() < 1e-9, "start drifted: {start}");
            assert!((sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        }
        other => panic!("unexpected {other:?}"),
    }
    match &db.entity(EntityId(2)).unwrap().geometry {
        SemanticGeometry::Ellipse {
            center,
            ratio,
            start,
            sweep,
            ..
        } => {
            assert_eq!(*center, point(10.0, -4.0));
            assert!((ratio - 0.5).abs() < 1e-12);
            assert!((start - 0.25).abs() < 1e-9, "start drifted: {start}");
            assert!((sweep - 1.5).abs() < 1e-9);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn drawing_arc_rotation_updates_the_start_parameter() {
    let mut db = writable_drawing();
    let arc = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbArc".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Arc {
            center: point(1.0, 1.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 2.0,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(arc))])
        .unwrap();

    // Rotate 90° CCW about Z.
    let mut rot = Transform3::identity();
    rot.matrix[0][0] = 0.0;
    rot.matrix[0][1] = -1.0;
    rot.matrix[1][0] = 1.0;
    rot.matrix[1][1] = 0.0;
    let mut tx = db
        .begin_drawing_transaction("rotate", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &rot).unwrap();
    tx.commit().unwrap();

    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Arc {
            center,
            start,
            sweep,
            ..
        } => {
            assert_eq!(*center, point(-1.0, 1.0));
            assert!(
                (start - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
                "expected start=pi/2, got {start}"
            );
            assert!((sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        }
        other => panic!("unexpected {other:?}"),
    }
}
