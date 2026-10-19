//! Douglas-Peucker simplification of hatch boundaries.

use super::common::EPS;

/// Douglas–Peucker simplification of an open polyline.
///
/// Hatch boundaries are tessellated curves with thousands of near-collinear
/// points; filling them directly makes ear clipping cubic. Simplifying first
/// keeps the shape while bounding the fill cost.
pub fn simplify(points: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    if points.len() < 3 || !tolerance.is_finite() || tolerance <= 0.0 {
        return points.to_vec();
    }
    let n = points.len();
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((first, last)) = stack.pop() {
        if last <= first + 1 {
            continue;
        }
        let mut max_distance = -1.0;
        let mut max_index = first;
        for i in first + 1..last {
            let d = point_segment_distance(points[i], points[first], points[last]);
            if d > max_distance {
                max_distance = d;
                max_index = i;
            }
        }
        if max_distance > tolerance {
            keep[max_index] = true;
            stack.push((first, max_index));
            stack.push((max_index, last));
        }
    }
    points
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, p)| *p)
        .collect()
}

fn point_segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 <= EPS {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
    let cx = a[0] + t * dx;
    let cy = a[1] + t * dy;
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}
