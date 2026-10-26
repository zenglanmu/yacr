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

pub(super) fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

/// A `DbEntity` carrying `geometry` (the grip model reads only `id` and
/// `geometry`; the rest is the ordinary scaffolding every stored entity has).
pub(super) fn ent(id: u128, geometry: SemanticGeometry) -> DbEntity {
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

/// A grip-less geometry, used wherever a test needs "an entity that exposes no
/// handle at all".
///
/// `SPLINE` used to play this role; it now has control-point grips (pinned in
/// `crates/cad-app/src/tests/grips_spline.rs`), so the fixture moved to an
/// `INSERT`, whose instance transform genuinely is not an entity handle.
fn insert() -> SemanticGeometry {
    SemanticGeometry::Insert {
        block: BlockId(0),
        transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
    }
}

pub(super) fn grip_id(entity: u128, kind: GripKind, index: u8) -> GripId {
    GripId {
        entity: EntityId(entity),
        kind,
        index,
    }
}

/// The message of a refusal, insisting on one of the two variants the model
/// documents (a `NotImplemented`/`StaleResult` would mean a silent placeholder
/// slipped in).
pub(super) fn reason(error: CadError) -> String {
    match error {
        CadError::Unsupported(message) | CadError::InvalidInput(message) => message,
        other => panic!("unexpected refusal variant: {other:?}"),
    }
}

pub(super) fn assert_reason(error: CadError, code: &str) {
    let message = reason(error);
    assert!(
        message.starts_with(code),
        "expected reason {code:?}, got {message:?}"
    );
}

pub(super) fn assert_bits_eq(actual: Point3, expected: Point3, what: &str) {
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

/// The documented exact midpoint of two stored points, computed independently of
/// the model (`(start + end) * 0.5`).
fn midpoint_of(start: Point3, end: Point3) -> Point3 {
    Point3 {
        x: (start.x + end.x) * 0.5,
        y: (start.y + end.y) * 0.5,
        z: (start.z + end.z) * 0.5,
    }
}

fn sub3(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn len3(v: Point3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

// ---------------------------------------------------------------------------
// 1. Which handles exist, and in which order
// ---------------------------------------------------------------------------

#[test]
fn line_exposes_two_endpoint_grips_then_the_midpoint() {
    let entity = ent(7, line(1.0, 2.0, 30.5, -4.25));
    let grips = grips_for_entity(&entity);
    assert_eq!(
        grips.len(),
        3,
        "a LINE has two endpoint grips and one midpoint grip"
    );
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
    assert_eq!(
        grips[2],
        Grip {
            id: grip_id(7, GripKind::Midpoint, 2),
            position: p3(15.75, -1.125, 0.0),
        },
        "index 2 is the exact f64 midpoint (start + end) * 0.5"
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
//    any of them has to come back and update *that* test on purpose. `CIRCLE`
//    and `ARC` are no longer on this list (they have their own handles, pinned
//    in `crates/cad-app/src/tests/grips_curves.rs`), neither is `POLYLINE`
//    (vertex grips, pinned in `crates/cad-app/src/tests/grips_polyline.rs`),
//    neither is `ELLIPSE` (centre + axis grips, pinned in
//    `crates/cad-app/src/tests/grips_ellipse.rs`), and neither is `SPLINE`
//    (control-point grips, pinned in `crates/cad-app/src/tests/grips_spline.rs`).
// ---------------------------------------------------------------------------

#[test]
fn ellipse_exposes_a_centre_and_axis_grips() {
    // A full ellipse is no longer on the refusal list: it has its own handles
    // (pinned in `tests::grips_ellipse`). This assertion keeps the shape of the
    // list honest — the kind has a grip model at all.
    let entity = ent(
        1,
        SemanticGeometry::Ellipse {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            major_axis: p3(5.0, 0.0, 0.0),
            ratio: 0.5,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
    );
    assert_eq!(grips_for_entity(&entity).len(), 5);
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
fn insert_exposes_no_grips() {
    let entity = ent(5, insert());
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
    let insert = ent(1, insert());
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
    // A query sitting exactly on the text's insertion point still misses: that is
    // not a grip (`INSERT` exposes no handle at all either).
    assert!(grip_hit(
        [insert.clone(), text.clone()].iter(),
        p3(0.0, 0.0, 0.0),
        1.0
    )
    .is_none());

    // A real grip elsewhere still wins while the grip-less entities are skipped.
    let line = ent(3, line(20.0, 0.0, 30.0, 0.0));
    let hit = grip_hit([insert, text, line].iter(), p3(20.1, 0.0, 0.0), 1.0).expect("a hit");
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

    // Within one LINE the candidates are `start` (0), `end` (1) and the midpoint
    // (2). At a quarter of the line the query is exactly equidistant from
    // `start` and the midpoint: the earlier grip of the list (index 0) wins.
    let line = ent(13, line(0.0, 0.0, 2.0, 0.0));
    let hit = grip_hit([line.clone()].iter(), p3(0.5, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(13, GripKind::Endpoint, 0),
        "start wins the exact tie with the midpoint (lower index)"
    );
    // At the exact midpoint the midpoint grip is strictly closer than both
    // endpoints, so it is a midpoint hit, not a tie.
    let hit = grip_hit([line].iter(), p3(1.0, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(hit.id, grip_id(13, GripKind::Midpoint, 2));
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
    let insert = ent(1, insert());
    assert!(grip_hit_planar([insert.clone()].iter(), p3(0.0, 0.0, 0.0), 1.0).is_none());
    let line = ent(3, line(20.0, 0.0, 30.0, 0.0));
    let hit = grip_hit_planar([insert, line].iter(), p3(20.1, 0.0, 0.0), 1.0).expect("a hit");
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

    // Within one LINE the candidates are `start` (0), `end` (1) and the midpoint
    // (2). At a quarter of the line the query is exactly equidistant from
    // `start` and the midpoint: the earlier grip of the list (index 0) wins,
    // exactly like the 3D rule.
    let line = ent(13, line(0.0, 0.0, 2.0, 0.0));
    let hit = grip_hit_planar([line.clone()].iter(), p3(0.5, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(13, GripKind::Endpoint, 0),
        "start wins the exact tie with the midpoint (lower index)"
    );
    // At the exact midpoint the midpoint grip is strictly closer.
    let hit = grip_hit_planar([line].iter(), p3(1.0, 0.0, 0.0), 5.0).expect("a hit");
    assert_eq!(hit.id, grip_id(13, GripKind::Midpoint, 2));
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
fn apply_grip_translates_the_whole_line_from_its_midpoint() {
    // Exact binary literals, so "moved by exactly the delta", "same length" and
    // "still parallel" are exact comparisons rather than tolerances.
    let original = line(0.0, 0.0, 10.0, 0.0);
    let dragged = apply_grip(
        &original,
        &grip_id(7, GripKind::Midpoint, 2),
        p3(6.0, 3.0, 0.0),
    )
    .expect("dragging the midpoint translates the line");
    match dragged {
        SemanticGeometry::Line { start, end } => {
            // The midpoint was (5, 0, 0), so delta = (1, 3, 0) for BOTH ends.
            assert_bits_eq(start, p3(1.0, 3.0, 0.0), "start moves by the delta");
            assert_bits_eq(end, p3(11.0, 3.0, 0.0), "end moves by the same delta");
            assert_bits_eq(
                midpoint_of(start, end),
                p3(6.0, 3.0, 0.0),
                "the new midpoint is exactly the drag target",
            );
            assert_eq!(len3(sub3(end, start)), 10.0, "length unchanged");
            assert_eq!(end.y - start.y, 0.0, "still parallel to the x axis");
            assert_eq!(end.z - start.z, 0.0, "still planar in z");
        }
        other => panic!("expected a LINE, got {other:?}"),
    }
}

#[test]
fn a_midpoint_drag_keeps_a_diagonal_line_parallel_and_the_same_length() {
    // Values that survive no decimal round-trip, dragged by a delta that is not
    // axis-aligned: the direction vector and the length must be preserved.
    let start = p3(1.0 / 3.0, -2.0 / 7.0, 0.25);
    let end = p3(11.0 / 7.0, 5.0 / 3.0, -1.5);
    let original = SemanticGeometry::Line { start, end };
    let from = midpoint_of(start, end);
    let to = p3(from.x + 7.5, from.y - 4.25, from.z + 2.0);

    let dragged = apply_grip(&original, &grip_id(7, GripKind::Midpoint, 2), to)
        .expect("a midpoint drag translates the line");
    match dragged {
        SemanticGeometry::Line {
            start: new_start,
            end: new_end,
        } => {
            let close = |actual: Point3, expected: Point3, what: &str| {
                for (axis, (a, e)) in [
                    ("x", (actual.x, expected.x)),
                    ("y", (actual.y, expected.y)),
                    ("z", (actual.z, expected.z)),
                ] {
                    assert!(
                        (a - e).abs() < 1e-12,
                        "{what}: {axis} is {a} but expected {e}"
                    );
                }
            };
            close(
                midpoint_of(new_start, new_end),
                to,
                "the translated midpoint follows the target",
            );
            let before = sub3(end, start);
            let after = sub3(new_end, new_start);
            close(after, before, "the direction vector is unchanged");
            assert!(
                (len3(after) - len3(before)).abs() < 1e-12,
                "the length is unchanged: {} vs {}",
                len3(after),
                len3(before)
            );
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
    // LINE: only 0 and 1 exist for endpoints (`2` is the midpoint's index, but
    // it belongs to a different kind, so an `Endpoint` at 2 is still stale).
    for index in [2u8, 7, u8::MAX] {
        let error = apply_grip(
            &line(0.0, 0.0, 10.0, 0.0),
            &grip_id(7, GripKind::Endpoint, index),
            p3(1.0, 1.0, 0.0),
        )
        .expect_err("a LINE endpoint index above 1 does not exist");
        assert_reason(error, REASON_STALE_HANDLE);
    }
    // LINE midpoint: exactly index 2.
    for index in [0u8, 1, 3, u8::MAX] {
        let error = apply_grip(
            &line(0.0, 0.0, 10.0, 0.0),
            &grip_id(7, GripKind::Midpoint, index),
            p3(1.0, 1.0, 0.0),
        )
        .expect_err("a LINE has exactly one midpoint grip, at index 2");
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
    // so the command must skip the transaction (no empty undo entry).
    let error = apply_grip(
        &line(0.0, 0.0, 10.0, 0.0),
        &grip_id(7, GripKind::Endpoint, 0),
        p3(0.0, 0.0, 0.0),
    )
    .expect_err("a zero displacement is refused, not an Ok no-op");
    assert_reason(error, REASON_ZERO_DISPLACEMENT);

    // The same for the line midpoint, whose handle is the computed midpoint.
    let error = apply_grip(
        &line(0.0, 0.0, 10.0, 0.0),
        &grip_id(7, GripKind::Midpoint, 2),
        p3(5.0, 0.0, 0.0),
    )
    .expect_err("a midpoint drag onto its own handle is nothing to write");
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
    // `INSERT` exposes no handle at all (an instance transform is not an entity
    // handle; `SPLINE` used to be the fixture here but now has control-point
    // grips), so any id naming one is the kind refusal — even though the
    // kind/index would also be "stale" against it.
    let error = apply_grip(
        &insert(),
        &grip_id(1, GripKind::Endpoint, 0),
        p3(6.0, 0.0, 0.0),
    )
    .expect_err("an INSERT exposes no grip model");
    assert_reason(error.clone(), REASON_UNSUPPORTED_KIND);
    assert!(
        reason(error).contains("AcDbBlockReference"),
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
    assert_eq!(grips.len(), 3);
    assert_bits_eq(grips[0].position, start, "grip 0 position");
    assert_bits_eq(grips[1].position, end, "grip 1 position");
    assert_bits_eq(
        grips[2].position,
        midpoint_of(start, end),
        "grip 2 is the exact midpoint of the stored endpoints",
    );

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
