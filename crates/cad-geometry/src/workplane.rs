//! workplane module.

use super::*;

/// Validate that a [`WorkPlane`] is a usable orthonormal frame.
///
/// A measurement work plane must have finite, non-degenerate and mutually
/// orthogonal `u`/`v`; a skewed or non-finite basis silently distorts projected
/// area, so callers must reject it (audit B24).
pub fn validate_work_plane(plane: &WorkPlane, tolerance: f64) -> CadResult<()> {
    if !is_finite(plane.origin) || !is_finite(plane.u) || !is_finite(plane.v) {
        return Err(CadError::InvalidInput(
            "work plane has non-finite values".to_string(),
        ));
    }
    let lu = length(plane.u);
    let lv = length(plane.v);
    let tol = tolerance.max(1e-12);
    if lu < tol || lv < tol {
        return Err(CadError::InvalidInput(
            "work plane basis vectors are degenerate".to_string(),
        ));
    }
    let ortho = dot(plane.u, plane.v).abs() / (lu * lv);
    if ortho > tol.max(1e-9) {
        return Err(CadError::InvalidInput(
            "work plane basis is not orthogonal".to_string(),
        ));
    }
    Ok(())
}

/// Unsigned distance of `p` from the plane through `origin` with unit `normal`.
pub fn distance_to_plane(p: Point3, origin: Point3, normal: Point3) -> f64 {
    let n = normalize(normal);
    dot(sub(p, origin), n).abs()
}
