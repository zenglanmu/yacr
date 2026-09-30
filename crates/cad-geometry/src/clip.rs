//! Rectangular clipping for paper-space viewports and culling (§3.2).
//!
//! Complex/polygonal viewport clipping is out of initial scope and is reported
//! as partial by the importer; only the rectangle is handled here.

use cad_domain::Point3;

/// Liang–Barsky segment clip against an XY rectangle.
pub fn clip_segment_to_xy_rect(
    a: Point3,
    b: Point3,
    min: (f64, f64),
    max: (f64, f64),
) -> Option<(Point3, Point3)> {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    let checks = [(-dx, a.x - min.0), (dx, max.0 - a.x), (-dy, a.y - min.1), (dy, max.1 - a.y)];
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
    let lerp = |t: f64| Point3 { x: a.x + dx * t, y: a.y + dy * t, z: a.z };
    Some((lerp(t0), lerp(t1)))
}

/// Clip a polyline (as a point list) to an XY rectangle.
///
/// The result keeps only the parts inside the rectangle, splitting the
/// polyline where it exits and re-enters.
pub fn clip_polyline_to_xy_rect(points: &[Point3], min: (f64, f64), max: (f64, f64)) -> Vec<Vec<Point3>> {
    let mut runs: Vec<Vec<Point3>> = Vec::new();
    let mut current: Vec<Point3> = Vec::new();
    for w in points.windows(2) {
        match clip_segment_to_xy_rect(w[0], w[1], min, max) {
            Some((s, e)) => {
                if current.last().map(|l| !same(*l, s)).unwrap_or(true) {
                    if current.len() >= 2 {
                        runs.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                }
                current.push(s);
                current.push(e);
            }
            None => {
                if current.len() >= 2 {
                    runs.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
        }
    }
    if current.len() >= 2 {
        runs.push(current);
    }
    runs
}

fn same(a: Point3, b: Point3) -> bool {
    (a.x - b.x).abs() < 1e-12 && (a.y - b.y).abs() < 1e-12 && (a.z - b.z).abs() < 1e-12
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn narrows_a_crossing_segment() {
        let (a, b) = clip_segment_to_xy_rect(p(-5.0, 5.0), p(15.0, 5.0), (0.0, 0.0), (10.0, 10.0)).unwrap();
        assert!((a.x - 0.0).abs() < 1e-9);
        assert!((b.x - 10.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_a_segment_outside() {
        assert!(clip_segment_to_xy_rect(p(20.0, 20.0), p(30.0, 30.0), (0.0, 0.0), (10.0, 10.0)).is_none());
    }

    #[test]
    fn splits_a_polyline_that_exits_and_reenters() {
        let pts = vec![p(0.0, 5.0), p(5.0, 5.0), p(5.0, 20.0), p(10.0, 20.0), p(10.0, 5.0), p(15.0, 5.0)];
        let runs = clip_polyline_to_xy_rect(&pts, (0.0, 0.0), (10.0, 10.0));
        assert_eq!(runs.len(), 2);
    }
}
