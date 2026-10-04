//! Regression contracts for finite geometry at extreme numeric scales.

use cad_domain::{Point3, WorkPlane};
use cad_geometry::{
    distance, length, measure_polygon_area, normalize, signed_area, validate_work_plane,
};

fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

#[test]
fn vector_length_avoids_intermediate_overflow_and_underflow() {
    for magnitude in [1e200, 1e-200] {
        let vector = point(3.0 * magnitude, 4.0 * magnitude, 0.0);
        let expected = 5.0 * magnitude;
        assert!((length(vector) / expected - 1.0).abs() < 1e-14);
        assert!((distance(vector, Point3::default()) / expected - 1.0).abs() < 1e-14);
    }
}

#[test]
fn normalization_preserves_direction_for_large_finite_vectors() {
    let unit = normalize(point(3e200, 4e200, 0.0));
    assert!(distance(unit, point(0.6, 0.8, 0.0)) < 1e-14);

    // Even when the actual norm is unrepresentable, the unit vector is finite.
    let unit = normalize(point(f64::MAX, f64::MAX, 0.0));
    let component = std::f64::consts::FRAC_1_SQRT_2;
    assert!(distance(unit, point(component, component, 0.0)) < 1e-14);
}

#[test]
fn translated_polygon_preserves_area_and_winding() {
    let offset = 1e12;
    let mut ring = [
        point(offset, offset, 0.0),
        point(offset + 3.0, offset, 0.0),
        point(offset + 3.0, offset + 2.0, 0.0),
        point(offset, offset + 2.0, 0.0),
    ];
    let original = ring;
    assert_eq!(signed_area(&ring), 6.0);
    assert_eq!(measure_polygon_area(&ring, 1e-6), Ok(6.0));
    assert_eq!(ring, original);
    ring.reverse();
    assert_eq!(signed_area(&ring), -6.0);
    assert_eq!(measure_polygon_area(&ring, 1e-6), Ok(6.0));
}

#[test]
fn large_work_plane_basis_does_not_bypass_orthogonality_validation() {
    for magnitude in [1e200, f64::MAX] {
        let parallel = WorkPlane {
            origin: Point3::default(),
            u: point(magnitude, magnitude, 0.0),
            v: point(magnitude, magnitude, 0.0),
        };
        assert!(validate_work_plane(&parallel, 1e-6).is_err());

        let orthogonal = WorkPlane {
            origin: Point3::default(),
            u: point(magnitude, magnitude, 0.0),
            v: point(-magnitude, magnitude, 0.0),
        };
        assert!(validate_work_plane(&orthogonal, 1e-6).is_ok());
    }
}
