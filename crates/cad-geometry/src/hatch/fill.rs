//! Solid HATCH fill: ear-clipping triangulation and even-odd ring filling.

use super::common::{collect_segments, EPS};
use super::Loop;

/// Upper bound on polygon vertices before ear clipping is refused (its worst
/// case is cubic); hatches are simplified first, so this is a safety valve.
pub const MAX_FILL_POINTS: usize = 2048;

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

/// A filled multi-ring triangulation in hatch-plane coordinates.
///
/// `triangles` index into `vertices`; the mesh is built under the even-odd
/// rule, so nested rings alternate between solid and hole. The caller is
/// responsible for mapping `vertices` into world space.
#[derive(Debug, Clone, PartialEq)]
pub struct FillMesh {
    pub vertices: Vec<[f64; 2]>,
    pub triangles: Vec<[u32; 3]>,
}

impl FillMesh {
    /// Total area covered by the triangles (always non-negative).
    pub fn area(&self) -> f64 {
        let mut acc = 0.0;
        for t in &self.triangles {
            let a = self.vertices[t[0] as usize];
            let b = self.vertices[t[1] as usize];
            let c = self.vertices[t[2] as usize];
            acc += cross2(a, b, c).abs() * 0.5;
        }
        acc
    }
}

/// Why a multi-ring fill could not be produced.
///
/// A caller that receives one of these must fall back to drawing the boundary
/// only and report `Partial`; it must never substitute an approximate fill.
#[derive(Debug, Clone, PartialEq)]
pub enum FillError {
    /// No ring had three distinct, finite, non-degenerate vertices.
    Empty,
    /// The combined vertex count exceeded [`MAX_FILL_POINTS`].
    Budget { points: usize, limit: usize },
    /// The even-odd fill would need more triangles than the safety cap allows.
    TooComplex,
    /// The rings are non-finite, zero-area or self-intersecting, so the
    /// even-odd region could not be triangulated consistently.
    Degenerate,
}

impl FillError {
    /// A human-readable reason suitable for a completeness report.
    pub fn reason(&self) -> String {
        match self {
            FillError::Empty => "hatch boundary has no usable ring".to_string(),
            FillError::Budget { points, limit } => {
                format!("hatch boundary has {points} points, over the {limit} point budget")
            }
            FillError::TooComplex => {
                "hatch fill needs too many triangles to triangulate safely".to_string()
            }
            FillError::Degenerate => {
                "hatch boundary is degenerate or self-intersecting".to_string()
            }
        }
    }
}

/// Safety cap on the number of fill triangles before [`fill_rings`] reports
/// [`FillError::TooComplex`]. The even-odd trapezoid decomposition is
/// `O(vertices^2)` in the worst case, so this keeps a pathological boundary
/// from exhausting memory while leaving ordinary hatches far below it.
pub const MAX_FILL_TRIANGLES: usize = MAX_FILL_POINTS * 64;

/// Triangulate one or more closed rings under the even-odd rule.
///
/// Rings are assumed to be non-self-intersecting closed boundaries. Nesting is
/// resolved by point-in-polygon containment: even depth is solid, odd depth is
/// a hole, and deeper even levels are islands again (spec §3.2 "HATCH holes").
///
/// The triangulation is a y-band trapezoid decomposition. Between two
/// consecutive vertex ordinates no edge can start or end, so the active edges
/// are monotone; sorting their crossings at the band midline and pairing them
/// (0,1), (2,3), ... is exactly the even-odd inside test and handles holes and
/// islands without any fragile hole-bridging. Each trapezoid becomes two
/// consistently counter-clockwise triangles.
///
/// The result is validated against the even-odd area derived from ring nesting:
/// a mismatch means the input was self-intersecting or otherwise inconsistent,
/// so the call fails with [`FillError::Degenerate`] instead of returning a
/// wrong fill.
pub fn fill_rings(rings: &[Loop]) -> Result<FillMesh, FillError> {
    let mut clean: Vec<Loop> = Vec::new();
    for ring in rings {
        if ring.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
            return Err(FillError::Degenerate);
        }
        let r = clean_ring(ring);
        if !r.is_empty() {
            clean.push(r);
        }
    }
    if clean.is_empty() {
        return Err(FillError::Empty);
    }
    let points: usize = clean.iter().map(|r| r.len()).sum();
    if points > MAX_FILL_POINTS {
        return Err(FillError::Budget {
            points,
            limit: MAX_FILL_POINTS,
        });
    }

    let depth = ring_depths(&clean);
    let expected: f64 = clean
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let area = ring_area(r).abs();
            if depth[i] % 2 == 0 {
                area
            } else {
                -area
            }
        })
        .sum();
    let expected = expected.abs();

    // Distinct vertex ordinates, tolerant of floating-point noise on large
    // coordinates, become the band boundaries.
    let mut ys: Vec<f64> = clean.iter().flatten().map(|p| p[1]).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
    for &y in &ys {
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let ytol = (max_y - min_y).abs().max(1.0) * 1e-12;
    let mut bands: Vec<f64> = Vec::new();
    for y in ys {
        if bands
            .last()
            .map(|last| (y - last).abs() > ytol)
            .unwrap_or(true)
        {
            bands.push(y);
        }
    }

    let edges = collect_segments(&clean);
    let mut vertices: Vec<[f64; 2]> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut cache: std::collections::HashMap<(u64, u64), u32> = std::collections::HashMap::new();

    for band in bands.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        if y1 - y0 <= ytol {
            continue;
        }
        let ymid = 0.5 * (y0 + y1);
        let mut crossings: Vec<(f64, [f64; 2], [f64; 2])> = Vec::new();
        for &(a, b) in &edges {
            if a[1] == b[1] {
                continue;
            }
            let (lo, hi) = if a[1] < b[1] {
                (a[1], b[1])
            } else {
                (b[1], a[1])
            };
            if ymid < lo || ymid > hi {
                continue;
            }
            let x = a[0] + (ymid - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if x.is_finite() {
                crossings.push((x, a, b));
            }
        }
        crossings.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut k = 0;
        while k + 1 < crossings.len() {
            let (_, la, lb) = crossings[k];
            let (_, ra, rb) = crossings[k + 1];
            let l0 = x_at_y(la, lb, y0);
            let l1 = x_at_y(la, lb, y1);
            let r0 = x_at_y(ra, rb, y0);
            let r1 = x_at_y(ra, rb, y1);
            // A band may taper to an apex at one end (a triangle at the top or
            // bottom of the boundary). Requiring both ends to have width would
            // drop that band entirely and lose the apex triangle, which made
            // every non-rectangular boundary fail the area cross-check below.
            // Emitting both candidate triangles is safe: at a degenerate end
            // the two corners intern to the same vertex, so `push_triangle`
            // discards the collapsed one and keeps the real triangle.
            if r0 - l0 > ytol || r1 - l1 > ytol {
                let i0 = intern(&mut vertices, &mut cache, [l0, y0]);
                let i1 = intern(&mut vertices, &mut cache, [r0, y0]);
                let i2 = intern(&mut vertices, &mut cache, [r1, y1]);
                let i3 = intern(&mut vertices, &mut cache, [l1, y1]);
                push_triangle(&mut triangles, i0, i1, i2);
                push_triangle(&mut triangles, i0, i2, i3);
            }
            k += 2;
            if triangles.len() > MAX_FILL_TRIANGLES {
                return Err(FillError::TooComplex);
            }
        }
    }

    let mesh = FillMesh {
        vertices,
        triangles,
    };
    let filled = mesh.area();
    let tolerance = expected.max(1.0) * 1e-6;
    if (filled - expected).abs() > tolerance {
        return Err(FillError::Degenerate);
    }
    Ok(mesh)
}

/// Remove consecutive/near-duplicate vertices, the implicit closing vertex and
/// any ring that collapses to zero area.
fn clean_ring(ring: &[[f64; 2]]) -> Loop {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(ring.len());
    for &p in ring {
        let dup = out
            .last()
            .map(|l| (l[0] - p[0]).abs() <= EPS && (l[1] - p[1]).abs() <= EPS)
            .unwrap_or(false);
        if !dup {
            out.push(p);
        }
    }
    while out.len() >= 2 {
        let first = out[0];
        let last = out[out.len() - 1];
        if (first[0] - last[0]).abs() <= EPS && (first[1] - last[1]).abs() <= EPS {
            out.pop();
        } else {
            break;
        }
    }
    if out.len() < 3 || ring_area(&out).abs() <= EPS {
        Vec::new()
    } else {
        out
    }
}

/// Even-odd nesting depth of each ring: how many other rings contain it.
fn ring_depths(rings: &[Loop]) -> Vec<usize> {
    let mut depths = vec![0usize; rings.len()];
    for (i, ring) in rings.iter().enumerate() {
        let probe = ring[0];
        for (j, other) in rings.iter().enumerate() {
            if i != j && point_in_ring(probe, other) {
                depths[i] += 1;
            }
        }
    }
    depths
}

/// Even-odd point-in-polygon test.
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

fn x_at_y(a: [f64; 2], b: [f64; 2], y: f64) -> f64 {
    if a[1] == b[1] {
        a[0]
    } else {
        a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
    }
}

fn intern(
    vertices: &mut Vec<[f64; 2]>,
    cache: &mut std::collections::HashMap<(u64, u64), u32>,
    p: [f64; 2],
) -> u32 {
    let key = (p[0].to_bits(), p[1].to_bits());
    if let Some(&index) = cache.get(&key) {
        return index;
    }
    let index = vertices.len() as u32;
    vertices.push(p);
    cache.insert(key, index);
    index
}

fn push_triangle(out: &mut Vec<[u32; 3]>, a: u32, b: u32, c: u32) {
    if a != b && b != c && a != c {
        out.push([a, b, c]);
    }
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

pub(super) fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    cross2(a, b, p) >= -EPS && cross2(b, c, p) >= -EPS && cross2(c, a, p) >= -EPS
}
