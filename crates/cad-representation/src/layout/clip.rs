use std::cmp::Ordering;

use cad_domain::*;

use crate::ConicSegment;

use super::clip_reason;
use super::selection::ClipRefusal;
use super::view::finite_point;

/// Full turn in radians.
const TAU: f64 = std::f64::consts::TAU;
/// Angular tolerance for merging/clamping clipped parameter intervals.
const ANGLE_EPS: f64 = 1e-12;

/// A set of parameter intervals inside `[0, TAU)`.
type Intervals = Vec<(f64, f64)>;

/// Clip one transformed polyline to a paper rectangle.
///
/// `paper` are points already in paper coordinates; the rectangle is
/// `[center - half, center + half]`. Each segment is clipped independently with
/// Liang–Barsky, and runs of surviving segments are joined into polylines so a
/// curve crossing the window yields a continuous clipped run. A polyline fully
/// outside yields no pieces; a polyline inside is returned unchanged.
pub fn clip_polyline_to_rect(
    paper: &[Point3],
    center: [f64; 2],
    half: [f64; 2],
) -> Vec<Vec<Point3>> {
    let (min, max) = (
        [center[0] - half[0], center[1] - half[1]],
        [center[0] + half[0], center[1] + half[1]],
    );
    let mut runs: Vec<Vec<Point3>> = Vec::new();
    let mut current: Vec<Point3> = Vec::new();
    let push_point = |current: &mut Vec<Point3>, p: Point3| {
        if current.last().map(|last| *last == p).unwrap_or(false) {
            return;
        }
        current.push(p);
    };
    for segment in paper.windows(2) {
        match clip_segment(segment[0], segment[1], min, max) {
            Some((a, b)) => {
                push_point(&mut current, a);
                push_point(&mut current, b);
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

/// Clip an analytic conic to an axis-aligned paper rectangle
/// `[center - half, center + half]`.
///
/// The conic is `center + major*cos(t) + minor*sin(t)` for `t` in
/// `[start, start+sweep]` (the whole circle when `full`). The result is the
/// exact set of parameter slices that lie inside the rectangle, emitted as
/// [`ConicSegment`]s with the original traversal direction. Circles and
/// ellipses therefore stay analytic (they are not tessellated), and a curve
/// that misses the window is dropped entirely.
///
/// Returns [`ClipRefusal`] only for non-finite geometry; the caller keeps the
/// original primitive and reports the degradation rather than guessing.
pub fn clip_conic_to_rect(
    conic: &ConicSegment,
    center: [f64; 2],
    half: [f64; 2],
) -> Result<Vec<ConicSegment>, ClipRefusal> {
    let finite = |p: &Point3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
    if !finite(&conic.center)
        || !finite(&conic.major)
        || !finite(&conic.minor)
        || !conic.start.is_finite()
        || !conic.sweep.is_finite()
    {
        return Err(ClipRefusal::new(
            clip_reason::CONIC_UNCLIPPABLE,
            "conic parameters are not finite and cannot be clipped exactly",
        ));
    }

    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    let by_x = axis_intervals(conic.center.x, conic.major.x, conic.minor.x, min[0], max[0]);
    let by_y = axis_intervals(conic.center.y, conic.major.y, conic.minor.y, min[1], max[1]);
    let mut inside = intersect_intervals(&by_x, &by_y);
    if inside.is_empty() {
        return Ok(Vec::new());
    }

    let domain = arc_domain(conic);
    let domain_len = total_length(&domain);
    inside = intersect_intervals(&inside, &domain);
    if inside.is_empty() {
        return Ok(Vec::new());
    }
    // The whole visible arc survives: keep the original conic untouched so a
    // full circle stays a full circle (SDF-friendly) and dash phase is stable.
    if (total_length(&inside) - domain_len).abs() <= ANGLE_EPS {
        return Ok(vec![*conic]);
    }

    // Emit slices in traversal order. A negative sweep keeps its direction so
    // downstream dash/cap semantics see the same parameter flow as the source.
    let forward = conic.sweep >= 0.0;
    let distance = |t: f64| -> f64 {
        if forward {
            (t - conic.start).rem_euclid(TAU)
        } else {
            (conic.start - t).rem_euclid(TAU)
        }
    };
    inside.sort_by(|a, b| {
        let start_a = if forward { a.0 } else { a.1 };
        let start_b = if forward { b.0 } else { b.1 };
        distance(start_a)
            .partial_cmp(&distance(start_b))
            .unwrap_or(Ordering::Equal)
    });

    let mut pieces = Vec::with_capacity(inside.len());
    for (lo, hi) in inside {
        if hi - lo <= ANGLE_EPS {
            continue;
        }
        pieces.push(ConicSegment {
            center: conic.center,
            major: conic.major,
            minor: conic.minor,
            start: if forward { lo } else { hi },
            sweep: if forward { hi - lo } else { lo - hi },
            full: false,
        });
    }
    Ok(pieces)
}

/// Parameter intervals of one axis constraint `lo <= c + a*cos t + b*sin t <= hi`.
fn axis_intervals(c: f64, a: f64, b: f64, lo: f64, hi: f64) -> Intervals {
    let radius = a.hypot(b);
    if radius == 0.0 {
        // A constant coordinate: either always inside or never.
        return if c >= lo && c <= hi {
            vec![(0.0, TAU)]
        } else {
            Vec::new()
        };
    }
    let phase = b.atan2(a);
    let allowed = cosine_interval_set((lo - c) / radius, (hi - c) / radius);
    shift_intervals(allowed, phase)
}

/// `theta` in `[0, TAU)` where `lo <= cos(theta) <= hi`.
fn cosine_interval_set(lo: f64, hi: f64) -> Intervals {
    if lo.is_nan() || hi.is_nan() || lo > hi {
        return Vec::new();
    }
    let below = if hi >= 1.0 {
        vec![(0.0, TAU)]
    } else if hi <= -1.0 {
        Vec::new()
    } else {
        let edge = hi.acos();
        vec![(edge, TAU - edge)]
    };
    let above = if lo <= -1.0 {
        vec![(0.0, TAU)]
    } else if lo >= 1.0 {
        Vec::new()
    } else {
        let edge = lo.acos();
        vec![(0.0, edge), (TAU - edge, TAU)]
    };
    intersect_intervals(&below, &above)
}

/// Rotate every interval by `phase` on the parameter circle.
fn shift_intervals(intervals: Intervals, phase: f64) -> Intervals {
    let mut shifted = Vec::with_capacity(intervals.len());
    for (lo, hi) in intervals {
        let length = hi - lo;
        if length >= TAU - ANGLE_EPS {
            shifted.push((0.0, TAU));
            continue;
        }
        let start = (lo + phase).rem_euclid(TAU);
        let end = (hi + phase).rem_euclid(TAU);
        if (end - start).rem_euclid(TAU) <= ANGLE_EPS {
            continue;
        }
        if start < end {
            shifted.push((start, end));
        } else {
            shifted.push((start, TAU));
            shifted.push((0.0, end));
        }
    }
    normalize_intervals(shifted)
}

/// The parameter set actually drawn by the conic, on `[0, TAU)`.
fn arc_domain(conic: &ConicSegment) -> Intervals {
    if conic.full || conic.sweep.abs() >= TAU - ANGLE_EPS {
        return vec![(0.0, TAU)];
    }
    if conic.sweep == 0.0 {
        return Vec::new();
    }
    let start = conic.start.rem_euclid(TAU);
    let end = (conic.start + conic.sweep).rem_euclid(TAU);
    if conic.sweep > 0.0 {
        if start < end {
            vec![(start, end)]
        } else {
            vec![(start, TAU), (0.0, end)]
        }
    } else if end < start {
        vec![(end, start)]
    } else {
        vec![(0.0, start), (end, TAU)]
    }
}

/// Intersection of two interval sets, merged and clamped to `[0, TAU)`.
fn intersect_intervals(left: &[(f64, f64)], right: &[(f64, f64)]) -> Intervals {
    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < left.len() && j < right.len() {
        let lo = left[i].0.max(right[j].0);
        let hi = left[i].1.min(right[j].1);
        if hi - lo > ANGLE_EPS {
            out.push((lo, hi));
        }
        if left[i].1 < right[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    normalize_intervals(out)
}

/// Sort, clamp and merge overlapping/touching intervals.
fn normalize_intervals(mut intervals: Intervals) -> Intervals {
    intervals.retain(|(lo, hi)| hi - lo > ANGLE_EPS);
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    let mut out: Intervals = Vec::with_capacity(intervals.len());
    for (lo, hi) in intervals {
        let lo = lo.max(0.0);
        let hi = hi.min(TAU);
        if hi - lo <= ANGLE_EPS {
            continue;
        }
        match out.last_mut() {
            Some(last) if lo <= last.1 + ANGLE_EPS => {
                if hi > last.1 {
                    last.1 = hi;
                }
            }
            _ => out.push((lo, hi)),
        }
    }
    out
}

fn total_length(intervals: &[(f64, f64)]) -> f64 {
    intervals.iter().map(|(lo, hi)| hi - lo).sum()
}

/// Liang–Barsky clip of one segment to an axis-aligned rectangle.
///
/// Returns `None` when the segment is fully outside, `Some((a, b))` with the
/// (possibly shortened) endpoints otherwise. A degenerate/zero-length segment
/// inside the rectangle is returned as an inside point.
fn clip_segment(a: Point3, b: Point3, min: [f64; 2], max: [f64; 2]) -> Option<(Point3, Point3)> {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    let checks = [
        (-dx, a.x - min[0]),
        (dx, max[0] - a.x),
        (-dy, a.y - min[1]),
        (dy, max[1] - a.y),
    ];
    for (p, q) in checks {
        if p == 0.0 {
            // Parallel to this edge: outside when already beyond it.
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
    let lerp = |t: f64| Point3 {
        x: a.x + dx * t,
        y: a.y + dy * t,
        z: a.z + (b.z - a.z) * t,
    };
    Some((lerp(t0), lerp(t1)))
}

/// Sutherland–Hodgman clip of a convex polygon against an axis-aligned paper
/// rectangle `[center - half, center + half]`.
///
/// Returns the surviving convex polygon (empty when the input lies fully
/// outside). Consecutive duplicate vertices are removed so the result can be
/// fan-triangulated safely. The viewport windows this build accepts are exactly
/// axis-aligned rectangles (a non-rectangular clip is refused upstream in
/// [`viewport_transform`]), which is the convex window this routine handles.
pub fn clip_polygon_to_rect(
    polygon: &[[f64; 2]],
    center: [f64; 2],
    half: [f64; 2],
) -> Vec<[f64; 2]> {
    if polygon.is_empty() {
        return Vec::new();
    }
    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    let mut out = polygon.to_vec();
    out = clip_points_half_plane(out, 0, min[0], true);
    out = clip_points_half_plane(out, 0, max[0], false);
    out = clip_points_half_plane(out, 1, min[1], true);
    out = clip_points_half_plane(out, 1, max[1], false);
    dedupe_points(&mut out);
    out
}

fn clip_points_half_plane(
    input: Vec<[f64; 2]>,
    axis: usize,
    bound: f64,
    keep_above: bool,
) -> Vec<[f64; 2]> {
    if input.is_empty() {
        return Vec::new();
    }
    let inside = |p: [f64; 2]| {
        if keep_above {
            p[axis] >= bound
        } else {
            p[axis] <= bound
        }
    };
    let n = input.len();
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let current = input[i];
        let previous = input[(i + n - 1) % n];
        let current_in = inside(current);
        let previous_in = inside(previous);
        if current_in {
            if !previous_in {
                out.push(intersect_axis(previous, current, axis, bound));
            }
            out.push(current);
        } else if previous_in {
            out.push(intersect_axis(previous, current, axis, bound));
        }
    }
    out
}

fn intersect_axis(a: [f64; 2], b: [f64; 2], axis: usize, bound: f64) -> [f64; 2] {
    let denom = b[axis] - a[axis];
    let t = if denom.abs() > 1e-300 {
        (bound - a[axis]) / denom
    } else {
        0.0
    };
    let other = 1 - axis;
    let mut out = a;
    out[axis] = bound;
    out[other] = a[other] + (b[other] - a[other]) * t;
    out
}

fn dedupe_points(points: &mut Vec<[f64; 2]>) {
    points.dedup();
    if points.len() >= 2 && points.first() == points.last() {
        points.pop();
    }
}

/// One triangle vertex carrying every attribute the mesh can interpolate.
#[derive(Clone, Copy)]
struct ClipVertex {
    pos: [f64; 2],
    z: f64,
    normal: Option<Point3>,
    color: Option<[u8; 3]>,
}

impl ClipVertex {
    /// Whether two vertices are identical in every clipped attribute, so a
    /// duplicate produced by clipping can be removed without losing data.
    fn same(&self, other: &ClipVertex) -> bool {
        self.pos == other.pos
            && self.z == other.z
            && self.normal == other.normal
            && self.color == other.color
    }

    fn lerp(&self, other: &ClipVertex, t: f64) -> ClipVertex {
        ClipVertex {
            pos: [
                self.pos[0] + (other.pos[0] - self.pos[0]) * t,
                self.pos[1] + (other.pos[1] - self.pos[1]) * t,
            ],
            z: self.z + (other.z - self.z) * t,
            normal: match (self.normal, other.normal) {
                (Some(a), Some(b)) => Some(Point3 {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                    z: a.z + (b.z - a.z) * t,
                }),
                _ => None,
            },
            color: match (self.color, other.color) {
                (Some(a), Some(b)) => Some([
                    lerp_channel(a[0], b[0], t),
                    lerp_channel(a[1], b[1], t),
                    lerp_channel(a[2], b[2], t),
                ]),
                _ => None,
            },
        }
    }
}

fn lerp_channel(a: u8, b: u8, t: f64) -> u8 {
    let value = a as f64 + (b as f64 - a as f64) * t;
    value.round().clamp(0.0, 255.0) as u8
}

fn clip_clip_vertices(
    input: Vec<ClipVertex>,
    axis: usize,
    bound: f64,
    keep_above: bool,
) -> Vec<ClipVertex> {
    if input.is_empty() {
        return Vec::new();
    }
    let inside = |p: [f64; 2]| {
        if keep_above {
            p[axis] >= bound
        } else {
            p[axis] <= bound
        }
    };
    let n = input.len();
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let current = input[i];
        let previous = input[(i + n - 1) % n];
        let current_in = inside(current.pos);
        let previous_in = inside(previous.pos);
        if current_in {
            if !previous_in {
                out.push(intersect_clip_vertices(previous, current, axis, bound));
            }
            out.push(current);
        } else if previous_in {
            out.push(intersect_clip_vertices(previous, current, axis, bound));
        }
    }
    out
}

fn intersect_clip_vertices(a: ClipVertex, b: ClipVertex, axis: usize, bound: f64) -> ClipVertex {
    let denom = b.pos[axis] - a.pos[axis];
    let t = if denom.abs() > 1e-300 {
        (bound - a.pos[axis]) / denom
    } else {
        0.0
    };
    let mut out = a.lerp(&b, t);
    // Snap exactly onto the clip plane so the boundary is exact, not rounded.
    out.pos[axis] = bound;
    out
}

fn push_clip_vertices(
    vertices: &mut Vec<Point3>,
    normals: &mut Vec<Point3>,
    colors: &mut Vec<[u8; 3]>,
    source: &[ClipVertex],
    has_normals: bool,
    has_colors: bool,
) -> Result<u32, ClipRefusal> {
    let base = u32::try_from(vertices.len()).map_err(|_| {
        ClipRefusal::new(
            clip_reason::MESH_UNCLIPPABLE,
            "clipped mesh exceeds 32-bit vertex addressing",
        )
    })?;
    if vertices.len().saturating_add(source.len()) > u32::MAX as usize {
        return Err(ClipRefusal::new(
            clip_reason::MESH_UNCLIPPABLE,
            "clipped mesh exceeds 32-bit vertex addressing",
        ));
    }
    for vertex in source {
        vertices.push(Point3 {
            x: vertex.pos[0],
            y: vertex.pos[1],
            z: vertex.z,
        });
        if has_normals {
            normals.push(crate::transform::normalize(
                vertex.normal.unwrap_or_default(),
            ));
        }
        if has_colors {
            colors.push(vertex.color.unwrap_or([0, 0, 0]));
        }
    }
    Ok(base)
}

/// Clip every triangle of `mesh` against the paper rectangle `[center ± half]`.
///
/// Each triangle is clipped independently with Sutherland–Hodgman, then
/// fan-triangulated; per-vertex z, normal and sRGB colour are interpolated at
/// the new edge vertices, so a partially visible gradient facet keeps its
/// correct colour ramp. A triangle wholly inside is reused verbatim; a triangle
/// wholly outside disappears.
///
/// `Ok(None)` means the whole mesh lies outside the window (an exact omission).
/// `Err` means the mesh cannot be clipped exactly (a missing index, a non-finite
/// coordinate, or a 32-bit vertex overflow); the caller must keep the original
/// geometry and report the refusal instead of dropping or faking it.
pub fn clip_mesh_to_rect(
    mesh: &Mesh,
    center: [f64; 2],
    half: [f64; 2],
) -> Result<Option<Mesh>, ClipRefusal> {
    let min = [center[0] - half[0], center[1] - half[1]];
    let max = [center[0] + half[0], center[1] + half[1]];
    let has_normals = mesh.normals.len() == mesh.vertices.len();
    let has_colors = mesh.colors.len() == mesh.vertices.len();
    let has_sources = mesh.face_sources.len() == mesh.triangles.len();

    for vertex in &mesh.vertices {
        if !finite_point(vertex) {
            return Err(ClipRefusal::new(
                clip_reason::MESH_UNCLIPPABLE,
                "mesh has a non-finite vertex; it cannot be clipped exactly",
            ));
        }
    }

    let mut vertices: Vec<Point3> = Vec::new();
    let mut normals: Vec<Point3> = Vec::new();
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut face_sources: Vec<Option<SubElementId>> = Vec::new();

    for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
        let indices = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        if indices.iter().any(|index| *index >= mesh.vertices.len()) {
            return Err(ClipRefusal::new(
                clip_reason::MESH_UNCLIPPABLE,
                format!("mesh triangle {triangle_index} references a missing vertex"),
            ));
        }
        let source = if has_sources {
            mesh.face_sources[triangle_index].clone()
        } else {
            None
        };
        let originals = [0usize, 1, 2].map(|k| {
            let vi = indices[k];
            ClipVertex {
                pos: [mesh.vertices[vi].x, mesh.vertices[vi].y],
                z: mesh.vertices[vi].z,
                normal: has_normals.then(|| mesh.normals[vi]),
                color: has_colors.then(|| mesh.colors[vi]),
            }
        });

        // A triangle wholly inside is reused without splitting or re-welding.
        if originals.iter().all(|v| {
            v.pos[0] >= min[0] && v.pos[0] <= max[0] && v.pos[1] >= min[1] && v.pos[1] <= max[1]
        }) {
            let base = push_clip_vertices(
                &mut vertices,
                &mut normals,
                &mut colors,
                &originals,
                has_normals,
                has_colors,
            )?;
            triangles.push([base, base + 1, base + 2]);
            face_sources.push(source.clone());
            continue;
        }

        let mut polygon = originals.to_vec();
        polygon = clip_clip_vertices(polygon, 0, min[0], true);
        polygon = clip_clip_vertices(polygon, 0, max[0], false);
        polygon = clip_clip_vertices(polygon, 1, min[1], true);
        polygon = clip_clip_vertices(polygon, 1, max[1], false);
        polygon.dedup_by(|a, b| a.same(b));
        if polygon.len() >= 2 {
            let (first, last) = (polygon[0], polygon[polygon.len() - 1]);
            if first.same(&last) {
                polygon.pop();
            }
        }
        if polygon.len() < 3 {
            // A fully clipped (or degenerate) triangle contributes no area.
            continue;
        }
        let base = push_clip_vertices(
            &mut vertices,
            &mut normals,
            &mut colors,
            &polygon,
            has_normals,
            has_colors,
        )?;
        for k in 1..polygon.len() - 1 {
            triangles.push([base, base + k as u32, base + k as u32 + 1]);
            face_sources.push(source.clone());
        }
    }

    if triangles.is_empty() {
        return Ok(None);
    }
    Ok(Some(Mesh {
        vertices,
        triangles,
        normals: if has_normals { normals } else { Vec::new() },
        face_sources,
        colors: if has_colors { colors } else { Vec::new() },
    }))
}
