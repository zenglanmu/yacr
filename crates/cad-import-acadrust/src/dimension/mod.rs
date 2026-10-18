//! Dimension display geometry synthesis.
//!
//! A DXF/DWG DIMENSION may carry a pre-rendered anonymous block (`*D...`); when
//! it does, the importer expands it as an INSERT. Many producers (including the
//! committed QCAD `flange` sample) store only the definition points and omit
//! the block. Without synthesis those dimensions have no display geometry and
//! are reported as unsupported.
//!
//! This module builds the standard linear/aligned, radius/diameter, angular,
//! ordinate, arc-length and large-radius geometry from the dimension points and
//! the resolved DIMSTYLE: extension lines, the dimension line, solid arrowheads,
//! the arc-length leader and the measurement text. Every subtype is synthesized;
//! only the large-radius jog is reported `Partial`, because its true zigzag is
//! approximated by a segment oriented from `jog_angle`. Nothing here claims
//! compatibility: it is a display representation for geometry that is already in
//! the database.

use super::*;

mod angular;
mod convert;
mod geom;
mod linear;
mod multileader;
mod values;

// Re-export every moved module-scope item so the names that resolved inside the
// former single-file `dimension` module keep resolving, including through the
// crate-root `use dimension::*` (`line_between` is used by `crate::primitives`).
#[allow(unused_imports)]
use angular::*;
#[allow(unused_imports)]
use convert::*;
#[allow(unused_imports)]
use linear::*;
#[allow(unused_imports)]
use multileader::*;
#[allow(unused_imports)]
use values::*;
// Explicit import wins over the glob below, so `line_between` stays reachable
// at crate scope.
pub(crate) use geom::line_between;
#[allow(unused_imports)]
use geom::*;

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::tables::DimStyle;

    fn style() -> DimStyleValues {
        DimStyleValues::from_style(&DimStyle::standard(), "Standard".to_string())
    }

    fn line_ends(children: &[SemanticGeometry]) -> Vec<(Point3, Point3)> {
        children
            .iter()
            .filter_map(|child| match child {
                SemanticGeometry::Line { start, end } => Some((*start, *end)),
                _ => None,
            })
            .collect()
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn approx_point(a: Point3, b: Point3) -> bool {
        approx(a.x, b.x) && approx(a.y, b.y) && approx(a.z, b.z)
    }

    #[test]
    fn aligned_dimension_builds_extension_lines_dimension_line_and_arrows() {
        let first = Point3 {
            x: -21.0,
            y: 0.0,
            z: 0.0,
        };
        let second = Point3 {
            x: 21.0,
            y: 0.0,
            z: 0.0,
        };
        let definition = Point3 {
            x: -21.0,
            y: -30.0,
            z: 0.0,
        };
        // Use the QCAD flange's DIMTXT/DIMGAP so the placement matches.
        let mut style = style();
        style.text_height = 2.5;
        style.text_gap = 0.625;
        let geometry = linear_geometry(
            first,
            second,
            definition,
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            &style,
        )
        .expect("synthesized");
        assert!(approx(geometry.measurement, 42.0));
        // Two extension lines plus the dimension line.
        assert_eq!(line_ends(&geometry.children).len(), 3);
        // Two solid arrowheads.
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
                .count(),
            2
        );
        // Default text sits just above the dimension line, toward the geometry.
        let expected = Point3 {
            x: 0.0,
            y: -28.125,
            z: 0.0,
        };
        assert!(
            approx_point(geometry.text_position, expected),
            "{:?}",
            geometry.text_position
        );
        assert!(approx(geometry.text_rotation, 0.0));
    }

    #[test]
    fn vertical_linear_dimension_rotates_text() {
        let first = Point3 {
            x: 30.0,
            y: 3.0,
            z: 0.0,
        };
        let second = Point3 {
            x: 30.0,
            y: -3.0,
            z: 0.0,
        };
        let definition = Point3 {
            x: 45.0,
            y: 3.0,
            z: 0.0,
        };
        // A 90° linear dimension measures along +Y.
        let direction = Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        };
        let geometry =
            linear_geometry(first, second, definition, direction, &style()).expect("synthesized");
        assert!(approx(geometry.measurement, 6.0));
        assert!(
            approx(geometry.text_rotation, 90.0_f64.to_radians()),
            "{:?}",
            geometry.text_rotation
        );
    }

    #[test]
    fn radius_dimension_measures_the_leader() {
        let center = Point3 {
            x: 30.0,
            y: 0.0,
            z: 0.0,
        };
        let point = Point3 {
            x: 33.0,
            y: -4.0,
            z: 0.0,
        };
        let geometry = radial_geometry(center, point, false, &style()).expect("synthesized");
        assert!(approx(geometry.measurement, 5.0));
        // One leader line and one arrow.
        assert_eq!(line_ends(&geometry.children).len(), 1);
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
    fn diameter_dimension_has_two_arrows() {
        let a = Point3 {
            x: -13.64147673972468,
            y: -3.14803309378262,
            z: 0.0,
        };
        let b = Point3 {
            x: 13.64147673972468,
            y: 3.14803309378262,
            z: 0.0,
        };
        let geometry = radial_geometry(a, b, true, &style()).expect("synthesized");
        assert!(approx(geometry.measurement, 28.0));
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
                .count(),
            2
        );
    }

    #[test]
    fn measurement_text_suppresses_trailing_zeros_and_replaces_placeholder() {
        let mut style = style();
        style.decimals = 2;
        style.suppress_trailing_zeros = true;
        let base = DimensionBase::new(acadrust::entities::DimensionType::Aligned);
        assert_eq!(resolve_measurement_text(&base, "", "", 42.0, &style), "42");
        assert_eq!(resolve_measurement_text(&base, "R", "", 5.0, &style), "R5");
        assert_eq!(
            resolve_measurement_text(&base, "", "\u{00B0}", 45.0, &style),
            "45°"
        );
        let mut overriding = base.clone();
        overriding.set_text_override(Some("R<>".to_string()));
        assert_eq!(
            resolve_measurement_text(&overriding, "R", "", 5.0, &style),
            "R5"
        );
    }

    #[test]
    fn angular_dimension_measures_the_arc_and_draws_arrows() {
        let vertex = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let geometry = angular_geometry(
            vertex,
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            5.0,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            &style(),
        )
        .expect("angular geometry");
        assert!(
            approx(geometry.measurement, 90.0),
            "{}",
            geometry.measurement
        );
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Arc { .. }))
                .count(),
            1
        );
        assert_eq!(
            geometry
                .children
                .iter()
                .filter(|c| matches!(c, SemanticGeometry::Mesh(_)))
                .count(),
            2
        );
    }

    #[test]
    fn ordinate_dimension_reports_the_feature_coordinate() {
        let geometry = ordinate_geometry(
            Point3 {
                x: 10.0,
                y: 3.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 8.0,
                z: 0.0,
            },
            false,
        )
        .expect("ordinate geometry");
        assert!(approx(geometry.measurement, 3.0));
    }

    #[test]
    fn arc_length_dimension_measures_the_arc() {
        let geometry = arc_length_geometry(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 5.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 5.0,
                z: 0.0,
            },
            0.0,
            std::f64::consts::FRAC_PI_2,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            &style(),
            None,
        )
        .expect("arc length geometry");
        assert!(approx(
            geometry.measurement,
            5.0 * std::f64::consts::FRAC_PI_2
        ));
    }
}
