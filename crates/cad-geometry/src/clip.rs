//! Rectangular clipping for paper-space viewports and viewport culling.
//!
//! Complex/polygonal viewport clipping is out of initial scope (spec §3.2) and
//! must be reported as partial by the importer; only the rectangle is handled.

use glam::DVec2;

/// Liang–Barsky segment clip against an axis-aligned rectangle.
pub fn clip_segment_to_rect(a: DVec2, b: DVec2, min: DVec2, max: DVec2) -> Option<(DVec2, DVec2)> {
    let d = b - a;
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    let checks = [(-d.x, a.x - min.x), (d.x, max.x - a.x), (-d.y, a.y - min.y), (d.y, max.y - a.y)];
    for (p, q) in checks {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                if r > t0 {
                    t0 = r;
                }
            } else {
                if r < t0 {
                    return None;
                }
                if r < t1 {
                    t1 = r;
                }
            }
        }
    }
    Some((a + d * t0, a + d * t1))
}

#[derive(Clone, Copy)]
enum HalfPlane {
    Min { axis: usize, bound: f64 },
    Max { axis: usize, bound: f64 },
}

impl HalfPlane {
    fn coord(&self, p: DVec2) -> f64 {
        match self {
            HalfPlane::Min { axis: 0, .. } | HalfPlane::Max { axis: 0, .. } => p.x,
            _ => p.y,
        }
    }

    fn bound(&self) -> f64 {
        match self {
            HalfPlane::Min { bound, .. } | HalfPlane::Max { bound, .. } => *bound,
        }
    }

    fn contains(&self, p: DVec2) -> bool {
        match self {
            HalfPlane::Min { .. } => self.coord(p) >= self.bound(),
            HalfPlane::Max { .. } => self.coord(p) <= self.bound(),
        }
    }
}

/// Sutherland–Hodgman polygon clip against an axis-aligned rectangle.
pub fn clip_polygon_to_rect(poly: &[DVec2], min: DVec2, max: DVec2) -> Vec<DVec2> {
    if poly.is_empty() {
        return Vec::new();
    }
    let planes = [
        HalfPlane::Min { axis: 0, bound: min.x },
        HalfPlane::Max { axis: 0, bound: max.x },
        HalfPlane::Min { axis: 1, bound: min.y },
        HalfPlane::Max { axis: 1, bound: max.y },
    ];
    let mut output = poly.to_vec();
    for plane in planes {
        if output.is_empty() {
            break;
        }
        let mut input = std::mem::take(&mut output);
        input.push(input[0]);
        for w in input.windows(2) {
            let (prev, cur) = (w[0], w[1]);
            let cur_in = plane.contains(cur);
            let prev_in = plane.contains(prev);
            if cur_in {
                if !prev_in {
                    output.push(intersect_half_plane(prev, cur, plane));
                }
                output.push(cur);
            } else if prev_in {
                output.push(intersect_half_plane(prev, cur, plane));
            }
        }
    }
    output
}

fn intersect_half_plane(a: DVec2, b: DVec2, plane: HalfPlane) -> DVec2 {
    let ca = plane.coord(a);
    let cb = plane.coord(b);
    let denom = cb - ca;
    let bound = plane.bound();
    let t = if denom.abs() < 1e-12 { 0.0 } else { (bound - ca) / denom };
    a + (b - a) * t.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrows_a_crossing_segment() {
        let min = DVec2::ZERO;
        let max = DVec2::new(10.0, 10.0);
        let (a, b) = clip_segment_to_rect(DVec2::new(-5.0, 5.0), DVec2::new(15.0, 5.0), min, max).unwrap();
        assert!((a.x - 0.0).abs() < 1e-9);
        assert!((b.x - 10.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_a_segment_outside() {
        let min = DVec2::ZERO;
        let max = DVec2::new(10.0, 10.0);
        assert!(clip_segment_to_rect(DVec2::new(20.0, 20.0), DVec2::new(30.0, 30.0), min, max).is_none());
    }

    #[test]
    fn clips_a_polygon() {
        let min = DVec2::ZERO;
        let max = DVec2::new(10.0, 10.0);
        let square = [
            DVec2::new(-5.0, -5.0),
            DVec2::new(15.0, -5.0),
            DVec2::new(15.0, 15.0),
            DVec2::new(-5.0, 15.0),
        ];
        let out = clip_polygon_to_rect(&square, min, max);
        assert_eq!(out.len(), 4);
        let area = crate::area::measure_polygon_area(&out).unwrap();
        assert!((area - 100.0).abs() < 1e-6);
    }
}
