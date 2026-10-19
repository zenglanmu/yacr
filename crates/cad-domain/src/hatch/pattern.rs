//! Pattern-line HATCH: parallel line families clipped by even-odd loops.

use super::common::{collect_segments, EPS};
use super::Loop;

/// One family of parallel pattern lines.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternLine {
    /// Line direction in radians (hatch-plane).
    pub angle: f64,
    /// A point on line `k = 0`.
    pub base: [f64; 2],
    /// Step from line `k` to `k + 1` (usually perpendicular to `angle`).
    pub offset: [f64; 2],
    /// Dash pattern: positive = dash, negative = gap. Empty means solid.
    pub dashes: Vec<f64>,
}

/// Intersect each pattern family with the region defined by `loops` (even-odd)
/// and return the visible pattern polylines.
pub fn pattern_polylines(loops: &[Loop], families: &[PatternLine]) -> Vec<Loop> {
    let segments = collect_segments(loops);
    if segments.is_empty() {
        return Vec::new();
    }
    let Some((min, max)) = bounds(loops) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for family in families {
        let (dx, dy) = (family.angle.cos(), family.angle.sin());
        // Unit normal to the line direction; crossings are classified by the
        // signed distance of the segment endpoints along it.
        let (nx, ny) = (-dy, dx);
        let off_len =
            (family.offset[0] * family.offset[0] + family.offset[1] * family.offset[1]).sqrt();
        let (k0, k1) = if off_len <= EPS {
            (0i64, 0i64)
        } else {
            let ux = family.offset[0] / off_len;
            let uy = family.offset[1] / off_len;
            let proj = |p: [f64; 2]| {
                ((p[0] - family.base[0]) * ux + (p[1] - family.base[1]) * uy) / off_len
            };
            let corners = [
                [min[0], min[1]],
                [max[0], min[1]],
                [max[0], max[1]],
                [min[0], max[1]],
            ];
            let lo = corners
                .iter()
                .map(|c| proj(*c))
                .fold(f64::INFINITY, f64::min);
            let hi = corners
                .iter()
                .map(|c| proj(*c))
                .fold(f64::NEG_INFINITY, f64::max);
            (lo.floor() as i64 - 1, hi.ceil() as i64 + 1)
        };
        let mut k = k0;
        // A degenerate spacing (near-zero offset over a large extent) would
        // request an unbounded number of lines; skip such a family.
        if k1.saturating_sub(k0) > 200_000 {
            continue;
        }
        while k <= k1 {
            let origin = [
                family.base[0] + family.offset[0] * k as f64,
                family.base[1] + family.offset[1] * k as f64,
            ];
            let mut hits: Vec<f64> = Vec::new();
            for (p0, p1) in &segments {
                let d0 = (p0[0] - origin[0]) * nx + (p0[1] - origin[1]) * ny;
                let d1 = (p1[0] - origin[0]) * nx + (p1[1] - origin[1]) * ny;
                // Half-open crossing rule: each vertex is counted once.
                let crosses = (d0 <= 0.0 && d1 > 0.0) || (d1 <= 0.0 && d0 > 0.0);
                if !crosses {
                    continue;
                }
                let t = d0 / (d0 - d1);
                let ix = p0[0] + (p1[0] - p0[0]) * t;
                let iy = p0[1] + (p1[1] - p0[1]) * t;
                hits.push((ix - origin[0]) * dx + (iy - origin[1]) * dy);
            }
            hits.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < hits.len() {
                emit_span(
                    &mut out,
                    origin,
                    [dx, dy],
                    hits[i],
                    hits[i + 1],
                    &family.dashes,
                );
                i += 2;
            }
            k += 1;
        }
    }
    out
}

fn emit_span(out: &mut Vec<Loop>, origin: [f64; 2], d: [f64; 2], s0: f64, s1: f64, dashes: &[f64]) {
    if s1 - s0 <= EPS {
        return;
    }
    let point = |s: f64| [origin[0] + d[0] * s, origin[1] + d[1] * s];
    if dashes.is_empty() {
        out.push(vec![point(s0), point(s1)]);
        return;
    }
    let cycle: f64 = dashes.iter().map(|d| d.abs()).sum();
    if cycle <= EPS {
        out.push(vec![point(s0), point(s1)]);
        return;
    }
    let length = s1 - s0;
    let mut pos = 0.0;
    let mut emit = true; // pattern alternates starting with a dash
    let mut idx = 0;
    let mut guard = 0;
    while pos < length && guard < 1_000_000 {
        guard += 1;
        let dash = dashes[idx % dashes.len()];
        let step = dash.abs().max(EPS);
        if emit {
            let start = s0 + pos;
            let end = s0 + (pos + step).min(length);
            if end - start > EPS {
                out.push(vec![point(start), point(end)]);
            }
        }
        // A negative entry is an explicit gap; otherwise alternate.
        emit = if dash < 0.0 { true } else { !emit };
        pos += step;
        idx += 1;
    }
}

fn bounds(loops: &[Loop]) -> Option<([f64; 2], [f64; 2])> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut any = false;
    for loop_points in loops {
        for p in loop_points {
            any = true;
            min[0] = min[0].min(p[0]);
            min[1] = min[1].min(p[1]);
            max[0] = max[0].max(p[0]);
            max[1] = max[1].max(p[1]);
        }
    }
    any.then_some((min, max))
}
