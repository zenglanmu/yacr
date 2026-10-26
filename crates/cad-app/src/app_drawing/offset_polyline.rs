//! The LWPOLYLINE arm of `OFFSET`: a constant-distance, **miter-joined** offset
//! of a polyline whose every segment is straight.
//!
//! ## What is supported
//!
//! Open and closed polylines whose every segment is a straight segment (a bulge
//! within [`STRAIGHT_BULGE`], the tessellator's own straight-segment rule — see
//! `app_drawing::explode`). The pick point (`side_point`) decides **one** side and
//! **one** distance for the whole polyline, exactly like the LINE arm: the
//! distance is the pick's in-plane perpendicular distance to the line of the
//! picked (nearest) segment, and every segment is moved that far to the pick's
//! side. Vertices are joined with **miter** joins, the vertex count and order are
//! preserved (`n` in → `n` out) and a closed polyline stays closed.
//!
//! ## Declared construction
//!
//! With `n` the polyline's plane unit normal, `u_i` the unit direction of segment
//! `i` and `m_i = cross(n, u_i)` its in-plane `+90°` direction, the offset of
//! segment `i` is `p + s·d·m_i` (`d` the distance, `s` the pick's side sign) and
//! the vertex between segments `a` and `b` moves by
//!
//! ```text
//! s·d·(m_a + m_b) / (1 + m_a·m_b)
//! ```
//!
//! which is the analytic miter point (the intersection of the two offset lines).
//! The end vertices of an **open** polyline are not mitred: they move with their
//! own single segment (`p + s·d·m_i`), so no end cap is invented.
//!
//! The plane is derived from the polyline itself (the first pair of non-parallel
//! adjacent segment directions); a non-planar polyline is refused. A polyline
//! whose segments are **all collinear** (a "straight" polyline) has no plane of
//! its own, so — like the LINE arm — its plane is the one the line spans with the
//! pick point. The pick's out-of-plane component is ignored: it decides only the
//! side and the distance *within* the polyline's plane.
//!
//! ## Refusals (explicit, nothing written)
//!
//! * any segment with a real bulge (an arc segment) ⇒ `Unsupported` naming
//!   `offset.bulge_segment`. Bridging a bulge segment honestly needs an arc-offset
//!   construction this command does not implement, and dropping the bulge would
//!   silently change the geometry;
//! * a degenerate (zero-length) segment, fewer than two vertices, a non-finite
//!   vertex, a zero-area closed polyline or a pick on the polyline itself
//!   (distance 0) ⇒ `InvalidInput`;
//! * a non-planar polyline ⇒ `Unsupported` naming `offset.non_planar` (there is no
//!   single offset side for a skew vertex);
//! * a result that **collapses at a vertex** — a near-180° reversal (the miter
//!   denominator vanishes), or an offset segment that folds back or shrinks to
//!   nothing — ⇒ `Unsupported` naming `offset.collapsed`, instead of emitting
//!   bow-tie geometry.
//!
//! ## Honest limitation
//!
//! Only **vertex-local** collapse is detected. An offset whose *distant* segments
//! cross each other (a global self-intersection that no single vertex reveals) is
//! NOT detected here; it is not silently reported as fine either — it simply is
//! not checked, and that is a known gap of this command.

use super::helpers::offset_tolerance;
use super::*;
use cad_geometry::{add, cross, dot, length, normalize, scale, sub};

/// Below this a polyline bulge is a straight segment (the tessellator's rule,
/// matching `app_drawing::explode`'s `STRAIGHT_BULGE`).
const STRAIGHT_BULGE: f64 = 1e-12;

/// The `sin` of the turn below which two adjacent unit segment directions count
/// as parallel.
///
/// A **direction** predicate, so it is dimensionless (the magnitude of a `cross`
/// of two unit vectors) rather than a world distance; it is derived from the
/// tolerance policy's computation tolerance so the judgement is not a second,
/// hand-picked constant.
fn turn_sine_tolerance() -> f64 {
    TolerancePolicy::default().computation_world.max(1e-12)
}

/// The miter denominator below which the two offset lines are treated as
/// parallel (a doubling-back vertex) and the join is refused as collapsed.
///
/// `1 + m_a·m_b` is `1 + cos(turn)`; it vanishes only as the turn approaches a
/// half turn. A *long* (but finite) miter at a very sharp corner is exact miter
/// geometry and is deliberately not refused.
fn miter_reversal_tolerance() -> f64 {
    TolerancePolicy::default().computation_world.max(1e-12)
}

/// Offset a straight-segment LWPOLYLINE to the side of `side_point`.
///
/// Returns the new geometry and the offset distance that was applied (the pick's
/// perpendicular distance to the picked segment's line), so the command can
/// report the exact value instead of a re-derivation.
pub(super) fn offset_polyline(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    side_point: Point3,
) -> CadResult<(SemanticGeometry, f64)> {
    let tol = offset_tolerance();
    // An arc segment is a capability gap, not bad input: name it and write
    // nothing (the caller has not started a transaction yet).
    for (index, bulge) in bulges.iter().enumerate() {
        if !bulge.is_finite() || bulge.abs() > STRAIGHT_BULGE {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.bulge_segment: LWPOLYLINE segment {index} is an arc \
                 (bulge {bulge}); only polylines whose every segment is straight can be offset"
            )));
        }
    }
    if points.len() < 2 {
        return Err(CadError::InvalidInput(
            "OFFSET source LWPOLYLINE needs at least two vertices".into(),
        ));
    }
    if !points.iter().all(|p| is_finite_point(*p)) {
        return Err(CadError::InvalidInput(
            "OFFSET source LWPOLYLINE has a non-finite vertex".into(),
        ));
    }

    let dirs = unit_segment_directions(points, closed, tol)?;
    let normal = polyline_normal(points, &dirs, closed, side_point, tol)?;
    let signed = pick_signed_distance(points, &dirs, normal, side_point);
    let distance = signed.abs();
    if !distance.is_finite() || distance <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET distance must be positive; pick a point off the polyline".into(),
        ));
    }

    // Every segment moves by the same signed vector magnitude, so all of them
    // stay parallel to their source segment (constant distance, no tapering).
    let offsets: Vec<Point3> = dirs
        .iter()
        .map(|u| scale(cross(normal, *u), signed))
        .collect();

    let count = points.len();
    let segments = dirs.len();
    let mut out: Vec<Point3> = Vec::with_capacity(count);
    if closed {
        for index in 0..count {
            let previous = (index + segments - 1) % segments;
            out.push(miter_vertex(
                points[index],
                dirs[previous],
                dirs[index],
                normal,
                signed,
            )?);
        }
    } else {
        // The two end vertices are not mitred: one segment each, no invented cap.
        out.push(add(points[0], offsets[0]));
        for index in 1..count - 1 {
            out.push(miter_vertex(
                points[index],
                dirs[index - 1],
                dirs[index],
                normal,
                signed,
            )?);
        }
        out.push(add(points[count - 1], offsets[segments - 1]));
    }
    require_no_vertex_collapse(&out, &dirs, tol)?;
    Ok((
        SemanticGeometry::Polyline {
            points: out,
            bulges: bulges.to_vec(),
            closed,
        },
        distance,
    ))
}

/// The unit direction of every segment, in ring order.
///
/// A zero-length segment is an explicit refusal: it has no direction, so there is
/// no offset side and no way to keep the vertex count meaningful.
fn unit_segment_directions(points: &[Point3], closed: bool, tol: f64) -> CadResult<Vec<Point3>> {
    let count = points.len();
    let segments = if closed { count } else { count - 1 };
    let mut dirs = Vec::with_capacity(segments);
    for index in 0..segments {
        let vector = sub(points[(index + 1) % count], points[index]);
        let len = length(vector);
        if !len.is_finite() || len <= tol {
            return Err(CadError::InvalidInput(format!(
                "OFFSET source LWPOLYLINE segment {index} is degenerate (zero length)"
            )));
        }
        dirs.push(scale(vector, 1.0 / len));
    }
    Ok(dirs)
}

/// The polyline's plane normal, derived from its own first real corner.
fn polyline_normal(
    points: &[Point3],
    dirs: &[Point3],
    closed: bool,
    side_point: Point3,
    tol: f64,
) -> CadResult<Point3> {
    let segments = dirs.len();
    let pairs = if closed { segments } else { segments - 1 };
    let sine_tol = turn_sine_tolerance();
    let mut normal: Option<Point3> = None;
    for index in 0..pairs {
        let turn = cross(dirs[index], dirs[(index + 1) % segments]);
        if length(turn) > sine_tol {
            normal = Some(normalize(turn));
            break;
        }
    }
    let Some(normal) = normal else {
        if closed {
            return Err(CadError::InvalidInput(
                "OFFSET source LWPOLYLINE is a degenerate closed shape (it has no real corner)"
                    .into(),
            ));
        }
        // A straight polyline: no plane of its own, so the LINE arm's frame is
        // used (the line plus the pick point).
        let toward = sub(side_point, points[0]);
        let reach = length(toward);
        if !reach.is_finite() || reach <= tol {
            return Err(CadError::InvalidInput(
                "OFFSET distance must be positive; pick a point off the polyline".into(),
            ));
        }
        let sine = length(cross(dirs[0], toward)) / reach;
        if !sine.is_finite() || sine <= sine_tol {
            return Err(CadError::InvalidInput(
                "OFFSET distance must be positive; pick a point off the polyline".into(),
            ));
        }
        return Ok(normalize(cross(dirs[0], toward)));
    };
    // The normal came from two of the polyline's own edges, so every vertex must
    // lie in that plane; a skew vertex has no single offset side.
    let base = points[0];
    for (index, point) in points.iter().enumerate() {
        if dot(sub(*point, base), normal).abs() > tol {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.non_planar: LWPOLYLINE vertex {index} is off the polyline's own \
                 plane; a non-planar polyline has no single offset side"
            )));
        }
    }
    Ok(normal)
}

/// The pick's **signed** in-plane perpendicular distance to the line of the
/// segment nearest the pick.
///
/// The nearest segment is the one with the smallest Euclidean distance to the
/// finite segment; the value returned is the pick's perpendicular distance to
/// that segment's own line (the LINE arm's rule), signed along `cross(n, u)`, so
/// its sign is the side to offset to.
fn pick_signed_distance(
    points: &[Point3],
    dirs: &[Point3],
    normal: Point3,
    side_point: Point3,
) -> f64 {
    let count = points.len();
    let mut best: Option<(f64, f64)> = None;
    for (index, u) in dirs.iter().enumerate() {
        let start = points[index];
        let end = points[(index + 1) % count];
        let toward = sub(side_point, start);
        let along = dot(toward, *u);
        let signed = dot(toward, cross(normal, *u));
        let reach = length(sub(end, start));
        let distance = if along <= 0.0 {
            length(toward)
        } else if along >= reach {
            length(sub(side_point, end))
        } else {
            signed.abs()
        };
        if best.map(|(current, _)| distance < current).unwrap_or(true) {
            best = Some((distance, signed));
        }
    }
    best.map(|(_, signed)| signed).unwrap_or(0.0)
}

/// The analytic miter vertex between two adjacent segments.
///
/// A vanishing denominator is a doubling-back vertex: the two offset lines are
/// parallel, so there is no join, only a spike of unbounded length — refused.
fn miter_vertex(
    vertex: Point3,
    before: Point3,
    after: Point3,
    normal: Point3,
    signed_distance: f64,
) -> CadResult<Point3> {
    let m_before = cross(normal, before);
    let m_after = cross(normal, after);
    let denominator = 1.0 + dot(m_before, m_after);
    if !denominator.is_finite() || denominator <= miter_reversal_tolerance() {
        return Err(CadError::Unsupported(format!(
            "OFFSET offset.collapsed: the polyline doubles back at vertex ({:.6}, {:.6}); \
             the offset has no miter join there",
            vertex.x, vertex.y
        )));
    }
    Ok(add(
        vertex,
        scale(add(m_before, m_after), signed_distance / denominator),
    ))
}

/// Refuse an offset that folded a segment back on itself or shrank it away.
///
/// Every offset segment must keep its source segment's direction (`dot > 0`) and
/// a real length; anything else is a bow-tie the user did not ask for.
fn require_no_vertex_collapse(out: &[Point3], dirs: &[Point3], tol: f64) -> CadResult<()> {
    for (index, u) in dirs.iter().enumerate() {
        let next = (index + 1) % out.len();
        let segment = sub(out[next], out[index]);
        let len = length(segment);
        let along = dot(segment, *u);
        if !len.is_finite() || len <= tol || !along.is_finite() || along <= tol {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.collapsed: offset segment {index} collapses to zero length or \
                 folds back (it would self-intersect at its vertex)"
            )));
        }
    }
    Ok(())
}
