//! HATCH fill geometry (spec §3.2): polygon triangulation and pattern lines.
//!
//! Imported hatches are reduced to closed loops in hatch-plane 2D coordinates;
//! this module turns them into triangle indices (solid fill) or pattern
//! polylines (line patterns) using plain even-odd scanline clipping. It is
//! deliberately free of any DWG types so it can be unit-tested directly.

/// A closed polygon in hatch-plane coordinates.
pub type Loop = Vec<[f64; 2]>;

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

const EPS: f64 = 1e-9;

/// Upper bound on polygon vertices before ear clipping is refused (its worst
/// case is cubic); hatches are simplified first, so this is a safety valve.
pub const MAX_FILL_POINTS: usize = 2048;

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

/// Ear-clipping triangulation of a simple polygon (no holes).
///
/// Returns index triples into `polygon`; an empty result means the contour was
/// degenerate (fewer than three distinct points, self-intersecting, or too
/// large to fill safely).
pub fn triangulate(polygon: &[[f64; 2]]) -> Vec<[u32; 3]> {
    let n = polygon.len();
    if n < 3 {
        return Vec::new();
    }
    if n > MAX_FILL_POINTS {
        return Vec::new();
    }
    if ring_area(polygon).abs() <= EPS {
        return Vec::new();
    }
    let mut indices: Vec<usize> = (0..n).collect();
    // Ear clipping expects counter-clockwise winding.
    if ring_area(polygon) < 0.0 {
        indices.reverse();
    }
    let mut out = Vec::new();
    let mut guard = n * n + 4;
    while indices.len() > 3 {
        guard = guard.saturating_sub(1);
        if guard == 0 {
            return Vec::new();
        }
        let m = indices.len();
        let mut clipped = false;
        for i in 0..m {
            let a = indices[(i + m - 1) % m];
            let b = indices[i];
            let c = indices[(i + 1) % m];
            if cross2(polygon[a], polygon[b], polygon[c]) <= EPS {
                continue; // reflex or collinear, not an ear
            }
            if indices
                .iter()
                .copied()
                .filter(|&j| j != a && j != b && j != c)
                .any(|j| point_in_triangle(polygon[j], polygon[a], polygon[b], polygon[c]))
            {
                continue;
            }
            out.push([a as u32, b as u32, c as u32]);
            indices.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            return Vec::new();
        }
    }
    out.push([indices[0] as u32, indices[1] as u32, indices[2] as u32]);
    out
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

fn collect_segments(loops: &[Loop]) -> Vec<([f64; 2], [f64; 2])> {
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

fn ring_area(points: &[[f64; 2]]) -> f64 {
    let mut acc = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        acc += a[0] * b[1] - b[0] * a[1];
    }
    acc / 2.0
}

fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    cross2(a, b, p) >= -EPS && cross2(b, c, p) >= -EPS && cross2(c, a, p) >= -EPS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_drops_collinear_points_but_keeps_corners() {
        let square = vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [2.0, 0.0],
            [2.0, 2.0],
            [0.0, 2.0],
            [0.0, 0.0],
        ];
        let simplified = simplify(&square, 1e-6);
        assert!(simplified.len() < square.len(), "{simplified:?}");
        assert!(simplified.contains(&[2.0, 2.0]), "{simplified:?}");
        assert!(simplified.contains(&[0.0, 2.0]), "{simplified:?}");
    }

    #[test]
    fn triangulate_refuses_oversized_contours() {
        let ring: Vec<[f64; 2]> = (0..(MAX_FILL_POINTS + 1))
            .map(|i| {
                let t = i as f64 * 0.01;
                [t.cos(), t.sin()]
            })
            .collect();
        assert!(triangulate(&ring).is_empty());
    }

    #[test]
    fn triangulates_a_square_into_two_triangles() {
        let square = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let tris = triangulate(&square);
        assert_eq!(tris.len(), 2, "{tris:?}");
        // Indices must reference real vertices and cover the area.
        let mut area = 0.0;
        for t in &tris {
            for &i in t {
                assert!((i as usize) < square.len());
            }
            let a = square[t[0] as usize];
            let b = square[t[1] as usize];
            let c = square[t[2] as usize];
            area += cross2(a, b, c).abs() / 2.0;
        }
        assert!((area - 4.0).abs() < 1e-9, "area {area}");
    }

    #[test]
    fn triangulates_clockwise_input_the_same() {
        let square = vec![[0.0, 0.0], [0.0, 2.0], [2.0, 2.0], [2.0, 0.0]];
        assert_eq!(triangulate(&square).len(), 2);
    }

    #[test]
    fn degenerate_contours_produce_no_triangles() {
        assert!(triangulate(&[]).is_empty());
        assert!(triangulate(&[[0.0, 0.0], [1.0, 0.0]]).is_empty());
        // Collinear points have no area for ear clipping.
        assert!(triangulate(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]).is_empty());
    }

    #[test]
    fn pattern_lines_fill_a_square() {
        let square = vec![vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]];
        let family = PatternLine {
            angle: 0.0,
            base: [0.0, 0.5],
            offset: [0.0, 1.0],
            dashes: Vec::new(),
        };
        let lines = pattern_polylines(&square, &[family]);
        assert_eq!(lines.len(), 4, "{lines:?}");
        for line in &lines {
            assert_eq!(line.len(), 2);
            assert!((line[0][0] - 0.0).abs() < 1e-9);
            assert!((line[1][0] - 4.0).abs() < 1e-9);
        }
    }

    #[test]
    fn pattern_even_odd_excludes_a_hole() {
        let outer = vec![[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [0.0, 6.0]];
        let hole = vec![[2.0, 2.0], [2.0, 4.0], [4.0, 4.0], [4.0, 2.0]];
        let family = PatternLine {
            angle: 0.0,
            base: [0.0, 0.5],
            offset: [0.0, 1.0],
            dashes: Vec::new(),
        };
        let lines = pattern_polylines(&[outer, hole], &[family]);
        // Rows y=0.5, 1.5, 4.5, 5.5 span the full width; rows y=2.5, 3.5 are
        // split by the hole into two 2-wide segments.
        let full = lines
            .iter()
            .filter(|l| (l[1][0] - l[0][0] - 6.0).abs() < 1e-9)
            .count();
        let split = lines
            .iter()
            .filter(|l| (l[1][0] - l[0][0] - 2.0).abs() < 1e-9)
            .count();
        assert_eq!(full, 4, "{lines:?}");
        assert_eq!(split, 4, "{lines:?}");
        assert_eq!(lines.len(), full + split);
    }

    #[test]
    fn dashes_break_a_span() {
        let square = vec![vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]];
        let family = PatternLine {
            angle: 0.0,
            base: [0.0, 0.5],
            offset: [0.0, 2.0],
            dashes: vec![1.0, -1.0],
        };
        let lines = pattern_polylines(&square, &[family]);
        // Two rows, each split into dash/gap/dash -> 2 segments per row.
        assert_eq!(lines.len(), 4, "{lines:?}");
        for line in &lines {
            assert!((line[1][0] - line[0][0] - 1.0).abs() < 1e-9, "{line:?}");
        }
    }
}
