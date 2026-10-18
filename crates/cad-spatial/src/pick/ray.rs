use cad_domain::*;

use super::math::{add, cross, dot3, is_finite_point, length3, scale3, sub};
use super::types::{BackFacePolicy, MIN_RAY_DIRECTION};

/// Validate a ray: finite origin/direction and a non-degenerate direction.
pub fn validate_ray(ray: &Ray3) -> CadResult<()> {
    if !is_finite_point(ray.origin) {
        return Err(CadError::InvalidInput("ray origin is not finite".into()));
    }
    if !is_finite_point(ray.direction) {
        return Err(CadError::InvalidInput("ray direction is not finite".into()));
    }
    if length3(ray.direction) < MIN_RAY_DIRECTION {
        return Err(CadError::InvalidInput("ray direction is degenerate".into()));
    }
    Ok(())
}

/// Closest approach between a ray and a segment.
///
/// Returns `(t, point_on_segment, distance)` where `t` is the parameter along
/// the ray (not clamped, so a caller can reject hits behind the origin) and
/// `point_on_segment` is the nearest point of the segment `a..b`. A degenerate
/// segment is treated as the single point `a`.
pub fn ray_segment_closest(ray: &Ray3, a: Point3, b: Point3) -> Option<(f64, Point3, f64)> {
    let d = ray.direction;
    let a1 = dot3(d, d);
    if !a1.is_finite() || a1 < MIN_RAY_DIRECTION * MIN_RAY_DIRECTION {
        return None;
    }
    let ab = sub(b, a);
    let c1 = dot3(ab, ab);
    let w0 = sub(ray.origin, a);
    let d1 = dot3(d, w0);
    let e1 = dot3(ab, w0);

    let s = if c1 < 1e-24 {
        // Degenerate segment: the nearest point is `a`.
        0.0
    } else {
        let denom = a1 * c1 - dot3(d, ab) * dot3(d, ab);
        let s = if denom.abs() < 1e-18 {
            // Near-parallel: project the ray origin onto the segment.
            e1 / c1
        } else {
            (a1 * e1 - dot3(d, ab) * d1) / denom
        };
        s.clamp(0.0, 1.0)
    };
    let point = add(a, scale3(ab, s));
    // The ray parameter of the closest point on the ray to `point`.
    let t = dot3(sub(point, ray.origin), d) / a1;
    let closest_on_ray = add(ray.origin, scale3(d, t));
    let dist = length3(sub(point, closest_on_ray));
    Some((t, point, dist))
}

/// Möller–Trumbore ray/triangle intersection.
///
/// Returns the ray parameter `t >= 0` of the intersection, or `None` for a
/// parallel/degenerate triangle or a miss. `policy` decides whether a triangle
/// whose world outward normal faces away from the origin is ignored.
pub fn ray_triangle(
    ray: &Ray3,
    v0: Point3,
    v1: Point3,
    v2: Point3,
    policy: BackFacePolicy,
) -> Option<f64> {
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let n = cross(e1, e2);
    if length3(n) < 1e-18 {
        // Degenerate triangle (zero area): never a hit.
        return None;
    }
    if policy == BackFacePolicy::Cull && dot3(n, ray.direction) >= 0.0 {
        // The outward normal points away from (or along) the ray: back face.
        return None;
    }
    let pvec = cross(ray.direction, e2);
    let det = dot3(e1, pvec);
    if det.abs() < 1e-15 {
        return None;
    }
    let inv = 1.0 / det;
    let tvec = sub(ray.origin, v0);
    let u = dot3(tvec, pvec) * inv;
    if !(-1e-12..=1.0 + 1e-12).contains(&u) {
        return None;
    }
    let qvec = cross(tvec, e1);
    let v = dot3(ray.direction, qvec) * inv;
    if v < -1e-12 || u + v > 1.0 + 1e-12 {
        return None;
    }
    let t = dot3(e2, qvec) * inv;
    if !t.is_finite() || t < 0.0 {
        return None;
    }
    Some(t)
}
