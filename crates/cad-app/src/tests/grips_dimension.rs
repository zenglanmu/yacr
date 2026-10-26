//! `DIMENSION` definition-point grip contract tests (`crate::grips`).
//!
//! Pins the whole dimension family of the grip model: the per-subtype handle order
//! (indices and bit-exact positions), the drag rule (one stored DXF point replaced,
//! then a **re-synthesis** with the entity's own style and text override), the
//! degenerate-definition refusals, and the hit-testing/tie-break behaviour.
//!
//! Fixtures use binary-exact, non-round coordinates (`0.5`, `-1.75`, `12.25`, …) so
//! "every other field is bit-identical" and "the measurement is this exact value"
//! are real comparisons. The two angular measurements are degrees of an `atan2`
//! sweep, i.e. transcendental, so those are asserted to `1e-12` instead of by bits.

use super::grips::{assert_bits_eq, assert_reason, ent, grip_id, p3};
use super::*;
use cad_db::DbEntity;

use crate::grips::{
    apply_grip, grip_hit, grip_hit_planar, grips_for_entity, GripKind, REASON_DIMENSION_DEGENERATE,
    REASON_NON_FINITE_TARGET, REASON_STALE_HANDLE, REASON_ZERO_DISPLACEMENT,
};
use cad_domain::{
    DimensionDefinition, DimensionGeometry, DimensionKind, DimensionPoints, DimensionStyleValues,
    StyleId,
};

/// The resolved style snapshot every fixture is synthesized with (`DIMASZ 2.5`,
/// `DIMTXT 2.5`, `DIMDEC 4`, …). It is a fixture input, not a claim about a
/// DIMSTYLE table: the grip model only ever passes it through.
fn style() -> DimensionStyleValues {
    DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        center_mark: 0.0,
        dim_line_increment: 3.75,
        text_vertical: 1,
        text_vertical_position: 0.0,
        suppress_ext1: false,
        suppress_ext2: false,
        suppress_dim1: false,
        suppress_dim2: false,
        linear_factor: 1.0,
        decimals: 4,
        rounding: 0.0,
        postfix: String::new(),
        zero_suppression: 0,
        linear_unit_format: 2,
        decimal_separator: 46,
        text_style_name: "Standard".into(),
        text_style: StyleId(0),
        text_font: None,
    }
}

/// The style name every fixture stores (deliberately not `"Standard"`, so "the
/// drag kept the entity's own name" is a real assertion).
const STYLE_NAME: &str = "ISO-25";

/// The user text override of the linear fixture (DXF group 1). It contains the
/// `<>` measurement placeholder, so a lost override is visible in the display.
const OVERRIDE: &str = "<> mm";

/// The `Linear { rotation: 0.0 }` fixture: the measurement is `|second.x -
/// first.x|` = `11.75` by construction.
fn linear_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Linear { rotation: 0.0 },
        points: DimensionPoints::Linear {
            first: p3(0.5, -1.75, 0.25),
            second: p3(12.25, 3.5, 0.25),
            dimension_line: p3(0.5, -6.25, 0.25),
        },
        text_middle: p3(6.125, -8.5, 0.25),
        text_rotation: 0.375,
        normal: p3(0.0, 0.0, 1.0),
    }
}

/// The `Aligned` fixture: the same point slots as a linear dimension, measured
/// along `first -> second`.
fn aligned_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Aligned,
        points: DimensionPoints::Linear {
            first: p3(1.5, 2.25, 0.0),
            second: p3(4.5, 6.25, 0.0),
            dimension_line: p3(0.5, 8.25, 0.0),
        },
        text_middle: p3(3.0, 10.5, 0.0),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    }
}

/// The `Radius` fixture: `point_on_arc - center` is the exact 3-4-5 vector
/// `(3, 4, 0)`, so the measurement is exactly `5.0`.
fn radial_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Radius,
        points: DimensionPoints::Radial {
            center: p3(0.5, -1.75, 2.5),
            point_on_arc: p3(3.5, 2.25, 2.5),
            // A scalar, not a point: it must be preserved by every drag and it is
            // never a handle.
            leader_length: 1.25,
        },
        text_middle: p3(7.25, -3.75, 2.5),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    }
}

/// The `Angular2Ln` fixture: line 1 is horizontal (`y = -2.25`), line 2 vertical
/// (`x = 0.5`), so the vertex is `(0.5, -2.25)` and the dimensioned sector is 90
/// degrees around `arc_point = (4.5, 1.75)`.
fn angular_2ln_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Angular2Ln,
        points: DimensionPoints::Angular2Ln {
            line1_start: p3(0.5, -2.25, 0.0),
            line1_end: p3(8.5, -2.25, 0.0),
            line2_start: p3(0.5, 6.75, 0.0),
            line2_end: p3(0.5, 14.75, 0.0),
            arc_point: p3(4.5, 1.75, 0.0),
        },
        text_middle: p3(5.75, 3.25, 0.0),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    }
}

/// The `Angular3Pt` fixture: two axis-aligned rays from `(0.5, -2.25)`, so the
/// dimensioned sector is 90 degrees and `arc_point` sits inside it.
fn angular_3pt_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Angular3Pt,
        points: DimensionPoints::Angular3Pt {
            vertex: p3(0.5, -2.25, 0.0),
            first: p3(10.5, -2.25, 0.0),
            second: p3(0.5, 5.75, 0.0),
            arc_point: p3(3.5, 0.75, 0.0),
        },
        text_middle: p3(4.75, 1.5, 0.0),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    }
}

/// The `Ordinate { x_ordinate: true }` fixture: the measurement is
/// `|feature.x - base.x|` = `8.125` by construction.
fn ordinate_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Ordinate { x_ordinate: true },
        points: DimensionPoints::Ordinate {
            feature: p3(9.125, -2.5, 1.5),
            leader_end: p3(9.125, -8.5, 1.5),
            base: p3(1.0, -2.5, 1.5),
        },
        text_middle: p3(9.125, -10.75, 1.5),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    }
}

/// Every fixture definition, in the order the subtype table documents them.
fn definitions() -> Vec<DimensionDefinition> {
    vec![
        linear_definition(),
        aligned_definition(),
        radial_definition(),
        angular_2ln_definition(),
        angular_3pt_definition(),
        ordinate_definition(),
    ]
}

/// Build the stored entity for `definition`, with `override_text` as the user text
/// override (DXF group 1).
fn dimension_geometry(
    definition: DimensionDefinition,
    override_text: Option<&str>,
) -> DimensionGeometry {
    DimensionGeometry::synthesized(
        definition,
        STYLE_NAME.to_string(),
        style(),
        override_text.map(str::to_string),
    )
    .expect("the fixture definition synthesizes")
}

fn dimension_entity(id: u128, definition: DimensionDefinition) -> DbEntity {
    ent(
        id,
        SemanticGeometry::Dimension(Box::new(dimension_geometry(definition, None))),
    )
}

/// The stored `DimensionGeometry` a drag returned, or a panic naming the kind.
fn dragged_dimension(geometry: SemanticGeometry) -> DimensionGeometry {
    match geometry {
        SemanticGeometry::Dimension(dimension) => *dimension,
        other => panic!("expected a DIMENSION, got {other:?}"),
    }
}

/// The documented per-subtype order **restated independently of the model**, so
/// "only that slot moved" is asserted against the contract and not against the
/// implementation's own table. The text middle point is always last.
fn documented_slots(definition: &DimensionDefinition) -> Vec<Point3> {
    let mut slots = match &definition.points {
        DimensionPoints::Linear {
            first,
            second,
            dimension_line,
        } => vec![*first, *second, *dimension_line],
        DimensionPoints::Radial {
            center,
            point_on_arc,
            // `leader_length` is deliberately absent: it is not a point.
            leader_length: _,
        } => vec![*center, *point_on_arc],
        DimensionPoints::Angular2Ln {
            line1_start,
            line1_end,
            line2_start,
            line2_end,
            arc_point,
        } => vec![
            *line1_start,
            *line1_end,
            *line2_start,
            *line2_end,
            *arc_point,
        ],
        DimensionPoints::Angular3Pt {
            vertex,
            first,
            second,
            arc_point,
        } => vec![*vertex, *first, *second, *arc_point],
        DimensionPoints::Ordinate {
            feature,
            leader_end,
            base,
        } => vec![*feature, *leader_end, *base],
    };
    slots.push(definition.text_middle);
    slots
}

/// A target that is not any fixture point, offset per slot so each drag is a
/// distinct edit.
fn target_for(index: usize) -> Point3 {
    p3(
        101.5 + index as f64 * 2.25,
        -57.75 - index as f64 * 0.5,
        13.0 + index as f64,
    )
}

// ---------------------------------------------------------------------------
// 1. Handle lists: indices, kind and bit-exact positions, per subtype.
// ---------------------------------------------------------------------------

/// Assert that `entity` exposes exactly one `DefinitionPoint` grip per documented
/// slot, with the documented index and the stored position bit for bit.
fn assert_handle_list(definition: &DimensionDefinition, entity: &DbEntity) {
    let expected = documented_slots(definition);
    let grips = grips_for_entity(entity);
    assert_eq!(
        grips.len(),
        expected.len(),
        "{:?} must expose one handle per definition point plus the text middle point",
        definition.kind
    );
    for (index, (grip, position)) in grips.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            grip.id,
            grip_id(entity.id.0, GripKind::DefinitionPoint, index as u8),
            "{:?} index {index}",
            definition.kind
        );
        assert_bits_eq(
            grip.position,
            *position,
            &format!("{:?} slot {index}", definition.kind),
        );
    }
}

#[test]
fn linear_dimension_exposes_four_definition_point_grips() {
    let definition = linear_definition();
    let entity = dimension_entity(31, definition.clone());
    assert_handle_list(&definition, &entity);
    let grips = grips_for_entity(&entity);
    // Restated by name, so the index-to-slot mapping is pinned in the test and not
    // only in the model's table: 0 first, 1 second, 2 dimension line, 3 text.
    assert_bits_eq(grips[0].position, p3(0.5, -1.75, 0.25), "first");
    assert_bits_eq(grips[1].position, p3(12.25, 3.5, 0.25), "second");
    assert_bits_eq(grips[2].position, p3(0.5, -6.25, 0.25), "dimension line");
    assert_bits_eq(grips[3].position, p3(6.125, -8.5, 0.25), "text middle");
}

#[test]
fn radial_dimension_exposes_center_point_on_arc_and_text_middle() {
    let definition = radial_definition();
    let entity = dimension_entity(32, definition.clone());
    assert_handle_list(&definition, &entity);
    let grips = grips_for_entity(&entity);
    assert_bits_eq(grips[0].position, p3(0.5, -1.75, 2.5), "centre");
    assert_bits_eq(grips[1].position, p3(3.5, 2.25, 2.5), "point on arc");
    assert_bits_eq(grips[2].position, p3(7.25, -3.75, 2.5), "text middle");
}

#[test]
fn angular_2ln_dimension_exposes_six_definition_point_grips() {
    let definition = angular_2ln_definition();
    let entity = dimension_entity(33, definition.clone());
    assert_handle_list(&definition, &entity);
    let grips = grips_for_entity(&entity);
    assert_bits_eq(grips[0].position, p3(0.5, -2.25, 0.0), "line 1 start");
    assert_bits_eq(grips[1].position, p3(8.5, -2.25, 0.0), "line 1 end");
    assert_bits_eq(grips[2].position, p3(0.5, 6.75, 0.0), "line 2 start");
    assert_bits_eq(grips[3].position, p3(0.5, 14.75, 0.0), "line 2 end");
    assert_bits_eq(grips[4].position, p3(4.5, 1.75, 0.0), "arc location");
    assert_bits_eq(grips[5].position, p3(5.75, 3.25, 0.0), "text middle");
}

#[test]
fn angular_3pt_dimension_exposes_five_definition_point_grips() {
    let definition = angular_3pt_definition();
    let entity = dimension_entity(34, definition.clone());
    assert_handle_list(&definition, &entity);
    let grips = grips_for_entity(&entity);
    assert_bits_eq(grips[0].position, p3(0.5, -2.25, 0.0), "vertex");
    assert_bits_eq(grips[1].position, p3(10.5, -2.25, 0.0), "first point");
    assert_bits_eq(grips[2].position, p3(0.5, 5.75, 0.0), "second point");
    assert_bits_eq(grips[3].position, p3(3.5, 0.75, 0.0), "arc location");
    assert_bits_eq(grips[4].position, p3(4.75, 1.5, 0.0), "text middle");
}

#[test]
fn ordinate_dimension_exposes_four_definition_point_grips() {
    let definition = ordinate_definition();
    let entity = dimension_entity(35, definition.clone());
    assert_handle_list(&definition, &entity);
    let grips = grips_for_entity(&entity);
    assert_bits_eq(grips[0].position, p3(9.125, -2.5, 1.5), "feature");
    assert_bits_eq(grips[1].position, p3(9.125, -8.5, 1.5), "leader end");
    assert_bits_eq(grips[2].position, p3(1.0, -2.5, 1.5), "datum base");
    assert_bits_eq(grips[3].position, p3(9.125, -10.75, 1.5), "text middle");
}

#[test]
fn aligned_dimension_uses_the_linear_handle_order() {
    // `Aligned` shares `DimensionPoints::Linear`, so it must expose the same four
    // handles in the same order — a second, differently shaped arm would be a
    // silent divergence.
    let definition = aligned_definition();
    let entity = dimension_entity(36, definition.clone());
    assert_handle_list(&definition, &entity);
    assert_eq!(grips_for_entity(&entity).len(), 4);
}

// ---------------------------------------------------------------------------
// 2. Drags: one slot moves, everything else is bit-identical, and the display is
//    re-synthesized (measurement asserted analytically).
// ---------------------------------------------------------------------------

/// Drag `index` of `definition` to `target_for(index)` and assert the contract:
/// only that stored point changed, every scalar/kind/style is bit-identical, the
/// measurement is finite and the display non-empty. Returns the rebuilt dimension.
fn drag_and_check(definition: &DimensionDefinition, index: usize) -> DimensionGeometry {
    let entity = dimension_entity(41, definition.clone());
    let original = dimension_geometry(definition.clone(), None);
    let target = target_for(index);
    let dragged = dragged_dimension(
        apply_grip(
            &entity.geometry,
            &grip_id(41, GripKind::DefinitionPoint, index as u8),
            target,
        )
        .unwrap_or_else(|error| panic!("{:?} slot {index}: {error:?}", definition.kind)),
    );

    let before = documented_slots(definition);
    let after = documented_slots(dragged.definition());
    assert_eq!(before.len(), after.len(), "the slot set never changes");
    for (slot, (old, new)) in before.iter().zip(after.iter()).enumerate() {
        if slot == index {
            assert_bits_eq(*new, target, &format!("the dragged slot {index}"));
        } else {
            assert_bits_eq(
                *new,
                *old,
                &format!("{:?} slot {slot} must not move", definition.kind),
            );
        }
    }
    // Everything the subsume carries beyond the points.
    assert_eq!(dragged.kind(), definition.kind, "the subtype is untouched");
    assert_eq!(
        dragged.definition().text_rotation.to_bits(),
        definition.text_rotation.to_bits(),
        "group 53 is not a handle"
    );
    assert_eq!(
        dragged.definition().normal,
        definition.normal,
        "the extrusion normal is untouched"
    );
    assert_eq!(dragged.style_name(), STYLE_NAME, "DXF group 3 is untouched");
    assert_eq!(dragged.style(), &style(), "the style snapshot is untouched");
    assert_eq!(
        dragged.text_override(),
        original.text_override(),
        "the user text override is never dropped"
    );
    assert!(
        dragged.measurement().is_finite(),
        "{:?} slot {index} produced a non-finite measurement",
        definition.kind
    );
    assert!(
        !dragged.display().is_empty(),
        "{:?} slot {index} produced an empty display",
        definition.kind
    );
    dragged
}

#[test]
fn every_documented_slot_of_every_subtype_is_draggable_and_exclusive() {
    for definition in definitions() {
        let slots = documented_slots(&definition);
        // Every listed index is a handle...
        for index in 0..slots.len() {
            drag_and_check(&definition, index);
        }
        // ...and one past the count is not, per subtype (the count differs).
        let entity = dimension_entity(42, definition.clone());
        let error = apply_grip(
            &entity.geometry,
            &grip_id(42, GripKind::DefinitionPoint, slots.len() as u8),
            target_for(0),
        )
        .unwrap_err();
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn dragging_the_linear_second_point_recomputes_the_measurement() {
    let definition = linear_definition();
    let entity = dimension_entity(43, definition.clone());
    // `Linear { rotation: 0.0 }` measures along +X, so the measurement is
    // `|second.x - first.x|` = 11.75 for the fixture.
    let original = dimension_geometry(definition.clone(), None);
    assert_eq!(original.measurement(), 11.75);

    let dragged = dragged_dimension(
        apply_grip(
            &entity.geometry,
            &grip_id(43, GripKind::DefinitionPoint, 1),
            p3(2.5, 3.5, 0.25),
        )
        .expect("moving `second` is supported"),
    );
    // Exact: both x values are binary-exact and the rule subtracts them.
    assert_eq!(dragged.measurement(), 2.0);
}

#[test]
fn dragging_the_radial_point_on_arc_recomputes_the_measurement() {
    let definition = radial_definition();
    let entity = dimension_entity(44, definition.clone());
    let original = dimension_geometry(definition.clone(), None);
    // `(3, 4, 0)` is an exact 3-4-5 vector.
    assert_eq!(original.measurement(), 5.0);

    let dragged = dragged_dimension(
        apply_grip(
            &entity.geometry,
            &grip_id(44, GripKind::DefinitionPoint, 1),
            // `center + (6, 8, 0)`: another exact 3-4-5 multiple.
            p3(6.5, 6.25, 2.5),
        )
        .expect("moving `point_on_arc` is supported"),
    );
    assert_eq!(dragged.measurement(), 10.0);
    // The centre and `leader_length` (a scalar, not a handle) are untouched.
    match &dragged.definition().points {
        DimensionPoints::Radial {
            center,
            point_on_arc,
            leader_length,
        } => {
            assert_bits_eq(*center, p3(0.5, -1.75, 2.5), "the centre");
            assert_bits_eq(*point_on_arc, p3(6.5, 6.25, 2.5), "the moved point");
            assert_eq!(
                leader_length.to_bits(),
                1.25f64.to_bits(),
                "group 40 is not a grip and is preserved"
            );
        }
        other => panic!("expected radial points, got {other:?}"),
    }
}

#[test]
fn dragging_an_angular_2ln_line_end_recomputes_the_angle() {
    let definition = angular_2ln_definition();
    let entity = dimension_entity(45, definition.clone());
    let original = dimension_geometry(definition.clone(), None);
    // Two perpendicular lines: 90 degrees (degrees of an `atan2` sweep, so this is
    // a tolerance assertion, not a bit comparison).
    assert!((original.measurement() - 90.0).abs() < 1e-12);

    // Swing line 1 to 45 degrees; line 2 stays vertical and the vertex stays put.
    let dragged = dragged_dimension(
        apply_grip(
            &entity.geometry,
            &grip_id(45, GripKind::DefinitionPoint, 1),
            p3(8.5, 5.75, 0.0),
        )
        .expect("moving `line1_end` is supported"),
    );
    assert!(
        (dragged.measurement() - 45.0).abs() < 1e-9,
        "expected a 45 degree sector, got {}",
        dragged.measurement()
    );
}

#[test]
fn dragging_an_angular_3pt_second_point_keeps_the_right_angle() {
    let definition = angular_3pt_definition();
    let entity = dimension_entity(46, definition.clone());
    let original = dimension_geometry(definition.clone(), None);
    assert!((original.measurement() - 90.0).abs() < 1e-12);

    // Sliding `second` further along +Y keeps it on the same ray, so the measured
    // sector stays a right angle while the stored point really moves.
    let dragged = dragged_dimension(
        apply_grip(
            &entity.geometry,
            &grip_id(46, GripKind::DefinitionPoint, 2),
            p3(0.5, 11.75, 0.0),
        )
        .expect("moving `second` is supported"),
    );
    assert!((dragged.measurement() - 90.0).abs() < 1e-12);
}

#[test]
fn dragging_the_ordinate_feature_recomputes_the_datum_relative_measurement() {
    let definition = ordinate_definition();
    let entity = dimension_entity(47, definition.clone());
    let original = dimension_geometry(definition.clone(), None);
    // `|feature.x - base.x|` = |9.125 - 1.0| = 8.125.
    assert_eq!(original.measurement(), 8.125);

    let dragged = dragged_dimension(
        apply_grip(
            &entity.geometry,
            &grip_id(47, GripKind::DefinitionPoint, 0),
            p3(12.375, -2.5, 1.5),
        )
        .expect("moving `feature` is supported"),
    );
    assert_eq!(dragged.measurement(), 11.375);
    match &dragged.definition().points {
        DimensionPoints::Ordinate {
            feature,
            leader_end,
            base,
        } => {
            assert_bits_eq(*feature, p3(12.375, -2.5, 1.5), "the moved feature");
            assert_bits_eq(*leader_end, p3(9.125, -8.5, 1.5), "the leader end");
            assert_bits_eq(*base, p3(1.0, -2.5, 1.5), "the datum base");
        }
        other => panic!("expected ordinate points, got {other:?}"),
    }
}

#[test]
fn dragging_the_text_middle_point_moves_only_that_point() {
    for definition in definitions() {
        let entity = dimension_entity(48, definition.clone());
        let original = dimension_geometry(definition.clone(), None);
        let slots = documented_slots(&definition);
        let text_index = slots.len() - 1;
        let target = p3(-31.5, 42.25, 7.0);
        let dragged = dragged_dimension(
            apply_grip(
                &entity.geometry,
                &grip_id(48, GripKind::DefinitionPoint, text_index as u8),
                target,
            )
            .unwrap_or_else(|error| panic!("{:?} text middle: {error:?}", definition.kind)),
        );
        // Only the text middle point moved...
        assert_bits_eq(
            dragged.definition().text_middle,
            target,
            "the dragged text middle point",
        );
        for (slot, position) in documented_slots(&definition)
            .into_iter()
            .take(text_index)
            .enumerate()
        {
            assert_bits_eq(
                documented_slots(dragged.definition())[slot],
                position,
                &format!("{:?} definition point {slot}", definition.kind),
            );
        }
        // ...the measurement does not depend on it...
        assert_eq!(
            dragged.measurement().to_bits(),
            original.measurement().to_bits(),
            "{:?} text middle must not change the measurement",
            definition.kind
        );
        // ...and the display is still a full synthesis.
        assert!(!dragged.display().is_empty());
    }
}

#[test]
fn a_kept_text_override_survives_a_definition_point_drag() {
    // The constructor takes the override, so a drag neither drops it nor
    // substitutes its own string: the drawn text still carries the user template.
    let definition = linear_definition();
    let geometry =
        SemanticGeometry::Dimension(Box::new(dimension_geometry(definition, Some(OVERRIDE))));
    let dragged = dragged_dimension(
        apply_grip(
            &geometry,
            &grip_id(49, GripKind::DefinitionPoint, 1),
            p3(2.5, 3.5, 0.25),
        )
        .expect("moving `second` is supported"),
    );
    assert_eq!(dragged.text_override(), Some(OVERRIDE));
    let Some(SemanticGeometry::Text { text, .. }) = dragged.display().last() else {
        panic!("the synthesis always appends the measurement text");
    };
    assert_eq!(text, "2.0000 mm", "the override's `<>` takes the new value");
}

#[test]
fn a_picture_branch_dimension_is_re_synthesized_by_a_drag() {
    // Declared limitation (see `grips/dimension.rs`): `DisplaySource` is private to
    // `cad-domain`, so a drag cannot tell the imported-picture branch from our
    // synthesis and always re-synthesizes. Pinned here so the behaviour is a
    // documented contract instead of a silent surprise: the picture is replaced by
    // our synthesis and the measurement follows the mutated definition.
    let definition = linear_definition();
    let picture = vec![SemanticGeometry::Line {
        start: p3(0.0, 0.0, 0.0),
        end: p3(1.0, 1.0, 0.0),
    }];
    let geometry = SemanticGeometry::Dimension(Box::new(
        DimensionGeometry::from_picture(
            definition,
            STYLE_NAME.to_string(),
            style(),
            None,
            picture.clone(),
        )
        .expect("the fixture definition measures"),
    ));
    let dragged = dragged_dimension(
        apply_grip(
            &geometry,
            &grip_id(50, GripKind::DefinitionPoint, 1),
            p3(2.5, 3.5, 0.25),
        )
        .expect("moving `second` is supported"),
    );
    assert_eq!(dragged.measurement(), 2.0, "recomputed from the definition");
    assert_ne!(
        dragged.display(),
        picture.as_slice(),
        "the picture display is replaced by the synthesis"
    );
    assert!(dragged.display().len() > 1, "a full synthesized display");
}

// ---------------------------------------------------------------------------
// 3. Refusals: wrong kind, stale index, non-finite target, zero displacement, and
//    the degenerate mutations (named, never clamped).
// ---------------------------------------------------------------------------

#[test]
fn a_non_definition_point_kind_on_a_dimension_is_a_stale_handle() {
    let definition = linear_definition();
    let entity = dimension_entity(51, definition);
    for kind in [
        GripKind::Endpoint,
        GripKind::Midpoint,
        GripKind::Center,
        GripKind::Quadrant,
        GripKind::Vertex,
        GripKind::ControlPoint,
        GripKind::Point,
        GripKind::Insertion,
    ] {
        let error = apply_grip(
            &entity.geometry,
            &grip_id(51, kind, 0),
            p3(300.5, -300.25, 1.0),
        )
        .unwrap_err();
        assert_reason(error, REASON_STALE_HANDLE);
    }
}

#[test]
fn an_index_past_the_subtypes_handle_count_is_a_stale_handle() {
    for definition in definitions() {
        let entity = dimension_entity(52, definition.clone());
        let count = documented_slots(&definition).len();
        for index in [count as u8, (count + 1) as u8, u8::MAX] {
            let error = apply_grip(
                &entity.geometry,
                &grip_id(52, GripKind::DefinitionPoint, index),
                target_for(0),
            )
            .unwrap_err();
            assert_reason(error, REASON_STALE_HANDLE);
        }
    }
}

#[test]
fn a_non_finite_dimension_target_is_refused_before_the_geometry_is_read() {
    let entity = dimension_entity(53, linear_definition());
    for target in [
        p3(f64::NAN, 0.0, 0.0),
        p3(0.0, f64::INFINITY, 0.0),
        p3(0.0, 0.0, f64::NEG_INFINITY),
    ] {
        let error = apply_grip(
            &entity.geometry,
            &grip_id(53, GripKind::DefinitionPoint, 0),
            target,
        )
        .unwrap_err();
        assert_reason(error, REASON_NON_FINITE_TARGET);
    }
}

#[test]
fn dragging_a_dimension_point_onto_itself_is_zero_displacement() {
    for definition in definitions() {
        let entity = dimension_entity(54, definition.clone());
        for (index, position) in documented_slots(&definition).into_iter().enumerate() {
            let error = apply_grip(
                &entity.geometry,
                &grip_id(54, GripKind::DefinitionPoint, index as u8),
                position,
            )
            .unwrap_err();
            assert_reason(error, REASON_ZERO_DISPLACEMENT);
        }
    }
}

#[test]
fn a_radial_point_on_arc_dragged_onto_the_centre_is_degenerate() {
    // The one case the requirement calls out: `point_on_arc == center` has no
    // radius, so the synthesis refuses. The drag must name the reason **before** it
    // can produce anything, and must never emit a `NaN` measurement or an empty
    // display (`apply_grip` returns `Err`, so there is no geometry at all).
    let definition = radial_definition();
    let entity = dimension_entity(55, definition.clone());
    let centre = match &definition.points {
        DimensionPoints::Radial { center, .. } => *center,
        other => panic!("expected radial points, got {other:?}"),
    };
    let error = apply_grip(
        &entity.geometry,
        &grip_id(55, GripKind::DefinitionPoint, 1),
        centre,
    )
    .unwrap_err();
    assert_reason(error, REASON_DIMENSION_DEGENERATE);

    // Same rule for a diameter dimension: dragging the group-10 slot (the opposite
    // chord point) onto `point_on_arc` collapses the leader all the same.
    let diameter = DimensionDefinition {
        kind: DimensionKind::Diameter,
        ..radial_definition()
    };
    let entity = dimension_entity(56, diameter.clone());
    let point_on_arc = match &diameter.points {
        DimensionPoints::Radial { point_on_arc, .. } => *point_on_arc,
        other => panic!("expected radial points, got {other:?}"),
    };
    let error = apply_grip(
        &entity.geometry,
        &grip_id(56, GripKind::DefinitionPoint, 0),
        point_on_arc,
    )
    .unwrap_err();
    assert_reason(error, REASON_DIMENSION_DEGENERATE);
}

#[test]
fn an_angular_arc_location_on_the_vertex_is_degenerate() {
    let definition = angular_3pt_definition();
    let entity = dimension_entity(57, definition.clone());
    let vertex = match &definition.points {
        DimensionPoints::Angular3Pt { vertex, .. } => *vertex,
        other => panic!("expected three-point points, got {other:?}"),
    };
    let error = apply_grip(
        &entity.geometry,
        &grip_id(57, GripKind::DefinitionPoint, 3),
        vertex,
    )
    .unwrap_err();
    assert_reason(error, REASON_DIMENSION_DEGENERATE);
}

#[test]
fn two_line_angular_dimension_driven_parallel_is_degenerate() {
    let definition = angular_2ln_definition();
    let entity = dimension_entity(58, definition);
    // Make line 2 horizontal as well: no vertex exists, so the dimension cannot be
    // synthesized and the drag refuses by name instead of storing a `NaN` group 42.
    let error = apply_grip(
        &entity.geometry,
        &grip_id(58, GripKind::DefinitionPoint, 3),
        p3(8.5, 6.75, 0.0),
    )
    .unwrap_err();
    assert_reason(error, REASON_DIMENSION_DEGENERATE);
}

#[test]
fn a_refused_dimension_drag_leaves_the_pure_model_untouched() {
    // The model is a pure function: the entity it was handed is not modified, so a
    // refused drag cannot have half-applied a point.
    let definition = radial_definition();
    let entity = dimension_entity(59, definition.clone());
    let before = entity.geometry.clone();
    let centre = match &definition.points {
        DimensionPoints::Radial { center, .. } => *center,
        other => panic!("expected radial points, got {other:?}"),
    };
    let _ = apply_grip(
        &entity.geometry,
        &grip_id(59, GripKind::DefinitionPoint, 1),
        centre,
    );
    assert_eq!(entity.geometry, before);
}

// ---------------------------------------------------------------------------
// 4. Hit-testing: exact hit, plan-view hit, tie-break and miss.
// ---------------------------------------------------------------------------

#[test]
fn grip_hit_finds_a_dimension_definition_point() {
    let definition = linear_definition();
    let entity = dimension_entity(61, definition);
    // Exactly on `second`, with zero tolerance: a hit, not a near-miss.
    let hit = grip_hit([entity.clone()].iter(), p3(12.25, 3.5, 0.25), 0.0)
        .expect("the definition point is a handle");
    assert_eq!(hit.id, grip_id(61, GripKind::DefinitionPoint, 1));
    assert_bits_eq(hit.position, p3(12.25, 3.5, 0.25), "the stored point");

    // ...and the text middle point is a handle of its own.
    let hit = grip_hit([entity].iter(), p3(6.125, -8.5, 0.25), 0.0).expect("text middle");
    assert_eq!(hit.id, grip_id(61, GripKind::DefinitionPoint, 3));
}

#[test]
fn grip_hit_planar_finds_a_dimension_point_at_another_elevation() {
    let definition = linear_definition();
    let entity = dimension_entity(62, definition);
    // The fixture sits at z = 0.25; the plan-view pointer resolves to z = 0.
    let plan = p3(12.25, 3.5, 0.0);
    assert!(
        grip_hit([entity.clone()].iter(), plan, 1e-9).is_none(),
        "the 3D rule measures the elevation difference"
    );
    let hit = grip_hit_planar([entity].iter(), plan, 0.0).expect("the plan-view rule hits");
    assert_eq!(hit.id, grip_id(62, GripKind::DefinitionPoint, 1));
    assert_bits_eq(
        hit.position,
        p3(12.25, 3.5, 0.25),
        "the position stays the stored 3D point",
    );
}

#[test]
fn grip_hit_tie_goes_to_the_lower_dimension_index() {
    // `first` and `second` are symmetric about the query point, so both are exactly
    // 2.0 away: the earlier list entry (`index` 0) wins, like every other family.
    let definition = DimensionDefinition {
        kind: DimensionKind::Linear { rotation: 0.0 },
        points: DimensionPoints::Linear {
            first: p3(0.5, 0.0, 0.0),
            second: p3(4.5, 0.0, 0.0),
            dimension_line: p3(0.5, -3.5, 0.0),
        },
        text_middle: p3(2.5, -4.5, 0.0),
        text_rotation: 0.0,
        normal: p3(0.0, 0.0, 1.0),
    };
    let entity = dimension_entity(63, definition);
    let hit = grip_hit([entity].iter(), p3(2.5, 0.0, 0.0), 3.0).expect("a hit");
    assert_eq!(
        hit.id,
        grip_id(63, GripKind::DefinitionPoint, 0),
        "the lower index keeps an exact tie"
    );
}

#[test]
fn grip_hit_misses_a_dimension_whose_points_are_out_of_tolerance() {
    let entity = dimension_entity(64, linear_definition());
    // Between two handles (the midpoint of `first`..`second` at the fixture's own z)
    // is not a handle: a dimension exposes its definition points only.
    let between = p3((0.5 + 12.25) * 0.5, (-1.75 + 3.5) * 0.5, 0.25);
    assert!(grip_hit([entity.clone()].iter(), between, 0.5).is_none());
    assert!(grip_hit_planar([entity].iter(), between, 0.5).is_none());
}
