//! Boundary validation for the creation path (ADR 0018 A.3, B6).
//!
//! Import never applies these rules (ADR B6): a loop set that cannot be filled
//! still shows its boundaries and is reported `Partial`.

use crate::CadError;
use crate::CadResult;

use super::geometry::{finite_point, length3};
use super::{HatchLoop, HatchPlane};

pub(super) fn validate_creation(loops: &[HatchLoop], plane: &HatchPlane) -> CadResult<()> {
    if !plane_is_usable(plane) {
        return Err(CadError::InvalidInput(
            "hatch plane normal must be finite and non-degenerate".to_string(),
        ));
    }
    if !plane.elevation.is_finite() {
        return Err(CadError::InvalidInput(
            "hatch elevation must be finite".to_string(),
        ));
    }
    if loops.is_empty() {
        return Err(CadError::InvalidInput(
            "hatch boundary has no loops".to_string(),
        ));
    }
    for loop_ in loops {
        if loop_.points.len() < 3 {
            return Err(CadError::InvalidInput(
                "hatch boundary loop has fewer than three vertices".to_string(),
            ));
        }
        if !loop_
            .points
            .iter()
            .all(|p| p[0].is_finite() && p[1].is_finite())
        {
            return Err(CadError::InvalidInput(
                "hatch boundary loop has a non-finite vertex".to_string(),
            ));
        }
        if !loop_.bulges.is_empty() && loop_.bulges.len() != loop_.points.len() {
            return Err(CadError::InvalidInput(
                "hatch boundary loop bulges must match its vertex count".to_string(),
            ));
        }
        if !loop_.bulges.is_empty() && !loop_.bulges.iter().all(|b| b.is_finite()) {
            return Err(CadError::InvalidInput(
                "hatch boundary loop has a non-finite bulge".to_string(),
            ));
        }
        if loop_.flags.0 & 32 != 0 {
            return Err(CadError::InvalidInput(
                "hatch boundary loop is flagged not closed".to_string(),
            ));
        }
        if ring_degenerate(&loop_.points) || ring_has_repeated_vertex(&loop_.points) {
            return Err(CadError::InvalidInput(
                "hatch boundary loop is degenerate".to_string(),
            ));
        }
        if ring_self_intersects(&loop_.points) {
            return Err(CadError::InvalidInput(
                "hatch boundary loop is self-intersecting".to_string(),
            ));
        }
    }
    for (i, a) in loops.iter().enumerate() {
        for b in loops.iter().skip(i + 1) {
            if rings_touch(&a.points, &b.points) {
                return Err(CadError::InvalidInput(
                    "hatch boundary loops touch or partially overlap".to_string(),
                ));
            }
            if point_in_ring(a.points[0], &b.points) && point_in_ring(b.points[0], &a.points) {
                return Err(CadError::InvalidInput(
                    "hatch boundary has duplicate loops".to_string(),
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn plane_is_usable(plane: &HatchPlane) -> bool {
    finite_point(plane.normal) && length3(plane.normal) > 1e-12
}

fn ring_degenerate(points: &[[f64; 2]]) -> bool {
    let mut area = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    area.abs() <= 1e-12
}

fn ring_has_repeated_vertex(points: &[[f64; 2]]) -> bool {
    let n = points.len();
    (0..n).any(|i| {
        let a = points[i];
        let b = points[(i + 1) % n];
        (a[0] - b[0]).abs() <= 1e-12 && (a[1] - b[1]).abs() <= 1e-12
    })
}

fn ring_self_intersects(points: &[[f64; 2]]) -> bool {
    let n = points.len();
    for i in 0..n {
        let a1 = points[i];
        let a2 = points[(i + 1) % n];
        for j in (i + 1)..n {
            // Adjacent segments share a vertex by construction.
            if j == i + 1 || (j + 1) % n == i {
                continue;
            }
            if segments_touch(a1, a2, points[j], points[(j + 1) % n]) {
                return true;
            }
        }
    }
    false
}

/// Whether two closed rings share a point or cross (including tangential
/// contact and collinear overlap).
fn rings_touch(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    for i in 0..a.len() {
        let a1 = a[i];
        let a2 = a[(i + 1) % a.len()];
        for j in 0..b.len() {
            if segments_touch(a1, a2, b[j], b[(j + 1) % b.len()]) {
                return true;
            }
        }
    }
    false
}

fn segments_touch(a1: [f64; 2], a2: [f64; 2], b1: [f64; 2], b2: [f64; 2]) -> bool {
    let eps = 1e-12;
    let d1 = orient(b1, b2, a1);
    let d2 = orient(b1, b2, a2);
    let d3 = orient(a1, a2, b1);
    let d4 = orient(a1, a2, b2);
    if ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
        && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
    {
        return true;
    }
    (d1.abs() <= eps && on_segment(b1, b2, a1))
        || (d2.abs() <= eps && on_segment(b1, b2, a2))
        || (d3.abs() <= eps && on_segment(a1, a2, b1))
        || (d4.abs() <= eps && on_segment(a1, a2, b2))
}

fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    p[0] >= a[0].min(b[0]) - 1e-12
        && p[0] <= a[0].max(b[0]) + 1e-12
        && p[1] >= a[1].min(b[1]) - 1e-12
        && p[1] <= a[1].max(b[1]) + 1e-12
}

fn point_in_ring(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
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

/// Containment count of each loop (0-based; 1-based depth = count + 1).
pub(super) fn loop_depths(loops: &[HatchLoop]) -> Vec<usize> {
    let mut depths = vec![0usize; loops.len()];
    for (i, loop_) in loops.iter().enumerate() {
        let Some(probe) = loop_.points.first() else {
            continue;
        };
        for (j, other) in loops.iter().enumerate() {
            if i != j && point_in_ring(*probe, &other.points) {
                depths[i] += 1;
            }
        }
    }
    depths
}
