//! Radial (Radius/Diameter) DIMENSION group-order contracts (ADR 0017 A.3,
//! P5 gate F1).
//!
//! Split from `dimension.rs` to keep both files under the 1000-line code limit.

use super::dimension::{dimensions, style_values};
use super::*;

/// The `(group 10, group 15)` points of the first DIMENSION record, read from
/// the raw group pairs rather than through acadrust's reader, so a written group
/// order can be asserted independently of the reader's field mapping.
///
/// `write_point3d` writes `x`, then `x + 10` (y), then `x + 20` (z) as
/// consecutive pairs, so group 10 is `10/20/30` and group 15 is `15/25/35`.
fn dimension_group_10_15(bytes: &[u8]) -> ([f64; 3], [f64; 3]) {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let start = (0..lines.len().saturating_sub(1))
        .find(|&i| lines[i].trim() == "0" && lines[i + 1].trim() == "DIMENSION")
        .expect("a DIMENSION record was written");
    let triple_at = |index: usize, codes: [&str; 3]| -> Option<[f64; 3]> {
        let mut values = [0.0f64; 3];
        for (offset, expected) in codes.iter().enumerate() {
            let at = index + offset * 2;
            if at + 1 >= lines.len() || lines[at].trim() != *expected {
                return None;
            }
            values[offset] = lines[at + 1].trim().parse().ok()?;
        }
        Some(values)
    };
    let mut point_10 = None;
    let mut point_15 = None;
    let mut j = start + 2;
    while j + 1 < lines.len() {
        let code = lines[j].trim();
        if code == "0" {
            break;
        }
        if point_10.is_none() && code == "10" {
            point_10 = triple_at(j, ["10", "20", "30"]);
        } else if point_15.is_none() && code == "15" {
            point_15 = triple_at(j, ["15", "25", "35"]);
        }
        j += 2;
    }
    (
        point_10.expect("the DIMENSION record writes group 10"),
        point_15.expect("the DIMENSION record writes group 15"),
    )
}

fn triple_point(value: [f64; 3]) -> Point3 {
    p(value[0], value[1], value[2])
}

/// P5 gate F1: the radial kinds write the ADR 0017 A.3 group order.
///
/// Radius: group 10 = centre, group 15 = the point on the circle. Diameter:
/// group 15 = the first chord point (our `point_on_arc`), group 10 = its
/// **opposite** (our `center` slot). The assertion is on the raw group values,
/// so the pre-F1 writer that carried the two points in the reverse order — and
/// was self-consistent with a mirrored importer, hence invisible to the round
/// trip — fails here.
#[test]
fn radial_dimensions_write_the_documented_group_10_and_15_order() {
    let radial = |kind: DimensionKind, center: Point3, point_on_arc: Point3| {
        DimensionGeometry::synthesized(
            DimensionDefinition {
                kind,
                points: DimensionPoints::Radial {
                    center,
                    point_on_arc,
                    leader_length: 0.0,
                },
                text_middle: p(0.0, 0.0, 0.0),
                text_rotation: 0.0,
                normal: p(0.0, 0.0, 1.0),
            },
            "Standard".to_string(),
            style_values(),
            None,
        )
        .expect("synthesized")
    };

    // Radius: `center` is the true centre and belongs in group 10.
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(radial(
        DimensionKind::Radius,
        p(1.0, 2.0, 0.0),
        p(1.0, 7.0, 0.0),
    ))));
    let (ten, fifteen) = dimension_group_10_15(&bytes);
    assert_point(triple_point(ten), p(1.0, 2.0, 0.0));
    assert_point(triple_point(fifteen), p(1.0, 7.0, 0.0));

    // Diameter: group 15 is the first chord point, group 10 its opposite.
    let (bytes, _report) = export_one(SemanticGeometry::Dimension(Box::new(radial(
        DimensionKind::Diameter,
        p(-4.0, 0.0, 0.0),
        p(4.0, 0.0, 0.0),
    ))));
    let (ten, fifteen) = dimension_group_10_15(&bytes);
    assert_point(triple_point(fifteen), p(4.0, 0.0, 0.0));
    assert_point(triple_point(ten), p(-4.0, 0.0, 0.0));
}

/// P5 gate F1 (import direction): a file that follows the documented convention
/// — group 15 the first chord point, group 10 its opposite — fills the domain
/// slots the right way round: `point_on_arc` from group 15, the group-10 slot
/// (`center`) from group 10. An importer that kept mirroring the old export bug
/// would put `point_on_arc` at the opposite point and fail here.
#[test]
fn a_conventional_diameter_imports_with_the_points_the_right_way_round() {
    let dxf = "0\nSECTION\n2\nENTITIES\n\
0\nDIMENSION\n5\n2F\n8\n0\n2\n*D1\n\
10\n-4.0\n20\n0.0\n30\n0.0\n\
11\n0.0\n21\n0.0\n31\n0.0\n\
70\n3\n42\n8.0\n3\nStandard\n\
15\n4.0\n25\n0.0\n35\n0.0\n40\n0.0\n\
0\nENDSEC\n0\nEOF\n";
    let imported = import_bytes(dxf.as_bytes());
    let dimensions = dimensions(&imported);
    assert_eq!(
        dimensions.len(),
        1,
        "the conventional diameter imports as a DIMENSION"
    );
    let dimension = &dimensions[0];
    assert_eq!(dimension.kind(), DimensionKind::Diameter);
    match &dimension.definition().points {
        DimensionPoints::Radial {
            center,
            point_on_arc,
            ..
        } => {
            assert_point(*point_on_arc, p(4.0, 0.0, 0.0));
            assert_point(*center, p(-4.0, 0.0, 0.0));
        }
        other => panic!("expected radial definition points, got {other:?}"),
    }
}
