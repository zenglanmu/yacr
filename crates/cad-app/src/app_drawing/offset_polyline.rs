//! The LWPOLYLINE arm of `OFFSET`: a constant-distance, **miter-joined** offset.
//!
//! ## What is supported
//!
//! Open and closed polylines in any plane. A segment whose bulge is within
//! [`STRAIGHT_BULGE`] is straight and is offset here; a segment with a real bulge
//! is a circular arc and is offset by the sibling arm
//! [`super::offset_polyline_bulge`] (concentric, exact, arc-aware joins — see that
//! module for the construction and its refusals). Both arms keep the vertex count
//! and order (`n` in → `n` out), the closed flag, and carry one bulge per vertex.
//!
//! The pick point (`side_point`) decides **one** side and **one** distance for the
//! whole polyline, exactly like the LINE arm: the distance is the pick's in-plane
//! distance to the picked (nearest) segment, and every segment is moved that far to
//! the pick's side. Straight vertices are joined with **miter** joins.
//!
//! ## `OFFSETGAPTYPE`
//!
//! [`OffsetGapType::Sharp`] (0) is this module's analytic miter. `Fillet` (1) and
//! `Chamfer` (2) fill each real gap at a joint through [`super::offset_gap`]: the
//! tangent arc of radius equal to the offset distance (centred on the source
//! vertex) or the straight segment between the two offset segment ends. Where the
//! offset segments cross (a concave corner) the miter trims and no gap is filled;
//! the sharp path is byte-identical to the pre-gap-type implementation.
//!
//! ## Declared construction (straight segments)
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
//! pick point. (With a bulge present that is no longer sound: an arc read in an
//! assumed plane is a different arc, so the arc arm refuses it as
//! `offset.ambiguous_plane`.) The pick's out-of-plane component is ignored: it
//! decides only the side and the distance *within* the polyline's plane.
//!
//! ## Refusals (explicit, nothing written)
//!
//! * a bulge that is not a constructible arc (non-finite, or a full turn or more)
//!   ⇒ `Unsupported` naming `offset.bulge_segment`; an arc segment that cannot be
//!   offset ⇒ the arc arm's own named refusals (`offset.arc_collapsed`,
//!   `offset.join_failed`, `offset.ambiguous_plane`, `offset.arc_mismatch`);
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
//! Only **vertex-local** collapse is detected by the miter/join checks. A
//! **closed** offset ring is additionally sampled and refused by name
//! (`offset.self_intersection`) if it crosses itself away from a vertex; an
//! **open** offset polyline is not self-intersection-checked (Autodesk does not
//! document a trimming rule for either case, and this build does not invent
//! one).

use super::helpers::offset_tolerance;
use super::*;
use crate::command_payload::OffsetGapType;
use cad_geometry::{
    add, arbitrary_axis, clamped_uniform_knots, cross, dot, length, normalize, scale, sub,
    tessellate_geometry, NurbsCurve, TessellationParams,
};

/// Below this a polyline bulge is a straight segment (the tessellator's rule,
/// matching `app_drawing::explode`'s `STRAIGHT_BULGE`).
pub(super) const STRAIGHT_BULGE: f64 = 1e-12;

/// The `sin` of the turn below which two adjacent unit segment directions count
/// as parallel.
///
/// A **direction** predicate, so it is dimensionless (the magnitude of a `cross`
/// of two unit vectors) rather than a world distance; it is derived from the
/// tolerance policy's computation tolerance so the judgement is not a second,
/// hand-picked constant.
pub(super) fn turn_sine_tolerance() -> f64 {
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

/// Offset a LWPOLYLINE to the side of `side_point`.
///
/// Returns the new geometry and the offset distance that was applied (the pick's
/// distance to the picked segment), so the command can report the exact value
/// instead of a re-derivation. A polyline with at least one real bulge is handed
/// to the arc arm ([`super::offset_polyline_bulge`]); everything here is then the
/// straight-only path and is byte-for-byte what it always was under
/// [`OffsetGapType::Sharp`].
pub(super) fn offset_polyline(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    side_point: Point3,
    gap: OffsetGapType,
) -> CadResult<(SemanticGeometry, f64)> {
    offset_polyline_with_distance(points, bulges, closed, side_point, None, gap)
}

/// The same arm with an optional fixed distance.
///
/// `distance = Some(d)` implements the `OFFSET` **distance** prompt: the side
/// point only selects the side, every segment still moves by exactly `d`.
/// `None` is the bare-command `Through` behaviour (the pick's distance to the
/// nearest segment is used).
///
/// `gap` is `OFFSETGAPTYPE`: [`OffsetGapType::Sharp`] keeps the analytic miter
/// (byte-identical to the pre-gap-type path), while [`OffsetGapType::Fillet`]
/// and [`OffsetGapType::Chamfer`] fill every real gap between two offset
/// segments at a joint and keep the miter where the offset segments already
/// cross (a trim, not a gap).
pub(super) fn offset_polyline_with_distance(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    side_point: Point3,
    distance: Option<f64>,
    gap: OffsetGapType,
) -> CadResult<(SemanticGeometry, f64)> {
    let tol = offset_tolerance();
    // An arc segment is the sibling arm's job; a bulge that is not even a finite
    // number is nobody's: name it and write nothing (the caller has not started a
    // transaction yet).
    if bulges
        .iter()
        .any(|bulge| bulge.is_finite() && bulge.abs() > STRAIGHT_BULGE)
    {
        return super::offset_polyline_bulge::offset_polyline_with_arcs(
            points, bulges, closed, side_point, distance, gap,
        );
    }
    for (index, bulge) in bulges.iter().enumerate() {
        if !bulge.is_finite() {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.bulge_segment: LWPOLYLINE segment {index} has a non-finite bulge \
                 ({bulge}), which is not a constructible arc"
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
    let picked = pick_signed_distance(points, &dirs, normal, side_point);
    let signed = match distance {
        Some(requested) => {
            if picked.abs() <= tol {
                return Err(CadError::InvalidInput(
                    "OFFSET side point must not lie on the polyline when a distance is given"
                        .into(),
                ));
            }
            picked.signum() * requested
        }
        None => picked,
    };
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
    if gap != OffsetGapType::Sharp {
        return super::offset_gap::offset_straight_with_gap(
            points, bulges, closed, gap, &dirs, &offsets, normal, signed, distance, tol,
        );
    }
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
    let geometry = SemanticGeometry::Polyline {
        points: out,
        bulges: bulges.to_vec(),
        closed,
    };
    if closed {
        require_no_closed_self_intersection(&geometry, normal, tol)?;
    }
    Ok((geometry, distance))
}

pub(super) fn unit_segment_directions(
    points: &[Point3],
    closed: bool,
    tol: f64,
) -> CadResult<Vec<Point3>> {
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
        let (distance, signed) = line_pick(start, end, *u, normal, side_point);
        if best.map(|(current, _)| distance < current).unwrap_or(true) {
            best = Some((distance, signed));
        }
    }
    best.map(|(_, signed)| signed).unwrap_or(0.0)
}

/// The pick's distance to one straight segment and its signed offset value.
///
/// The distance is the Euclidean distance to the **finite** segment (an end point
/// for a pick beyond an end), while the signed value is always the pick's
/// perpendicular displacement along `m = cross(n, u)` — the LINE arm's rule. The
/// arc arm shares this function, so a line pick can never mean two different
/// things in the two arms.
pub(super) fn line_pick(
    start: Point3,
    end: Point3,
    u: Point3,
    normal: Point3,
    side_point: Point3,
) -> (f64, f64) {
    let toward = sub(side_point, start);
    let along = dot(toward, u);
    let signed = dot(toward, cross(normal, u));
    let reach = length(sub(end, start));
    let distance = if along <= 0.0 {
        length(toward)
    } else if along >= reach {
        length(sub(side_point, end))
    } else {
        signed.abs()
    };
    (distance, signed)
}

/// The analytic miter vertex between two adjacent segments.
///
/// A vanishing denominator is a doubling-back vertex: the two offset lines are
/// parallel, so there is no join, only a spike of unbounded length — refused.
pub(super) fn miter_vertex(
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

// ---------------------------------------------------------------------------
// Sampled offset arms: ELLIPSE / elliptical arc and SPLINE sources.
//
// A constant-distance offset of an ellipse is **not** an ellipse (the offset
// curve has a different eccentricity), so both arms produce a real B-spline
// through sampled offset points — the "oval-shaped spline" AutoCAD produces —
// and never an `Ellipse`.
//
// ## Sampling tolerance
//
// The samples are chosen so the sampled polyline's chord deviation from the
// true offset is at most `scale · 1e-4` world units (and never below the
// tolerance policy's predicate tolerance), where `scale` is the curve's largest
// radius / control-polygon extent. The step follows the circle sagitta
// `h ≈ sqrt(8 · tol · scale)`; the sample count is clamped to `[16, 2048]`, so
// the tolerance is honoured up to that budget and reported as a documented
// approximation, never as an exact curve.
//
// ## Result kind
//
// The result is a **clamped, approximating** cubic B-spline whose control
// points are the sampled offsets (stored with the `knots: []` clamped-uniform
// shorthand and `weights: []` all-ones shorthand). The curve interpolates its
// first and last sample; interior samples are approximated. A full ellipse
// repeats its first sample at the end, so the result closes visually; the
// domain has no closed-spline flag, and that is stated here rather than
// invented in the geometry.
// ---------------------------------------------------------------------------

/// The world sampling tolerance shared by both sampled arms.
fn sample_tolerance(scale: f64) -> f64 {
    (scale.abs() * 1e-4).max(offset_tolerance())
}

/// The number of sampling steps for a parameter span and a curve scale.
fn sample_steps(span: f64, scale: f64) -> usize {
    let scale = scale.abs().max(offset_tolerance());
    let step = (8.0 * sample_tolerance(scale) * scale).sqrt();
    // Arc length is at most `span · scale`; `step` is the chord budget.
    let steps = (span.max(0.0) * scale / step.max(1e-12)).ceil();
    if !steps.is_finite() {
        return 2048;
    }
    (steps as usize).clamp(16, 2048)
}

/// Build the clamped cubic (degree 3, degree 1 for a degenerate count) spline
/// through `points`, using the database's shorthand conventions.
pub(super) fn spline_through_points(points: Vec<Point3>) -> CadResult<SemanticGeometry> {
    if points.len() < 2 || !points.iter().all(|point| is_finite_point(*point)) {
        return Err(CadError::InvalidInput(
            "OFFSET sampled offset needs at least two finite points".into(),
        ));
    }
    let degree = if points.len() >= 4 { 3 } else { 1 };
    Ok(SemanticGeometry::Spline {
        degree,
        // Empty vectors are the database's documented shorthand:
        // clamped-uniform knots and unit weights.
        knots: Vec::new(),
        control_points: points,
        weights: Vec::new(),
    })
}

/// Offset an `ELLIPSE` (full) or an elliptical arc into a spline.
///
/// `distance = Some(d)` is the fixed-distance prompt; `None` lets
/// `side_point` define the distance (the bare/`Through` behaviour). The side
/// point also selects the side in both cases.
#[allow(clippy::too_many_arguments)]
pub(super) fn offset_ellipse_as_spline(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    side_point: Point3,
    distance_override: Option<f64>,
) -> CadResult<(SemanticGeometry, f64)> {
    let tol = offset_tolerance();
    let major_len = length(major_axis);
    if !is_finite_point(center)
        || !is_finite_point(side_point)
        || !major_len.is_finite()
        || major_len <= tol
        || !ratio.is_finite()
        || ratio.abs() <= tol
        || !start.is_finite()
        || !sweep.is_finite()
    {
        return Err(CadError::InvalidInput(
            "OFFSET ellipse: source parameters must be finite and non-degenerate".into(),
        ));
    }
    let span = sweep.abs();
    let full_turn = std::f64::consts::TAU;
    if span <= tol || span > full_turn + tol {
        return Err(CadError::Unsupported(format!(
            "OFFSET offset.ellipse_span: an elliptical arc sweep of {sweep} is not a storable \
             span (0 < |sweep| <= 2π)"
        )));
    }
    let unit_normal = if length(normal) <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET ellipse: source plane normal is degenerate".into(),
        ));
    } else {
        normalize(normal)
    };
    let major = scale(major_axis, 1.0 / major_len);
    let minor_dir = normalize(cross(unit_normal, major));
    if !is_finite_point(minor_dir) || length(minor_dir) <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET ellipse: source plane frame is degenerate".into(),
        ));
    }
    let minor = scale(minor_dir, major_len * ratio.abs());
    let point_at = |angle: f64| {
        add(
            center,
            add(scale(major_axis, angle.cos()), scale(minor, angle.sin())),
        )
    };
    let outward_at = |angle: f64| {
        let tangent = add(scale(major_axis, -angle.sin()), scale(minor, angle.cos()));
        let outward = cross(tangent, unit_normal);
        (length(tangent) > tol && length(outward) > tol).then(|| normalize(outward))
    };
    let scale_extent = major_len.max(major_len * ratio.abs());
    let steps = sample_steps(span, scale_extent);
    let mut samples = Vec::with_capacity(steps + 1);
    let mut normals = Vec::with_capacity(steps + 1);
    for index in 0..=steps {
        let angle = start + sweep * (index as f64) / (steps as f64);
        let Some(outward) = outward_at(angle) else {
            return Err(CadError::Unsupported(
                "OFFSET offset.collapsed: the ellipse has a point with no offset direction".into(),
            ));
        };
        samples.push(point_at(angle));
        normals.push(outward);
    }
    let (signed, applied) =
        pick_side_and_distance(&samples, &normals, side_point, distance_override, tol)?;
    let offsets = samples
        .iter()
        .zip(&normals)
        .map(|(point, outward)| add(*point, scale(*outward, signed)))
        .collect();
    let geometry = spline_through_points(offsets)?;
    if span >= full_turn - tol {
        require_no_closed_self_intersection(&geometry, unit_normal, tol)?;
    }
    Ok((geometry, applied))
}

/// Offset a `SPLINE` into a new spline through sampled offset points.
///
/// The source must be **planar**: a general 3D spline has no single offset
/// plane, so a non-planar spline is refused by name (`offset.non_planar`). The
/// plane comes from the control polygon (or, for a collinear control polygon,
/// from the first tangent and the side point exactly like the LINE arm).
pub(super) fn offset_spline_as_spline(
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
    side_point: Point3,
    distance_override: Option<f64>,
) -> CadResult<(SemanticGeometry, f64)> {
    let tol = offset_tolerance();
    let count = control_points.len();
    if count < 2 || !control_points.iter().all(|point| is_finite_point(*point)) {
        return Err(CadError::InvalidInput(
            "OFFSET source SPLINE needs at least two finite control points".into(),
        ));
    }
    let degree_usize = degree as usize;
    if degree_usize == 0 || count < degree_usize + 1 {
        return Err(CadError::InvalidInput(format!(
            "OFFSET source SPLINE degree {degree} is not indexable with {count} control points"
        )));
    }
    let expanded_knots = if knots.is_empty() {
        clamped_uniform_knots(count, degree_usize)
    } else {
        knots.to_vec()
    };
    let curve = NurbsCurve::new(
        degree_usize,
        control_points.to_vec(),
        expanded_knots,
        weights.to_vec(),
    )?;
    let (t0, t1) = curve.domain();
    if t1 <= t0 {
        return Err(CadError::InvalidInput(
            "OFFSET source SPLINE has an empty parameter domain".into(),
        ));
    }
    // The plane comes from the control polygon; a non-planar one is refused.
    let mut normal: Option<Point3> = None;
    let sine_tol = turn_sine_tolerance();
    for index in 0..count.saturating_sub(2) {
        let turn = cross(
            sub(control_points[index + 1], control_points[index]),
            sub(control_points[index + 2], control_points[index]),
        );
        if length(turn) > sine_tol {
            normal = Some(normalize(turn));
            break;
        }
    }
    let normal = match normal {
        Some(normal) => {
            let base = control_points[0];
            for (index, point) in control_points.iter().enumerate() {
                if dot(sub(*point, base), normal).abs() > tol {
                    return Err(CadError::Unsupported(format!(
                        "OFFSET offset.non_planar: SPLINE control point {index} is off the curve's \
                         own plane; a non-planar spline has no single offset side"
                    )));
                }
            }
            normal
        }
        None => {
            // Collinear control polygon: the plane is the tangent plus the pick.
            let tangent = spline_tangent(&curve, t0, t0, t1, tol)?;
            let toward = sub(side_point, control_points[0]);
            if length(cross(tangent, toward)) <= sine_tol {
                return Err(CadError::InvalidInput(
                    "OFFSET distance must be positive; pick a point off the spline".into(),
                ));
            }
            normalize(cross(tangent, toward))
        }
    };
    let polygon_len: f64 = control_points
        .windows(2)
        .map(|pair| length(sub(pair[1], pair[0])))
        .sum();
    let steps = sample_steps(
        std::f64::consts::TAU,
        polygon_len.max(length(sub(control_points[1], control_points[0]))),
    );
    let mut samples = Vec::with_capacity(steps + 1);
    let mut normals = Vec::with_capacity(steps + 1);
    for index in 0..=steps {
        let t = t0 + (t1 - t0) * (index as f64) / (steps as f64);
        let tangent = spline_tangent(&curve, t, t0, t1, tol)?;
        let outward = cross(tangent, normal);
        if length(outward) <= sine_tol {
            return Err(CadError::Unsupported(
                "OFFSET offset.collapsed: the spline has a point with no offset direction".into(),
            ));
        }
        samples.push(curve.evaluate(t));
        normals.push(normalize(outward));
    }
    let (signed, applied) =
        pick_side_and_distance(&samples, &normals, side_point, distance_override, tol)?;
    let offsets = samples
        .iter()
        .zip(&normals)
        .map(|(point, outward)| add(*point, scale(*outward, signed)))
        .collect();
    let geometry = spline_through_points(offsets)?;
    let closed = length(sub(samples[samples.len() - 1], samples[0])) <= tol.max(polygon_len * 1e-6);
    if closed {
        require_no_closed_self_intersection(&geometry, normal, tol)?;
    }
    Ok((geometry, applied))
}

/// The finite-difference tangent of a spline at `t`.
fn spline_tangent(curve: &NurbsCurve, t: f64, t0: f64, t1: f64, tol: f64) -> CadResult<Point3> {
    let span = t1 - t0;
    let h = (span * 1e-6).max(span * 1e-12);
    let (a, b) = if t - h < t0 {
        (t, (t + h).min(t1))
    } else if t + h > t1 {
        ((t - h).max(t0), t)
    } else {
        (t - h, t + h)
    };
    let tangent = sub(curve.evaluate(b), curve.evaluate(a));
    let len = length(tangent);
    if !len.is_finite() || len <= tol {
        return Err(CadError::Unsupported(
            "OFFSET offset.collapsed: the spline has no tangent at a sampled parameter".into(),
        ));
    }
    Ok(scale(tangent, 1.0 / len))
}

/// The pick's side sign and the applied distance for a sampled curve.
fn pick_side_and_distance(
    samples: &[Point3],
    normals: &[Point3],
    side_point: Point3,
    distance_override: Option<f64>,
    tol: f64,
) -> CadResult<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for (point, outward) in samples.iter().zip(normals) {
        let toward = sub(side_point, *point);
        let distance = length(toward);
        if best.map(|(current, _)| distance < current).unwrap_or(true) {
            best = Some((distance, dot(toward, *outward)));
        }
    }
    let (_, signed_pick) = best.unwrap_or((0.0, 0.0));
    let signed = match distance_override {
        Some(requested) => {
            if signed_pick.abs() <= tol {
                return Err(CadError::InvalidInput(
                    "OFFSET side point must not lie on the curve when a distance is given".into(),
                ));
            }
            signed_pick.signum() * requested
        }
        None => signed_pick,
    };
    let applied = signed.abs();
    if !applied.is_finite() || applied <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET distance must be positive; pick a point off the curve".into(),
        ));
    }
    Ok((signed, applied))
}

/// Refuse a **closed** offset ring whose sampled curve crosses itself.
///
/// Closed self-intersection behaviour is not documented by Autodesk, so this
/// build does not trim/fill/merge the ring: it reports the named refusal
/// `offset.self_intersection` and writes nothing. The check samples the result
/// (tolerance `32 ×` the predicate tolerance, at most 512 segments) and looks
/// for a proper crossing between non-adjacent ring segments.
pub(super) fn require_no_closed_self_intersection(
    geometry: &SemanticGeometry,
    normal: Point3,
    tol: f64,
) -> CadResult<()> {
    let params = TessellationParams {
        tolerance: (tol * 32.0).max(1e-6),
        max_segments: 512,
        min_segments: 16,
    };
    let mut samples = tessellate_geometry(geometry, params);
    if samples.len() < 4 {
        return Ok(());
    }
    if length(sub(samples[samples.len() - 1], samples[0])) <= tol * 4.0 {
        samples.pop();
    }
    let ring = samples.len();
    if ring < 4 {
        return Ok(());
    }
    let (ax, ay, _) = arbitrary_axis(normal);
    let projected: Vec<(f64, f64)> = samples.iter().map(|p| (dot(*p, ax), dot(*p, ay))).collect();
    for i in 0..ring {
        let a0 = projected[i];
        let a1 = projected[(i + 1) % ring];
        for j in (i + 1)..ring {
            // Adjacent ring segments share a vertex; that is not a crossing.
            if (i + 1) % ring == j || (j + 1) % ring == i {
                continue;
            }
            if segments_properly_cross(a0, a1, projected[j], projected[(j + 1) % ring]) {
                return Err(CadError::Unsupported(
                    "OFFSET offset.self_intersection: the offset ring crosses itself; closed \
                     self-intersection behaviour is not documented, so this build refuses rather \
                     than trimming or filling the result"
                        .into(),
                ));
            }
        }
    }
    Ok(())
}

/// A proper crossing of two 2D segments (interior intersection, not a shared end).
fn segments_properly_cross(a0: (f64, f64), a1: (f64, f64), b0: (f64, f64), b1: (f64, f64)) -> bool {
    const EPS: f64 = 1e-9;
    let cross2 = |p: (f64, f64), q: (f64, f64)| p.0 * q.1 - p.1 * q.0;
    let r = (a1.0 - a0.0, a1.1 - a0.1);
    let s = (b1.0 - b0.0, b1.1 - b0.1);
    let denominator = cross2(r, s);
    if denominator.abs() <= EPS {
        return false;
    }
    let delta = (b0.0 - a0.0, b0.1 - a0.1);
    let t = cross2(delta, s) / denominator;
    let u = cross2(delta, r) / denominator;
    t > EPS && t < 1.0 - EPS && u > EPS && u < 1.0 - EPS
}

#[cfg(test)]
mod self_intersection_tests {
    use super::*;

    /// Closed self-intersection is not documented by Autodesk, so the ring check
    /// must report the named refusal (never trim/fill). A bow-tie ring is the
    /// smallest shape that crosses itself away from any vertex.
    #[test]
    fn a_closed_ring_that_crosses_itself_is_refused_by_name() {
        let geometry = SemanticGeometry::Polyline {
            points: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 10.0,
                    y: 10.0,
                    z: 0.0,
                },
                Point3 {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: 10.0,
                    z: 0.0,
                },
            ],
            bulges: Vec::new(),
            closed: true,
        };
        let error = require_no_closed_self_intersection(
            &geometry,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            1e-6,
        )
        .expect_err("a bow-tie ring crosses itself");
        match error {
            CadError::Unsupported(reason) => {
                assert!(reason.contains("offset.self_intersection"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }
}
