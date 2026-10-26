//! Spline definition invariants on the validated write path.
//!
//! A NURBS definition is internally consistent only when the knot vector has
//! `control_points.len() + degree + 1` entries (one per span end plus the
//! `degree + 1` end multiplicity on either side) and the weight vector has one
//! weight per control point. Before these checks existed, only the degree, the
//! minimum control-point count, finiteness and weight positivity were enforced,
//! so an inconsistent definition could be stored. `cad-app`'s control-point grip
//! model refuses to invent handles for such a definition; these tests pin the
//! database side of the same contract: refuse before any mutation with a stable
//! reason code that names the actual and expected counts.
//!
//! Nothing here repairs, pads or uniformises a vector — a refusal is the whole
//! answer.

use super::*;

/// A consistent clamped cubic: 4 control points, 8 knots, 4 unit weights.
fn cubic() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 2.0),
            point(3.0, 2.0),
            point(4.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0, 1.0],
    }
}

fn typed_entity(id: u128, type_key: &str, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: type_key.into(),
            revision: Revision(0),
            source_handle: None,
        },
        ..raw_entity(id, SpaceId::Model, geometry)
    }
}

fn spline_entity(id: u128, geometry: SemanticGeometry) -> DbEntity {
    typed_entity(id, "AcDbSpline", geometry)
}

/// The full refusal text, so a test can assert the code and the counts.
fn refusal(geometry: &SemanticGeometry) -> String {
    crate::validate_geometry(geometry)
        .expect_err("expected the spline definition to be refused")
        .to_string()
}

#[test]
fn accepts_a_consistent_cubic_spline() {
    crate::validate_geometry(&cubic()).expect("a clamped cubic is consistent");

    // …and the same geometry is accepted by the write path.
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create spline", TransactionId(1))
        .unwrap();
    tx.insert_entity(spline_entity(1, cubic())).unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(changes.before, Revision(0));
    assert_eq!(changes.after, Revision(1));
    assert_eq!(db.entity_count(), 1);
    assert_eq!(db.entity(EntityId(1)).map(|e| &e.geometry), Some(&cubic()));
}

#[test]
fn refuses_a_knot_vector_one_entry_too_short() {
    // 4 control points at degree 3 need 8 knots; 7 cannot cover the domain.
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 2.0),
            point(3.0, 2.0),
            point(4.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0, 1.0],
    };
    let message = refusal(&geometry);
    assert!(
        message.contains(crate::REASON_SPLINE_KNOT_COUNT),
        "stable reason code missing: {message}"
    );
    assert!(
        message.contains("8 knots"),
        "expected count missing: {message}"
    );
    assert!(
        message.contains("stores 7"),
        "actual count missing: {message}"
    );
    assert!(message.contains("degree 3"), "degree missing: {message}");
    assert!(message.contains("4 control points"), "{message}");
}

#[test]
fn refuses_a_knot_vector_one_entry_too_long() {
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 2.0),
            point(3.0, 2.0),
            point(4.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0, 1.0],
    };
    let message = refusal(&geometry);
    assert!(
        message.contains(crate::REASON_SPLINE_KNOT_COUNT),
        "{message}"
    );
    assert!(message.contains("8 knots"), "{message}");
    assert!(message.contains("stores 9"), "{message}");
}

#[test]
fn refuses_a_non_empty_weight_vector_that_does_not_match_the_control_points() {
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 2.0),
            point(3.0, 2.0),
            point(4.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0],
    };
    let message = refusal(&geometry);
    assert!(
        message.contains(crate::REASON_SPLINE_WEIGHT_COUNT),
        "stable reason code missing: {message}"
    );
    assert!(message.contains("needs 4 weights"), "{message}");
    assert!(message.contains("stores 3"), "{message}");
}

#[test]
fn accepts_an_empty_weight_vector_as_the_documented_all_ones_convention() {
    // `cad-geometry::NurbsCurve::new` expands an empty weight vector to one
    // weight per control point, so the empty vector is a real convention in this
    // repo (the MULTILEADER leader path stores it). Only a *non-empty* vector has
    // to match the control-point count.
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 2.0),
            point(3.0, 2.0),
            point(4.0, 0.0),
        ],
        weights: Vec::new(),
    };
    crate::validate_geometry(&geometry).expect("empty weights mean all ones");
}

#[test]
fn accepts_an_empty_knot_vector_as_the_documented_clamped_uniform_convention() {
    // `knots: []` is what both MULTILEADER leader-path constructors store
    // (`cad-domain/src/multileader.rs`, `cad-import-acadrust/src/dimension/
    // multileader.rs`) to mean "clamped uniform". `tessellate_spline` expands it
    // through `clamped_uniform_knots`, so it is a real form the repo produces and
    // it must stay writable; only a *non-empty* vector has to match
    // `n + degree + 1`.
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: Vec::new(),
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(3.0, 1.5),
            point(5.0, 1.0),
        ],
        weights: Vec::new(),
    };
    crate::validate_geometry(&geometry).expect("empty knots mean clamped uniform");

    // …and it round-trips through insert + update, because an imported
    // MULTILEADER edit re-writes exactly this form.
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create shorthand spline", TransactionId(1))
        .unwrap();
    tx.insert_entity(spline_entity(1, geometry.clone()))
        .unwrap();
    tx.commit().unwrap();
    let inserted = db.revision();
    assert_eq!(db.entity(EntityId(1)).map(|e| &e.geometry), Some(&geometry));

    let mut moved = db.entity(EntityId(1)).cloned().unwrap();
    moved.geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: Vec::new(),
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(3.0, 2.5),
            point(5.0, 1.0),
        ],
        weights: Vec::new(),
    };
    let mut tx = db
        .begin_drawing_transaction("move control point", TransactionId(2))
        .unwrap();
    tx.update_entity(moved.clone()).unwrap();
    tx.commit().unwrap();
    assert_eq!(db.revision(), Revision(inserted.0 + 1));
    assert_eq!(db.entity(EntityId(1)), Some(&moved));
}

#[test]
fn the_two_empty_vector_exemptions_do_not_mask_each_other() {
    // Empty knots are sanctioned; a *non-empty* weight vector of the wrong length
    // is not, and the knot exemption must not hide it.
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: Vec::new(),
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(3.0, 1.5),
            point(5.0, 1.0),
        ],
        weights: vec![1.0, 1.0],
    };
    let message = refusal(&geometry);
    assert!(
        message.contains(crate::REASON_SPLINE_WEIGHT_COUNT),
        "stable reason code missing: {message}"
    );
    assert!(message.contains("needs 4 weights"), "{message}");
    assert!(message.contains("stores 2"), "{message}");
    assert!(
        !message.contains(crate::REASON_SPLINE_KNOT_COUNT),
        "the knot exemption must not be reported: {message}"
    );

    // The mirror case: empty weights are sanctioned, a non-empty knot vector of
    // the wrong length is not, and the weight exemption must not hide it.
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 1.0),
            point(3.0, 1.5),
            point(5.0, 1.0),
        ],
        weights: Vec::new(),
    };
    let message = refusal(&geometry);
    assert!(
        message.contains(crate::REASON_SPLINE_KNOT_COUNT),
        "stable reason code missing: {message}"
    );
    assert!(message.contains("needs 8 knots"), "{message}");
    assert!(message.contains("stores 6"), "{message}");
    assert!(
        !message.contains(crate::REASON_SPLINE_WEIGHT_COUNT),
        "the weight exemption must not be reported: {message}"
    );
}

#[test]
fn a_multileader_whose_display_is_the_clamped_uniform_spline_is_writable() {
    // The import path derives a spline leader path with `knots: []` /
    // `weights: []`. Since `validate_geometry` recurses into `display()`, the
    // exemption is what keeps an *imported* MULTILEADER editable; this test pins
    // that end to end through the database.
    let (geometry, reasons) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::Spline,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 2.0,
            landing_gap: 0.5,
            arrowhead_size: 1.0,
            text_height: 2.5,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: "Note".into(),
            location: point(12.0, 3.0),
            rotation: 0.0,
            height: 2.5,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 0,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: point(10.0, 2.0),
            dogleg_vector: point(1.0, 0.0),
            dogleg_length: 2.0,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![
                point(0.0, 0.0),
                point(1.0, 1.0),
                point(3.0, 1.5),
                point(5.0, 1.0),
            ],
            path_type: MLeaderPathType::Spline,
            override_flags: MLeaderLine::PATH_TYPE,
            arrowhead_size: 1.0,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    assert!(
        reasons
            .iter()
            .all(|reason| reason.contains("resolved but not applied")),
        "unexpected degradation: {reasons:?}"
    );
    let stored = SemanticGeometry::MultiLeader(Box::new(geometry));
    match &stored {
        SemanticGeometry::MultiLeader(leader) => match &leader.display()[0] {
            SemanticGeometry::Spline {
                degree,
                knots,
                control_points,
                weights,
            } => {
                assert_eq!(*degree, 3);
                assert!(knots.is_empty(), "the producer form is the shorthand");
                assert!(weights.is_empty());
                assert_eq!(control_points.len(), 4);
            }
            other => panic!("expected the leader spline, got {other:?}"),
        },
        other => panic!("expected a multileader, got {other:?}"),
    }
    crate::validate_geometry(&stored).expect("the imported form must stay writable");

    // Insert, then edit the text (the channel an imported multileader uses):
    // both writes go through the validated path and keep the shorthand spline.
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("import multileader", TransactionId(1))
        .unwrap();
    tx.insert_entity(typed_entity(1, "AcDbMLeader", stored))
        .unwrap();
    tx.commit().unwrap();
    let inserted = db.revision();

    let mut edited = db.entity(EntityId(1)).cloned().unwrap();
    edited.geometry = match &edited.geometry {
        SemanticGeometry::MultiLeader(leader) => SemanticGeometry::MultiLeader(Box::new(
            leader
                .with_text("Note 2")
                .expect("text content is replaceable"),
        )),
        other => panic!("expected a multileader, got {other:?}"),
    };
    let mut tx = db
        .begin_drawing_transaction("edit multileader text", TransactionId(2))
        .unwrap();
    tx.update_entity(edited.clone()).unwrap();
    tx.commit()
        .expect("an imported multileader must stay editable");
    assert_eq!(db.revision(), Revision(inserted.0 + 1));
    assert_eq!(db.entity(EntityId(1)), Some(&edited));
}

#[test]
fn refuses_a_knot_vector_that_decreases() {
    let geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 0.75, 0.5, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 2.0),
            point(3.0, 2.0),
            point(4.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0, 1.0],
    };
    let message = refusal(&geometry);
    assert!(
        message.contains(crate::REASON_SPLINE_KNOT_ORDER),
        "stable reason code missing: {message}"
    );
    assert!(message.contains("non-decreasing"), "{message}");
}

#[test]
fn pre_existing_degree_and_control_point_rules_still_refuse() {
    // Regression: the new length relations must not shadow the older checks, and
    // the older checks must not be relaxed to let a shorter vector through.
    let degree_zero = SemanticGeometry::Spline {
        degree: 0,
        knots: vec![0.0, 1.0],
        control_points: vec![point(0.0, 0.0)],
        weights: vec![1.0],
    };
    let message = refusal(&degree_zero);
    assert!(
        message.contains("spline degree must be at least 1"),
        "{message}"
    );
    assert!(
        !message.contains(crate::REASON_SPLINE_KNOT_COUNT),
        "{message}"
    );

    let too_few_control_points = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0],
        control_points: vec![point(0.0, 0.0), point(1.0, 1.0)],
        weights: vec![1.0, 1.0],
    };
    let message = refusal(&too_few_control_points);
    assert!(
        message.contains("fewer control points than its degree requires"),
        "{message}"
    );
    assert!(
        !message.contains(crate::REASON_SPLINE_KNOT_COUNT),
        "{message}"
    );
}

#[test]
fn refusal_leaves_the_revision_the_entity_table_and_the_stored_spline_untouched() {
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create spline", TransactionId(1))
        .unwrap();
    tx.insert_entity(spline_entity(1, cubic())).unwrap();
    tx.commit().unwrap();
    let revision = db.revision();
    let stored = db.entity(EntityId(1)).cloned().unwrap();

    // An insert of an inconsistent definition is refused at commit time.
    let mut tx = db
        .begin_drawing_transaction("create bad spline", TransactionId(2))
        .unwrap();
    tx.insert_entity(spline_entity(
        2,
        SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![
                point(0.0, 0.0),
                point(1.0, 2.0),
                point(3.0, 2.0),
                point(4.0, 0.0),
            ],
            weights: vec![1.0, 1.0, 1.0, 1.0],
        },
    ))
    .unwrap();
    let error = tx.commit().unwrap_err();
    assert!(
        error.to_string().contains(crate::REASON_SPLINE_KNOT_COUNT),
        "{error}"
    );
    assert_eq!(db.revision(), revision, "a refused write must not bump");
    assert_eq!(db.entity_count(), 1, "a refused write must not insert");
    assert_eq!(db.entity(EntityId(2)), None);
    assert_eq!(db.entity(EntityId(1)), Some(&stored));

    // An update of an existing spline into an inconsistent definition is refused
    // for the same reason, through the same path, and stores nothing.
    let mut inconsistent = stored.clone();
    inconsistent.geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: match &stored.geometry {
            SemanticGeometry::Spline { control_points, .. } => control_points.clone(),
            other => panic!("expected a stored spline, got {other:?}"),
        },
        weights: vec![1.0, 1.0],
    };
    let mut tx = db
        .begin_drawing_transaction("bad spline edit", TransactionId(3))
        .unwrap();
    tx.update_entity(inconsistent).unwrap();
    let error = tx.commit().unwrap_err();
    assert!(
        error
            .to_string()
            .contains(crate::REASON_SPLINE_WEIGHT_COUNT),
        "{error}"
    );
    assert_eq!(db.revision(), revision);
    assert_eq!(db.entity_count(), 1);
    assert_eq!(db.entity(EntityId(1)), Some(&stored));
}

#[test]
fn a_valid_spline_round_trips_through_an_update() {
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create spline", TransactionId(1))
        .unwrap();
    tx.insert_entity(spline_entity(1, cubic())).unwrap();
    tx.commit().unwrap();
    let revision = db.revision();

    // Moving one control point keeps every vector length: a definition that was
    // consistent stays consistent, and the update goes through.
    let mut moved = db.entity(EntityId(1)).cloned().unwrap();
    moved.geometry = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            point(0.0, 0.0),
            point(1.0, 3.0),
            point(3.0, 2.0),
            point(4.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0, 1.0],
    };
    let mut tx = db
        .begin_drawing_transaction("move control point", TransactionId(2))
        .unwrap();
    tx.update_entity(moved.clone()).unwrap();
    let changes = tx.commit().unwrap();
    assert_eq!(changes.after, Revision(revision.0 + 1));
    assert_eq!(db.revision(), Revision(revision.0 + 1));
    assert_eq!(db.entity(EntityId(1)), Some(&moved));
}
