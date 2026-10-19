//! Bulge sampling and the small vector/plane algebra the hatch needs.
//!
//! `cad-geometry` depends on this crate, so the few helpers the display builder
//! needs (the OCS basis, normalization) are implemented locally, exactly like
//! `DimensionGeometry`'s vector algebra.

use crate::{Point3, Transform3};

use super::HatchLoop;

/// Sample a loop's bulges into a dense 2D ring (closed, no repeated last point).
pub(super) fn sample_loop(loop_: &HatchLoop) -> Vec<[f64; 2]> {
    let n = loop_.points.len();
    if n == 0 {
        return Vec::new();
    }
    if loop_.bulges.iter().all(|b| *b == 0.0) {
        return loop_.points.clone();
    }
    let mut out: Vec<[f64; 2]> = Vec::new();
    for i in 0..n {
        let a = loop_.points[i];
        let b = loop_.points[(i + 1) % n];
        let bulge = loop_.bulges.get(i).copied().unwrap_or(0.0);
        push_bulge_points(&mut out, a, b, bulge);
    }
    // The wrap segment ends at the start vertex; drop the duplicate.
    if out.len() >= 2 {
        let first = out[0];
        let last = out[out.len() - 1];
        if (first[0] - last[0]).abs() <= 1e-9 && (first[1] - last[1]).abs() <= 1e-9 {
            out.pop();
        }
    }
    out
}

/// Sample one bulged segment, mirroring the importer's arc sampling so a loop
/// stored with bulges tessellates like the source it came from.
fn push_bulge_points(out: &mut Vec<[f64; 2]>, p0: [f64; 2], p1: [f64; 2], bulge: f64) {
    push_unique_point(out, p0);
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let chord = (dx * dx + dy * dy).sqrt();
    if !bulge.is_finite() || bulge.abs() <= 1e-9 || chord <= 1e-9 {
        push_unique_point(out, p1);
        return;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta / 2.0;
    let (sin_half, cos_half) = half.sin_cos();
    if sin_half.abs() <= 1e-9 {
        push_unique_point(out, p1);
        return;
    }
    let radius = chord / (2.0 * sin_half);
    let mid = [0.5 * (p0[0] + p1[0]), 0.5 * (p0[1] + p1[1])];
    let perp = [-dy / chord, dx / chord];
    let center = [
        mid[0] + perp[0] * radius * cos_half,
        mid[1] + perp[1] * radius * cos_half,
    ];
    if !radius.is_finite() || !center[0].is_finite() || !center[1].is_finite() {
        push_unique_point(out, p1);
        return;
    }
    let radius = radius.abs();
    let start_angle = (p0[1] - center[1]).atan2(p0[0] - center[0]);
    let steps = ((theta.abs() / 0.05).ceil() as usize).clamp(2, 4096);
    for i in 1..steps {
        let t = start_angle + theta * (i as f64) / (steps as f64);
        let (sin, cos) = t.sin_cos();
        push_unique_point(out, [center[0] + radius * cos, center[1] + radius * sin]);
    }
    push_unique_point(out, p1);
}

fn push_unique_point(out: &mut Vec<[f64; 2]>, p: [f64; 2]) {
    if let Some(last) = out.last() {
        if (last[0] - p[0]).abs() <= 1e-9 && (last[1] - p[1]).abs() <= 1e-9 {
            return;
        }
    }
    out.push(p);
}

pub(super) fn fill_tolerance(sampled: &[Vec<[f64; 2]>]) -> f64 {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in sampled.iter().flatten() {
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[1]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[1]);
    }
    let diagonal = ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2)).sqrt();
    (diagonal * 1e-3).max(1e-9)
}

/// The same arbitrary in-plane axis the importer/renderer derive from a normal.
///
/// Duplicated locally (like `DimensionGeometry`'s small vector algebra) because
/// `cad-geometry` depends on this crate, not the other way round.
pub(super) fn arbitrary_axis(normal: Point3) -> (Point3, Point3, Point3) {
    let n = if length3(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        normalize3(normal)
    };
    const ONE_64TH: f64 = 1.0 / 64.0;
    let ax = if n.x.abs() < ONE_64TH && n.y.abs() < ONE_64TH {
        cross3(
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            n,
        )
    } else {
        cross3(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            n,
        )
    };
    let ax = if length3(ax) < 1e-24 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        normalize3(ax)
    };
    let ay = normalize3(cross3(n, ax));
    (ax, ay, n)
}

pub(super) fn column_scale(transform: &Transform3) -> f64 {
    let m = &transform.matrix;
    (m[0][0] * m[0][0] + m[1][0] * m[1][0] + m[2][0] * m[2][0]).sqrt()
}

pub(super) fn finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

pub(super) fn add3(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub(super) fn sub3(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub(super) fn scale3(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}

pub(super) fn dot3(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross3(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub(super) fn length3(a: Point3) -> f64 {
    a.x.hypot(a.y).hypot(a.z)
}

pub(super) fn normalize3(a: Point3) -> Point3 {
    let len = length3(a);
    if len < 1e-24 || !len.is_finite() {
        a
    } else {
        scale3(a, 1.0 / len)
    }
}
