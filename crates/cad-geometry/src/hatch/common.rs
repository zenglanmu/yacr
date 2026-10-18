//! Small helpers shared by the HATCH submodules.

use super::Loop;

/// Tolerance for hatch-plane comparisons (dedup, degeneracy, gaps).
pub(super) const EPS: f64 = 1e-9;

pub(super) fn collect_segments(loops: &[Loop]) -> Vec<([f64; 2], [f64; 2])> {
    let mut segments = Vec::new();
    for loop_points in loops {
        if loop_points.len() < 2 {
            continue;
        }
        for i in 0..loop_points.len() {
            let a = loop_points[i];
            let b = loop_points[(i + 1) % loop_points.len()];
            if (a[0] - b[0]).abs() > EPS || (a[1] - b[1]).abs() > EPS {
                segments.push((a, b));
            }
        }
    }
    segments
}
