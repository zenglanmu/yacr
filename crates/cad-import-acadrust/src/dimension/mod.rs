//! Dimension display geometry synthesis.
//!
//! A DXF/DWG DIMENSION may carry a pre-rendered anonymous block (`*D...`); when
//! it does, the importer expands it into the dimension's display. Many
//! producers (including the committed QCAD `flange` sample) store only the
//! definition points and omit the block. Without synthesis those dimensions
//! have no display geometry and are reported as unsupported.
//!
//! The seven subtypes the domain variant models (ADR 0017 A.1/A.3) are built by
//! `cad-domain`'s shared synthesis (`DimensionGeometry::synthesized`), so the
//! block-less import and the creation path share one builder. Arc-length and
//! large-radial dimensions, which the ADR kind set does not cover, keep the
//! pre-P5a compound synthesis (see [`crate::dimension::angular`]).

use super::*;

mod angular;
mod convert;
mod multileader;
mod values;

#[allow(unused_imports)]
use angular::*;
#[allow(unused_imports)]
use convert::*;
#[allow(unused_imports)]
use multileader::*;
#[allow(unused_imports)]
use values::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> DimensionStyleValues {
        // The pinned metric Standard, so these synthesis tests exercise the same
        // resolved values the product uses (ADR 0017 N7).
        cad_db::DimStyle::standard_metric(DimStyleId(0)).resolved(
            "Standard".to_string(),
            StyleId(0),
            None,
        )
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
        let geometry = linear_synthesis(
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
            linear_synthesis(first, second, definition, direction, &style()).expect("synthesized");
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
        let geometry =
            radial_synthesis(center, point, center, false, &style()).expect("synthesized");
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
        let center = Point3 {
            x: (a.x + b.x) / 2.0,
            y: (a.y + b.y) / 2.0,
            z: 0.0,
        };
        let geometry = radial_synthesis(a, b, center, true, &style()).expect("synthesized");
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
    fn angular_dimension_measures_the_arc_and_draws_arrows() {
        let vertex = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let geometry = angular_synthesis(
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
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
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
        let geometry = ordinate_synthesis(
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
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            false,
        )
        .expect("ordinate geometry");
        assert!(approx(geometry.measurement, 3.0));
    }

    #[test]
    fn ordinate_dimension_measures_relative_to_a_non_origin_datum() {
        // Review B2: the ordinate measurement is datum-relative, matching the
        // documented AutoCAD semantics and acadrust's writer.
        let geometry = ordinate_synthesis(
            Point3 {
                x: 15.0,
                y: 8.0,
                z: 0.0,
            },
            Point3 {
                x: 15.0,
                y: 12.0,
                z: 0.0,
            },
            Point3 {
                x: 5.0,
                y: 5.0,
                z: 0.0,
            },
            true,
        )
        .expect("ordinate geometry");
        assert!(
            approx(geometry.measurement, 10.0),
            "{}",
            geometry.measurement
        );
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
