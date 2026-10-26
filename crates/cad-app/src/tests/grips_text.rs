//! `TEXT`/`MTEXT` insertion-point grip contract tests (`crate::grips`).
//!
//! TEXT and MTEXT are the **same** `SemanticGeometry::Text` variant — only
//! `DbEntity.object.type_key` tells the DXF classes apart — so both expose the
//! same single handle: [`GripKind::Insertion`] `index` 0 at the **stored** insertion
//! point. These tests pin the three claims that make the handle unambiguous:
//!
//! * it is exactly one grip, at the stored `position` bit for bit, and it is
//!   **not** offset by `h_align`/`v_align` and **not** rotated about by `rotation`
//!   (an attachment or a rotation therefore never moves the handle);
//! * a drag writes `position` only: every other field (including
//!   `wrap_width: Some(w)`) is returned bit-identically;
//! * the refusals keep the module's fixed order (handle identity → target
//!   finiteness → geometric rule) and use the documented codes.
//!
//! The kind is deliberately `Insertion` and not `Point`: `Point` is the `POINT`
//! entity's own handle, and sharing it would make `grip.stale_handle` unable to
//! tell a text from a point entity.

use super::*;

use super::grips::{assert_bits_eq, assert_reason, ent, grip_id, p3};
use crate::grips::{
    apply_grip, grip_hit, grip_hit_planar, grips_for_entity, Grip, GripId, GripKind,
    REASON_NON_FINITE_TARGET, REASON_STALE_HANDLE, REASON_ZERO_DISPLACEMENT,
};

/// A plain `TEXT`: default alignment, no rotation, no wrap boundary.
fn plain(position: Point3) -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "GRIP".into(),
        position,
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Baseline,
        wrap_width: None,
    }
}

/// An MTEXT-ish text: `wrap_width: Some(..)` plus a non-default attachment, a
/// non-zero rotation and a font — the fields a drag must never touch.
const MTEXT_POSITION: Point3 = Point3 {
    x: 1.0 / 3.0,
    y: -2.0 / 7.0,
    z: 0.25,
};
const MTEXT_WRAP: f64 = 12.5;
const MTEXT_ROTATION: f64 = 0.75;

fn mtext_like() -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "line one\\Pline two".into(),
        position: MTEXT_POSITION,
        style: StyleId(4),
        height: 1.125,
        rotation: MTEXT_ROTATION,
        font: Some("arial.ttf".into()),
        h_align: TextAlignH::Center,
        v_align: TextAlignV::Middle,
        wrap_width: Some(MTEXT_WRAP),
    }
}

/// Assert every `Text` field except `position` is bit-identical to `before`.
///
/// The whole claim in one place: the string, style id, height, rotation, font key,
/// both attachment fields and the wrap boundary (including `Some(w)` vs `None`).
fn assert_untouched(actual: &SemanticGeometry, before: &SemanticGeometry) {
    let SemanticGeometry::Text {
        text,
        position: _,
        style,
        height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width,
    } = actual
    else {
        panic!("expected a TEXT, got {actual:?}");
    };
    let SemanticGeometry::Text {
        text: expected_text,
        position: _,
        style: expected_style,
        height: expected_height,
        rotation: expected_rotation,
        font: expected_font,
        h_align: expected_h,
        v_align: expected_v,
        wrap_width: expected_wrap,
    } = before
    else {
        panic!("the reference geometry is not a TEXT: {before:?}");
    };
    assert_eq!(text, expected_text, "the string is bit-identical");
    assert_eq!(*style, *expected_style, "the style id is bit-identical");
    assert_eq!(
        height.to_bits(),
        expected_height.to_bits(),
        "the height is bit-identical"
    );
    assert_eq!(
        rotation.to_bits(),
        expected_rotation.to_bits(),
        "the rotation is bit-identical"
    );
    assert_eq!(font, expected_font, "the font key is bit-identical");
    assert_eq!(*h_align, *expected_h, "h_align is bit-identical");
    assert_eq!(*v_align, *expected_v, "v_align is bit-identical");
    match (wrap_width, expected_wrap) {
        (Some(actual), Some(expected)) => assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "wrap_width is bit-identical when present"
        ),
        (None, None) => {}
        other => panic!("wrap_width changed: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1. The handle list: exactly one insertion grip, at the stored position
// ---------------------------------------------------------------------------

#[test]
fn text_exposes_exactly_one_insertion_grip_at_the_stored_position() {
    let position = p3(1.0 / 3.0, -2.0 / 7.0, 0.125);
    let entity = ent(71, plain(position));
    let grips = grips_for_entity(&entity);
    assert_eq!(grips.len(), 1, "a TEXT exposes exactly one grip");
    assert_eq!(
        grips[0].id,
        grip_id(71, GripKind::Insertion, 0),
        "the single handle is Insertion 0, never Point"
    );
    assert_bits_eq(
        grips[0].position,
        position,
        "the handle is the stored insertion point",
    );
}

#[test]
fn mtext_ish_text_exposes_the_same_single_insertion_grip() {
    // Same variant, so the handle model cannot differ: `wrap_width: Some(..)`, a
    // centre/middle attachment and a non-zero rotation move neither the handle
    // count nor its position.
    let entity = ent(72, mtext_like());
    let grips = grips_for_entity(&entity);
    assert_eq!(grips.len(), 1, "MTEXT exposes exactly one grip too");
    assert_eq!(grips[0].id, grip_id(72, GripKind::Insertion, 0));
    assert_bits_eq(
        grips[0].position,
        MTEXT_POSITION,
        "neither the attachment nor the rotation offsets the handle",
    );

    // The same text with a right/top attachment: still one grip, still exactly at
    // the stored position (no `h_align`/`v_align` offset is added, in either
    // direction).
    let right_top = SemanticGeometry::Text {
        text: "GRIP".into(),
        position: MTEXT_POSITION,
        style: StyleId(4),
        height: 1.125,
        rotation: MTEXT_ROTATION,
        font: Some("arial.ttf".into()),
        h_align: TextAlignH::Right,
        v_align: TextAlignV::Top,
        wrap_width: Some(MTEXT_WRAP),
    };
    let grips = grips_for_entity(&ent(73, right_top));
    assert_eq!(grips.len(), 1);
    assert_bits_eq(
        grips[0].position,
        MTEXT_POSITION,
        "a right/top attachment does not move the insertion handle",
    );
}

// ---------------------------------------------------------------------------
// 2. The drag rule: position only, every other field bit-identical
// ---------------------------------------------------------------------------

#[test]
fn dragging_a_plain_text_moves_only_the_position() {
    let original = plain(p3(1.0, 2.0, 3.0));
    let target = p3(-4.5, 0.25, 1.0 / 3.0);
    let dragged = apply_grip(&original, &grip_id(71, GripKind::Insertion, 0), target)
        .expect("dragging the insertion point is supported");
    match &dragged {
        SemanticGeometry::Text { position, .. } => {
            assert_bits_eq(*position, target, "the insertion point takes the target")
        }
        other => panic!("expected a TEXT, got {other:?}"),
    }
    assert_untouched(&dragged, &original);
}

#[test]
fn dragging_an_mtext_ish_text_keeps_wrap_width_and_every_other_field() {
    let original = mtext_like();
    let target = p3(MTEXT_POSITION.x + 7.5, MTEXT_POSITION.y - 4.25, 9.0);
    let dragged = apply_grip(&original, &grip_id(72, GripKind::Insertion, 0), target)
        .expect("dragging the insertion point of an MTEXT-ish text is supported");
    match &dragged {
        SemanticGeometry::Text { position, .. } => {
            assert_bits_eq(*position, target, "the insertion point takes the target")
        }
        other => panic!("expected a TEXT, got {other:?}"),
    }
    assert_untouched(&dragged, &original);
    // The same claim as one whole-struct comparison: the drag output is the stored
    // geometry with `position` replaced, nothing else.
    let SemanticGeometry::Text {
        text,
        position: _,
        style,
        height,
        rotation,
        font,
        h_align,
        v_align,
        wrap_width,
    } = original
    else {
        panic!("the fixture is a TEXT");
    };
    assert_eq!(
        dragged,
        SemanticGeometry::Text {
            text,
            position: target,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
        },
        "only `position` changed"
    );
}

#[test]
fn a_text_drag_keeps_wrap_width_none_and_some_apart() {
    // `Some(w)` and `None` are different stored values; neither is synthesized
    // from the other, and a drag never wraps a text that had no boundary.
    for wrap in [None, Some(0.5), Some(1e-9), Some(1234.5)] {
        let original = SemanticGeometry::Text {
            text: "W".into(),
            position: p3(0.0, 0.0, 0.0),
            style: StyleId(1),
            height: 1.0,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: wrap,
        };
        let dragged = apply_grip(
            &original,
            &grip_id(74, GripKind::Insertion, 0),
            p3(2.0, 0.0, 0.0),
        )
        .expect("the insertion drag is supported");
        match dragged {
            SemanticGeometry::Text { wrap_width, .. } => match (wrap_width, wrap) {
                (Some(actual), Some(expected)) => assert_eq!(actual.to_bits(), expected.to_bits()),
                (None, None) => {}
                other => panic!("wrap_width changed from {wrap:?} to {other:?}"),
            },
            other => panic!("expected a TEXT, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Refusals, in the documented order
// ---------------------------------------------------------------------------

#[test]
fn a_non_insertion_kind_on_a_text_is_stale() {
    let geometry = mtext_like();
    for (kind, index) in [
        (GripKind::Point, 0u8),
        (GripKind::Endpoint, 0),
        (GripKind::Endpoint, 1),
        (GripKind::Midpoint, 2),
        (GripKind::Center, 0),
        (GripKind::Quadrant, 0),
        (GripKind::Vertex, 0),
        (GripKind::ControlPoint, 0),
    ] {
        let error = apply_grip(&geometry, &grip_id(72, kind, index), p3(1.0, 1.0, 0.0))
            .expect_err("a TEXT exposes an Insertion grip only");
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn an_out_of_range_insertion_index_is_stale() {
    let geometry = plain(p3(1.0, 1.0, 0.0));
    for index in [1u8, 2, u8::MAX] {
        let error = apply_grip(
            &geometry,
            &grip_id(71, GripKind::Insertion, index),
            p3(4.0, 5.0, 0.0),
        )
        .expect_err("a TEXT has exactly one insertion grip, at index 0");
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn a_text_id_is_stale_even_when_the_target_is_the_insertion_point() {
    // Handle identity is validated before the geometric rules, so a wrong kind
    // never reaches the zero-displacement check.
    let geometry = plain(p3(1.0, 1.0, 0.0));
    let error = apply_grip(
        &geometry,
        &grip_id(71, GripKind::Point, 0),
        p3(1.0, 1.0, 0.0),
    )
    .expect_err("a POINT id is stale on a TEXT, whatever the target");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn a_zero_displacement_text_drag_is_refused() {
    // Exact equality (IEEE `==`, no tolerance): the command must skip the
    // transaction, so this is an error, not an empty `Ok`.
    let position = p3(1.0 / 3.0, -2.0 / 7.0, 0.25);
    for geometry in [plain(position), mtext_like()] {
        let error = apply_grip(&geometry, &grip_id(71, GripKind::Insertion, 0), position)
            .expect_err("a drag onto the stored insertion point writes nothing");
        assert_reason(error, REASON_ZERO_DISPLACEMENT);
    }

    // A one-ulp move is a real (tiny) edit, so it is *not* a zero displacement.
    let nudged = Point3 {
        x: f64::from_bits(position.x.to_bits() + 1),
        y: position.y,
        z: position.z,
    };
    assert!(
        apply_grip(
            &plain(position),
            &grip_id(71, GripKind::Insertion, 0),
            nudged
        )
        .is_ok(),
        "a sub-tolerance drag is a real edit, not a zero displacement"
    );
}

#[test]
fn a_non_finite_text_target_is_refused() {
    let geometry = mtext_like();
    for target in [
        p3(f64::NAN, 0.0, 0.0),
        p3(0.0, f64::INFINITY, 0.0),
        p3(0.0, 0.0, f64::NEG_INFINITY),
    ] {
        let error = apply_grip(&geometry, &grip_id(72, GripKind::Insertion, 0), target)
            .expect_err("a non-finite target is refused instead of stored");
        assert_reason(error, REASON_NON_FINITE_TARGET);
    }
    // Finiteness is checked before the zero-displacement rule, so a NaN target
    // that is nobody's handle is named as non-finite (the documented order).
    let error = apply_grip(
        &plain(p3(1.0, 1.0, 0.0)),
        &grip_id(71, GripKind::Insertion, 0),
        p3(f64::NAN, 1.0, 0.0),
    )
    .expect_err("a NaN target is non-finite");
    assert_reason(error, REASON_NON_FINITE_TARGET);
}

// ---------------------------------------------------------------------------
// 4. Hit-testing: the insertion point is the hit, tie-break unchanged
// ---------------------------------------------------------------------------

#[test]
fn a_query_at_the_insertion_point_hits_the_insertion_grip() {
    let entity = ent(75, plain(p3(1.0 / 3.0, -2.0 / 7.0, 0.0)));
    let position = p3(1.0 / 3.0, -2.0 / 7.0, 0.0);
    assert!(
        grip_hit([entity.clone()].iter(), MTEXT_POSITION, 0.0).is_none(),
        "a query at another point (and a zero tolerance) is a miss"
    );

    let hit =
        grip_hit([entity.clone()].iter(), position, 0.0).expect("an exactly coincident query hits");
    assert_eq!(hit.id, grip_id(75, GripKind::Insertion, 0));
    assert_bits_eq(hit.position, position, "the handle");

    // Outside the tolerance is a miss; exactly at it is a hit.
    let entity = ent(75, plain(p3(0.0, 0.0, 0.0)));
    assert!(grip_hit([entity.clone()].iter(), p3(0.0, 1.5, 0.0), 1.0).is_none());
    assert!(grip_hit([entity.clone()].iter(), p3(0.0, 1.0, 0.0), 1.0).is_some());
    // Non-finite queries/tolerances never hit.
    assert!(grip_hit([entity.clone()].iter(), p3(f64::NAN, 0.0, 0.0), 1.0).is_none());
    assert!(grip_hit([entity].iter(), p3(0.0, 0.0, 0.0), f64::NAN).is_none());
}

#[test]
fn the_plan_view_rule_hits_a_text_insertion_point_at_another_elevation() {
    // A text stored at z = 5 aimed at from the plan view: the 3D rule measures the
    // elevation difference, the plan-view rule deliberately ignores `z`.
    let entity = ent(76, plain(p3(3.0, 4.0, 5.0)));
    let plan = p3(3.1, 4.0, 0.0);
    assert!(
        grip_hit([entity.clone()].iter(), plan, 1.0).is_none(),
        "the 3D rule misses the elevated handle"
    );
    let hit = grip_hit_planar([entity].iter(), plan, 1.0).expect("the plan-view rule hits");
    assert_eq!(hit.id, grip_id(76, GripKind::Insertion, 0));
    assert_bits_eq(
        hit.position,
        p3(3.0, 4.0, 5.0),
        "the reported position stays the stored 3D insertion point",
    );
}

#[test]
fn an_exact_tie_at_a_shared_point_goes_to_the_first_candidate() {
    // A TEXT and a POINT entity at the very same point: the tie-break is "the
    // first candidate in list order wins", not "Point beats Insertion" or vice
    // versa. This also pins that the query at the insertion point is the
    // insertion grip's hit whenever the text comes first.
    let text = ent(77, plain(p3(2.0, -1.0, 0.0)));
    let point = ent(78, SemanticGeometry::Point(p3(2.0, -1.0, 0.0)));
    let hit = grip_hit(
        [text.clone(), point.clone()].iter(),
        p3(2.0, -1.0, 0.0),
        5.0,
    )
    .expect("a hit");
    assert_eq!(
        hit.id,
        GripId {
            entity: EntityId(77),
            kind: GripKind::Insertion,
            index: 0,
        },
        "the text's insertion grip wins the tie when it is first"
    );
    let hit = grip_hit(
        [point.clone(), text.clone()].iter(),
        p3(2.0, -1.0, 0.0),
        5.0,
    )
    .expect("a hit");
    assert_eq!(
        hit.id,
        GripId {
            entity: EntityId(78),
            kind: GripKind::Point,
            index: 0,
        },
        "input order decides, not the kind"
    );

    // The plan-view rule breaks the same tie the same way.
    let hit = grip_hit_planar([text, point].iter(), p3(2.0, -1.0, 0.0), 5.0).expect("a hit");
    assert_eq!(
        hit.id.entity,
        EntityId(77),
        "first candidate wins in plan view"
    );
}

#[test]
fn a_grip_less_entity_still_exposes_nothing_while_the_text_is_hittable() {
    // An INSERT has no handle (its instance transform is not an entity handle), so
    // it is skipped even when the query sits on its translation, while the text's
    // insertion grip is the nearest real candidate.
    let insert = ent(
        79,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p3(0.0, 0.0, 0.0)),
        },
    );
    let text = ent(80, plain(p3(0.25, 0.0, 0.0)));
    assert!(grips_for_entity(&insert).is_empty());
    assert_eq!(
        grips_for_entity(&text),
        vec![Grip {
            id: grip_id(80, GripKind::Insertion, 0),
            position: p3(0.25, 0.0, 0.0),
        }]
    );
    let hit = grip_hit([insert, text].iter(), p3(0.0, 0.0, 0.0), 1.0).expect("the text handle");
    assert_eq!(hit.id, grip_id(80, GripKind::Insertion, 0));
}
