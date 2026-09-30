//! Rays for 3D picking (spec v2.0 §16.1, §18.3).
//!
//! A 3D screen click becomes a ray; intersections return real world points or
//! nothing. Unhit picks must not fabricate a spatial point.

use glam::DVec3;

/// A world-space ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: DVec3,
    pub direction: DVec3,
}

impl Ray {
    pub fn new(origin: DVec3, direction: DVec3) -> Option<Self> {
        let len = direction.length();
        if len < 1e-12 {
            return None;
        }
        Some(Ray { origin, direction: direction / len })
    }

    pub fn at(&self, t: f64) -> DVec3 {
        self.origin + self.direction * t
    }
}

/// Intersect a ray with an infinite plane. Returns the ray parameter `t`.
pub fn ray_plane_intersection(ray: &Ray, plane_point: DVec3, plane_normal: DVec3) -> Option<f64> {
    let denom = ray.direction.dot(plane_normal);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = (plane_point - ray.origin).dot(plane_normal) / denom;
    if t.is_finite() && t >= 0.0 {
        Some(t)
    } else {
        None
    }
}

/// Distance from a ray to a finite segment, and the closest points.
///
/// Used for 3D wireframe picking. Returns `(ray_t, segment_t, distance)`.
pub fn ray_segment_distance(ray: &Ray, a: DVec3, b: DVec3) -> Option<(f64, f64, f64)> {
    let u = ray.direction;
    let v = b - a;
    let w0 = ray.origin - a;
    let aa = u.dot(u);
    let bb = u.dot(v);
    let cc = v.dot(v);
    let dd = u.dot(w0);
    let ee = v.dot(w0);
    let denom = aa * cc - bb * bb;
    let (sc, tc) = if denom.abs() < 1e-12 {
        (0.0, if cc > 1e-12 { ee / cc } else { 0.0 })
    } else {
        ((bb * ee - cc * dd) / denom, (aa * ee - bb * dd) / denom)
    };
    let sc = sc.max(0.0);
    let tc = tc.clamp(0.0, 1.0);
    let p_ray = ray.origin + u * sc;
    let p_seg = a + v * tc;
    let d = (p_ray - p_seg).length();
    if d.is_finite() {
        Some((sc, tc, d))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_plane_at_expected_point() {
        let ray = Ray::new(DVec3::new(0.0, 0.0, 5.0), DVec3::new(0.0, 0.0, -1.0)).unwrap();
        let t = ray_plane_intersection(&ray, DVec3::ZERO, DVec3::Z).unwrap();
        assert!((ray.at(t) - DVec3::ZERO).length() < 1e-9);
    }

    #[test]
    fn parallel_ray_misses() {
        let ray = Ray::new(DVec3::ZERO, DVec3::X).unwrap();
        assert!(ray_plane_intersection(&ray, DVec3::new(0.0, 1.0, 0.0), DVec3::Z).is_none());
    }

    #[test]
    fn ray_segment_distance_is_zero_when_crossing() {
        let ray = Ray::new(DVec3::new(0.0, -1.0, 0.0), DVec3::Y).unwrap();
        let (_, _, d) = ray_segment_distance(&ray, DVec3::new(-1.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0)).unwrap();
        assert!(d < 1e-9);
    }
}
