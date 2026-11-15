//! Clipping primitives against a **general** paper clip polygon.
//!
//! The axis-aligned rectangle keeps its dedicated half-plane clippers in
//! [`super::clip`]; every other simple polygon is handled here with a crossing
//! algorithm, because a concave boundary has no half-plane representation.
//!
//! * **Polylines / line segments**: each segment is split at every boundary
//!   crossing and each surviving span is classified by the even-odd point-in-
//!   polygon rule at its midpoint. A concave clip therefore yields *several*
//!   visible spans of one segment, never its bounding box.
//! * **Analytic conics**: the exact parameters where the conic meets each
//!   boundary edge are solved in closed form, so a circle stays a circle and
//!   only its inside arcs survive.
//! * **Filled geometry (meshes) and polygons**: the clip polygon is
//!   ear-clipped into convex triangles. A subject triangle is intersected with
//!   each convex window (Sutherland–Hodgman, exact for a convex window); the
//!   windows partition the clip, so the union of the pieces is exactly
//!   `subject ∩ clip` — concave clips included, with no bounding-box fallback
//!   and no lost area.
//!
//! Every entry point refuses only non-finite geometry, with the same stable
//! [`super::clip_reason`] codes the rectangle path uses; the caller keeps the
//! original primitive and reports the degradation.

use std::cmp::Ordering;

use cad_domain::*;

use crate::ConicSegment;

use super::clip::{
    clip_conic_to_rect, clip_mesh_to_polygon, clip_polygon_to_rect, clip_polyline_to_rect,
};
use super::clip_reason;
use super::polygon::PaperClip;
use super::selection::ClipRefusal;

/// Full turn in radians.
const TAU: f64 = std::f64::consts::TAU;
/// Parameter/angle tolerance for merging and deduplicating split points.
const PARAM_EPS: f64 = 1e-12;
/// Angular tolerance for clipping conic parameter intervals.
const ANGLE_EPS: f64 = 1e-12;

/// Clip a transformed polyline to `clip`, dispatching to the rectangle path when
/// the clip *is* an axis-aligned rectangle so its output stays identical.
pub fn clip_polyline(paper: &[Point3], clip: &PaperClip) -> Vec<Vec<Point3>> {
    match clip.as_rect() {
        Some(_) => clip_polyline_to_rect(paper, clip.center(), clip.half()),
        None => clip_polyline_to_polygon(paper, clip),
    }
}

/// Clip an analytic conic to `clip`, dispatching for rectangles as above.
pub fn clip_conic(
    conic: &ConicSegment,
    clip: &PaperClip,
) -> Result<Vec<ConicSegment>, ClipRefusal> {
    match clip.as_rect() {
        Some(_) => clip_conic_to_rect(conic, clip.center(), clip.half()),
        None => clip_conic_to_polygon(conic, clip),
    }
}

/// Clip a mesh to `clip`, dispatching for rectangles as above.
pub fn clip_mesh(mesh: &Mesh, clip: &PaperClip) -> Result<Option<Mesh>, ClipRefusal> {
    match clip.as_rect() {
        Some(_) => super::clip::clip_mesh_to_rect(mesh, clip.center(), clip.half()),
        None => clip_mesh_to_polygon(mesh, clip),
    }
}

/// Clip a filled polygon to `clip`, returning its convex pieces.
///
/// A rectangular clip returns its single Sutherland–Hodgman result, exactly as
/// before; a general polygon returns the (disjoint) intersections with each
/// convex window of the clip triangulation.
pub fn clip_filled_polygon(polygon: &[[f64; 2]], clip: &PaperClip) -> Vec<Vec<[f64; 2]>> {
    if clip.as_rect().is_some() {
        let clipped = clip_polygon_to_rect(polygon, clip.center(), clip.half());
        return if clipped.is_empty() {
            Vec::new()
        } else {
            vec![clipped]
        };
    }
    clip_polygon_to_polygon(polygon, clip)
}

/// Clip one polyline to a simple (possibly concave) clip polygon.
///
/// Each segment is split at every boundary crossing; a span is kept when its
/// midpoint is inside (even-odd rule). Runs of surviving spans are joined into
/// polylines so a curve crossing the viewport stays one continuous run.
pub fn clip_polyline_to_polygon(paper: &[Point3], clip: &PaperClip) -> Vec<Vec<Point3>> {
    let mut runs: Vec<Vec<Point3>> = Vec::new();
    let mut current: Vec<Point3> = Vec::new();
    for segment in paper.windows(2) {
        let spans = segment_spans(segment[0], segment[1], clip);
        if spans.is_empty() {
            flush_run(&mut runs, &mut current);
            continue;
        }
        for (index, (start, end)) in spans.into_iter().enumerate() {
            // Only the first span of a segment can continue the previous run:
            // it starts at the vertex the previous segment ended on.
            let joins = index == 0 && current.last().map(|last| *last == start).unwrap_or(false);
            if !joins {
                flush_run(&mut runs, &mut current);
            }
            push_point(&mut current, start);
            push_point(&mut current, end);
        }
    }
    flush_run(&mut runs, &mut current);
    runs
}

fn flush_run(runs: &mut Vec<Vec<Point3>>, current: &mut Vec<Point3>) {
    if current.len() >= 2 {
        runs.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

fn push_point(current: &mut Vec<Point3>, point: Point3) {
    if current.last().map(|last| *last == point).unwrap_or(false) {
        return;
    }
    current.push(point);
}

/// The inside sub-spans of one segment, in traversal order.
fn segment_spans(a: Point3, b: Point3, clip: &PaperClip) -> Vec<(Point3, Point3)> {
    let mut cuts = vec![0.0, 1.0];
    for (start, end) in clip_edges(clip) {
        if let Some(t) = segment_edge_parameter(a, b, start, end) {
            if t > PARAM_EPS && t < 1.0 - PARAM_EPS {
                cuts.push(t);
            }
        }
    }
    cuts.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    cuts.dedup_by(|left, right| (*left - *right).abs() <= PARAM_EPS);
    let lerp = |t: f64| {
        if t == 0.0 {
            a
        } else if t == 1.0 {
            b
        } else {
            Point3 {
                x: a.x + (b.x - a.x) * t,
                y: a.y + (b.y - a.y) * t,
                z: a.z + (b.z - a.z) * t,
            }
        }
    };
    let mut spans = Vec::new();
    for pair in cuts.windows(2) {
        let (low, high) = (pair[0], pair[1]);
        if high - low <= PARAM_EPS {
            continue;
        }
        // Crossings are exactly the cut parameters, so the midpoint of a span
        // is strictly inside or strictly outside: one test classifies it.
        let middle = lerp(0.5 * (low + high));
        if clip.contains([middle.x, middle.y]) {
            spans.push((lerp(low), lerp(high)));
        }
    }
    spans
}

/// The parameter `t` on segment `a`–`b` where it meets edge `p`–`q`, if it does.
///
/// Parallel and collinear cases return `None`: the segment either never crosses
/// the edge line or lies on it, and a span lying on the boundary is classified
/// as inside by [`PaperClip::contains`] without needing a synthetic cut.
fn segment_edge_parameter(a: Point3, b: Point3, p: [f64; 2], q: [f64; 2]) -> Option<f64> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (ex, ey) = (q[0] - p[0], q[1] - p[1]);
    let extent = dx.abs().max(dy.abs()).max(ex.abs()).max(ey.abs());
    let denominator = dx * ey - dy * ex;
    if extent == 0.0 || denominator.abs() <= 1e-12 * extent * extent {
        return None;
    }
    let (wx, wy) = (p[0] - a.x, p[1] - a.y);
    let t = (wx * ey - wy * ex) / denominator;
    let u = (wx * dy - wy * dx) / denominator;
    if !(-PARAM_EPS..=1.0 + PARAM_EPS).contains(&u) {
        return None;
    }
    Some(t)
}

/// Clip an analytic conic to a simple (possibly concave) clip polygon.
///
/// The parameters where the conic meets each boundary edge are solved exactly
/// (`A·cos t + B·sin t = C`), the drawn parameter domain is cut there, and each
/// slice is kept when its midpoint is inside. A conic that does not touch the
/// boundary is kept whole or dropped whole — never approximated by chords.
pub fn clip_conic_to_polygon(
    conic: &ConicSegment,
    clip: &PaperClip,
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

    let forward = conic.sweep >= 0.0;
    // The drawn set expressed as a distance travelled from `start`, so negative
    // sweeps keep their traversal direction.
    let domain = if conic.full || conic.sweep.abs() >= TAU - ANGLE_EPS {
        TAU
    } else {
        conic.sweep.abs()
    };
    if domain <= ANGLE_EPS {
        return Ok(Vec::new());
    }
    let parameter_at = |distance: f64| {
        if forward {
            (conic.start + distance).rem_euclid(TAU)
        } else {
            (conic.start - distance).rem_euclid(TAU)
        }
    };
    let point_at = |t: f64| {
        [
            conic.center.x + conic.major.x * t.cos() + conic.minor.x * t.sin(),
            conic.center.y + conic.major.y * t.cos() + conic.minor.y * t.sin(),
        ]
    };

    let mut cuts = vec![0.0, domain];
    for (start, end) in clip_edges(clip) {
        // Line through the edge: normal `n`, offset `rhs`, so the edge is
        // `n · (p - center) = rhs`.
        let n = [end[1] - start[1], start[0] - end[0]];
        let rhs = n[0] * (start[0] - conic.center.x) + n[1] * (start[1] - conic.center.y);
        let a = n[0] * conic.major.x + n[1] * conic.major.y;
        let b = n[0] * conic.minor.x + n[1] * conic.minor.y;
        let radius = a.hypot(b);
        if radius <= 0.0 {
            // A degenerate conic flattened along this edge line: either the
            // whole conic is on the line or none of it is, and the midpoint
            // classification below decides.
            continue;
        }
        let ratio = rhs / radius;
        if ratio.abs() > 1.0 + ANGLE_EPS {
            continue;
        }
        let phase = b.atan2(a);
        let alpha = ratio.clamp(-1.0, 1.0).acos();
        for candidate in [phase + alpha, phase - alpha] {
            let t = candidate.rem_euclid(TAU);
            let distance = if forward {
                (t - conic.start).rem_euclid(TAU)
            } else {
                (conic.start - t).rem_euclid(TAU)
            };
            if distance > ANGLE_EPS && distance < domain - ANGLE_EPS {
                cuts.push(distance);
            }
        }
    }
    cuts.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    cuts.dedup_by(|left, right| (*left - *right).abs() <= ANGLE_EPS);

    let mut pieces: Vec<ConicSegment> = Vec::new();
    for pair in cuts.windows(2) {
        let (low, high) = (pair[0], pair[1]);
        if high - low <= ANGLE_EPS {
            continue;
        }
        let inside = clip.contains(point_at(parameter_at(0.5 * (low + high))));
        if !inside {
            continue;
        }
        let start = parameter_at(low);
        let span = high - low;
        if let Some(previous) = pieces.last_mut() {
            let previous_end = previous.start + previous.sweep;
            let gap = if forward {
                (start - previous_end).rem_euclid(TAU)
            } else {
                (previous_end - start).rem_euclid(TAU)
            };
            if gap <= ANGLE_EPS {
                // Contiguous with the previous slice: one arc, not two.
                previous.sweep += if forward { span } else { -span };
                continue;
            }
        }
        pieces.push(ConicSegment {
            center: conic.center,
            major: conic.major,
            minor: conic.minor,
            start,
            sweep: if forward { span } else { -span },
            full: false,
        });
    }

    // No crossing at all means the conic is wholly inside or wholly outside; the
    // inside case keeps the source untouched (a full circle stays analytic and
    // dash phase stays stable).
    if cuts.len() == 2 && pieces.len() == 1 {
        if let Some(piece) = pieces.first() {
            if (piece.sweep.abs() - domain).abs() <= ANGLE_EPS {
                return Ok(vec![*conic]);
            }
        }
    }
    Ok(pieces)
}

/// Clip a filled polygon to a simple (possibly concave) clip polygon.
///
/// Returns the disjoint convex pieces of `polygon ∩ clip`. A concave clip can
/// cut one polygon into several pieces (an L-shaped window across a quad yields
/// two), which is why the result is a list of convex polygons rather than one.
pub fn clip_polygon_to_polygon(polygon: &[[f64; 2]], clip: &PaperClip) -> Vec<Vec<[f64; 2]>> {
    if polygon.len() < 3 {
        return Vec::new();
    }
    if clip.holds(polygon) {
        return vec![polygon.to_vec()];
    }
    let mut pieces = Vec::new();
    for window in clip.triangles() {
        let piece = clip_points_to_convex(polygon.to_vec(), &window);
        if piece.len() >= 3 {
            pieces.push(piece);
        }
    }
    pieces
}

/// Sutherland–Hodgman clip of a polygon against a **convex** counter-clockwise
/// window.
pub(super) fn clip_points_to_convex(input: Vec<[f64; 2]>, window: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out = input;
    let count = window.len();
    for index in 0..count {
        if out.is_empty() {
            return out;
        }
        out = clip_points_edge(out, window[index], window[(index + 1) % count]);
    }
    out.dedup();
    if out.len() >= 2 && out.first() == out.last() {
        out.pop();
    }
    out
}

fn clip_points_edge(input: Vec<[f64; 2]>, a: [f64; 2], b: [f64; 2]) -> Vec<[f64; 2]> {
    if input.is_empty() {
        return Vec::new();
    }
    let side = |p: [f64; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    let count = input.len();
    let mut out = Vec::with_capacity(count + 1);
    for index in 0..count {
        let current = input[index];
        let previous = input[(index + count - 1) % count];
        let current_side = side(current);
        let previous_side = side(previous);
        if current_side >= 0.0 {
            if previous_side < 0.0 {
                out.push(intersect_line(
                    previous,
                    current,
                    previous_side,
                    current_side,
                ));
            }
            out.push(current);
        } else if previous_side >= 0.0 {
            out.push(intersect_line(
                previous,
                current,
                previous_side,
                current_side,
            ));
        }
    }
    out
}

/// The point where `previous`–`current` meets the clipping line, from the two
/// signed side values.
fn intersect_line(
    previous: [f64; 2],
    current: [f64; 2],
    previous_side: f64,
    current_side: f64,
) -> [f64; 2] {
    let denominator = previous_side - current_side;
    let t = if denominator.abs() > 1e-300 {
        previous_side / denominator
    } else {
        0.0
    };
    [
        previous[0] + (current[0] - previous[0]) * t,
        previous[1] + (current[1] - previous[1]) * t,
    ]
}

/// The clip boundary's edges as `(start, end)` pairs.
fn clip_edges(clip: &PaperClip) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
    let boundary = clip.boundary();
    (0..boundary.len()).map(move |index| (boundary[index], boundary[(index + 1) % boundary.len()]))
}
