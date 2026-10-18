//! Drawing write transaction (docs/drawing-edit.md §1).

use super::*;

// ---------------------------------------------------------------------------
// Drawing write transaction (docs/drawing-edit.md §1)
// ---------------------------------------------------------------------------

#[test]
fn drawing_insert_commits_in_order_and_bumps_revision() {
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create line", TransactionId(7))
        .unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    tx.insert_entity(line_entity(2, point(0.0, 0.0), point(0.0, 1.0)))
        .unwrap();
    assert_eq!(tx.staged_len(), 2);
    let changes = tx.commit().unwrap();
    assert_eq!(changes.before, Revision(0));
    assert_eq!(changes.after, Revision(1));
    assert_eq!(db.revision(), Revision(1));
    // Caller staging order is preserved, even though ids are not sorted.
    assert_eq!(
        changes.changes,
        vec![
            ObjectChange::Insert(ObjectId(1)),
            ObjectChange::Insert(ObjectId(2)),
        ]
    );
    assert!(changes.follows(DatabaseId(1), Revision(0)));
}

#[test]
fn drawing_empty_transaction_is_a_noop() {
    let mut db = writable_db();
    let before = db.revision();
    let tx = db
        .begin_drawing_transaction("nothing", TransactionId(1))
        .unwrap();
    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(changes.before, changes.after);
    assert_eq!(changes.before, before);
    assert_eq!(db.revision(), before);
    assert_eq!(db.entity_count(), 0);
}

#[test]
fn drawing_rollback_leaves_no_state() {
    let mut db = writable_db();
    {
        let mut tx = db
            .begin_drawing_transaction("edit", TransactionId(1))
            .unwrap();
        tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
            .unwrap();
        assert_eq!(tx.staged_len(), 1);
        tx.rollback();
    }
    assert_eq!(db.entity_count(), 0);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn drawing_invalid_change_rejects_the_whole_transaction() {
    let mut db = writable_db();
    let before_identity = db.scene_identity();
    // Entity 99 has a zero-length line, which validation rejects.
    let mut tx = db
        .begin_drawing_transaction("mixed", TransactionId(1))
        .unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    tx.insert_entity(line_entity(99, point(2.0, 2.0), point(2.0, 2.0)))
        .unwrap();
    let err = tx.commit().unwrap_err();
    assert!(matches!(err, CadError::Invariant(_)), "got {err:?}");
    // Nothing was written: no entity 1, no revision bump, same scene identity.
    assert_eq!(db.entity_count(), 0);
    assert_eq!(db.revision(), Revision(0));
    assert_eq!(db.scene_identity(), before_identity);
}

#[test]
fn drawing_key_mismatch_and_object_mismatch_are_rejected() {
    let mut db = writable_db();
    // Change key 5 but the entity carries id 6.
    let mut wrong = line_entity(6, point(0.0, 0.0), point(1.0, 0.0));
    wrong.object.id = ObjectId(6);
    let err = db
        .apply_drawing_changes("bad", TransactionId(1), vec![(EntityId(5), Some(wrong))])
        .unwrap_err();
    assert!(matches!(err, CadError::Invariant(_)));

    // Key/id agree but the object id does not share the value.
    let mut wrong = line_entity(5, point(0.0, 0.0), point(1.0, 0.0));
    wrong.object.id = ObjectId(1234);
    let err = db
        .apply_drawing_changes("bad", TransactionId(1), vec![(EntityId(5), Some(wrong))])
        .unwrap_err();
    assert!(matches!(err, CadError::Invariant(_)));
    assert_eq!(db.entity_count(), 0);
}

#[test]
fn drawing_missing_layer_and_missing_space_are_rejected() {
    let mut db = writable_db();
    let mut bad_layer = line_entity(1, point(0.0, 0.0), point(1.0, 0.0));
    bad_layer.layer = LayerId(42);
    assert!(db
        .apply_drawing_changes("x", TransactionId(1), vec![(EntityId(1), Some(bad_layer))])
        .is_err());

    let mut bad_layout = line_entity(2, point(0.0, 0.0), point(1.0, 0.0));
    bad_layout.space = SpaceId::Paper(LayoutId(9));
    assert!(db
        .apply_drawing_changes("x", TransactionId(1), vec![(EntityId(2), Some(bad_layout))])
        .is_err());

    let mut bad_block = line_entity(3, point(0.0, 0.0), point(1.0, 0.0));
    bad_block.space = SpaceId::Block(BlockId(9));
    assert!(db
        .apply_drawing_changes("x", TransactionId(1), vec![(EntityId(3), Some(bad_block))])
        .is_err());
    assert_eq!(db.entity_count(), 0);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn drawing_double_modification_is_rejected() {
    let mut db = writable_db();
    let mut tx = db.begin_drawing_transaction("x", TransactionId(1)).unwrap();
    tx.insert_entity(line_entity(1, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    // Staging the same id again is rejected immediately, before commit.
    assert!(tx
        .insert_entity(line_entity(1, point(0.0, 0.0), point(2.0, 0.0)))
        .is_err());

    // The shared apply path also rejects a duplicated id in one batch.
    let err = db
        .apply_drawing_changes(
            "dup",
            TransactionId(1),
            vec![
                (
                    EntityId(1),
                    Some(line_entity(1, point(0.0, 0.0), point(1.0, 0.0))),
                ),
                (EntityId(1), None),
            ],
        )
        .unwrap_err();
    assert!(matches!(err, CadError::Invariant(_)));
    assert_eq!(db.entity_count(), 0);
}

#[test]
fn drawing_delete_missing_entity_is_rejected() {
    let mut db = writable_db();
    let mut tx = db.begin_drawing_transaction("x", TransactionId(1)).unwrap();
    assert!(tx.delete_entity(EntityId(99)).is_err());
    // Nothing valid was staged; commit is a no-op rather than a bump.
    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn drawing_update_masks_distinguish_geometry_and_style() {
    let mut db = writable_db();
    db.apply_drawing_changes(
        "seed",
        TransactionId(1),
        vec![(
            EntityId(1),
            Some(line_entity(1, point(0.0, 0.0), point(1.0, 0.0))),
        )],
    )
    .unwrap();

    // Geometry-only update.
    let changeset = db
        .apply_drawing_changes(
            "geometry",
            TransactionId(2),
            vec![(
                EntityId(1),
                Some(line_entity(1, point(0.0, 0.0), point(5.0, 0.0))),
            )],
        )
        .unwrap();
    assert_eq!(
        changeset.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::GEOMETRY)]
    );

    // Style-only update (draw order changes, geometry identical).
    let mut styled = line_entity(1, point(0.0, 0.0), point(5.0, 0.0));
    styled.draw_order = 99;
    let changeset = db
        .apply_drawing_changes("style", TransactionId(3), vec![(EntityId(1), Some(styled))])
        .unwrap();
    assert_eq!(
        changeset.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::STYLE)]
    );

    // Geometry and style together report both bits.
    let mut both = line_entity(1, point(0.0, 0.0), point(8.0, 0.0));
    both.draw_order = 100;
    let changeset = db
        .apply_drawing_changes("both", TransactionId(4), vec![(EntityId(1), Some(both))])
        .unwrap();
    assert_eq!(
        changeset.changes,
        vec![ObjectChange::Update(
            ObjectId(1),
            ChangeMask::GEOMETRY.union(ChangeMask::STYLE)
        )]
    );

    // An identical replacement is still an Update, never an empty mask.
    let same = line_entity(1, point(0.0, 0.0), point(8.0, 0.0));
    let mut same = same;
    same.draw_order = 100;
    let changeset = db
        .apply_drawing_changes("same", TransactionId(5), vec![(EntityId(1), Some(same))])
        .unwrap();
    match &changeset.changes[0] {
        ObjectChange::Update(_, mask) => assert_ne!(mask.0, 0),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn drawing_transform_emits_transform_and_geometry_and_bakes_the_move() {
    let mut db = writable_db();
    db.apply_drawing_changes(
        "seed",
        TransactionId(1),
        vec![(
            EntityId(1),
            Some(line_entity(1, point(0.0, 0.0), point(1.0, 0.0))),
        )],
    )
    .unwrap();

    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::translation(point(10.0, 0.0)))
        .unwrap();
    let changeset = tx.commit().unwrap();
    assert_eq!(
        changeset.changes,
        vec![ObjectChange::Update(
            ObjectId(1),
            ChangeMask::TRANSFORM.union(ChangeMask::GEOMETRY)
        )]
    );
    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Line { start, end } => {
            assert_eq!(*start, point(10.0, 0.0));
            assert_eq!(*end, point(11.0, 0.0));
        }
        other => panic!("unexpected geometry {other:?}"),
    }
}

#[test]
fn drawing_transform_unsupported_kind_is_rejected_and_stops_nothing() {
    let mut db = writable_db();
    // Text is not baked by this build.
    let text = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbText".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Text {
            text: "hi".into(),
            position: point(0.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(text))])
        .unwrap();
    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(2))
        .unwrap();
    let err = tx
        .transform_entity(EntityId(1), &Transform3::translation(point(1.0, 0.0)))
        .unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "got {err:?}");
    // The failed stage left nothing; commit is a no-op.
    assert_eq!(tx.staged_len(), 0);
    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(db.revision(), Revision(1), "seed revision unchanged");
}

#[test]
fn drawing_transform_circle_requires_similarity() {
    let mut db = writable_db();
    let circle = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbCircle".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 2.0,
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(circle))])
        .unwrap();

    // A rotation + uniform scale keeps it a circle.
    let mut tx = db
        .begin_drawing_transaction("scale", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::scale(3.0))
        .unwrap();
    tx.commit().unwrap();
    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Circle { radius, center, .. } => {
            assert_eq!(*center, point(0.0, 0.0));
            assert!((radius - 6.0).abs() < 1e-9);
        }
        other => panic!("unexpected {other:?}"),
    }

    // A non-uniform scale would make it an ellipse: refused, not mis-stored.
    let mut shear = Transform3::identity();
    shear.matrix[0][0] = 2.0;
    shear.matrix[1][1] = 1.0;
    let mut tx = db
        .begin_drawing_transaction("squash", TransactionId(3))
        .unwrap();
    let err = tx.transform_entity(EntityId(1), &shear).unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "got {err:?}");
    assert_eq!(tx.staged_len(), 0);
    let _ = tx.commit().unwrap();
    // Circle radius unchanged by the rejected transform.
    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Circle { radius, .. } => assert!((radius - 6.0).abs() < 1e-9),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn drawing_transform_mirror_is_rejected_for_arcs_not_dropped() {
    let mut db = writable_db();
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
            center: point(0.0, 0.0),
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
    // Mirror about the Y axis (negative determinant).
    let mut mirror = Transform3::identity();
    mirror.matrix[0][0] = -1.0;
    let mut tx = db
        .begin_drawing_transaction("mirror", TransactionId(2))
        .unwrap();
    let err = tx.transform_entity(EntityId(1), &mirror).unwrap_err();
    assert!(matches!(err, CadError::Unsupported(_)), "got {err:?}");
    assert_eq!(tx.staged_len(), 0);
}

#[test]
fn drawing_transform_insert_composes_onto_the_instance() {
    let mut db = writable_db();
    let insert = DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbBlockReference".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(1),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry: SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(point(1.0, 0.0)),
        },
        draw_order: 1,
    };
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(1), Some(insert))])
        .unwrap();
    let mut tx = db
        .begin_drawing_transaction("move", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::translation(point(5.0, 0.0)))
        .unwrap();
    tx.commit().unwrap();
    match &db.entity(EntityId(1)).unwrap().geometry {
        SemanticGeometry::Insert { transform, .. } => {
            let p = transform.apply_point(point(0.0, 0.0));
            assert_eq!(p, point(6.0, 0.0));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn drawing_delete_cleans_block_membership_and_render_attributes() {
    let mut db = writable_db();
    // A block-member entity plus a model insert referencing the block.
    let mut member = line_entity(10, point(0.0, 0.0), point(1.0, 0.0));
    member.space = SpaceId::Block(BlockId(0));
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(10), Some(member))])
        .unwrap();
    // Insert path auto-registered membership.
    assert!(db
        .block(BlockId(0))
        .unwrap()
        .entities
        .contains(&EntityId(10)));

    db.apply_drawing_changes(
        "attrs",
        TransactionId(2),
        vec![(
            EntityId(10),
            Some({
                let mut m = db.entity(EntityId(10)).unwrap().clone();
                m.draw_order = 5;
                m
            }),
        )],
    )
    .unwrap();
    assert!(db.entity_render_attributes(EntityId(10)).color == EntityColor::ByLayer);

    let mut tx = db
        .begin_drawing_transaction("delete", TransactionId(3))
        .unwrap();
    tx.delete_entity(EntityId(10)).unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(changes.changes, vec![ObjectChange::Delete(ObjectId(10))]);
    assert!(db.entity(EntityId(10)).is_none());
    assert!(!db
        .block(BlockId(0))
        .unwrap()
        .entities
        .contains(&EntityId(10)));
}

#[test]
fn drawing_delete_prunes_dynamic_visibility_lists() {
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
        let mut e = raw_entity(
            id,
            SpaceId::Block(BlockId(0)),
            SemanticGeometry::Point(point(id as f64, 0.0)),
        );
        e.layer = LayerId(0);
        b.insert_entity(e).unwrap();
    }
    b.set_block_dynamic_visibility(
        BlockId(0),
        DynamicBlockVisibility {
            member_entities: vec![EntityId(10), EntityId(11)],
            states: vec![DynamicBlockState {
                name: "A".into(),
                entities: vec![EntityId(10)],
            }],
            active_state: None,
        },
    )
    .unwrap();
    let mut db = b.finish().unwrap();

    let mut tx = db
        .begin_drawing_transaction("delete", TransactionId(1))
        .unwrap();
    tx.delete_entity(EntityId(10)).unwrap();
    tx.commit().unwrap();
    let vis = db.block_dynamic_visibility(BlockId(0)).unwrap();
    assert!(!vis.member_entities.contains(&EntityId(10)));
    assert!(!vis.states[0].entities.contains(&EntityId(10)));
    assert!(vis.member_entities.contains(&EntityId(11)));
}

#[test]
fn drawing_id_allocation_is_monotonic_and_never_reuses_a_deleted_id() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(line_entity(7, point(0.0, 0.0), point(1.0, 0.0)))
        .unwrap();
    let mut db = b.finish().unwrap();
    // Initialised from the largest existing id.
    assert_eq!(db.next_entity_id(), EntityId(8));

    let first = db.allocate_entity_id();
    let second = db.allocate_entity_id();
    assert_eq!(first, EntityId(8));
    assert_eq!(second, EntityId(9));
    assert_eq!(db.next_entity_id(), EntityId(10));

    // Insert id 8 then delete it; the allocator must not hand out 8 again.
    let mut e = line_entity(8, point(0.0, 0.0), point(1.0, 0.0));
    e.layer = LayerId(0);
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(8), Some(e))])
        .unwrap();
    db.apply_drawing_changes("del", TransactionId(2), vec![(EntityId(8), None)])
        .unwrap();
    assert!(db.entity(EntityId(8)).is_none());
    assert_eq!(db.allocate_entity_id(), EntityId(10));
}

#[test]
fn drawing_builder_initialises_allocator_from_block_members_too() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    let mut member = line_entity(50, point(0.0, 0.0), point(1.0, 0.0));
    member.space = SpaceId::Block(BlockId(0));
    member.layer = LayerId(0);
    b.insert_entity(member).unwrap();
    b.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(50)],
        dynamic_visibility: None,
    })
    .unwrap();
    let db = b.finish().unwrap();
    assert_eq!(db.next_entity_id(), EntityId(51));
}

#[test]
fn drawing_scene_identity_changes_on_every_write() {
    let mut db = writable_db();
    let identity0 = db.scene_identity();
    db.apply_drawing_changes(
        "insert",
        TransactionId(1),
        vec![(
            EntityId(1),
            Some(line_entity(1, point(0.0, 0.0), point(1.0, 0.0))),
        )],
    )
    .unwrap();
    let identity1 = db.scene_identity();
    assert_ne!(identity0, identity1);

    let changeset = db
        .apply_drawing_changes(
            "update",
            TransactionId(2),
            vec![(
                EntityId(1),
                Some(line_entity(1, point(0.0, 0.0), point(9.0, 0.0))),
            )],
        )
        .unwrap();
    assert_eq!(changeset.after, Revision(2));
    let identity2 = db.scene_identity();
    assert_ne!(identity1, identity2);

    db.apply_drawing_changes("delete", TransactionId(3), vec![(EntityId(1), None)])
        .unwrap();
    assert_ne!(db.scene_identity(), identity2);
}

#[test]
fn drawing_insert_into_block_auto_registers_membership() {
    let mut db = writable_db();
    let mut member = line_entity(20, point(0.0, 0.0), point(1.0, 0.0));
    member.space = SpaceId::Block(BlockId(0));
    db.apply_drawing_changes("seed", TransactionId(1), vec![(EntityId(20), Some(member))])
        .unwrap();
    assert!(db
        .block(BlockId(0))
        .unwrap()
        .entities
        .contains(&EntityId(20)));
}

#[test]
fn drawing_reason_is_required() {
    let mut db = writable_db();
    match db.begin_drawing_transaction("   ", TransactionId(1)) {
        Err(CadError::InvalidInput(_)) => {}
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(_) => panic!("an empty reason must be rejected"),
    }
}

#[test]
fn drawing_validate_geometry_covers_supported_kinds() {
    // A representative valid geometry of each supported kind passes.
    let valid = vec![
        SemanticGeometry::Line {
            start: point(0.0, 0.0),
            end: point(1.0, 0.0),
        },
        SemanticGeometry::Polyline {
            points: vec![point(0.0, 0.0), point(1.0, 0.0)],
            bulges: vec![0.0, 0.0],
            closed: false,
        },
        SemanticGeometry::Circle {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 1.0,
        },
        SemanticGeometry::Arc {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 1.0,
            start: 0.0,
            sweep: 1.0,
        },
        SemanticGeometry::Ellipse {
            center: point(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            major_axis: point(2.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
        SemanticGeometry::Spline {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![point(0.0, 0.0), point(1.0, 1.0), point(2.0, 0.0)],
            weights: vec![1.0, 1.0, 1.0],
        },
        SemanticGeometry::Point(point(3.0, 4.0)),
        SemanticGeometry::Text {
            text: "x".into(),
            position: point(0.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
        },
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::identity(),
        },
        SemanticGeometry::Mesh(Mesh {
            vertices: vec![point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
            triangles: vec![[0, 1, 2]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        }),
        SemanticGeometry::Image {
            origin: point(0.0, 0.0),
            u: point(0.5, 0.0),
            v: point(0.0, 0.5),
            pixels: [640.0, 480.0],
            file: Some("logo.png".into()),
            clip: None,
            visible: true,
        },
        SemanticGeometry::Mask {
            boundary: vec![point(0.0, 0.0), point(2.0, 0.0), point(2.0, 2.0)],
            inverted: false,
        },
    ];
    for geometry in valid {
        crate::validate_geometry(&geometry)
            .unwrap_or_else(|e| panic!("expected valid, got {e:?} for {geometry:?}"));
    }

    // The same kinds with degenerate values are rejected.
    assert!(crate::validate_geometry(&SemanticGeometry::Mesh(Mesh {
        vertices: vec![point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
        triangles: vec![[0, 1, 99]],
        normals: Vec::new(),
        face_sources: Vec::new(),
        colors: Vec::new(),
    }))
    .is_err());
    assert!(crate::validate_geometry(&SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0],
        control_points: vec![point(0.0, 0.0)],
        weights: Vec::new(),
    })
    .is_err());
    assert!(crate::validate_geometry(&SemanticGeometry::Ellipse {
        center: point(0.0, 0.0),
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0
        },
        major_axis: point(0.0, 0.0),
        ratio: 0.5,
        start: 0.0,
        sweep: 1.0,
    })
    .is_err());
}

#[test]
fn drawing_transform_bakes_image_placement_and_mask_boundary() {
    let image = SemanticGeometry::Image {
        origin: point(1.0, 2.0),
        u: point(0.5, 0.0),
        v: point(0.0, 0.25),
        pixels: [640.0, 480.0],
        file: Some("logo.png".into()),
        clip: Some(ImageClip {
            vertices: vec![[0.0, 0.0], [1.0, 1.0]],
            inside: true,
        }),
        visible: true,
    };
    // Translate + uniform scale: `origin` moves as a point while `u`/`v` scale
    // as direction vectors (no translation applied to them).
    let mut t = Transform3::scale(2.0);
    t.matrix[0][3] = 10.0;
    t.matrix[1][3] = 20.0;
    match crate::transform_geometry(&image, &t).unwrap() {
        SemanticGeometry::Image {
            origin,
            u,
            v,
            pixels,
            file,
            clip,
            visible,
        } => {
            assert_eq!(origin, point(12.0, 24.0));
            assert_eq!(u, point(1.0, 0.0));
            assert_eq!(v, point(0.0, 0.5));
            assert_eq!(pixels, [640.0, 480.0]);
            assert_eq!(file.as_deref(), Some("logo.png"));
            assert!(clip.expect("clip").inside);
            assert!(visible);
        }
        other => panic!("expected Image, got {other:?}"),
    }

    let mask = SemanticGeometry::Mask {
        boundary: vec![point(0.0, 0.0), point(1.0, 0.0), point(1.0, 1.0)],
        inverted: true,
    };
    match crate::transform_geometry(&mask, &t).unwrap() {
        SemanticGeometry::Mask { boundary, inverted } => {
            assert_eq!(
                boundary,
                vec![point(10.0, 20.0), point(12.0, 20.0), point(12.0, 22.0)]
            );
            assert!(inverted);
        }
        other => panic!("expected Mask, got {other:?}"),
    }
}
