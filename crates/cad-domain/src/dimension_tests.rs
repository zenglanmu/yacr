//! Dimension unit tests (split out to keep `dimension.rs` under the file-size
//! limit).

use crate::dimension::*;
use crate::dimension_synthesis::{add, length, scale, sub, synthesize};
use crate::*;

fn style() -> DimensionStyleValues {
    DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        center_mark: 0.0,
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
        text_style_name: "Standard".to_string(),
        text_style: StyleId(0),
        text_font: None,
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

fn aligned_definition() -> DimensionDefinition {
    DimensionDefinition {
        kind: DimensionKind::Aligned,
        points: DimensionPoints::Linear {
            first: point(-21.0, 0.0, 0.0),
            second: point(21.0, 0.0, 0.0),
            dimension_line: point(-21.0, -30.0, 0.0),
        },
        text_middle: point(0.0, 0.0, 0.0),
        text_rotation: 0.0,
        normal: point(0.0, 0.0, 1.0),
    }
}

#[test]
fn linear_synthesis_builds_extension_lines_dimension_line_and_arrows() {
    let geometry = linear_synthesis(
        point(-21.0, 0.0, 0.0),
        point(21.0, 0.0, 0.0),
        point(-21.0, -30.0, 0.0),
        point(1.0, 0.0, 0.0),
        &style(),
    )
    .expect("synthesized");
    assert!(approx(geometry.measurement, 42.0));
    assert_eq!(
        geometry
            .children
            .iter()
            .filter(|c| matches!(c, SemanticGeometry::Line { .. }))
            .count(),
        3
    );
    assert_eq!(
        geometry
            .children
            .iter()
            .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
            .count(),
        2
    );
    assert!(approx(geometry.text_position.x, 0.0));
    assert!(approx(geometry.text_rotation, 0.0));
}

#[test]
fn vertical_linear_synthesis_rotates_text() {
    let geometry = linear_synthesis(
        point(30.0, 3.0, 0.0),
        point(30.0, -3.0, 0.0),
        point(45.0, 3.0, 0.0),
        point(0.0, 1.0, 0.0),
        &style(),
    )
    .expect("synthesized");
    assert!(approx(geometry.measurement, 6.0));
    assert!(approx(geometry.text_rotation, 90.0_f64.to_radians()));
}

#[test]
fn radial_synthesis_measures_the_leader() {
    let geometry = radial_synthesis(
        point(30.0, 0.0, 0.0),
        point(33.0, -4.0, 0.0),
        point(30.0, 0.0, 0.0),
        false,
        &style(),
    )
    .expect("synthesized");
    assert!(approx(geometry.measurement, 5.0));
    assert_eq!(
        geometry
            .children
            .iter()
            .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
            .count(),
        1
    );
}

#[test]
fn angular_synthesis_measures_the_arc() {
    let geometry = angular_synthesis(
        point(0.0, 0.0, 0.0),
        point(1.0, 0.0, 0.0),
        point(0.0, 1.0, 0.0),
        5.0,
        point(0.0, 0.0, 1.0),
        &style(),
    )
    .expect("angular geometry");
    assert!(approx(geometry.measurement, 90.0));
}

#[test]
fn resolve_measurement_text_suppresses_trailing_zeros() {
    let mut style = style();
    style.decimals = 2;
    style.zero_suppression = 8;
    assert_eq!(resolve_measurement_text(None, "", "", 42.0, &style), "42");
    assert_eq!(resolve_measurement_text(None, "R", "", 5.0, &style), "R5");
    assert_eq!(
        resolve_measurement_text(None, "", "\u{00B0}", 45.0, &style),
        "45°"
    );
    assert_eq!(
        resolve_measurement_text(Some("R<>"), "R", "", 5.0, &style),
        "R5"
    );
}

#[test]
fn measurement_text_applies_the_linear_factor() {
    // Review: the formatting path had no DIMLFAC != 1 test.
    let mut style = style();
    style.linear_factor = 2.0;
    style.decimals = 2;
    assert_eq!(
        resolve_measurement_text(None, "", "", 21.0, &style),
        "42.00"
    );
}

#[test]
fn picture_branch_transformed_refuses_an_unplaceable_member() {
    // ADR 0017 A.5: a picture member the transform surface cannot place
    // (Spline/Shape/Opaque) refuses the whole entity, never a partial picture.
    let members = vec![
        SemanticGeometry::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(1.0, 0.0, 0.0),
        },
        SemanticGeometry::Opaque {
            type_key: "AcDbProxyEntity".into(),
            version: 1,
            payload: vec![1],
        },
    ];
    let geometry = DimensionGeometry::from_picture(
        aligned_definition(),
        "Standard".to_string(),
        style(),
        None,
        members,
    )
    .expect("from_picture");
    assert!(
        geometry
            .transformed(&Transform3::translation(point(1.0, 0.0, 0.0)))
            .is_err(),
        "an unplaceable picture member must refuse the whole entity"
    );
}

#[test]
fn synthesized_display_matches_synthesis_of_the_definition() {
    // A.4 invariant 1: display == synthesis(definition, style).
    let definition = aligned_definition();
    let geometry =
        DimensionGeometry::synthesized(definition.clone(), "Standard".to_string(), style(), None)
            .expect("synthesized");
    let synthesized = synthesize(&definition, geometry.style()).expect("synthesis");
    let text = place_text(&definition, &synthesized, geometry.style(), None, "", "");
    let mut expected = synthesized.children;
    expected.push(text);
    assert_eq!(geometry.display(), expected.as_slice());
    assert!(approx(geometry.measurement(), 42.0));
}

#[test]
fn translation_moves_definition_and_display_together() {
    let geometry =
        DimensionGeometry::synthesized(aligned_definition(), "Standard".to_string(), style(), None)
            .expect("synthesized");
    let delta = point(100.0, -5.0, 0.0);
    let moved = geometry
        .transformed(&Transform3::translation(delta))
        .expect("transformed");
    // Measurement is unchanged by a pure translation.
    assert!(approx(moved.measurement(), geometry.measurement()));
    // Every display child moved by exactly the translation.
    let text_of = |g: &DimensionGeometry| match g.display().last() {
        Some(SemanticGeometry::Text { position, .. }) => *position,
        other => panic!("expected text child, got {other:?}"),
    };
    let before = text_of(&geometry);
    let after = text_of(&moved);
    assert!(approx(after.x, before.x + delta.x));
    assert!(approx(after.y, before.y + delta.y));
    // The definition moved with the display.
    match moved.definition().points {
        DimensionPoints::Linear { first, .. } => {
            assert!(approx(first.x, 79.0));
            assert!(approx(first.y, -5.0));
        }
        ref other => panic!("expected linear points, got {other:?}"),
    }
}

#[test]
fn uniform_scale_rescales_measurement_and_text() {
    let geometry =
        DimensionGeometry::synthesized(aligned_definition(), "Standard".to_string(), style(), None)
            .expect("synthesized");
    let scaled = geometry
        .transformed(&Transform3::scale(3.0))
        .expect("transformed");
    // measurement is recomputed from the transformed definition: 42 * 3.
    assert!(approx(scaled.measurement(), 126.0));
    // text content tracks the measurement.
    let text = scaled
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("text child");
    assert_eq!(text, "126.0000");
}

#[test]
fn non_uniform_scale_is_refused() {
    let geometry =
        DimensionGeometry::synthesized(aligned_definition(), "Standard".to_string(), style(), None)
            .expect("synthesized");
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = 2.0;
    matrix[1][1] = 3.0;
    let non_uniform = Transform3 { matrix };
    assert!(geometry.transformed(&non_uniform).is_err());
}

/// A rotation about Z by `theta` radians.
fn rotation_z(theta: f64) -> Transform3 {
    let (s, c) = theta.sin_cos();
    let mut matrix = Transform3::identity().matrix;
    matrix[0][0] = c;
    matrix[0][1] = -s;
    matrix[1][0] = s;
    matrix[1][1] = c;
    Transform3 { matrix }
}

#[test]
fn rotation_delta_updates_kind_rotation_and_text_rotation() {
    // Review: a rotation must add its angle to the Linear group-50 rotation and
    // to the group-53 text rotation.
    let mut definition = aligned_definition();
    definition.kind = DimensionKind::Linear { rotation: 0.0 };
    definition.text_rotation = 0.5;
    let geometry =
        DimensionGeometry::synthesized(definition, "Standard".to_string(), style(), None)
            .expect("synthesized");
    let quarter = std::f64::consts::FRAC_PI_2;
    let rotated = geometry.transformed(&rotation_z(quarter)).expect("rotated");
    match rotated.definition().kind {
        DimensionKind::Linear { rotation } => {
            assert!(approx(rotation, quarter), "group-50 rotation = {rotation}");
        }
        other => panic!("expected a linear kind, got {other:?}"),
    }
    assert!(
        approx(rotated.definition().text_rotation, 0.5 + quarter),
        "group-53 text rotation = {}",
        rotated.definition().text_rotation
    );
}

#[test]
fn picture_branch_freezes_measurement_and_text_under_uniform_scale() {
    // Review B1: the picture is the authoritative display, so a uniform scale
    // must NOT recompute the measurement or re-derive the text; group 42 stays
    // consistent with the drawn number instead of inventing a value the picture
    // does not show.
    let members = vec![
        SemanticGeometry::Line {
            start: point(0.0, 0.0, 0.0),
            end: point(10.0, 0.0, 0.0),
        },
        SemanticGeometry::Text {
            text: "42".to_string(),
            position: point(5.0, 1.0, 0.0),
            style: StyleId(0),
            height: 2.5,
            rotation: 0.0,
            font: None,
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Middle,
        },
    ];
    let geometry = DimensionGeometry::from_picture(
        aligned_definition(),
        "Standard".to_string(),
        style(),
        None,
        members,
    )
    .expect("from_picture");
    assert!(approx(geometry.measurement(), 42.0));

    let scaled = geometry
        .transformed(&Transform3::scale(3.0))
        .expect("transformed");
    // The frozen measurement is pinned: NOT 126.
    assert!(
        approx(scaled.measurement(), 42.0),
        "frozen measurement = {}",
        scaled.measurement()
    );
    // The drawn number is unchanged too, so 42 == the picture's text.
    let text = scaled
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("text child");
    assert_eq!(text, "42", "the picture's drawn number is frozen");
    // The display itself still moved (height scaled): only the measurement and
    // the text content are frozen, not the transform.
    let height = scaled
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Text { height, .. } => Some(*height),
            _ => None,
        })
        .expect("text height");
    assert!(approx(height, 7.5), "scaled text height = {height}");
}

#[test]
fn angular_synthesis_uses_the_definition_normal() {
    // Review N1: a -Z extrusion must keep its winding, as the pre-P5a synthesis
    // did (it passed `base.normal`).
    let definition = DimensionDefinition {
        kind: DimensionKind::Angular3Pt,
        points: DimensionPoints::Angular3Pt {
            vertex: point(0.0, 0.0, 0.0),
            first: point(1.0, 0.0, 0.0),
            second: point(0.0, 1.0, 0.0),
            arc_point: point(0.0, 0.0, 0.0),
        },
        text_middle: point(0.0, 0.0, 0.0),
        text_rotation: 0.0,
        normal: point(0.0, 0.0, -1.0),
    };
    let geometry =
        DimensionGeometry::synthesized(definition, "Standard".to_string(), style(), None)
            .expect("synthesized");
    let arc_normal = geometry
        .display()
        .iter()
        .find_map(|child| match child {
            SemanticGeometry::Arc { normal, .. } => Some(*normal),
            _ => None,
        })
        .expect("an angular arc child");
    assert!(
        approx(arc_normal.z, -1.0),
        "arc normal must carry the definition's -Z extrusion: {arc_normal:?}"
    );
}

// ---------------------------------------------------------------------------
// P5b: one numeric contract test per newly consumed B.2(a) variable. Each fails
// if the variable stops changing the synthesized display.
// ---------------------------------------------------------------------------

/// Linear synthesis of the standard 42-unit test dimension, with the dimension
/// line at `line_y` (so `line_y < 0` puts the measured geometry above the line).
fn linear_text_position(style: &DimensionStyleValues, line_y: f64) -> Point3 {
    linear_synthesis(
        point(-21.0, 0.0, 0.0),
        point(21.0, 0.0, 0.0),
        point(-21.0, line_y, 0.0),
        point(1.0, 0.0, 0.0),
        style,
    )
    .expect("synthesized")
    .text_position
}

fn line_count(children: &[SemanticGeometry]) -> usize {
    children
        .iter()
        .filter(|c| matches!(c, SemanticGeometry::Line { .. }))
        .count()
}

fn arrow_count(children: &[SemanticGeometry]) -> usize {
    children
        .iter()
        .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
        .count()
}

#[test]
fn dimtad_one_places_text_above_the_dimension_line() {
    let mut style = style();
    style.text_vertical = 1;
    let position = linear_text_position(&style, -30.0);
    // Line at y = -30, offset = DIMGAP + text_height/2 = 0.625 + 1.25.
    assert!(approx(position.y, -28.125), "text y = {}", position.y);
}

#[test]
fn dimtad_zero_centres_text_and_honours_dimtvp() {
    let mut style = style();
    style.text_vertical = 0;
    style.text_vertical_position = 0.0;
    assert!(
        approx(linear_text_position(&style, -30.0).y, -30.0),
        "DIMTAD=0 with DIMTVP=0 centres the text on the line"
    );
    // DIMTVP is a fraction of the text height: 0.5 * 2.5 = 1.25.
    style.text_vertical_position = 0.5;
    assert!(
        approx(linear_text_position(&style, -30.0).y, -28.75),
        "DIMTVP offsets the text by DIMTVP * text_height"
    );
}

#[test]
fn dimtad_two_is_the_far_side_from_the_measured_points() {
    let mut style = style();
    style.text_vertical = 2;
    // Geometry above the line (y = -30): the far side is below.
    let above = linear_text_position(&style, -30.0);
    assert!(approx(above.y, -31.875), "far side below: y = {}", above.y);
    // Geometry below the line (y = +30): the far side is above.
    let below = linear_text_position(&style, 30.0);
    assert!(
        approx(below.y, 31.875),
        "far side above when the points are below: y = {}",
        below.y
    );
}

#[test]
fn dimtad_four_places_text_below_the_dimension_line() {
    let mut style = style();
    style.text_vertical = 4;
    // "Below" is a fixed direction, unlike DIMTAD=2's far side: with the
    // geometry below the line, DIMTAD=4 still sits below the line (whereas
    // DIMTAD=2 would sit above).
    let position = linear_text_position(&style, 30.0);
    assert!(approx(position.y, 28.125), "below: y = {}", position.y);
}

#[test]
fn dimtad_three_jis_places_text_above() {
    // JIS is a distinct code (not collapsed to "above" silently): its vertical
    // placement equals DIMTAD=1, and the JIS-specific horizontal/fit rules are
    // declared unimplemented. This pins the chosen interpretation.
    let mut style = style();
    style.text_vertical = 3;
    let position = linear_text_position(&style, -30.0);
    assert!(approx(position.y, -28.125), "JIS above: y = {}", position.y);
}

#[test]
fn dimse1_and_dimse2_suppress_extension_lines() {
    let definition = aligned_definition();
    let base =
        |style: &DimensionStyleValues| synthesize(&definition, style).expect("synthesis").children;
    // Neither suppressed: two extension lines plus the dimension line.
    assert_eq!(line_count(&base(&style())), 3);
    let mut only_first = style();
    only_first.suppress_ext1 = true;
    assert_eq!(
        line_count(&base(&only_first)),
        2,
        "DIMSE1 removes the first extension line"
    );
    let mut only_second = style();
    only_second.suppress_ext2 = true;
    assert_eq!(
        line_count(&base(&only_second)),
        2,
        "DIMSE2 removes the second extension line"
    );
    let mut both = style();
    both.suppress_ext1 = true;
    both.suppress_ext2 = true;
    assert_eq!(
        line_count(&base(&both)),
        1,
        "both suppressed leaves only the dimension line"
    );
}

#[test]
fn dimsd1_and_dimsd2_suppress_a_dimension_line_segment_and_arrow() {
    let definition = aligned_definition();
    let base =
        |style: &DimensionStyleValues| synthesize(&definition, style).expect("synthesis").children;
    let mut first = style();
    first.suppress_dim1 = true;
    let children = base(&first);
    assert_eq!(arrow_count(&children), 1, "DIMSD1 removes the first arrow");
    let mut both = style();
    both.suppress_dim1 = true;
    both.suppress_dim2 = true;
    let children = base(&both);
    assert_eq!(arrow_count(&children), 0, "both arrows removed");
    assert_eq!(
        line_count(&children),
        2,
        "only the two extension lines remain"
    );
}

#[test]
fn dimcen_draws_a_center_mark_or_center_lines_for_radial_dimensions() {
    let center = point(0.0, 0.0, 0.0);
    let on_arc = point(3.0, 4.0, 0.0);
    let mark = |value: f64| {
        let mut style = style();
        style.center_mark = value;
        radial_synthesis(center, on_arc, center, false, &style)
            .expect("synthesized")
            .children
    };
    // 0: no centre mark (leader + arrow only).
    assert_eq!(line_count(&mark(0.0)), 1, "DIMCEN=0 draws nothing");
    // > 0: a two-arm centre mark of half-length DIMCEN around the centre.
    let marked = mark(2.5);
    assert_eq!(line_count(&marked), 3, "DIMCEN>0 adds a centre mark");
    // < 0: a centre line reaching past the circle (radius 5 + 2.5 = 7.5).
    let lined = mark(-2.5);
    assert_eq!(line_count(&lined), 3, "DIMCEN<0 adds centre lines");
    let reaches = |children: &[SemanticGeometry]| {
        children
            .iter()
            .filter_map(|c| match c {
                SemanticGeometry::Line { start, end } => Some(length(sub(*end, *start))),
                _ => None,
            })
            .fold(0.0_f64, f64::max)
    };
    assert!(
        approx(reaches(&marked), 5.0),
        "a centre mark spans 2 * DIMCEN"
    );
    assert!(
        approx(reaches(&lined), 15.0),
        "a centre line spans 2 * (radius + |DIMCEN|)"
    );
}

/// Review B1: for a `Diameter` dimension the group-10 slot is the **opposite
/// chord point**, not the centre, so the `DIMCEN` mark must be drawn at the
/// midpoint of the two chord points (the true centre) — not on an arrow
/// endpoint — and the centre-line reach is measured from the **radius** (half
/// the leader length), not the full diameter.
#[test]
fn dimcen_centers_a_diameter_mark_on_the_midpoint_not_the_chord_point() {
    let opposite = point(-3.0, -4.0, 0.0);
    let on_arc = point(3.0, 4.0, 0.0);
    let center = point(0.0, 0.0, 0.0);
    let definition = DimensionDefinition {
        kind: DimensionKind::Diameter,
        points: DimensionPoints::Radial {
            center: opposite,
            point_on_arc: on_arc,
            leader_length: 0.0,
        },
        text_middle: point(0.0, 0.0, 0.0),
        text_rotation: 0.0,
        normal: point(0.0, 0.0, 1.0),
    };
    // Lines as (midpoint, length): the leader (length = diameter = 10) plus the
    // two DIMCEN arms.
    let arms = |mark: f64| {
        let mut style = style();
        style.center_mark = mark;
        let geometry =
            DimensionGeometry::synthesized(definition.clone(), "Standard".to_string(), style, None)
                .expect("synthesized");
        geometry
            .display()
            .iter()
            .filter_map(|child| match child {
                SemanticGeometry::Line { start, end } => {
                    Some((scale(add(*start, *end), 0.5), length(sub(*end, *start))))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    // DIMCEN > 0: the two arms are `2 * DIMCEN` long and centred on the true
    // centre (the midpoint of the chord points), not on the group-10 slot.
    let marked = arms(2.5);
    let crosses: Vec<Point3> = marked
        .iter()
        .filter(|(_, len)| approx(*len, 5.0))
        .map(|(mid, _)| *mid)
        .collect();
    assert_eq!(crosses.len(), 2, "two centre-mark arms");
    for mid in &crosses {
        assert!(
            approx(length(sub(*mid, center)), 0.0),
            "the centre mark must sit at the midpoint of the two chord points, got {mid:?}"
        );
    }

    // DIMCEN < 0: the centre line reaches `radius + |DIMCEN| = 5 + 2.5 = 7.5`
    // from the centre (span 15), i.e. relative to the radius, not the diameter.
    let lined = arms(-2.5);
    let longest = lined.iter().map(|(_, len)| *len).fold(0.0_f64, f64::max);
    assert!(
        approx(longest, 15.0),
        "a diameter centre line spans 2 * (radius + |DIMCEN|), got {longest}"
    );
}

#[test]
fn dimrnd_rounds_the_displayed_distance_only() {
    let mut style = style();
    style.decimals = 2;
    style.rounding = 0.5;
    // 42.3 rounds to 42.5 at a 0.5 step.
    assert_eq!(
        resolve_measurement_text(None, "", "", 42.3, &style),
        "42.50"
    );
    // Group 42 (the raw measurement) is unaffected; only the text is rounded.
    let definition = aligned_definition();
    let synthesized = synthesize(&definition, &style).expect("synthesis");
    assert!(approx(synthesized.measurement, 42.0));
}

#[test]
fn dimpost_wraps_the_measurement_with_a_prefix_or_suffix() {
    let mut style = style();
    style.decimals = 2;
    style.postfix = "<> mm".to_string();
    assert_eq!(
        resolve_measurement_text(None, "", "", 42.0, &style),
        "42.00 mm"
    );
    // The posted value is what a user override's `<>` substitutes.
    assert_eq!(
        resolve_measurement_text(Some("L=<>"), "", "", 42.0, &style),
        "L=42.00 mm"
    );
}

#[test]
fn dimdsep_replaces_the_decimal_separator() {
    let mut style = style();
    style.decimals = 2;
    style.decimal_separator = 44; // ','
    assert_eq!(
        resolve_measurement_text(None, "", "", 42.0, &style),
        "42,00"
    );
}

#[test]
fn dimzin_bit_two_suppresses_the_leading_zero() {
    let mut style = style();
    style.decimals = 4;
    style.zero_suppression = 4;
    assert_eq!(
        resolve_measurement_text(None, "", "", 0.5, &style),
        ".5000",
        "DIMZIN bit 2 drops the leading zero"
    );
}

#[test]
fn dimlunit_other_than_decimal_is_a_declared_degradation() {
    // ADR 0017 F.4: a non-decimal unit format must be declared unimplemented,
    // not silently rendered as decimal.
    let mut style = style();
    style.decimals = 2;
    assert!(style.measurement_unit_degradation().is_none());
    for code in [1_i16, 3, 4, 5] {
        style.linear_unit_format = code;
        let reason = style
            .measurement_unit_degradation()
            .unwrap_or_else(|| panic!("DIMLUNIT {code} must be declared unimplemented"));
        assert!(
            reason.contains("not implemented") && reason.contains(&code.to_string()),
            "the declaration must name the format: {reason}"
        );
        // The render is the declared decimal fallback, not a silent success.
        assert_eq!(
            resolve_measurement_text(None, "", "", 42.0, &style),
            "42.00"
        );
    }
    style.linear_unit_format = 2;
    assert!(style.measurement_unit_degradation().is_none());
}
