//! Planar area measurement with explicit self-intersection handling (§3.3).
//!
//! Area is only defined for a coplanar, non-self-intersecting ring; anything
//! else is refused rather than returning a meaningless number.

use cad_domain::Point3;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AreaError {
    #[error("a polygon needs at least three distinct points, got {0}")]
    TooFewPoints(usize),
    #[error("the polygon is self-intersecting; area is undefined for this input")]
    SelfIntersecting,
    #[error("the polygon has zero area")]
    Degenerate,
    #[error("the polygon is not coplanar within the given tolerance")]
    NonPlanar,
}

/// Signed area via the shoelace formula on the XY projection (positive = CCW).
pub fn signed_area(pts: &[Point3]) -> f64 {
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
///
/// Rejects non-coplanar rings, self-intersections and non-finite input;
/// `planarity_tolerance` is a world-space value supplied by the caller's
/// [`cad_domain::TolerancePolicy`].
pub fn measure_polygon_area(pts: &[Point3], planarity_tolerance: f64) -> Result<f64, AreaError> {
    // A non-finite coordinate would make the shoelace sum NaN and could still
    // compare as "not degenerate"; refuse it up front (audit B24).
    if pts
        .iter()
        .any(|p| !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite())
    {
        return Err(AreaError::Degenerate);
    }
    let mut clean: Vec<Point3> = Vec::with_capacity(pts.len());
    for &p in pts {
        if clean.last().map(|l| dist2(*l, p) > 1e-24).unwrap_or(true) {
            clean.push(p);
        }
    }
    if clean.len() >= 2 {
        let first = clean[0];
        if dist2(*clean.last().unwrap(), first) <= 1e-24 {
            clean.pop();
        }
    }
    if clean.len() < 3 {
        return Err(AreaError::TooFewPoints(clean.len()));
    }
    // Coplanarity: all points must lie in one plane, checked against the first
    // three non-collinear points.
    if !is_coplanar(&clean, planarity_tolerance.max(1e-9)) {
        return Err(AreaError::NonPlanar);
    }
    if self_intersects(&clean) {
        return Err(AreaError::SelfIntersecting);
    }
    let a = signed_area(&clean).abs();
    if !a.is_finite() || a <= 1e-15 {
        return Err(AreaError::Degenerate);
    }
    Ok(a)
}

fn is_coplanar(pts: &[Point3], tol: f64) -> bool {
    if pts.len() <= 3 {
        return true;
    }
    // Find a normal from the first non-degenerate triple.
    let mut normal = None;
    for i in 1..pts.len() - 1 {
        let n = cross(sub(pts[i], pts[0]), sub(pts[i + 1], pts[0]));
        if len(n) > tol {
            normal = Some(normalize(n));
            break;
        }
    }
    let Some(n) = normal else { return true };
    pts.iter()
        .all(|p| dot(sub(*p, pts[0]), n).abs() <= tol * 1000.0)
}

fn self_intersects(pts: &[Point3]) -> bool {
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
            if properly_cross(a1, a2, b1, b2) {
                return true;
            }
        }
    }
    false
}

fn properly_cross(a1: Point3, a2: Point3, b1: Point3, b2: Point3) -> bool {
    let d1 = orient(b1, b2, a1);
    let d2 = orient(b1, b2, a2);
    let d3 = orient(a1, a2, b1);
    let d4 = orient(a1, a2, b2);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

fn orient(a: Point3, b: Point3, c: Point3) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn len(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: Point3) -> Point3 {
    let l = len(a);
    if l < 1e-24 {
        a
    } else {
        Point3 {
            x: a.x / l,
            y: a.y / l,
            z: a.z / l,
        }
    }
}

fn dist2(a: Point3, b: Point3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn unit_square_area() {
        let sq = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        assert!((measure_polygon_area(&sq, 1e-6).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn bowtie_is_rejected() {
        let bowtie = [p(0.0, 0.0), p(1.0, 1.0), p(1.0, 0.0), p(0.0, 1.0)];
        assert_eq!(
            measure_polygon_area(&bowtie, 1e-6),
            Err(AreaError::SelfIntersecting)
        );
    }

    #[test]
    fn non_planar_ring_is_rejected() {
        let ring = [
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 5.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        assert_eq!(measure_polygon_area(&ring, 1e-6), Err(AreaError::NonPlanar));
    }
}
