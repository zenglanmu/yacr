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

// ---------------------------------------------------------------------------
// Gradient fills (spec §3.2 gradient HATCH)
// ---------------------------------------------------------------------------
//
// A gradient fill is baked into per-vertex sRGB so it reuses the existing mesh
// pipeline (no texture, no new shader state). The model below is intentionally
// a small, honest subset of the DXF gradient vocabulary:
//
//   * `Linear` — a 1D ramp along `angle` (DXF "LINEAR").
//   * `Spherical` — a radial ramp from the centre of the boundary extent
//     (DXF "SPHERICAL"/"CYLINDER" family, where the radius drives the value).
//
// Curved / multi-segment gradients (e.g. "HEMISPHERICAL", "CURVED") are not
// approximated: the importer reports them explicitly as unsupported.

/// One colour stop: `value` is the normalised position (`0.0..=1.0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub value: f64,
    pub rgb: [u8; 3],
}

/// Which parametric ramp a gradient uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientKind {
    /// A 1D ramp along the gradient angle.
    Linear,
    /// A radial ramp from the centre of the boundary's extent.
    Spherical,
}

/// A resolved gradient definition in hatch-plane coordinates.
///
/// `angle` is the gradient direction in radians (DXF group 452), `shift` the
/// normalised start offset in `0.0..=1.0` (DXF group 453), and `stops` the
/// colour entries (DXF group 463/421). `single_color` records the DXF
/// single-colour flag; when set, `stops` is expected to hold one entry and the
/// ramp blends that colour towards `tint` (positive → white, negative → black).
#[derive(Debug, Clone, PartialEq)]
pub struct GradientDef {
    pub kind: GradientKind,
    pub angle: f64,
    pub shift: f64,
    pub single_color: bool,
    /// Tint in `-1.0..=1.0`; only meaningful for `single_color`.
    pub tint: f64,
    pub stops: Vec<GradientStop>,
}

impl GradientDef {
    /// Whether the definition carries the data needed to produce a ramp.
    ///
    /// A gradient with no usable stop cannot be rendered as a gradient and must
    /// be reported rather than silently drawn solid.
    pub fn is_usable(&self) -> bool {
        !self.stops.is_empty()
            && self.angle.is_finite()
            && self.shift.is_finite()
            && self
                .stops
                .iter()
                .all(|s| s.value.is_finite() && s.value >= 0.0 && s.value <= 1.0)
    }
}

/// Sample a gradient at normalised position `t` (clamped to `0.0..=1.0`).
///
/// Stops must be sorted by `value` via [`normalize_stops`]; a single stop is a
/// constant colour. Between two stops the colour is linearly interpolated in
/// sRGB byte space, which matches the no-colour-management path the renderer
/// already uses.
pub fn sample_gradient(def: &GradientDef, t: f64) -> [u8; 3] {
    let stops = &def.stops;
    if stops.is_empty() {
        return [255, 255, 255];
    }
    let t = t.clamp(0.0, 1.0);
    if stops.len() == 1 {
        return stops[0].rgb;
    }
    if t <= stops[0].value {
        return stops[0].rgb;
    }
    if t >= stops[stops.len() - 1].value {
        return stops[stops.len() - 1].rgb;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t >= a.value && t <= b.value {
            let span = b.value - a.value;
            if span <= f64::EPSILON {
                return b.rgb;
            }
            let f = (t - a.value) / span;
            return lerp_rgb(a.rgb, b.rgb, f);
        }
    }
    stops[stops.len() - 1].rgb
}

fn lerp_rgb(a: [u8; 3], b: [u8; 3], f: f64) -> [u8; 3] {
    let f = f.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| {
        (x as f64 + (y as f64 - x as f64) * f)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]
}

/// Sort stops ascending and drop non-finite / out-of-range entries.
///
/// Keeps the gradient deterministic even when a file lists stops out of order,
/// which is common. Duplicate values are retained; the sampler resolves a
/// zero-width span to the later stop.
pub fn normalize_stops(mut stops: Vec<GradientStop>) -> Vec<GradientStop> {
    stops.retain(|s| s.value.is_finite() && (0.0..=1.0).contains(&s.value));
    stops.sort_by(|a, b| {
        a.value
            .partial_cmp(&b.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    stops
}

/// The parameter each vertex feeds into the ramp, in `0.0..=1.0`.
///
/// * Linear: projection onto the gradient direction, normalised across the
///   boundary extent, then shifted by `shift` (wrapping, as DXF does).
/// * Spherical: normalised radial distance from the centre of the extent.
fn gradient_parameter(points: &[[f64; 2]], def: &GradientDef) -> Vec<f64> {
    let Some((min, max)) = bounds_of(points) else {
        return Vec::new();
    };
    let center = [0.5 * (min[0] + max[0]), 0.5 * (min[1] + max[1])];
    match def.kind {
        GradientKind::Linear => {
            let (dx, dy) = (def.angle.cos(), def.angle.sin());
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for p in points {
                let s = (p[0] - center[0]) * dx + (p[1] - center[1]) * dy;
                lo = lo.min(s);
                hi = hi.max(s);
            }
            let span = hi - lo;
            points
                .iter()
                .map(|p| {
                    let s = (p[0] - center[0]) * dx + (p[1] - center[1]) * dy;
                    let t = if span <= f64::EPSILON {
                        0.5
                    } else {
                        (s - lo) / span
                    };
                    // The shift is an offset along the ramp. This subset clamps
                    // at the ends rather than wrapping, so the two boundary
                    // endpoints keep their exact stop colours; wrapping is a
                    // documented gap (see docs/hatch-gradient.md).
                    (t + def.shift).clamp(0.0, 1.0)
                })
                .collect()
        }
        GradientKind::Spherical => {
            let mut max_r = 0.0f64;
            for p in points {
                let dx = p[0] - center[0];
                let dy = p[1] - center[1];
                max_r = max_r.max((dx * dx + dy * dy).sqrt());
            }
            points
                .iter()
                .map(|p| {
                    let dx = p[0] - center[0];
                    let dy = p[1] - center[1];
                    let r = (dx * dx + dy * dy).sqrt();
                    let t = if max_r <= f64::EPSILON {
                        0.0
                    } else {
                        r / max_r
                    };
                    (t + def.shift).clamp(0.0, 1.0)
                })
                .collect()
        }
    }
}

fn bounds_of(points: &[[f64; 2]]) -> Option<([f64; 2], [f64; 2])> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut any = false;
    for p in points {
        any = true;
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[1]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[1]);
    }
    any.then_some((min, max))
}

/// Bake a gradient into per-vertex sRGB for a fill triangulation.
///
/// Each returned colour corresponds to `fill.vertices[i]`, so the existing
/// fill indices apply unchanged and the ramp is clipped to the hatch boundary
/// by construction (only interior vertices are coloured). Returns an empty
/// vector when the definition is unusable, which the caller must report rather
/// than draw solid.
pub fn gradient_vertex_colors(fill: &FillMesh, def: &GradientDef) -> Vec<[u8; 3]> {
    if !def.is_usable() {
        return Vec::new();
    }
    let stops = normalize_stops(def.stops.clone());
    if stops.is_empty() {
        return Vec::new();
    }
    let effective = if def.single_color {
        single_color_stops(&stops, def.tint)
    } else {
        stops
    };
    let def = GradientDef {
        stops: effective,
        ..def.clone()
    };
    let params = gradient_parameter(&fill.vertices, &def);
    params.iter().map(|t| sample_gradient(&def, *t)).collect()
}

/// Expand a DXF single-colour gradient into a two-stop ramp.
///
/// The single listed colour stays at value 0; value 1 blends it towards white
/// (`tint > 0`) or black (`tint < 0`), mirroring AutoCAD's tint/shade control.
fn single_color_stops(stops: &[GradientStop], tint: f64) -> Vec<GradientStop> {
    let base = stops[0];
    let tint = tint.clamp(-1.0, 1.0);
    let target = if tint >= 0.0 {
        [255, 255, 255]
    } else {
        [0, 0, 0]
    };
    let end = lerp_rgb(base.rgb, target, tint.abs());
    vec![
        GradientStop {
            value: 0.0,
            rgb: base.rgb,
        },
        GradientStop {
            value: 1.0,
            rgb: end,
        },
    ]
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

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn fill_rings_single_ring_matches_its_area() {
        let mesh = fill_rings(&[rect(0.0, 0.0, 3.0, 2.0)]).expect("single ring fills");
        assert!(!mesh.triangles.is_empty());
        assert!((mesh.area() - 6.0).abs() < 1e-9, "area {}", mesh.area());
    }

    #[test]
    fn fill_rings_donut_excludes_the_hole() {
        // 10x10 outer with a 4x4 hole -> 100 - 16 = 84.
        let outer = rect(0.0, 0.0, 10.0, 10.0);
        let hole = rect(3.0, 3.0, 7.0, 7.0);
        let mesh = fill_rings(&[outer, hole]).expect("donut fills");
        assert!(
            (mesh.area() - 84.0).abs() < 1e-6,
            "donut area {}",
            mesh.area()
        );
        // No triangle may sit entirely inside the hole.
        for t in &mesh.triangles {
            let cx = (mesh.vertices[t[0] as usize][0]
                + mesh.vertices[t[1] as usize][0]
                + mesh.vertices[t[2] as usize][0])
                / 3.0;
            let cy = (mesh.vertices[t[0] as usize][1]
                + mesh.vertices[t[1] as usize][1]
                + mesh.vertices[t[2] as usize][1])
                / 3.0;
            let in_hole = (3.0..7.0).contains(&cx) && (3.0..7.0).contains(&cy);
            assert!(!in_hole, "triangle centroid {cx},{cy} inside the hole");
        }
    }

    #[test]
    fn fill_rings_nested_island_is_solid_again() {
        // Even-odd: 10x10 - 6x6 hole + 2x2 island = 100 - 36 + 4 = 68.
        let mesh = fill_rings(&[
            rect(0.0, 0.0, 10.0, 10.0),
            rect(2.0, 2.0, 8.0, 8.0),
            rect(4.0, 4.0, 6.0, 6.0),
        ])
        .expect("nested rings fill");
        assert!(
            (mesh.area() - 68.0).abs() < 1e-6,
            "nested area {}",
            mesh.area()
        );
    }

    #[test]
    fn fill_rings_two_disjoint_outers_both_fill() {
        let mesh = fill_rings(&[rect(0.0, 0.0, 2.0, 2.0), rect(5.0, 0.0, 6.0, 1.0)])
            .expect("disjoint outers fill");
        assert!((mesh.area() - 5.0).abs() < 1e-9, "area {}", mesh.area());
    }

    #[test]
    fn fill_rings_handles_a_polygon_with_apex_bands() {
        // A triangle tapers to an apex at the top and bottom of its band
        // decomposition; the fill must not drop those bands.
        let mesh = fill_rings(&[vec![[0.0, 0.0], [2.0, 0.0], [1.0, 2.0]]]).expect("triangle fills");
        assert!((mesh.area() - 2.0).abs() < 1e-9, "area {}", mesh.area());
        // A general (non-axis-aligned) pentagon must also fill.
        let pentagon = vec![
            [1.0, 0.0],
            [0.309, 0.951],
            [-0.809, 0.588],
            [-0.809, -0.588],
            [0.309, -0.951],
        ];
        let pent = fill_rings(&[pentagon]).expect("pentagon fills");
        assert!(pent.area() > 2.0, "pentagon area {}", pent.area());
    }

    #[test]
    fn fill_rings_over_budget_is_an_error_not_a_partial_fill() {
        let ring: Loop = (0..=MAX_FILL_POINTS)
            .map(|i| {
                let t = i as f64 * 0.01;
                [t.cos(), t.sin()]
            })
            .collect();
        assert!(matches!(fill_rings(&[ring]), Err(FillError::Budget { .. })));
    }

    #[test]
    fn fill_rings_rejects_non_finite_and_degenerate() {
        assert!(matches!(fill_rings(&[]), Err(FillError::Empty)));
        assert!(matches!(
            fill_rings(&[vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]]),
            Err(FillError::Empty)
        ));
        assert!(matches!(
            fill_rings(&[vec![[0.0, 0.0], [f64::NAN, 0.0], [1.0, 1.0]]]),
            Err(FillError::Degenerate)
        ));
    }

    #[test]
    fn fill_rings_rejects_self_intersecting_bowtie() {
        // A figure-eight ring has zero net even-odd area but a naive ear clip
        // would happily fill both lobes; the area cross-check refuses it.
        let bowtie = vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]];
        match fill_rings(&[bowtie]) {
            Err(FillError::Degenerate) | Err(FillError::Empty) => {}
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    // --- gradient fills ---

    fn linear_def(angle: f64, stops: Vec<GradientStop>) -> GradientDef {
        GradientDef {
            kind: GradientKind::Linear,
            angle,
            shift: 0.0,
            single_color: false,
            tint: 0.0,
            stops,
        }
    }

    fn red_blue() -> Vec<GradientStop> {
        vec![
            GradientStop {
                value: 0.0,
                rgb: [255, 0, 0],
            },
            GradientStop {
                value: 1.0,
                rgb: [0, 0, 255],
            },
        ]
    }

    #[test]
    fn sample_gradient_interpolates_and_clamps_between_stops() {
        let def = linear_def(0.0, red_blue());
        assert_eq!(sample_gradient(&def, 0.0), [255, 0, 0]);
        assert_eq!(sample_gradient(&def, 1.0), [0, 0, 255]);
        assert_eq!(sample_gradient(&def, 0.5), [128, 0, 128]);
        // Out-of-range clamps to the end stops.
        assert_eq!(sample_gradient(&def, -3.0), [255, 0, 0]);
        assert_eq!(sample_gradient(&def, 9.0), [0, 0, 255]);
    }

    #[test]
    fn sample_gradient_uses_a_single_stop_as_constant() {
        let def = linear_def(
            0.0,
            vec![GradientStop {
                value: 0.5,
                rgb: [10, 20, 30],
            }],
        );
        assert_eq!(sample_gradient(&def, 0.0), [10, 20, 30]);
        assert_eq!(sample_gradient(&def, 1.0), [10, 20, 30]);
    }

    #[test]
    fn normalize_stops_sorts_and_drops_out_of_range() {
        let stops = normalize_stops(vec![
            GradientStop {
                value: 0.9,
                rgb: [1, 2, 3],
            },
            GradientStop {
                value: -0.5,
                rgb: [9, 9, 9],
            },
            GradientStop {
                value: 0.1,
                rgb: [4, 5, 6],
            },
        ]);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].rgb, [4, 5, 6]);
        assert_eq!(stops[1].rgb, [1, 2, 3]);
    }

    #[test]
    fn linear_gradient_along_x_varies_left_to_right() {
        // Unit square fill; LINEAR angle 0 => red at x=0, blue at x=1.
        let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
        let colors = gradient_vertex_colors(&mesh, &linear_def(0.0, red_blue()));
        assert_eq!(colors.len(), mesh.vertices.len());
        for (i, p) in mesh.vertices.iter().enumerate() {
            let expected_r = (255.0 * (1.0 - p[0])).round() as u8;
            let expected_b = (255.0 * p[0]).round() as u8;
            assert!(
                (colors[i][0] as i16 - expected_r as i16).abs() <= 1,
                "vertex {p:?} red {} vs {expected_r}",
                colors[i][0]
            );
            assert!(
                (colors[i][2] as i16 - expected_b as i16).abs() <= 1,
                "vertex {p:?} blue {} vs {expected_b}",
                colors[i][2]
            );
            assert_eq!(colors[i][1], 0, "green channel must stay 0");
        }
    }

    #[test]
    fn linear_gradient_angle_rotates_the_ramp() {
        // Angle PI/2 => ramp along +Y: red at y=0, blue at y=1.
        let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
        let colors =
            gradient_vertex_colors(&mesh, &linear_def(std::f64::consts::FRAC_PI_2, red_blue()));
        for (i, p) in mesh.vertices.iter().enumerate() {
            let expected_b = (255.0 * p[1]).round() as u8;
            assert!(
                (colors[i][2] as i16 - expected_b as i16).abs() <= 1,
                "vertex {p:?} blue {} vs {expected_b}",
                colors[i][2]
            );
        }
    }

    #[test]
    fn single_color_gradient_blends_toward_white_or_black() {
        let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
        let white_tint = GradientDef {
            kind: GradientKind::Linear,
            angle: 0.0,
            shift: 0.0,
            single_color: true,
            tint: 1.0,
            stops: vec![GradientStop {
                value: 0.0,
                rgb: [255, 0, 0],
            }],
        };
        let colors = gradient_vertex_colors(&mesh, &white_tint);
        // Leftmost vertex (x=0) is pure red; rightmost (x=1) is white.
        let left = mesh
            .vertices
            .iter()
            .position(|p| p[0] == 0.0)
            .expect("left vertex");
        let right = mesh
            .vertices
            .iter()
            .position(|p| p[0] == 1.0)
            .expect("right vertex");
        assert_eq!(colors[left], [255, 0, 0]);
        assert_eq!(colors[right], [255, 255, 255]);
    }

    #[test]
    fn spherical_gradient_ramps_by_radius_across_the_boundary() {
        // A kite has boundary vertices at more than one radius once the y-band
        // decomposition adds edge crossings, so the radial ramp is visible. A
        // plain rectangle only yields its four corners and collapses to a
        // constant (a documented tessellation gap).
        let kite: Loop = vec![[1.0, 0.0], [0.0, 3.0], [-1.0, 0.0], [0.0, -1.0]];
        let mesh = fill_rings(&[kite]).expect("fills");
        let def = GradientDef {
            kind: GradientKind::Spherical,
            angle: 0.0,
            shift: 0.0,
            single_color: false,
            tint: 0.0,
            stops: red_blue(),
        };
        let colors = gradient_vertex_colors(&mesh, &def);
        // The gradient centre is the bounds midpoint (0, 1). The farthest
        // vertices from it (0, 3) and (0, -1) are at t = 1 (blue); the nearest,
        // (±1, 0), are at t = 1/sqrt(2) and must be redder.
        let color_at = |want: [f64; 2]| -> [u8; 3] {
            let i = mesh
                .vertices
                .iter()
                .position(|p| (p[0] - want[0]).abs() < 1e-9 && (p[1] - want[1]).abs() < 1e-9)
                .unwrap_or_else(|| panic!("vertex {want:?} present"));
            colors[i]
        };
        assert_eq!(color_at([0.0, 3.0]), [0, 0, 255]);
        assert_eq!(color_at([0.0, -1.0]), [0, 0, 255]);
        let near = color_at([1.0, 0.0]);
        assert!(near[2] < 255 && near[0] > 0, "near vertex {near:?}");
    }

    #[test]
    fn unusable_gradient_produces_no_colors() {
        let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
        let empty = linear_def(0.0, Vec::new());
        assert!(gradient_vertex_colors(&mesh, &empty).is_empty());
        let nan_angle = linear_def(f64::NAN, red_blue());
        assert!(gradient_vertex_colors(&mesh, &nan_angle).is_empty());
    }

    #[test]
    fn shift_moves_the_ramp_start() {
        let mesh = fill_rings(&[rect(0.0, 0.0, 1.0, 1.0)]).expect("fills");
        let mut def = linear_def(0.0, red_blue());
        def.shift = 0.5;
        let colors = gradient_vertex_colors(&mesh, &def);
        // At shift 0.5 the left edge (t=0) maps to 0.5 => mid purple.
        let left = mesh
            .vertices
            .iter()
            .position(|p| p[0] == 0.0)
            .expect("left vertex");
        assert_eq!(colors[left], [128, 0, 128]);
    }

    #[test]
    fn pattern_respects_a_nested_island() {
        // A hole inside the outer, an island inside the hole: even-odd means the
        // pattern must paint the island but not the hole ring.
        let family = PatternLine {
            angle: 0.0,
            base: [0.0, 0.5],
            offset: [0.0, 1.0],
            dashes: Vec::new(),
        };
        let lines = pattern_polylines(
            &[
                rect(0.0, 0.0, 10.0, 10.0),
                rect(2.0, 2.0, 8.0, 8.0),
                rect(4.0, 4.0, 6.0, 6.0),
            ],
            &[family],
        );
        // Row y=4.5 crosses the island (4..6) and must be painted; rows in the
        // hole but outside the island (e.g. y=2.5) must not span the hole.
        fn spans_y(lines: &[Loop], y: f64) -> Vec<(f64, f64)> {
            lines
                .iter()
                .filter(|l| (l[0][1] - y).abs() < 1e-9 && (l[1][1] - y).abs() < 1e-9)
                .map(|l| (l[0][0].min(l[1][0]), l[0][0].max(l[1][0])))
                .collect()
        }
        let island_row = spans_y(&lines, 4.5);
        assert_eq!(
            island_row,
            vec![(0.0, 2.0), (4.0, 6.0), (8.0, 10.0)],
            "island row {island_row:?}"
        );
        let hole_row = spans_y(&lines, 2.5);
        // y=2.5 is inside the hole but outside the island: only the two outer
        // bands (x=0..2 and 8..10) are painted.
        assert_eq!(hole_row, vec![(0.0, 2.0), (8.0, 10.0)], "{hole_row:?}");
    }
}
