//! Small helpers shared by the HATCH submodules.

use super::Loop;

/// Tolerance for hatch-plane comparisons (dedup, degeneracy, gaps).
pub(super) const EPS: f64 = 1e-9;

/// How many vertices of a ring the containment probe samples.
const DEPTH_PROBES: usize = 8;

/// Containment count of each ring (how many other rings contain it).
///
/// A ring is counted inside another one when the majority of its
/// [`probe_points`] lies inside that ring. One probe (the first vertex) is
/// exact for a boundary set that reached the creation path, because the
/// creation path rejects touching, crossing and duplicate rings; the import
/// path keeps such sets (ADR B6), and there the first vertex of a ring can sit
/// exactly on another ring's boundary, where the even-odd crossing test is
/// arbitrary. Sampling several vertices spread around the ring keeps one
/// unlucky boundary vertex from flipping a ring's depth and mis-filling it (as
/// a hole, or as solid).
///
/// A ring whose probes split evenly (a partial overlap, which the import path
/// also keeps) has no single containment answer; the first vertex decides so
/// the result stays deterministic.
pub(crate) fn ring_depths(rings: &[Loop]) -> Vec<usize> {
    let mut depths = vec![0usize; rings.len()];
    for (i, ring) in rings.iter().enumerate() {
        for (j, other) in rings.iter().enumerate() {
            if i != j && ring_is_inside(ring, other) {
                depths[i] += 1;
            }
        }
    }
    depths
}

/// Up to [`DEPTH_PROBES`] vertices evenly spread around the ring, starting at
/// index 0 (so the first vertex is always probed).
fn probe_points(ring: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let len = ring.len();
    let count = len.min(DEPTH_PROBES);
    (0..count).map(|k| ring[k * len / count]).collect()
}

pub(crate) fn ring_is_inside(ring: &[[f64; 2]], container: &[[f64; 2]]) -> bool {
    let Some(&first) = ring.first() else {
        return false;
    };
    let probes = probe_points(ring);
    let inside = probes
        .iter()
        .filter(|p| point_in_ring(**p, container))
        .count();
    if probes.is_empty() || inside * 2 == probes.len() {
        point_in_ring(first, container)
    } else {
        inside * 2 > probes.len()
    }
}

/// Even-odd point-in-polygon test.
pub(crate) fn point_in_ring(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    let mut inside = false;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

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
