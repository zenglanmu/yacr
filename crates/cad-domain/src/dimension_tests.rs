//! Dimension unit tests (split out to keep `dimension.rs` under the file-size
//! limit).

use crate::dimension::*;
use crate::dimension_synthesis::synthesize;
use crate::*;

fn style() -> DimensionStyleValues {
    DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        text_above: true,
        linear_factor: 1.0,
        decimals: 4,
        suppress_trailing_zeros: false,
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
    style.suppress_trailing_zeros = true;
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
