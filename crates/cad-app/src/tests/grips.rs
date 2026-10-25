//! Grip model contract tests (`crate::grips`).
//!
//! These pin the whole implemented surface: which handles exist, in which order,
//! how a handle is hit-tested (including the tie-break), and what a single-handle
//! drag writes or refuses. There is no command, transaction or renderer involved
//! — the model is pure, so the assertions are exact `f64` bit comparisons instead
//! of tolerances wherever "nothing else moved" is the claim.

use super::*;

use crate::grips::{
    apply_grip, grip_hit, grip_hit_planar, grips_for_entity, Grip, GripId, GripKind,
    REASON_NON_FINITE_TARGET, REASON_STALE_HANDLE, REASON_UNSUPPORTED_KIND,
    REASON_ZERO_DISPLACEMENT,
};
use cad_db::{DbEntity, DbObject};

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A `DbEntity` carrying `geometry` (the grip model reads only `id` and
/// `geometry`; the rest is the ordinary scaffolding every stored entity has).
fn ent(id: u128, geometry: SemanticGeometry) -> DbEntity {
    DbEntity {
        object: DbObject {
            id: ObjectId(id),
            type_key: "AcDbEntity".into(),
            revision: Revision(0),
            source_handle: None,
        },
        id: EntityId(id),
        layer: LayerId(0),
        space: SpaceId::Model,
        geometry,
        draw_order: 0,
    }
}

fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> SemanticGeometry {
    SemanticGeometry::Line {
        start: p3(x1, y1, 0.0),
        end: p3(x2, y2, 0.0),
    }
}

fn grip_id(entity: u128, kind: GripKind, index: u8) -> GripId {
    GripId {
        entity: EntityId(entity),
        kind,
        index,
    }
}

/// The message of a refusal, insisting on one of the two variants the model
/// documents (a `NotImplemented`/`StaleResult` would mean a silent placeholder
/// slipped in).
fn reason(error: CadError) -> String {
    match error {
        CadError::Unsupported(message) | CadError::InvalidInput(message) => message,
        other => panic!("unexpected refusal variant: {other:?}"),
    }
}

fn assert_reason(error: CadError, code: &str) {
    let message = reason(error);
    assert!(
        message.starts_with(code),
        "expected reason {code:?}, got {message:?}"
    );
}

fn assert_bits_eq(actual: Point3, expected: Point3, what: &str) {
    assert_eq!(
        (actual.x.to_bits(), actual.y.to_bits(), actual.z.to_bits()),
        (
            expected.x.to_bits(),
            expected.y.to_bits(),
            expected.z.to_bits()
        ),
        "{what} must stay bit-identical: {actual:?} vs {expected:?}"
    );
}

// ---------------------------------------------------------------------------
// 1. Which handles exist, and in which order
// ---------------------------------------------------------------------------

#[test]
fn line_exposes_two_endpoint_grips_start_before_end() {
    let entity = ent(7, line(1.0, 2.0, 30.5, -4.25));
    let grips = grips_for_entity(&entity);
    assert_eq!(grips.len(), 2, "a LINE has exactly two endpoint grips");
    assert_eq!(
        grips[0],
        Grip {
            id: grip_id(7, GripKind::Endpoint, 0),
            position: p3(1.0, 2.0, 0.0),
        },
        "index 0 is `start`"
    );
    assert_eq!(
        grips[1],
        Grip {
            id: grip_id(7, GripKind::Endpoint, 1),
            position: p3(30.5, -4.25, 0.0),
        },
        "index 1 is `end`"
    );
}

#[test]
fn point_exposes_one_point_grip() {
    let entity = ent(9, SemanticGeometry::Point(p3(-3.0, 0.5, 8.0)));
    let grips = grips_for_entity(&entity);
    assert_eq!(grips.len(), 1, "a POINT has exactly one grip");
    assert_eq!(
        grips[0],
        Grip {
            id: grip_id(9, GripKind::Point, 0),
            position: p3(-3.0, 0.5, 8.0),
        }
    );
}

// ---------------------------------------------------------------------------
// 2. The refusal list: one assertion per kind, so a partial implementation of
//    any of them has to come back and update *that* test on purpose.
// ---------------------------------------------------------------------------

#[test]
fn circle_exposes_no_grips() {
    let entity = ent(
        1,
        SemanticGeometry::Circle {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
    );
    assert!(grips_for_entity(&entity).is_empty());
}

#[test]
fn arc_exposes_no_grips() {
    let entity = ent(
        2,
        SemanticGeometry::Arc {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
            start: 0.0,
            sweep: 1.0,
        },
    );
    assert!(grips_for_entity(&entity).is_empty());
}

#[test]
fn text_exposes_no_grips() {
    let entity = ent(
        3,
        SemanticGeometry::Text {
            text: "GRIP".into(),
            position: p3(1.0, 1.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
        },
    );
    assert!(grips_for_entity(&entity).is_empty());
}

#[test]
fn lwpolyline_exposes_no_grips() {
    // A two-vertex polyline is *not* a LINE: its vertices carry bulges and this
    // model has no rule for dragging one, so it exposes nothing.
    let entity = ent(
        4,
        SemanticGeometry::Polyline {
            points: vec![p3(0.0, 0.0, 0.0), p3(10.0, 0.0, 0.0)],
            bulges: vec![0.0, 0.0],
            closed: false,
        },
    );
    assert!(grips_for_entity(&entity).is_empty());
}

#[test]
fn insert_exposes_no_grips() {
    let entity = ent(
        5,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
        },
    );
    assert!(grips_for_entity(&entity).is_empty());
}

// ---------------------------------------------------------------------------
// 3. Hit-testing: nearest within tolerance, nothing outside, skip empty
//    entities, deterministic tie-break.
// ---------------------------------------------------------------------------

#[test]
fn grip_hit_picks_the_nearest_handle_within_tolerance() {
    let entity = ent(7, line(0.0, 0.0, 10.0, 0.0));
    // Nearer to `end` (10, 0) than to `start` (0, 0).
    let hit = grip_hit([entity.clone()].iter(), p3(10.2, 0.3, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(7, GripKind::Endpoint, 1));

    // Nearer to `start`.
    let hit = grip_hit([entity].iter(), p3(0.4, -0.4, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(7, GripKind::Endpoint, 0));
}

#[test]
fn grip_hit_returns_none_outside_tolerance_and_for_degenerate_inputs() {
    let entity = ent(7, line(0.0, 0.0, 10.0, 0.0));
    assert!(
        grip_hit([entity.clone()].iter(), p3(5.0, 5.0, 0.0), 1.0).is_none(),
        "no handle within the tolerance"
    );
    // Exactly at the tolerance is a hit; just beyond it is not.
    assert!(grip_hit([entity.clone()].iter(), p3(0.0, 1.0, 0.0), 1.0).is_some());
    assert!(grip_hit([entity.clone()].iter(), p3(0.0, 1.0 + 1e-9, 0.0), 1.0).is_none());
    // Zero tolerance only hits an exactly coincident handle.
    assert!(grip_hit([entity.clone()].iter(), p3(0.0, 0.0, 0.0), 0.0).is_some());
    // Negative / non-finite tolerance and non-finite query points never hit.
    for tolerance in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(
            grip_hit([entity.clone()].iter(), p3(0.0, 0.0, 0.0), tolerance).is_none(),
            "tolerance {tolerance:?} must not hit"
        );
    }
    for world in [p3(f64::NAN, 0.0, 0.0), p3(0.0, f64::INFINITY, 0.0)] {
        assert!(grip_hit([entity.clone()].iter(), world, 1.0).is_none());
    }
}

#[test]
fn grip_hit_skips_entities_that_expose_no_grips() {
    let circle = ent(
        1,
        SemanticGeometry::Circle {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
    );
    let text = ent(
        2,
        SemanticGeometry::Text {
            text: "no grip".into(),
            position: p3(0.0, 0.0, 0.0),
            style: StyleId(0),
            height: 1.0,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Left,
            v_align: TextAlignV::Baseline,
            wrap_width: None,
        },
    );
    // A query sitting exactly on the circle's centre / the text's insertion
    // point still misses: those are not grips.
    assert!(grip_hit(
        [circle.clone(), text.clone()].iter(),
        p3(0.0, 0.0, 0.0),
        1.0
    )
    .is_none());

    // A real grip elsewhere still wins while the grip-less entities are skipped.
    let line = ent(3, line(20.0, 0.0, 30.0, 0.0));
    let hit = grip_hit([circle, text, line].iter(), p3(20.1, 0.0, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(3, GripKind::Endpoint, 0));
}

#[test]
fn grip_hit_tie_goes_to_the_first_candidate_in_input_order() {
    // Two POINT entities at the same distance from the query point: the earlier
    // entity of the input wins (documented tie-break).
    let left = ent(11, SemanticGeometry::Point(p3(-2.0, 0.0, 0.0)));
    let right = ent(12, SemanticGeometry::Point(p3(2.0, 0.0, 0.0)));
    let hit =
        grip_hit([left.clone(), right.clone()].iter(), p3(0.0, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(hit.id, grip_id(11, GripKind::Point, 0), "first entity wins");
    let hit = grip_hit([right, left].iter(), p3(0.0, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(12, GripKind::Point, 0),
        "input order decides, not entity id"
    );

    // Within one LINE, the midpoint query is equidistant from both endpoints:
    // the lower index (start) wins.
    let line = ent(13, line(0.0, 0.0, 2.0, 0.0));
    let hit = grip_hit([line].iter(), p3(1.0, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(13, GripKind::Endpoint, 0),
        "start wins the tie"
    );
}

// ---------------------------------------------------------------------------
// 3b. Plan-view hit-testing (`grip_hit_planar`): same rules, x/y distance only.
// ---------------------------------------------------------------------------

#[test]
fn grip_hit_planar_picks_the_nearest_handle_within_tolerance() {
    let entity = ent(7, line(0.0, 0.0, 10.0, 0.0));
    let hit = grip_hit_planar([entity.clone()].iter(), p3(10.2, 0.3, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(7, GripKind::Endpoint, 1));
    assert_bits_eq(hit.position, p3(10.0, 0.0, 0.0), "the stored handle");

    let hit = grip_hit_planar([entity].iter(), p3(0.4, -0.4, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(7, GripKind::Endpoint, 0));
}

#[test]
fn grip_hit_planar_ignores_z_where_the_3d_rule_misses() {
    // A LINE drawn at z = 5, aimed at from the plan view: the pointer resolves
    // onto the work plane (z = 0), so the 3D rule misses the handle it is sitting
    // on while the plan-view rule hits it.
    let elevated = ent(
        21,
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, 5.0),
            end: p3(10.0, 0.0, 5.0),
        },
    );
    let plan = p3(10.1, 0.0, 0.0);
    assert!(
        grip_hit([elevated.clone()].iter(), plan, 1.0).is_none(),
        "the 3D rule measures the elevation difference"
    );
    let hit = grip_hit_planar([elevated].iter(), plan, 1.0).expect("the plan-view rule hits");
    assert_eq!(hit.id, grip_id(21, GripKind::Endpoint, 1));
    assert_bits_eq(
        hit.position,
        p3(10.0, 0.0, 5.0),
        "the position stays the stored 3D handle, never a projected one",
    );
}

#[test]
fn grip_hit_planar_returns_none_outside_tolerance_and_for_degenerate_inputs() {
    let entity = ent(7, line(0.0, 0.0, 10.0, 0.0));
    assert!(grip_hit_planar([entity.clone()].iter(), p3(5.0, 5.0, 0.0), 1.0).is_none());
    // Exactly at the tolerance is a hit; just beyond it is not.
    assert!(grip_hit_planar([entity.clone()].iter(), p3(0.0, 1.0, 0.0), 1.0).is_some());
    assert!(grip_hit_planar([entity.clone()].iter(), p3(0.0, 1.0 + 1e-9, 0.0), 1.0).is_none());
    // Zero tolerance only hits an exactly coincident handle in x/y (z is ignored).
    assert!(grip_hit_planar([entity.clone()].iter(), p3(0.0, 0.0, 42.0), 0.0).is_some());
    // Negative / non-finite tolerance and non-finite query points never hit.
    for tolerance in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(
            grip_hit_planar([entity.clone()].iter(), p3(0.0, 0.0, 0.0), tolerance).is_none(),
            "tolerance {tolerance:?} must not hit"
        );
    }
    for world in [p3(f64::NAN, 0.0, 0.0), p3(0.0, f64::INFINITY, 0.0)] {
        assert!(grip_hit_planar([entity.clone()].iter(), world, 1.0).is_none());
    }
    // A stored handle with a non-finite coordinate never hits, on any axis: the
    // ignored z must not turn a corrupt position into a hit.
    let corrupt = ent(
        22,
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, f64::NAN),
            end: p3(10.0, 0.0, 0.0),
        },
    );
    assert!(
        grip_hit_planar([corrupt].iter(), p3(0.0, 0.0, 0.0), 1.0).is_none(),
        "a NaN stored z is still a corrupt handle, never a hit"
    );
}

#[test]
fn grip_hit_planar_skips_entities_that_expose_no_grips() {
    let circle = ent(
        1,
        SemanticGeometry::Circle {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
    );
    assert!(grip_hit_planar([circle.clone()].iter(), p3(0.0, 0.0, 0.0), 1.0).is_none());
    let line = ent(3, line(20.0, 0.0, 30.0, 0.0));
    let hit = grip_hit_planar([circle, line].iter(), p3(20.1, 0.0, 0.0), 1.0).expect("a hit");
    assert_eq!(hit.id, grip_id(3, GripKind::Endpoint, 0));
}

#[test]
fn grip_hit_planar_tie_goes_to_the_first_candidate_in_input_order() {
    let left = ent(11, SemanticGeometry::Point(p3(-2.0, 0.0, 0.0)));
    let right = ent(12, SemanticGeometry::Point(p3(2.0, 0.0, 0.0)));
    let hit = grip_hit_planar([left.clone(), right.clone()].iter(), p3(0.0, 0.0, 0.0), 5.0)
        .expect("a hit");
    assert_eq!(hit.id, grip_id(11, GripKind::Point, 0), "first entity wins");
    let hit = grip_hit_planar([right, left].iter(), p3(0.0, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(12, GripKind::Point, 0),
        "input order decides, not entity id"
    );

    // Within one LINE the midpoint query is equidistant from both endpoints:
    // the lower index (start) wins, exactly like the 3D rule.
    let line = ent(13, line(0.0, 0.0, 2.0, 0.0));
    let hit = grip_hit_planar([line].iter(), p3(1.0, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(13, GripKind::Endpoint, 0),
        "start wins the tie"
    );
}

// ---------------------------------------------------------------------------
// 4. Applying a drag
// ---------------------------------------------------------------------------

#[test]
fn apply_grip_moves_only_the_dragged_line_endpoint() {
    let original = line(1.0, 2.0, 10.0, 20.0);
    let (start, end) = (p3(1.0, 2.0, 0.0), p3(10.0, 20.0, 0.0));

    let dragged = apply_grip(
        &original,
        &grip_id(7, GripKind::Endpoint, 0),
        p3(-5.5, 7.25, 3.0),
    )
    .expect("dragging the start endpoint is supported");
    match dragged {
        SemanticGeometry::Line {
            start: new_start,
            end: new_end,
        } => {
            assert_bits_eq(new_start, p3(-5.5, 7.25, 3.0), "the dragged endpoint");
            assert_bits_eq(new_end, end, "the other endpoint");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }

    let dragged = apply_grip(
        &original,
        &grip_id(7, GripKind::Endpoint, 1),
        p3(40.0, -1.0, 2.5),
    )
    .expect("dragging the end endpoint is supported");
    match dragged {
        SemanticGeometry::Line {
            start: new_start,
            end: new_end,
        } => {
            assert_bits_eq(new_start, start, "the other endpoint");
            assert_bits_eq(new_end, p3(40.0, -1.0, 2.5), "the dragged endpoint");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
}

#[test]
fn apply_grip_moves_the_point_position() {
    let original = SemanticGeometry::Point(p3(1.0, 2.0, 3.0));
    let dragged = apply_grip(
        &original,
        &grip_id(9, GripKind::Point, 0),
        p3(4.0, 5.0, 6.0),
    )
    .expect("dragging a POINT is supported");
    match dragged {
        SemanticGeometry::Point(position) => {
            assert_bits_eq(position, p3(4.0, 5.0, 6.0), "position")
        }
        other => panic!("expected a POINT, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 5. Refusals, each with its documented reason
// ---------------------------------------------------------------------------

#[test]
fn apply_grip_refuses_a_point_grip_on_a_line() {
    // The id kind does not match the geometry's grip model: stale, not a
    // silent reinterpretation as `start`.
    let error = apply_grip(
        &line(0.0, 0.0, 10.0, 0.0),
        &grip_id(7, GripKind::Point, 0),
        p3(1.0, 1.0, 0.0),
    )
    .expect_err("a LINE has no Point grip");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn apply_grip_refuses_an_endpoint_grip_on_a_point_entity() {
    let error = apply_grip(
        &SemanticGeometry::Point(p3(1.0, 2.0, 0.0)),
        &grip_id(9, GripKind::Endpoint, 0),
        p3(4.0, 5.0, 0.0),
    )
    .expect_err("a POINT has no Endpoint grip");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn apply_grip_refuses_an_out_of_range_index() {
    // LINE: only 0 and 1 exist.
    for index in [2u8, 7, u8::MAX] {
        let error = apply_grip(
            &line(0.0, 0.0, 10.0, 0.0),
            &grip_id(7, GripKind::Endpoint, index),
            p3(1.0, 1.0, 0.0),
        )
        .expect_err("a LINE endpoint index above 1 does not exist");
        assert_reason(error, REASON_STALE_HANDLE);
    }
    // POINT: only 0 exists.
    let error = apply_grip(
        &SemanticGeometry::Point(p3(1.0, 2.0, 0.0)),
        &grip_id(9, GripKind::Point, 1),
        p3(4.0, 5.0, 0.0),
    )
    .expect_err("a POINT index above 0 does not exist");
    assert_reason(error, REASON_STALE_HANDLE);
}

#[test]
fn apply_grip_refuses_zero_displacement() {
    // The target equals the current handle position exactly → nothing to write,
    // so the future command must skip the transaction (no empty undo entry).
    let error = apply_grip(
        &line(0.0, 0.0, 10.0, 0.0),
        &grip_id(7, GripKind::Endpoint, 0),
        p3(0.0, 0.0, 0.0),
    )
    .expect_err("a zero displacement is refused, not an Ok no-op");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);

    let error = apply_grip(
        &SemanticGeometry::Point(p3(1.0, 2.0, 3.0)),
        &grip_id(9, GripKind::Point, 0),
        p3(1.0, 2.0, 3.0),
    )
    .expect_err("a zero displacement is refused for a POINT too");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);
}

#[test]
fn apply_grip_refuses_a_kind_with_no_grip_model() {
    let error = apply_grip(
        &SemanticGeometry::Circle {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
        &grip_id(1, GripKind::Endpoint, 0),
        p3(6.0, 0.0, 0.0),
    )
    .expect_err("a CIRCLE exposes no grip model");
    assert_reason(error, REASON_UNSUPPORTED_KIND);
    assert!(
        reason(
            apply_grip(
                &SemanticGeometry::Circle {
                    center: p3(0.0, 0.0, 0.0),
                    normal: p3(0.0, 0.0, 1.0),
                    radius: 5.0,
                },
                &grip_id(1, GripKind::Endpoint, 0),
                p3(6.0, 0.0, 0.0),
            )
            .expect_err("still refused")
        )
        .contains("AcDbCircle"),
        "the refusal names the DXF class"
    );
}

#[test]
fn apply_grip_refuses_a_non_finite_target() {
    for target in [
        p3(f64::NAN, 0.0, 0.0),
        p3(0.0, f64::INFINITY, 0.0),
        p3(0.0, 0.0, f64::NEG_INFINITY),
    ] {
        let error = apply_grip(
            &line(0.0, 0.0, 10.0, 0.0),
            &grip_id(7, GripKind::Endpoint, 1),
            target,
        )
        .expect_err("a non-finite target is refused instead of stored");
        assert_reason(error, REASON_NON_FINITE_TARGET);
    }
}

// ---------------------------------------------------------------------------
// 6. Reported positions are the geometry's own values (no rounding drift)
// ---------------------------------------------------------------------------

#[test]
fn grip_positions_match_the_geometry_without_rounding_drift() {
    // Values that no decimal round-trip would preserve.
    let start = p3(1.0 / 3.0, -2.0 / 7.0, 0.1);
    let end = p3(1e-9, 123_456.789_012_345, -0.0);
    let entity = ent(21, SemanticGeometry::Line { start, end });
    let grips = grips_for_entity(&entity);
    assert_bits_eq(grips[0].position, start, "grip 0 position");
    assert_bits_eq(grips[1].position, end, "grip 1 position");

    // A drag of one endpoint reports the untouched handle at exactly the other
    // grip's stored position.
    let dragged = apply_grip(&entity.geometry, &grips[1].id, p3(7.5, 8.5, 9.5)).expect("drag");
    match dragged {
        SemanticGeometry::Line {
            start: new_start, ..
        } => assert_bits_eq(new_start, grips[0].position, "untouched endpoint"),
        other => panic!("expected a LINE, got {other:?}"),
    }

    // The same for a POINT whose coordinates survive no round-trip.
    let position = p3(0.300_000_000_000_000_04, -1.0 / 9.0, 2.0 / 3.0);
    let point = ent(22, SemanticGeometry::Point(position));
    let point_grips = grips_for_entity(&point);
    assert_bits_eq(point_grips[0].position, position, "the POINT grip position");
}
