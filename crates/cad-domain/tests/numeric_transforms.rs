//! Synthetic regression contracts for finite transforms and unit conversion.

use cad_domain::{Point3, Transform3, UnitContext};

#[test]
fn similarity_classification_is_stable_at_extreme_finite_scales() {
    for scale in [1e-300, 1e-200, 1.0, 1e200, 1e300] {
        assert!(Transform3::scale(scale).is_uniform_scale(1e-9));
        assert!(Transform3::scale(-scale).is_uniform_scale(1e-9));

        let mut non_uniform = Transform3::scale(scale);
        non_uniform.matrix[1][1] *= 0.5;
        assert!(!non_uniform.is_uniform_scale(1e-9));

        let mut shear = Transform3::scale(scale);
        shear.matrix[0][1] = scale;
        assert!(!shear.is_uniform_scale(1e-9));
    }
}

#[test]
fn oblique_rotation_remains_uniform_at_extreme_finite_scales() {
    let angle = std::f64::consts::FRAC_PI_4;
    for scale in [1e-300, 1.0, 1e300] {
        let mut transform = Transform3::scale(scale);
        transform.matrix[0][0] = scale * angle.cos();
        transform.matrix[0][1] = -scale * angle.sin();
        transform.matrix[1][0] = scale * angle.sin();
        transform.matrix[1][1] = scale * angle.cos();
        assert!(transform.is_uniform_scale(1e-9));
    }
}

#[test]
fn nan_linear_coefficients_are_not_hidden_by_max_scale() {
    for row in 0..3 {
        for column in 0..3 {
            let mut transform = Transform3::identity();
            transform.matrix[row][column] = f64::NAN;
            assert!(transform.max_scale().is_nan());
            assert!(!transform.is_uniform_scale(1e-9));
        }
    }
}

#[test]
fn invalid_similarity_tolerances_are_rejected() {
    for tolerance in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(!Transform3::identity().is_uniform_scale(tolerance));
    }
    assert!(Transform3::identity().is_uniform_scale(0.0));
}

#[test]
fn loose_tolerance_does_not_accept_exactly_singular_linear_parts() {
    let mut zero_column = Transform3::identity();
    zero_column.matrix[0][0] = 0.0;
    assert!(!zero_column.is_uniform_scale(2.0));

    let mut dependent_columns = Transform3::identity();
    dependent_columns.matrix[0][1] = 1.0;
    dependent_columns.matrix[1][1] = 0.0;
    assert!(!dependent_columns.is_uniform_scale(2.0));
}

#[test]
fn three_dimensional_affine_composition_preserves_application_order() {
    let translation = Transform3::translation(Point3 {
        x: 10.0,
        y: -20.0,
        z: 30.0,
    });
    let mut linear = Transform3::identity();
    linear.matrix[0][0] = -2.0;
    linear.matrix[0][1] = 0.5;
    linear.matrix[1][1] = 3.0;
    linear.matrix[2][2] = 4.0;
    let point = Point3 {
        x: 2.0,
        y: 4.0,
        z: 8.0,
    };
    assert_eq!(
        translation.matrix_mul(&linear).apply_point(point),
        translation.apply_point(linear.apply_point(point))
    );
    assert_eq!(
        linear.matrix_mul(&translation).apply_point(point),
        linear.apply_point(translation.apply_point(point))
    );
}

#[test]
fn unit_conversion_rejects_unknown_or_invalid_ratios() {
    assert_eq!(UnitContext::drawing_units().to_display(12.0), None);
    for ratio in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut units = UnitContext::from_meter();
        units.display_per_source = Some(ratio);
        assert_eq!(units.to_display(12.0), None);
    }
}

#[test]
fn unit_conversion_rejects_non_finite_inputs_and_overflow() {
    let mut units = UnitContext::from_meter();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(units.to_display(value), None);
    }
    units.display_per_source = Some(2.0);
    assert_eq!(units.to_display(f64::MAX), None);
    assert_eq!(units.to_display(-f64::MAX), None);
}

#[test]
fn valid_unit_conversion_preserves_signed_and_zero_values() {
    let mut units = UnitContext::from_meter();
    units.display_per_source = Some(1000.0);
    assert_eq!(units.to_display(1.25), Some(1250.0));
    assert_eq!(units.to_display(-1.25), Some(-1250.0));
    assert_eq!(units.to_display(0.0), Some(0.0));
    units.display_per_source = Some(1e-300);
    assert!((units.to_display(1e300).unwrap() - 1.0).abs() < 1e-12);
}
