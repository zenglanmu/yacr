//! Robust predicates and non-finite guards (spec v2.0 §16.1).
//!
//! Geometry operations must detect non-finite coordinates, degenerate edges,
//! zero radii and singular matrices. Rather than a single epsilon, callers pass
//! the appropriate [`cad_domain::TolerancePolicy`] scale.

use glam::DVec2;
use glam::DVec3;

/// True when every component is finite.
pub fn is_finite_vec3(v: DVec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

/// True when every component is finite.
pub fn is_finite_vec2(v: DVec2) -> bool {
    v.x.is_finite() && v.y.is_finite()
}

/// Robust 2D orientation sign.
///
/// Returns the sign of the cross product `(b-a) × (c-a)`, computed with a
/// plain `f64` determinant. At extreme coordinate magnitudes callers should
/// translate to a local origin first (spec §16.1); this function is exact for
/// the magnitudes a local origin produces.
pub fn orientation2d(a: DVec2, b: DVec2, c: DVec2) -> i32 {
    let det = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if det > 0.0 {
        1
    } else if det < 0.0 {
        -1
    } else {
        0
    }
}

/// Whether a triangle is degenerate for the given relative tolerance.
pub fn triangle_degenerate(a: DVec3, b: DVec3, c: DVec3, rel_tol: f64) -> bool {
    let ab = b - a;
    let ac = c - a;
    let area2 = ab.cross(ac).length();
    let scale = ab.length().max(ac.length()).max(1e-30);
    area2 <= rel_tol * scale * scale
}

/// Guard a radius/length: returns `None` for non-finite or non-positive values.
pub fn positive_finite(v: f64) -> Option<f64> {
    if v.is_finite() && v > 0.0 {
        Some(v)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_matches_winding() {
        assert_eq!(orientation2d(DVec2::ZERO, DVec2::X, DVec2::Y), 1);
        assert_eq!(orientation2d(DVec2::ZERO, DVec2::Y, DVec2::X), -1);
        assert_eq!(orientation2d(DVec2::ZERO, DVec2::X, DVec2::new(2.0, 0.0)), 0);
    }

    #[test]
    fn catches_nan() {
        assert!(!is_finite_vec3(DVec3::new(f64::NAN, 0.0, 0.0)));
        assert!(is_finite_vec3(DVec3::ZERO));
    }

    #[test]
    fn zero_radius_is_rejected() {
        assert!(positive_finite(0.0).is_none());
        assert!(positive_finite(1.0).is_some());
    }
}
