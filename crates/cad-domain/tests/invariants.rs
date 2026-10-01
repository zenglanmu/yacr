//! Contract tests for transform-level numerical invariants (audit B23/B10).
//!
//! These live in `cad-domain` because the predicates are properties of the
//! transform itself, not of any particular geometry engine.

use cad_domain::*;

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

#[test]
fn identity_is_uniform_and_has_unit_scale() {
    let t = Transform3::identity();
    assert!(t.is_uniform_scale(1e-9));
    assert!((t.max_scale() - 1.0).abs() < 1e-12);
    assert!((t.determinant() - 1.0).abs() < 1e-12);
}

#[test]
fn uniform_scale_and_rotation_are_uniform() {
    let mut m = Transform3::scale(2.0).matrix;
    // 90° rotation about Z composed with the 2x scale.
    m[0][0] = 0.0;
    m[0][1] = -2.0;
    m[1][0] = 2.0;
    m[1][1] = 0.0;
    let t = Transform3 { matrix: m };
    assert!(t.is_uniform_scale(1e-9));
    assert!((t.max_scale() - 2.0).abs() < 1e-12);
    // Rotation about Z preserves the scale in every axis: det = 2^3 = 8.
    assert!((t.determinant() - 8.0).abs() < 1e-9);
}

#[test]
fn non_uniform_scale_is_not_uniform() {
    let mut m = Transform3::identity().matrix;
    m[0][0] = 2.0;
    m[1][1] = 1.0;
    let t = Transform3 { matrix: m };
    assert!(!t.is_uniform_scale(1e-9));
}

#[test]
fn shear_is_not_uniform() {
    let mut m = Transform3::identity().matrix;
    m[0][1] = 1.0; // shear
    let t = Transform3 { matrix: m };
    assert!(!t.is_uniform_scale(1e-9));
}

#[test]
fn singular_matrix_has_zero_determinant_and_is_not_uniform() {
    let mut m = Transform3::identity().matrix;
    m[0][0] = 0.0;
    m[1][1] = 0.0;
    let t = Transform3 { matrix: m };
    assert!(t.determinant().abs() < 1e-12);
    assert!(!t.is_uniform_scale(1e-9));
}

#[test]
fn mirror_keeps_uniformity_but_flips_determinant_sign() {
    let mut m = Transform3::identity().matrix;
    m[0][0] = -1.0; // reflection
    let t = Transform3 { matrix: m };
    assert!(t.is_uniform_scale(1e-9));
    assert!(t.determinant() < 0.0);
}

#[test]
fn non_finite_matrix_is_not_uniform_and_max_scale_reports_it() {
    let mut m = Transform3::identity().matrix;
    m[0][0] = f64::INFINITY;
    let t = Transform3 { matrix: m };
    assert!(!t.is_uniform_scale(1e-9));
    assert!(t.max_scale().is_infinite());
}

#[test]
fn transform_composition_matches_sequential_application() {
    // `matrix_mul` is documented as "apply rhs first, then self"; verify the
    // round-trip and that translation composes additively.
    let translate = Transform3::translation(p(5.0, -2.0));
    let scale = Transform3::scale(3.0);
    let period = p(1.0, 2.0);
    let composed = scale.matrix_mul(&translate);
    let sequential = scale.apply_point(translate.apply_point(period));
    let via_composed = composed.apply_point(period);
    assert_eq!(sequential, via_composed);
    assert_eq!(via_composed, p(18.0, 0.0));
}

#[test]
fn inverse_of_a_uniform_transform_round_trips() {
    // Inverse of scale(2) then translate is built explicitly; applying both
    // must return the original point (round-trip invariant).
    let t = Transform3::translation(p(10.0, 20.0)).matrix_mul(&Transform3::scale(2.0));
    let inv_scale = Transform3::scale(0.5);
    let inv_translate = Transform3::translation(p(-10.0, -20.0));
    let inv = inv_scale.matrix_mul(&inv_translate);
    let q = t.apply_point(p(3.0, 4.0));
    let back = inv.apply_point(q);
    assert!(cad_domain_close(back, p(3.0, 4.0)), "{back:?}");
}

fn cad_domain_close(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9
}
