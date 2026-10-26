//! `INSERT` (block reference) insertion-point grip contract tests (`crate::grips`).
//!
//! A block reference is the first geometry in this model whose handle position is
//! **derived from a transform**: `SemanticGeometry::Insert { block, transform }`
//! stores a 4x4 affine placement, and the handle is that placement's **insertion
//! point** — the image of the block-space origin under it (the importer folds the
//! block base point into the placement, so this is the stored `INSERT` insertion
//! point; see `cad-import-acadrust`'s `insert_array_transform`/`block_placement`).
//! `Transform3::apply_point` has no homogeneous divide, so that image is exactly
//! the matrix's translation column.
//!
//! These tests pin the two claims that make the rule unambiguous:
//!
//! * the handle is the **transformed origin**, at the exact `f64` bits — for a
//!   translation-only placement and, crucially, for a rotated + non-uniformly
//!   scaled one (the second catches "the handle is the translation read as a
//!   point" and any confusion with a base point or a centroid);
//! * a drag is a **rigid move**: only the translation column changes, so the whole
//!   3x3 linear part (rotation, non-uniform scale, mirror), row 3 and `block` stay
//!   bit-identical, and the new insertion point is the target bit for bit. A drag
//!   therefore never rotates or scales a block reference.
//!
//! The anti-bug assertion is
//! [`a_rigid_drag_keeps_every_block_space_point_consistent`]: `new ∘ old⁻¹` must be
//! the pure translation `to - old_origin` for every sample point, which fails for
//! any rule that writes the target somewhere other than the origin's image.
//!
//! Block **members** (instance paths), block **attributes** and points **inside** a
//! block are **NOT IMPLEMENTED** and are not reachable from a `GripId` at all; this
//! module only describes the top-level model-space reference it is handed.

use super::*;

use super::grips::{assert_bits_eq, assert_reason, ent, grip_id, multi_leader, p3};
use crate::grips::{
    apply_grip, grip_hit, grip_hit_planar, grips_for_entity, Grip, GripId, GripKind,
    REASON_DEGENERATE_TRANSFORM, REASON_NON_FINITE_TARGET, REASON_STALE_HANDLE,
    REASON_UNSUPPORTED_KIND, REASON_ZERO_DISPLACEMENT,
};

const BLOCK: BlockId = BlockId(7);

/// A translation-only placement: for an axis-aligned insert the transform *is* the
/// insertion point, so the simplest case pins "the handle is where the block is
/// anchored".
const PLAIN_AT: Point3 = Point3 {
    x: 12.5,
    y: -3.25,
    z: 2.0,
};

/// A placement with a rotation and a **non-uniform** scale: the insertion point is
/// no longer "the matrix read as a point" in any accidental sense, and a drag that
/// touched the linear part would show up immediately.
const ROTATED_AT: Point3 = Point3 {
    x: -4.5,
    y: 8.125,
    z: 1.0,
};
const ROTATION: f64 = 0.5;
const SCALE: Point3 = Point3 {
    x: 2.0,
    y: 3.0,
    z: 4.0,
};

/// A one-cell block reference placed by a pure translation.
fn translated_at(at: Point3) -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BLOCK,
        transform: Transform3::translation(at),
    }
}

/// The importer's own composition order for a single INSERT cell —
/// `translate * rotate(Z) * scale`, with the block base point already folded into
/// the translation — so the fixture has the shape the drawing really stores.
fn rotated_transform() -> Transform3 {
    let (sin, cos) = ROTATION.sin_cos();
    let rotate = Transform3 {
        matrix: [
            [cos, -sin, 0.0, 0.0],
            [sin, cos, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let scale = Transform3 {
        matrix: [
            [SCALE.x, 0.0, 0.0, 0.0],
            [0.0, SCALE.y, 0.0, 0.0],
            [0.0, 0.0, SCALE.z, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    Transform3::translation(ROTATED_AT)
        .matrix_mul(&rotate)
        .matrix_mul(&scale)
}

fn rotated() -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BLOCK,
        transform: rotated_transform(),
    }
}

fn insert_parts(geometry: &SemanticGeometry) -> (BlockId, Transform3) {
    match geometry {
        SemanticGeometry::Insert { block, transform } => (*block, *transform),
        other => panic!("expected an INSERT, got {other:?}"),
    }
}

/// The model's definition of the handle, recomputed independently of the module:
/// the placement applied to the block-space origin.
fn transformed_origin(transform: &Transform3) -> Point3 {
    transform.apply_point(p3(0.0, 0.0, 0.0))
}

/// The 3x3 linear part as raw bits, so "bit-identical" is a real comparison.
fn linear_bits(transform: &Transform3) -> [[u64; 3]; 3] {
    let m = transform.matrix;
    [
        [m[0][0].to_bits(), m[0][1].to_bits(), m[0][2].to_bits()],
        [m[1][0].to_bits(), m[1][1].to_bits(), m[1][2].to_bits()],
        [m[2][0].to_bits(), m[2][1].to_bits(), m[2][2].to_bits()],
    ]
}

/// The stored scale factors: the lengths of the linear part's three columns.
fn column_lengths(transform: &Transform3) -> [u64; 3] {
    let m = transform.matrix;
    let mut out = [0u64; 3];
    for (c, cell) in out.iter_mut().enumerate() {
        *cell = m[0][c].hypot(m[1][c]).hypot(m[2][c]).to_bits();
    }
    out
}

fn sub3(a: Point3, b: Point3) -> Point3 {
    p3(a.x - b.x, a.y - b.y, a.z - b.z)
}

/// Both placement fixtures, so every handle/drag claim is pinned for the
/// translation-only *and* the rotated + scaled shape.
fn fixtures() -> [(&'static str, SemanticGeometry, Point3); 2] {
    [
        (
            "a translation-only insert",
            translated_at(PLAIN_AT),
            PLAIN_AT,
        ),
        (
            "a rotated, non-uniformly scaled insert",
            rotated(),
            ROTATED_AT,
        ),
    ]
}

fn assert_close(actual: Point3, expected: Point3, what: &str) {
    let magnitude = expected
        .x
        .abs()
        .max(expected.y.abs())
        .max(expected.z.abs())
        .max(1.0);
    for (axis, (a, e)) in [
        ("x", (actual.x, expected.x)),
        ("y", (actual.y, expected.y)),
        ("z", (actual.z, expected.z)),
    ] {
        assert!(
            (a - e).abs() <= 1e-12 * magnitude,
            "{what}: {axis} {a} is not {e} (tolerance {})",
            1e-12 * magnitude
        );
    }
}

// ---------------------------------------------------------------------------
// 1. The handle list: exactly one insertion grip, at the transformed origin
// ---------------------------------------------------------------------------

#[test]
fn insert_exposes_exactly_one_insertion_grip_at_the_transformed_origin() {
    for (what, geometry, at) in fixtures() {
        let entity = ent(101, geometry.clone());
        let grips = grips_for_entity(&entity);
        assert_eq!(grips.len(), 1, "{what} exposes exactly one grip");
        assert_eq!(
            grips[0].id,
            grip_id(101, GripKind::Insertion, 0),
            "{what}: the single handle is Insertion 0"
        );
        let (_, transform) = insert_parts(&geometry);
        assert_bits_eq(grips[0].position, transformed_origin(&transform), what);
        // ... and that point is the placement's insertion point, i.e. exactly where
        // the block is anchored.
        assert_bits_eq(grips[0].position, at, what);
    }
}

#[test]
fn the_handle_is_the_origin_image_not_a_point_of_the_linear_part() {
    // Same translation, different rotation/scale: the insertion point must not move
    // at all, because the origin's image never depends on the linear part. A rule
    // that used a block-space base point, a centroid or an axis point would drift
    // here.
    let base = grips_for_entity(&ent(102, rotated()))[0].position;
    let other = Transform3 {
        matrix: [
            [-3.0, 0.5, 0.0, ROTATED_AT.x],
            [0.25, 5.0, 0.0, ROTATED_AT.y],
            [0.0, 0.0, 0.125, ROTATED_AT.z],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let moved_linear = grips_for_entity(&ent(
        103,
        SemanticGeometry::Insert {
            block: BLOCK,
            transform: other,
        },
    ))[0]
        .position;
    assert_bits_eq(moved_linear, base, "the handle ignores rotation and scale");
    assert_bits_eq(base, ROTATED_AT, "the handle is the anchor point");
}

// ---------------------------------------------------------------------------
// 2. The drag rule: a rigid move (linear part and block untouched)
// ---------------------------------------------------------------------------

#[test]
fn dragging_the_insertion_grip_moves_the_reference_rigidly() {
    for (what, geometry, at) in fixtures() {
        let (block, transform) = insert_parts(&geometry);
        let target = p3(at.x + 3.5, at.y - 1.25, at.z + 0.5);
        let dragged = apply_grip(&geometry, &grip_id(104, GripKind::Insertion, 0), target)
            .unwrap_or_else(|error| panic!("{what}: a finite drag must be accepted: {error:?}"));

        let (block_after, transform_after) = insert_parts(&dragged);
        assert_eq!(block_after, block, "{what}: the block is bit-identical");
        assert_eq!(
            linear_bits(&transform_after),
            linear_bits(&transform),
            "{what}: the rotation/scale part is bit-identical"
        );
        assert_eq!(
            transform_after.matrix[3], transform.matrix[3],
            "{what}: row 3 of the placement is untouched"
        );
        // The scale is a derived view of the linear part, pinned separately so "the
        // reference is not scaled" is explicit.
        assert_eq!(
            column_lengths(&transform_after),
            column_lengths(&transform),
            "{what}: the stored scale factors are bit-identical"
        );
        // Only the translation column moved, and it is the target exactly.
        assert_bits_eq(transformed_origin(&transform_after), target, what);
        // The reported handle of the *result* is the target bit for bit.
        assert_bits_eq(
            grips_for_entity(&ent(105, dragged))[0].position,
            target,
            what,
        );
        // Nothing else in the matrix changed: the result equals the stored
        // transform with the translation column replaced.
        let mut expected = transform;
        expected.matrix[0][3] = target.x;
        expected.matrix[1][3] = target.y;
        expected.matrix[2][3] = target.z;
        assert_eq!(
            transform_after, expected,
            "{what}: only the translation moved"
        );
    }
}

#[test]
fn a_rigid_drag_keeps_every_block_space_point_consistent() {
    // The anti-bug test for "changed the translation instead of the origin": a
    // block-space point `p` sits at `old.apply_point(p)`, and after the drag it must
    // be at `old.apply_point(p) + (to - old_origin)` — i.e. `new ∘ old⁻¹` is the
    // pure translation by the handle's delta. A rule that wrote the target anywhere
    // else (or touched the linear part) breaks this.
    let geometry = rotated();
    let (_, old) = insert_parts(&geometry);
    let old_origin = transformed_origin(&old);
    let to = p3(2.0, -6.5, 4.25);
    let dragged = apply_grip(&geometry, &grip_id(106, GripKind::Insertion, 0), to).expect("drag");
    let (_, new) = insert_parts(&dragged);
    let delta = sub3(to, old_origin);
    assert_bits_eq(transformed_origin(&new), to, "the dragged handle");

    for p in [
        p3(0.0, 0.0, 0.0),
        p3(1.0, 0.0, 0.0),
        p3(0.0, 1.0, 0.0),
        p3(1.0, 1.0, 0.0),
        p3(-2.5, 0.75, 3.0),
        p3(1.0 / 3.0, -2.0 / 7.0, 0.125),
    ] {
        let before = old.apply_point(p);
        let after = new.apply_point(p);
        // `new ∘ old⁻¹` is the translation by `delta`.
        assert_close(
            after,
            p3(before.x + delta.x, before.y + delta.y, before.z + delta.z),
            "the block-space point moved by exactly the handle delta",
        );
        // The same claim read in block space: the offset from the insertion point is
        // preserved, so the reference did not rotate or change size.
        assert_close(
            sub3(after, to),
            sub3(before, old_origin),
            "the offset from the insertion point is preserved",
        );
    }
}

#[test]
fn a_singular_but_finite_placement_is_still_translatable() {
    // The rule needs no matrix inverse or decomposition (the target is written
    // straight into the translation column), so a flattened instance is neither
    // refused nor silently "repaired": it translates exactly and its singular
    // linear part stays bit-identical. (`cad-db` refuses to *store* a singular insert
    // transform, so this is the corrupt-input case.)
    let flat = Transform3 {
        matrix: [
            [2.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 2.0],
            [0.0, 0.0, 4.0, 3.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    assert_eq!(flat.determinant(), 0.0, "the fixture really is singular");
    let geometry = SemanticGeometry::Insert {
        block: BLOCK,
        transform: flat,
    };
    let dragged = apply_grip(
        &geometry,
        &grip_id(107, GripKind::Insertion, 0),
        p3(-9.0, 8.0, 7.5),
    )
    .expect("a singular but finite placement still translates");
    let (block, transform) = insert_parts(&dragged);
    assert_eq!(block, BLOCK);
    assert_eq!(
        linear_bits(&transform),
        linear_bits(&flat),
        "the singular linear part is bit-identical"
    );
    assert_bits_eq(
        transformed_origin(&transform),
        p3(-9.0, 8.0, 7.5),
        "the insertion point takes the target",
    );
}

// ---------------------------------------------------------------------------
// 3. Refusals, in the module's fixed order
// ---------------------------------------------------------------------------

#[test]
fn insert_refuses_every_kind_and_index_it_has_no_handle_for() {
    for (what, geometry, _) in fixtures() {
        // Any other kind on a block reference is a **stale handle**, never an
        // unsupported kind: the geometry does have a grip model.
        for (kind, index) in [
            (GripKind::Endpoint, 0u8),
            (GripKind::Endpoint, 1),
            (GripKind::Midpoint, 2),
            (GripKind::Center, 0),
            (GripKind::Quadrant, 0),
            (GripKind::Vertex, 0),
            (GripKind::ControlPoint, 0),
            (GripKind::Point, 0),
            (GripKind::Insertion, 1),
            (GripKind::Insertion, u8::MAX),
        ] {
            let error = apply_grip(&geometry, &grip_id(108, kind, index), p3(1.0, 1.0, 1.0))
                .expect_err("an INSERT has no handle for this kind/index");
            assert_reason(error, REASON_STALE_HANDLE);
        }
        // The kind is not on the refusal list any more: a valid id is accepted (and
        // therefore never reports `grip.unsupported_kind`).
        assert!(
            apply_grip(
                &geometry,
                &grip_id(108, GripKind::Insertion, 0),
                p3(1.0, 1.0, 1.0)
            )
            .is_ok(),
            "{what}: an insert has a grip model now"
        );
    }
}

#[test]
fn an_insert_drag_onto_its_own_insertion_point_writes_nothing() {
    for (_, geometry, at) in fixtures() {
        let error = apply_grip(&geometry, &grip_id(109, GripKind::Insertion, 0), at)
            .expect_err("a drag onto the handle itself is refused");
        assert_reason(error, REASON_ZERO_DISPLACEMENT);
    }
}

#[test]
fn insert_refuses_a_non_finite_target() {
    for target in [
        p3(f64::NAN, 0.0, 0.0),
        p3(0.0, f64::INFINITY, 0.0),
        p3(0.0, 0.0, f64::NEG_INFINITY),
    ] {
        let error = apply_grip(&rotated(), &grip_id(110, GripKind::Insertion, 0), target)
            .expect_err("a non-finite target is refused instead of stored");
        assert_reason(error, REASON_NON_FINITE_TARGET);
    }
}

#[test]
fn handle_identity_is_checked_before_the_target_and_before_the_placement() {
    // A wrong kind reports a stale handle even when the target is also non-finite
    // (identity first) ...
    let error = apply_grip(
        &rotated(),
        &grip_id(111, GripKind::Center, 0),
        p3(f64::NAN, 0.0, 0.0),
    )
    .expect_err("a wrong kind is stale");
    assert_reason(error, REASON_STALE_HANDLE);

    // ... and a non-finite *stored* placement does not turn a stale id into a
    // transform refusal either.
    let error = apply_grip(
        &SemanticGeometry::Insert {
            block: BLOCK,
            transform: corrupt_placement(),
        },
        &grip_id(112, GripKind::Vertex, 0),
        p3(1.0, 1.0, 1.0),
    )
    .expect_err("a wrong kind is stale even on a corrupt placement");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn a_non_finite_stored_placement_is_refused_by_name() {
    // The rigid rule keeps every coefficient except the translation column, so a
    // non-finite stored coefficient would be written straight back — `cad-db`
    // refuses that at commit, so the model names the cause instead of staging it.
    for transform in [
        corrupt_placement(),
        Transform3 {
            matrix: [
                [1.0, 0.0, 0.0, f64::INFINITY],
                [0.0, 1.0, 0.0, 2.0],
                [0.0, 0.0, 1.0, 3.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        },
        Transform3 {
            matrix: [
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, f64::NAN],
                [0.0, 0.0, 0.0, 1.0],
            ],
        },
    ] {
        let geometry = SemanticGeometry::Insert {
            block: BLOCK,
            transform,
        };
        let error = apply_grip(
            &geometry,
            &grip_id(113, GripKind::Insertion, 0),
            p3(1.0, 1.0, 1.0),
        )
        .expect_err("an unstorable placement is refused");
        assert_reason(error, REASON_DEGENERATE_TRANSFORM);
        // Nothing was written: the rule is pure, so the stored placement still has
        // exactly the corrupt coefficients that went in (compared bit by bit, since
        // `NaN != NaN`).
        let (_, stored) = insert_parts(&geometry);
        for (r, row) in transform.matrix.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                assert_eq!(
                    stored.matrix[r][c].to_bits(),
                    cell.to_bits(),
                    "coefficient [{r}][{c}] is untouched"
                );
            }
        }
    }
}

#[test]
fn a_grip_less_kind_still_reports_unsupported_kind() {
    // `INSERT` left the refusal list in this lane, so the "no grip model at all"
    // code is pinned against the kind that replaced it as the shared fixture.
    let error = apply_grip(
        &multi_leader(),
        &grip_id(114, GripKind::Insertion, 0),
        p3(1.0, 1.0, 1.0),
    )
    .expect_err("a MULTILEADER exposes no grip model");
    assert_reason(error, REASON_UNSUPPORTED_KIND);
}

// ---------------------------------------------------------------------------
// 4. Hit-testing and tie-break: unchanged rules, one ordinary candidate
// ---------------------------------------------------------------------------

#[test]
fn a_query_at_the_insertion_point_hits_the_insertion_grip() {
    let entity = ent(115, rotated());
    let hit = grip_hit([entity.clone()].iter(), ROTATED_AT, 0.0).expect("an exact hit");
    assert_eq!(hit.id, grip_id(115, GripKind::Insertion, 0));
    assert_bits_eq(hit.position, ROTATED_AT, "the reported handle");

    // A query elsewhere is a miss; the tolerance boundary behaves like every other
    // handle (exactly at the tolerance hits, beyond it misses).
    let off = |dy: f64| p3(ROTATED_AT.x, ROTATED_AT.y + dy, ROTATED_AT.z);
    assert!(grip_hit([entity.clone()].iter(), off(5.0), 1.0).is_none());
    assert!(grip_hit([entity.clone()].iter(), off(1.0), 1.0).is_some());
    assert!(grip_hit([entity.clone()].iter(), off(1.0 + 1e-9), 1.0).is_none());
    // Non-finite queries and tolerances never hit.
    for (world, tolerance) in [
        (p3(f64::NAN, 0.0, 0.0), 1.0),
        (p3(0.0, f64::INFINITY, 0.0), 1.0),
        (ROTATED_AT, f64::NAN),
        (ROTATED_AT, f64::INFINITY),
        (ROTATED_AT, -1.0),
    ] {
        assert!(
            grip_hit([entity.clone()].iter(), world, tolerance).is_none(),
            "query {world:?} at tolerance {tolerance:?} must not hit"
        );
    }
}

#[test]
fn the_plan_view_rule_hits_an_insert_at_another_elevation() {
    // The reference sits at z = 5; a plan-view pointer resolves onto the work plane
    // (z = 0), so only the plan-view rule can arm its handle while the position that
    // is committed stays the stored 3D point.
    let at = p3(3.0, 4.0, 5.0);
    let entity = ent(116, translated_at(at));
    let plan = p3(3.1, 4.0, 0.0);
    assert!(
        grip_hit([entity.clone()].iter(), plan, 1.0).is_none(),
        "the 3D rule measures the elevation difference"
    );
    let hit = grip_hit_planar([entity].iter(), plan, 1.0).expect("the plan-view rule hits");
    assert_eq!(hit.id, grip_id(116, GripKind::Insertion, 0));
    assert_bits_eq(
        hit.position,
        at,
        "the reported position stays the stored 3D insertion point",
    );
}

#[test]
fn an_exact_tie_at_a_shared_point_goes_to_the_first_candidate() {
    // An INSERT and a POINT entity at the very same point: the tie-break is "the
    // first candidate in list order wins", not "Insertion beats Point" or vice
    // versa.
    let insert = ent(117, translated_at(p3(2.0, -1.0, 0.0)));
    let point = ent(118, SemanticGeometry::Point(p3(2.0, -1.0, 0.0)));
    let query = p3(2.0, -1.0, 0.0);
    let hit = grip_hit([insert.clone(), point.clone()].iter(), query, 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(117, GripKind::Insertion, 0),
        "the first candidate wins"
    );
    let hit = grip_hit([point.clone(), insert.clone()].iter(), query, 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(118, GripKind::Point, 0),
        "input order decides, not the kind"
    );
    let hit = grip_hit_planar([insert, point].iter(), query, 5.0).expect("a plan-view hit");
    assert_eq!(
        hit.id.entity,
        EntityId(117),
        "the plan-view rule breaks the tie the same way"
    );
}

#[test]
fn a_non_finite_insertion_point_is_never_a_hit() {
    // A corrupt placement (a `NaN` translation column) still exposes its one grip,
    // like every other non-finite stored handle, but neither hit rule reports it.
    let entity = ent(
        119,
        SemanticGeometry::Insert {
            block: BLOCK,
            transform: nan_translation_placement(),
        },
    );
    let grips = grips_for_entity(&entity);
    assert_eq!(
        grips.len(),
        1,
        "the handle list is honest about the geometry"
    );
    assert!(
        grips[0].position.x.is_nan(),
        "the position is the stored column"
    );
    assert!(grip_hit([entity.clone()].iter(), p3(0.0, 0.0, 0.0), 1.0).is_none());
    assert!(grip_hit_planar([entity].iter(), p3(0.0, 0.0, 0.0), 1.0).is_none());
}

// ---------------------------------------------------------------------------
// 5. The handle keeps the shared `Grip`/`GripId` shape (no new kind, no new field)
// ---------------------------------------------------------------------------

#[test]
fn the_insert_handle_is_the_same_grip_value_shape_as_every_other_kind() {
    // The renderer marker and the host's plan-view hit consume `Grip`/`GripId`
    // generically, so a block reference must not need a bespoke payload.
    let entity = ent(120, translated_at(PLAIN_AT));
    assert_eq!(
        grips_for_entity(&entity),
        vec![Grip {
            id: GripId {
                entity: EntityId(120),
                kind: GripKind::Insertion,
                index: 0,
            },
            position: PLAIN_AT,
        }]
    );
}

/// A placement with a non-finite coefficient: unstorable, so only a corrupt
/// in-memory entity carries one.
fn corrupt_placement() -> Transform3 {
    Transform3 {
        matrix: [
            [f64::NAN, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 2.0],
            [0.0, 0.0, 1.0, 3.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// A placement whose **translation column** is non-finite: the handle position
/// itself is then non-finite, which is the "corrupt handle" case the hit rules
/// must skip (exactly like a `NaN` line endpoint).
fn nan_translation_placement() -> Transform3 {
    Transform3 {
        matrix: [
            [1.0, 0.0, 0.0, f64::NAN],
            [0.0, 1.0, 0.0, 3.0],
            [0.0, 0.0, 1.0, 4.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}
