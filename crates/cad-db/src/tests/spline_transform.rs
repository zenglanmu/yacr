//! Baking an affine transform into a `SPLINE` definition.
//!
//! `transform_geometry` used to refuse every spline ("not modelled"), which made
//! MOVE/COPY/ROTATE/SCALE/MIRROR unusable on a NURBS while `STRETCH` had to keep
//! its own in-app control-point translation for the fully enclosed case. That
//! asymmetry is gone: a NURBS curve is **affinely invariant**, so mapping every
//! control point through the transform with `degree`, `knots` and `weights`
//! copied verbatim is exactly the affine image of the curve.
//!
//! These tests pin the four consequences that make the rule safe:
//!
//! * the control points are the transform applied per point, bit for bit, and the
//!   three definition vectors are **unchanged** (they are copies, so no
//!   reparameterisation and no weight change can creep in);
//! * every affine case is accepted — translation, rotation, uniform scale,
//!   **non-uniform** scale/shear and mirror — because the mapping is affine, not
//!   similarity-only;
//! * a non-finite matrix **and** a finite matrix that overflows a finite control
//!   point are both refused, so no unevaluable definition can leave this call;
//! * the structural NURBS relations (`knots == n + degree + 1`, `weights == n`,
//!   non-decreasing knots) hold after the map by construction, while an input
//!   definition that already violated them is still refused by the pre-existing
//!   validation instead of being quietly accepted through the transform path.

use super::*;

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A consistent clamped cubic: 4 control points, 8 knots, 4 unit weights, with
/// non-zero z on the interior points so a map that drops a coordinate is visible.
fn cubic() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            p3(0.0, 0.0, 0.0),
            p3(1.0, 2.0, 0.5),
            p3(3.0, 2.0, 1.0),
            p3(4.0, 0.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0, 1.0],
    }
}

/// The same shape with a non-trivial rational weight vector, so "the weights are
/// copied verbatim" is not testing an all-ones vector only.
fn rational() -> SemanticGeometry {
    SemanticGeometry::Spline {
        degree: 2,
        knots: vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0],
        control_points: vec![
            p3(0.0, 0.0, 0.0),
            p3(2.0, 4.0, 0.0),
            p3(4.0, 0.0, 0.0),
            p3(6.0, 4.0, 0.0),
        ],
        weights: vec![1.0, 0.5, 2.0, 1.0],
    }
}

fn spline_parts(geometry: &SemanticGeometry) -> (u32, Vec<f64>, Vec<Point3>, Vec<f64>) {
    match geometry {
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => (
            *degree,
            knots.clone(),
            control_points.clone(),
            weights.clone(),
        ),
        other => panic!("expected a spline, got {other:?}"),
    }
}

/// Apply `transform` and assert the three definition vectors are bit-identical to
/// the source's — the property that makes the map exact and count-preserving.
fn transform_spline(source: &SemanticGeometry, transform: &Transform3) -> SemanticGeometry {
    let moved = transform_geometry(source, transform).expect("spline transforms");
    let (degree_before, knots_before, _, weights_before) = spline_parts(source);
    let (degree_after, knots_after, _, weights_after) = spline_parts(&moved);
    assert_eq!(degree_after, degree_before, "the degree is copied verbatim");
    assert_eq!(knots_after, knots_before, "the knots are copied verbatim");
    assert_eq!(
        weights_after, weights_before,
        "the weights are copied verbatim"
    );
    moved
}

#[test]
fn spline_translation_maps_every_control_point_and_keeps_the_definition() {
    let source = cubic();
    let delta = p3(10.0, -3.0, 2.0);
    let moved = transform_spline(&source, &Transform3::translation(delta));
    let (_, knots, control_points, weights) = spline_parts(&moved);
    let (_, _, original_points, _) = spline_parts(&source);
    for (after, before) in control_points.iter().zip(&original_points) {
        assert_eq!(*after, p3(before.x + 10.0, before.y - 3.0, before.z + 2.0));
    }
    // The source is untouched: the transform returns a new definition.
    assert_eq!(spline_parts(&source).2, original_points);
    assert_eq!(
        knots.len(),
        8,
        "the knot vector is neither expanded nor cut"
    );
    assert_eq!(weights, vec![1.0, 1.0, 1.0, 1.0]);
    crate::validate_geometry(&moved).expect("a translated spline is still valid");
}

#[test]
fn spline_rotation_maps_every_control_point_exactly() {
    let source = cubic();
    let moved = transform_spline(&source, &rotation_z(std::f64::consts::FRAC_PI_2));
    let (_, _, control_points, _) = spline_parts(&moved);
    let (_, _, original_points, _) = spline_parts(&source);
    for (after, before) in control_points.iter().zip(&original_points) {
        // 90° about +Z: (x, y) -> (-y, x); z is unchanged.
        assert!((after.x + before.y).abs() < 1e-12, "{after:?}");
        assert!((after.y - before.x).abs() < 1e-12, "{after:?}");
        assert_eq!(after.z, before.z);
    }
}

#[test]
fn spline_uniform_scale_maps_every_control_point_exactly() {
    let source = rational();
    let moved = transform_spline(&source, &Transform3::scale(2.5));
    let (_, knots, control_points, weights) = spline_parts(&moved);
    let (_, _, original_points, _) = spline_parts(&source);
    for (after, before) in control_points.iter().zip(&original_points) {
        assert_eq!(after.x, before.x * 2.5);
        assert_eq!(after.y, before.y * 2.5);
        assert_eq!(after.z, before.z * 2.5);
    }
    assert_eq!(knots, vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0]);
    assert_eq!(
        weights,
        vec![1.0, 0.5, 2.0, 1.0],
        "a scale must not be compensated through the weights"
    );
}

/// The case the old refusal was really about: a **non-uniform** scale distorts a
/// circle into an ellipse but is *exact* on a NURBS control hull, so it must be
/// accepted — with a shear on top, which no similarity check would allow.
#[test]
fn spline_non_uniform_scale_and_shear_map_every_control_point_exactly() {
    let source = rational();
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = 3.0;
    matrix[1][1] = 0.5;
    matrix[0][1] = 2.0; // shear: x += 2y
    let moved = transform_spline(&source, &Transform3 { matrix });
    let (_, knots, control_points, _) = spline_parts(&moved);
    assert_eq!(knots, vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0]);
    let (_, _, original_points, _) = spline_parts(&source);
    for (after, before) in control_points.iter().zip(&original_points) {
        assert_eq!(after.x, 3.0 * before.x + 2.0 * before.y);
        assert_eq!(after.y, 0.5 * before.y);
        assert_eq!(after.z, before.z);
    }
    crate::validate_geometry(&moved).expect("a non-uniformly scaled spline is still valid");
}

#[test]
fn spline_mirror_maps_every_control_point_and_keeps_the_definition() {
    let source = cubic();
    // Reflection across the YZ plane: only the linear part flips x.
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = -1.0;
    let moved = transform_spline(&source, &Transform3 { matrix });
    let (_, knots, control_points, _) = spline_parts(&moved);
    let (_, _, original_points, _) = spline_parts(&source);
    for (after, before) in control_points.iter().zip(&original_points) {
        assert_eq!(after.x, -before.x);
        assert_eq!(after.y, before.y);
        assert_eq!(after.z, before.z);
    }
    assert_eq!(knots.len(), 8, "a mirror is not a reparameterisation");
    crate::validate_geometry(&moved).expect("a mirrored spline is still valid");
}

#[test]
fn spline_with_the_empty_knot_and_weight_shorthands_keeps_them_empty() {
    // `knots: []` (clamped uniform) and `weights: []` (all ones) are sanctioned
    // shorthand in this repo. The transform copies them, so the shorthand stays a
    // shorthand instead of being expanded into vectors the source never had.
    let source = SemanticGeometry::Spline {
        degree: 3,
        knots: Vec::new(),
        control_points: vec![
            p3(0.0, 0.0, 0.0),
            p3(1.0, 1.0, 0.0),
            p3(3.0, 1.5, 0.0),
            p3(5.0, 1.0, 0.0),
        ],
        weights: Vec::new(),
    };
    let moved = transform_spline(&source, &rotation_z(0.25));
    let (_, knots, control_points, weights) = spline_parts(&moved);
    assert!(
        knots.is_empty(),
        "the clamped-uniform shorthand is preserved"
    );
    assert!(weights.is_empty(), "the all-ones shorthand is preserved");
    assert_eq!(control_points.len(), 4);
    crate::validate_geometry(&moved).expect("shorthands stay writable after a move");
}

#[test]
fn a_non_finite_transform_is_refused() {
    let source = cubic();
    let mut matrix = Transform3::identity().matrix;
    matrix[0][3] = f64::NAN;
    let error = transform_geometry(&source, &Transform3 { matrix })
        .expect_err("a NaN translation is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");

    let mut matrix = Transform3::identity().matrix;
    matrix[1][1] = f64::INFINITY;
    let error = transform_geometry(&source, &Transform3 { matrix })
        .expect_err("an infinite scale is refused");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
}

/// A finite matrix can still overflow a finite control point; the result must be
/// refused rather than returned as an unevaluable definition.
#[test]
fn a_finite_transform_that_overflows_a_control_point_is_refused() {
    let source = SemanticGeometry::Spline {
        degree: 1,
        knots: vec![0.0, 0.0, 1.0, 2.0],
        control_points: vec![p3(1.0e10, 0.0, 0.0), p3(2.0e10, 0.0, 0.0)],
        weights: vec![1.0, 1.0],
    };
    let error = transform_geometry(&source, &Transform3::scale(1.0e300))
        .expect_err("the mapped control point overflows");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(error.to_string().contains("non-finite"), "{error}");
}

/// The transform copies vectors, so a definition that was already inconsistent
/// stays inconsistent — and the pre-existing validation still refuses it. The new
/// arm must not become a way to store a NURBS the database rejects on every other
/// path.
#[test]
fn an_inconsistent_input_spline_is_still_refused_by_the_existing_validation() {
    let inconsistent = SemanticGeometry::Spline {
        degree: 3,
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        control_points: vec![
            p3(0.0, 0.0, 0.0),
            p3(1.0, 2.0, 0.0),
            p3(3.0, 2.0, 0.0),
            p3(4.0, 0.0, 0.0),
        ],
        weights: vec![1.0, 1.0, 1.0], // one weight short
    };
    // The pure map only moves points (it is not a validator), so the counts it
    // copies are still wrong…
    let moved = transform_geometry(&inconsistent, &Transform3::translation(point(1.0, 0.0)))
        .expect("the mapping itself neither repairs nor rejects counts");
    let (_, _, control_points, weights) = spline_parts(&moved);
    assert_eq!(control_points.len(), 4);
    assert_eq!(weights.len(), 3, "the copy keeps the wrong count");
    // …and the existing validation refuses it with the same stable code.
    let error = crate::validate_geometry(&moved).expect_err("still inconsistent");
    assert!(
        error
            .to_string()
            .contains(crate::REASON_SPLINE_WEIGHT_COUNT),
        "{error}"
    );

    // The write path refuses it too, so it can never reach the store.
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("insert inconsistent", TransactionId(1))
        .unwrap();
    tx.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbSpline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        ..raw_entity(1, SpaceId::Model, moved)
    })
    .unwrap();
    let error = tx.commit().unwrap_err();
    assert!(
        error
            .to_string()
            .contains(crate::REASON_SPLINE_WEIGHT_COUNT),
        "{error}"
    );
}

/// MOVE's database entry point: one staged transform, one committed `Update` with
/// `TRANSFORM` **and** `GEOMETRY`, one revision bump, and the stored definition
/// passes validation.
#[test]
fn transform_entity_moves_a_stored_spline_in_one_transform_change() {
    let mut db = writable_db();
    let mut tx = db
        .begin_drawing_transaction("create spline", TransactionId(1))
        .unwrap();
    tx.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbSpline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        ..raw_entity(1, SpaceId::Model, cubic())
    })
    .unwrap();
    tx.commit().unwrap();
    let start = db.revision();

    let mut tx = db
        .begin_drawing_transaction("move spline", TransactionId(2))
        .unwrap();
    tx.transform_entity(EntityId(1), &Transform3::translation(point(5.0, 6.0)))
        .unwrap();
    let changes = tx.commit().expect("a spline move commits");
    assert_eq!(
        changes.changes,
        vec![ObjectChange::Update(
            ObjectId(1),
            ChangeMask::GEOMETRY.union(ChangeMask::TRANSFORM)
        )],
        "MOVE carries the TRANSFORM bit and applies as a single change"
    );
    assert_eq!(changes.before, start);
    assert_eq!(changes.after, Revision(start.0 + 1));

    let stored = db.entity(EntityId(1)).expect("the moved spline").clone();
    crate::validate_geometry(&stored.geometry).expect("the stored definition is valid");
    let (_, knots, control_points, weights) = spline_parts(&stored.geometry);
    assert_eq!(knots, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
    assert_eq!(weights, vec![1.0, 1.0, 1.0, 1.0]);
    assert_eq!(control_points[0], p3(5.0, 6.0, 0.0));
    assert_eq!(control_points[3], p3(9.0, 6.0, 0.0));
    // Id, space, layer and draw order are preserved by the move.
    assert_eq!(stored.id, EntityId(1));
    assert_eq!(stored.space, SpaceId::Model);
    assert_eq!(stored.layer, LayerId(0));
    assert_eq!(stored.draw_order, 1);
}

/// The locked-layer rule covers the new arm automatically (it lives in the shared
/// staging gate, not per kind): a spline on a locked layer stages nothing, and the
/// stored definition is bit-identical afterwards.
#[test]
fn a_spline_on_a_locked_layer_refuses_a_move_without_staging() {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    for (id, name) in [(LayerId(0), "0"), (LayerId(3), "WALLS")] {
        b.insert_layer(Layer {
            id,
            name: name.into(),
            visible: true,
        })
        .unwrap();
    }
    b.set_layer_attributes(
        LayerId(3),
        LayerAttributes {
            locked: true,
            ..LayerAttributes::default()
        },
    )
    .unwrap();
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
    let mut entity = raw_entity(1, SpaceId::Model, cubic());
    entity.layer = LayerId(3);
    b.insert_entity(DbEntity {
        object: DbObject {
            id: ObjectId(1),
            type_key: "AcDbSpline".into(),
            revision: Revision(0),
            source_handle: None,
        },
        ..entity
    })
    .unwrap();
    let mut db = b.finish().unwrap();
    let before = db.revision();
    let stored = db.entity(EntityId(1)).cloned().unwrap();

    let mut tx = db
        .begin_drawing_transaction("locked spline move", TransactionId(1))
        .unwrap();
    let error = tx
        .transform_entity(EntityId(1), &Transform3::translation(point(1.0, 1.0)))
        .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("layer.locked"), "stable code missing: {text}");
    assert!(text.contains("WALLS"), "the layer is named: {text}");
    assert_eq!(tx.staged_len(), 0, "a refused move stages nothing");
    let changes = tx.commit().unwrap();
    assert!(changes.is_empty());
    assert_eq!(db.revision(), before);
    assert_eq!(db.entity(EntityId(1)), Some(&stored));
}

/// Rotation about +Z, built from the public matrix so the test pins the exact
/// arithmetic instead of a helper's tolerance.
fn rotation_z(angle: f64) -> Transform3 {
    let mut matrix = Transform3::identity().matrix;
    let (sin, cos) = angle.sin_cos();
    matrix[0][0] = cos;
    matrix[0][1] = -sin;
    matrix[1][0] = sin;
    matrix[1][1] = cos;
    Transform3 { matrix }
}
