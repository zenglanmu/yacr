//! Planar area measurement with explicit self-intersection handling (§3.3).

use glam::DVec2;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AreaError {
    #[error("a polygon needs at least three distinct points, got {0}")]
    TooFewPoints(usize),
    #[error("the polygon is self-intersecting; area is undefined for this input")]
    SelfIntersecting,
    #[error("the polygon has zero area")]
    Degenerate,
    #[error("the polygon is not planar; area is undefined")]
    NonPlanar,
}

/// Signed area via the shoelace formula (positive = counter-clockwise).
pub fn signed_area(pts: &[DVec2]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut acc = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        acc += a.x * b.y - b.x * a.y;
    }
    acc * 0.5
}

/// Measure the area of a user-selected closed polygon.
pub fn measure_polygon_area(pts: &[DVec2]) -> Result<f64, AreaError> {
    let mut clean: Vec<DVec2> = Vec::with_capacity(pts.len());
    for &p in pts {
        if clean.last().map(|l: &DVec2| (*l - p).length() > 1e-12).unwrap_or(true) {
            clean.push(p);
        }
    }
    if clean.len() >= 2 {
        let first = clean[0];
        if (clean.last().unwrap() - first).length() <= 1e-12 {
            clean.pop();
        }
    }
    if clean.len() < 3 {
        return Err(AreaError::TooFewPoints(clean.len()));
    }
    if polygon_self_intersects(&clean) {
        return Err(AreaError::SelfIntersecting);
    }
    let a = signed_area(&clean).abs();
    if a <= 1e-15 {
        return Err(AreaError::Degenerate);
    }
    Ok(a)
}

/// Detect any proper crossing between non-adjacent edges.
pub fn polygon_self_intersects(pts: &[DVec2]) -> bool {
    let n = pts.len();
    for i in 0..n {
        let a1 = pts[i];
        let a2 = pts[(i + 1) % n];
        for j in 0..n {
            if j == i || (j + 1) % n == i || (i + 1) % n == j {
                continue;
            }
            let b1 = pts[j];
            let b2 = pts[(j + 1) % n];
            if segments_properly_cross(a1, a2, b1, b2) {
                return true;
            }
        }
    }
    false
}

fn segments_properly_cross(a1: DVec2, a2: DVec2, b1: DVec2, b2: DVec2) -> bool {
    let d1 = orient(b1, b2, a1);
    let d2 = orient(b1, b2, a2);
    let d3 = orient(a1, a2, b1);
    let d4 = orient(a1, a2, b2);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn orient(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_square_area() {
        let sq = [
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(0.0, 1.0),
        ];
        assert!((measure_polygon_area(&sq).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn bowtie_is_rejected() {
        let bowtie = [
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(0.0, 1.0),
        ];
        assert_eq!(measure_polygon_area(&bowtie), Err(AreaError::SelfIntersecting));
    }

    #[test]
    fn signed_area_sign_follows_winding() {
        let ccw = [DVec2::ZERO, DVec2::new(1.0, 0.0), DVec2::new(1.0, 1.0)];
        assert!(signed_area(&ccw) > 0.0);
    }
}
