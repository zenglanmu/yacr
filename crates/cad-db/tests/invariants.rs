//! Contract tests for database invariants (audit B12, plus B28 overlap).
//!
//! Locks atomicity, revision monotonicity, ChangeSet ordering, precise change
//! masks and the annotation validation boundary.

use cad_db::*;
use cad_domain::*;

fn point(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

fn text_annotation(id: u128) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        space: SpaceId::Model,
        geometry: AnnotationGeometry::Text(point(0.0, 0.0)),
        text: "note".to_string(),
        style: AnnotationStyle::default(),
        created_unix_ms: 0,
        modified_unix_ms: 0,
        anchor: None,
        precision: Precision::Analytic,
    }
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
// Revision monotonicity and ChangeSet ordering.
// ---------------------------------------------------------------------------

#[test]
fn commit_is_ordered_and_revision_is_monotonic() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut last = db.revision();
    for i in 1..=5u128 {
        let cs = db
            .apply_annotation_changes(
                "create",
                TransactionId(i),
                vec![(AnnotationId(i), Some(text_annotation(i)))],
            )
            .unwrap();
        assert_eq!(cs.before, last);
        assert_eq!(cs.after, Revision(last.0 + 1));
        assert!(cs.follows(DatabaseId(1), last));
        assert_eq!(cs.changes, vec![ObjectChange::Insert(ObjectId(i))]);
        last = cs.after;
        assert_eq!(db.revision(), last);
    }
}

#[test]
fn empty_transaction_does_not_advance_revision_or_claim_changes() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let cs = db
        .apply_annotation_changes("noop", TransactionId(1), Vec::new())
        .unwrap();
    assert!(cs.is_empty());
    assert_eq!(cs.before, cs.after);
    assert_eq!(db.revision(), Revision(0));
    assert!(!db.is_dirty());
}

// ---------------------------------------------------------------------------
// B12: atomicity -- a single invalid change leaves nothing behind.
// ---------------------------------------------------------------------------

#[test]
fn invalid_annotation_rejects_whole_transaction_atomically() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    // Seed one valid annotation.
    db.apply_annotation_changes(
        "seed",
        TransactionId(1),
        vec![(AnnotationId(1), Some(text_annotation(1)))],
    )
    .unwrap();
    let revision_before = db.revision();
    let len_before = db.len();

    // One valid insert plus one NaN-geometry insert.
    let mut bad = text_annotation(2);
    bad.geometry = AnnotationGeometry::Text(Point3 {
        x: f64::NAN,
        y: 0.0,
        z: 0.0,
    });
    let result = db.apply_annotation_changes(
        "mixed",
        TransactionId(2),
        vec![
            (AnnotationId(3), Some(text_annotation(3))),
            (AnnotationId(2), Some(bad)),
        ],
    );
    assert!(matches!(result, Err(CadError::Invariant(_))), "{result:?}");
    // Nothing changed: no partial insert, no revision bump.
    assert_eq!(db.len(), len_before);
    assert!(db.get(AnnotationId(3)).is_none());
    assert_eq!(db.revision(), revision_before);
}

#[test]
fn change_key_must_match_annotation_id() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    // The map key (7) disagrees with annotation.id (8).
    let result = db.apply_annotation_changes(
        "mismatch",
        TransactionId(1),
        vec![(AnnotationId(7), Some(text_annotation(8)))],
    );
    assert!(matches!(result, Err(CadError::Invariant(_))), "{result:?}");
    assert_eq!(db.len(), 0);
    assert_eq!(db.revision(), Revision(0));
}

#[test]
fn zero_axis_ellipse_annotation_is_rejected() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut a = text_annotation(1);
    a.geometry = AnnotationGeometry::Ellipse {
        center: point(0.0, 0.0),
        axis_u: point(0.0, 0.0), // zero-length axis
        axis_v: point(0.0, 1.0),
    };
    assert!(db
        .apply_annotation_changes("bad", TransactionId(1), vec![(AnnotationId(1), Some(a))])
        .is_err());
    assert_eq!(db.len(), 0);
}

#[test]
fn negative_or_non_finite_style_is_rejected() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut a = text_annotation(1);
    a.style.logical_width = -1.0;
    assert!(db
        .apply_annotation_changes("bad", TransactionId(1), vec![(AnnotationId(1), Some(a))])
        .is_err());
    assert_eq!(db.len(), 0);
}

#[test]
fn modified_before_created_is_rejected() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut a = text_annotation(1);
    a.created_unix_ms = 100;
    a.modified_unix_ms = 50;
    assert!(db
        .apply_annotation_changes("bad", TransactionId(1), vec![(AnnotationId(1), Some(a))])
        .is_err());
    assert_eq!(db.len(), 0);
}

#[test]
fn empty_leader_geometry_is_rejected() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut a = text_annotation(1);
    a.geometry = AnnotationGeometry::Leader(Vec::new());
    assert!(db
        .apply_annotation_changes("bad", TransactionId(1), vec![(AnnotationId(1), Some(a))])
        .is_err());
    assert_eq!(db.len(), 0);
}

#[test]
fn measurement_with_skewed_plane_is_rejected() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut a = text_annotation(1);
    a.geometry = AnnotationGeometry::Measurement(MeasurementRecord {
        algorithm: MeasurementAlgorithm::PlanarPolygonArea,
        inputs: vec![point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
        plane: Some(WorkPlane {
            origin: point(0.0, 0.0),
            u: point(1.0, 0.0),
            v: point(1.0, 1.0), // 45°, not orthogonal
        }),
        value: 0.5,
        units: UnitContext::drawing_units(),
        source: GeometrySource::UserPoints,
        precision: Precision::Analytic,
    });
    assert!(db
        .apply_annotation_changes("bad", TransactionId(1), vec![(AnnotationId(1), Some(a))])
        .is_err());
    assert_eq!(db.len(), 0);
}

#[test]
fn non_finite_anchor_fallback_is_rejected() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut a = text_annotation(1);
    a.anchor = Some(EntityAnchor {
        source_handle: "1A".into(),
        instance: InstancePath::default(),
        sub_element: None,
        fallback: Point3 {
            x: f64::INFINITY,
            y: 0.0,
            z: 0.0,
        },
        status: AnchorStatus::Valid,
    });
    assert!(db
        .apply_annotation_changes("bad", TransactionId(1), vec![(AnnotationId(1), Some(a))])
        .is_err());
    assert_eq!(db.len(), 0);
}

#[test]
fn empty_anchor_handle_is_rejected() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    let mut a = text_annotation(1);
    a.anchor = Some(EntityAnchor {
        source_handle: "   ".into(),
        instance: InstancePath::default(),
        sub_element: None,
        fallback: point(0.0, 0.0),
        status: AnchorStatus::Valid,
    });
    assert!(db
        .apply_annotation_changes("bad", TransactionId(1), vec![(AnnotationId(1), Some(a))])
        .is_err());
    assert_eq!(db.len(), 0);
}

// ---------------------------------------------------------------------------
// B12: precise masks -- metadata edits must not force a geometry rebuild.
// ---------------------------------------------------------------------------

#[test]
fn metadata_only_update_reports_metadata_mask() {
    let before = text_annotation(1);
    let mut after = before.clone();
    after.modified_unix_ms = 5;
    let mask = ChangeMask::for_annotation_update(&before, &after);
    assert_eq!(mask, ChangeMask::METADATA);
    assert!(!mask.invalidates_representation());
}

#[test]
fn text_payload_change_reports_style_mask_not_geometry() {
    let before = text_annotation(1);
    let mut after = before.clone();
    after.text = "changed".into();
    let mask = ChangeMask::for_annotation_update(&before, &after);
    assert!(mask.contains(ChangeMask::STYLE));
    assert!(!mask.contains(ChangeMask::GEOMETRY));
}

#[test]
fn geometry_change_reports_geometry_mask() {
    let before = text_annotation(1);
    let mut after = before.clone();
    after.geometry = AnnotationGeometry::Text(point(9.0, 9.0));
    let mask = ChangeMask::for_annotation_update(&before, &after);
    assert!(mask.contains(ChangeMask::GEOMETRY));
    assert!(mask.invalidates_representation());
}

#[test]
fn anchor_move_reports_transform_mask() {
    let mut before = text_annotation(1);
    before.anchor = Some(EntityAnchor {
        source_handle: "1A".into(),
        instance: InstancePath::default(),
        sub_element: None,
        fallback: point(0.0, 0.0),
        status: AnchorStatus::Valid,
    });
    let mut after = before.clone();
    after.anchor.as_mut().unwrap().fallback = point(5.0, 0.0);
    let mask = ChangeMask::for_annotation_update(&before, &after);
    assert!(mask.contains(ChangeMask::TRANSFORM));
    assert!(!mask.contains(ChangeMask::REFERENCES));
}

#[test]
fn committed_metadata_only_update_is_reported_as_metadata() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    db.apply_annotation_changes(
        "seed",
        TransactionId(1),
        vec![(AnnotationId(1), Some(text_annotation(1)))],
    )
    .unwrap();
    let mut edited = text_annotation(1);
    edited.modified_unix_ms = 10;
    let cs = db
        .apply_annotation_changes(
            "edit timestamp",
            TransactionId(2),
            vec![(AnnotationId(1), Some(edited))],
        )
        .unwrap();
    assert_eq!(
        cs.changes,
        vec![ObjectChange::Update(ObjectId(1), ChangeMask::METADATA)]
    );
}

// ---------------------------------------------------------------------------
// Export/dirty invariants (B07 seam).
// ---------------------------------------------------------------------------

#[test]
fn marking_a_future_revision_is_refused_and_dirty_stays() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    db.apply_annotation_changes(
        "seed",
        TransactionId(1),
        vec![(AnnotationId(1), Some(text_annotation(1)))],
    )
    .unwrap();
    assert!(db.is_dirty());
    assert!(db.mark_exported(Revision(9)).is_err());
    assert!(db.is_dirty());
    db.mark_exported(Revision(1)).unwrap();
    assert!(!db.is_dirty());
}

#[test]
fn later_edit_after_mark_is_dirty_again() {
    let mut db = AnnotationDatabase::new(DatabaseId(1));
    db.apply_annotation_changes(
        "seed",
        TransactionId(1),
        vec![(AnnotationId(1), Some(text_annotation(1)))],
    )
    .unwrap();
    db.mark_exported(db.revision()).unwrap();
    assert!(!db.is_dirty());
    db.apply_annotation_changes(
        "edit",
        TransactionId(2),
        vec![(AnnotationId(1), Some(text_annotation(1)))],
    )
    .unwrap();
    assert!(
        db.is_dirty(),
        "a later revision must invalidate saved state"
    );
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
    })
    .unwrap();
    ok.insert_entity(line_entity(1, LayerId(0))).unwrap();
    assert!(ok.finish().is_ok());

    let mut bad = DrawingDatabaseBuilder::new(DatabaseId(1));
    bad.insert_layer(layer()).unwrap();
    bad.insert_block(BlockDefinition {
        id: BlockId(0),
        entities: vec![EntityId(99)],
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
