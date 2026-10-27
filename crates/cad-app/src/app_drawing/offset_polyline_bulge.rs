//! The **arc-segment (bulge)** arm of the LWPOLYLINE `OFFSET`.
//!
//! [`super::offset_polyline::offset_polyline`] handles polylines whose every
//! segment is straight; this module handles the rest. A segment with a real bulge
//! (`|bulge| > STRAIGHT_BULGE`) is a circular arc, and it is offset **exactly**:
//! the result is a concentric arc, never a sampled or polyline-approximated one.
//!
//! ## The frame
//!
//! Everything is computed in the polyline's own plane, whose normal `n` comes from
//! [`cad_geometry::polyline_plane_normal`] — the single authority the tessellator
//! and EXPLODE already use. For 2D (XY-parallel) data that is `+Z` even when the
//! vertex winding is clockwise, so a clockwise ring's bulges are read in the DXF
//! sign convention and are never silently mirrored. A polyline whose **chords** are
//! all collinear has no plane of its own; if it still carries a bulge the plane is
//! genuinely ambiguous, so it is refused (`offset.ambiguous_plane`) instead of
//! assuming `+Z` — a bulge read in the wrong plane is a different arc.
//!
//! ## One pick, one side, one distance
//!
//! As in the straight arm, the pick decides **one** signed distance `s` for the
//! whole polyline: the pick's displacement along `m = cross(n, u)` at the nearest
//! point of the nearest segment, where `u` is that segment's unit tangent.
//!
//! * **line**: `s = (pick − start)·m` (the LINE arm's rule, shared through
//!   [`super::offset_polyline::line_pick`]);
//! * **arc** of radius `R` and signed sweep `σ` about `n`: with
//!   `u = sign(σ)·(−sin φ, cos φ)` and `r̂ = (cos φ, sin φ)`,
//!   `m = cross(n, u) = −sign(σ)·r̂`. The pick's distance to the arc's own circle
//!   is the radial difference `|pick − c| − R`, so
//!   `s = (|pick − c| − R)·(−sign σ)`. This is the same quantity the line rule
//!   computes (the pick's displacement along `m`), just measured radially.
//!
//! ## Offsetting a segment
//!
//! A line moves by the vector `s·m` (unchanged from the straight arm). An arc is
//! **concentric**, because its every point moves by
//! `s·m = −s·sign(σ)·r̂`, i.e. along the radius:
//!
//! ```text
//! R' = R − s·sweep.signum()        (centre unchanged)
//! ```
//!
//! `s` already carries the side and `sweep.signum()` is `+1` counter-clockwise
//! about `n`; with `R' = |pick − c|` whenever the picked segment is that arc, the
//! two rules agree by construction. `R' <= tol` is refused as
//! `offset.arc_collapsed` — the segment is neither deleted nor blended into its
//! neighbours (that is AutoCAD's behaviour, not this command's).
//!
//! ## Joins
//!
//! Every interior vertex joins its two offset segments at the intersection
//! **nearest the source vertex**: the analytic miter for line/line (the straight
//! arm's formula, shared through
//! [`super::offset_polyline::miter_vertex`]), and
//! [`cad_geometry::GeometryEngine::intersect_local`]'s analytic conic
//! intersection for line/circle and circle/circle. The circle is fed as a
//! `Circle` and never as an `Arc`: the `Arc` arm filters hits by the source
//! segment's span, and a miter point legitimately lies outside it. The line probe
//! is extended past the corner exactly as FILLET extends its own lines.
//!
//! * zero candidates ⇒ `offset.join_failed`;
//! * two candidates equidistant from the source vertex (within the world
//!   tolerance) ⇒ `offset.join_failed`: nothing picks one over the other;
//! * one candidate is taken as it is — a tangent join is one point, not an
//!   ambiguity;
//! * a candidate outside the source segment's own span is a legitimate **trim**,
//!   not a refusal.
//!
//! A **tangency** is resolved in closed form rather than by the engine: at a
//! tangent pair the engine's analytic solve returns the double root of a quadratic
//! whose discriminant is ~0, where a relative error of `eps` becomes `sqrt(eps)`.
//! Measured on this machine, that drifts the root by ~3e-10 and can flip the
//! discriminant negative, i.e. turn the most common bulged-polyline join (every
//! join of a FILLET-rounded outline) into a spurious `join_failed`. The tangent
//! case has an exact closed form (the perpendicular foot for a line/circle pair,
//! the centre-line touch point for a circle/circle pair), and it is classified
//! with the same distance/radius comparison `cad-geometry`'s own circle solve
//! uses, so the two never disagree about what "tangent" means.
//!
//! ## Re-deriving a trimmed arc
//!
//! An interior arc is trimmed to its two join points. `start`/`sweep` are
//! re-derived from the **centre angles** of those points in the traversal
//! direction, so the sweep's sign is preserved by construction and the result is
//! the forward piece of the offset circle. The sweep is converted with
//! [`cad_geometry::arc_sweep_to_bulge`] and the emitted bulge is fed back through
//! [`cad_geometry::bulge_arc_geometry`]: the recovered centre, radius and sweep
//! must equal the planned ones (`offset.arc_mismatch` otherwise), so a wrong sign
//! can never reach the database.
//!
//! A trimmed arc may **grow**: an outward miter extends *both* of its neighbours,
//! and for an arc "longer" means a larger sweep, exactly as a straight segment
//! gets longer at the same corner. The one thing it may not do is wrap to the far
//! side of its own circle, so growth is capped at half a turn
//! (`|new| <= |old| + π`) — the same bound EXTEND puts on one extension step.
//!
//! ## Refusals (explicit, nothing written)
//!
//! | code | meaning |
//! | --- | --- |
//! | `offset.bulge_segment` | a non-finite bulge, a bulge that is not a constructible arc (a full turn or more, or a degenerate chord) |
//! | `offset.arc_collapsed` | `R' <= tol`: the concentric radius vanishes |
//! | `offset.join_failed` | no intersection between two offset segments, or two equally near ones |
//! | `offset.ambiguous_plane` | every chord is collinear yet a bulge is present (no plane) |
//! | `offset.non_planar` | a vertex off the polyline's own plane |
//! | `offset.arc_mismatch` | the trimmed arc does not survive the bulge round-trip, or wrapped to the far side |
//! | `offset.collapsed` | a trimmed arc or an offset line segment that folds back or shrinks away |
//!
//! A non-finite vertex, a zero-length segment, a pick on the polyline and fewer
//! than two vertices stay `InvalidInput`.
//!
//! ## Declared limitations / deviations
//!
//! * **No global self-intersection trimming.** An offset whose *distant* pieces
//!   cross each other is not detected — the same known gap the straight arm
//!   declares.
//! * **No segment deletion or neighbour blending.** AutoCAD erases an arc whose
//!   offset radius vanishes and re-cases the neighbours; here it is a named
//!   refusal (`offset.arc_collapsed`). Nothing is re-centred, mirrored or flipped
//!   to the other side of its circle ("CYLINDER-style" recasing does not happen).
//! * **Tangent joins are one point.** A tangency is resolved by its closed form
//!   (see above) and accepted, not refused as "ambiguous"; two *distinct*
//!   candidates equally near the vertex are refused.
//! * **Arc growth is allowed** (see above): a strict `|new| <= |old|` rule would
//!   refuse every outward offset at a convex corner.

use super::helpers::offset_tolerance;
use super::offset_polyline::{
    line_pick, miter_vertex, turn_sine_tolerance, unit_segment_directions, STRAIGHT_BULGE,
};
use super::*;
use cad_geometry::{
    add, arbitrary_axis, arc_sweep_to_bulge, bulge_arc_geometry, cross, dot, length,
    polyline_plane_normal, scale, sub, DefaultGeometryEngine, GeometryEngine,
};

/// One full turn: the sweep a single bulge segment cannot reach.
const FULL_TURN: f64 = std::f64::consts::TAU;

/// The most a trimmed arc may grow, in radians (a half turn).
const MAX_SWEEP_GROWTH: f64 = std::f64::consts::PI;

/// One source arc segment, resolved in the polyline's own plane.
///
/// `start`/`sweep` are the angles of [`cad_geometry::bulge_arc_geometry`]'s frame
/// (`arbitrary_axis(plane_normal)`), i.e. the very angles the tessellator and
/// EXPLODE read the bulge with.
#[derive(Debug, Clone, Copy)]
struct SourceArc {
    center: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
}

impl SourceArc {
    /// The arc's point at one plane angle.
    fn point_at(&self, angle: f64, ax: Point3, ay: Point3) -> Point3 {
        add(
            self.center,
            scale(
                add(scale(ax, angle.cos()), scale(ay, angle.sin())),
                self.radius,
            ),
        )
    }

    /// The plane angle of `point` about this arc's centre.
    fn angle_of(&self, point: Point3, ax: Point3, ay: Point3) -> f64 {
        let offset = sub(point, self.center);
        dot(offset, ay).atan2(dot(offset, ax))
    }
}

/// One offset segment, untrimmed, in the polyline's plane.
#[derive(Debug, Clone, Copy)]
enum OffsetSegment {
    /// Already moved by `s·m`; `start`/`end` are the moved endpoints.
    Line { start: Point3, end: Point3 },
    /// Concentric with its source (radius already offset).
    Arc(SourceArc),
}

/// Offset a bulged LWPOLYLINE exactly to the side of `side_point`.
///
/// Returns the new geometry and the offset distance that was applied, exactly
/// like the straight arm. The plan is pure and all-or-nothing: every refusal
/// happens before the caller starts a transaction.
pub(super) fn offset_polyline_with_arcs(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    side_point: Point3,
) -> CadResult<(SemanticGeometry, f64)> {
    let tol = offset_tolerance();
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
    // A non-finite bulge is not an arc at all: name the segment, never guess.
    for (index, bulge) in bulges.iter().enumerate() {
        if !bulge.is_finite() {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.bulge_segment: LWPOLYLINE segment {index} has a non-finite bulge \
                 ({bulge}), which is not a constructible arc"
            )));
        }
    }

    let count = points.len();
    let segments = if closed { count } else { count - 1 };
    let dirs = unit_segment_directions(points, closed, tol)?;
    let normal = arc_plane_normal(points, &dirs, closed, tol)?;
    let arcs = source_arcs(points, bulges, normal, &dirs, tol)?;

    let signed = pick_signed_distance(points, &dirs, &arcs, normal, side_point);
    let distance = signed.abs();
    if !distance.is_finite() || distance <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET distance must be positive; pick a point off the polyline".into(),
        ));
    }

    let offset: Vec<OffsetSegment> = (0..segments)
        .map(|index| offset_segment(index, points, &dirs, &arcs, normal, signed, tol))
        .collect::<CadResult<Vec<_>>>()?;

    let (ax, ay, _) = arbitrary_axis(normal);
    let mut out: Vec<Point3> = Vec::with_capacity(count);
    if closed {
        // One single pass including the wrap corner: a closed ring has a join at
        // every vertex, so the segment count and the vertex count coincide.
        for index in 0..count {
            out.push(join_vertex(
                index, points, &dirs, &offset, normal, signed, tol,
            )?);
        }
    } else {
        // The two ends are not mitred and no cap is invented: they move with
        // their own single segment (for an arc: along its own radius).
        out.push(open_vertex(&offset[0], ax, ay, true));
        for index in 1..count - 1 {
            out.push(join_vertex(
                index, points, &dirs, &offset, normal, signed, tol,
            )?);
        }
        out.push(open_vertex(&offset[segments - 1], ax, ay, false));
    }

    let out_bulges = rederive_bulges(bulges, &offset, &out, normal, tol)?;
    require_no_line_collapse(&out, &offset, &dirs, tol)?;
    Ok((
        SemanticGeometry::Polyline {
            points: out,
            bulges: out_bulges,
            closed,
        },
        distance,
    ))
}

/// The polyline's plane normal, or a named refusal when a bulge has no plane.
///
/// The **chord** pair decides whether the polyline has a plane of its own at all
/// (a bulge cannot invent one: a rotation about its chord leaves every arc of a
/// dumbbell-shaped polyline a valid arc, so `+Z` would be a guess). Once a corner
/// exists, [`cad_geometry::polyline_plane_normal`] supplies the single authoritative
/// normal — including its `+Z` rule for 2D data, which keeps a clockwise ring's
/// bulges in the DXF sign convention.
fn arc_plane_normal(
    points: &[Point3],
    dirs: &[Point3],
    closed: bool,
    tol: f64,
) -> CadResult<Point3> {
    let segments = dirs.len();
    let pairs = if closed { segments } else { segments - 1 };
    let sine_tol = turn_sine_tolerance();
    let mut corner = false;
    for index in 0..pairs {
        if length(cross(dirs[index], dirs[(index + 1) % segments])) > sine_tol {
            corner = true;
            break;
        }
    }
    if !corner {
        return Err(CadError::Unsupported(
            "OFFSET offset.ambiguous_plane: every chord of the LWPOLYLINE is collinear, so an arc \
             segment has no plane of its own; a bulge read in an assumed plane would be a \
             different arc"
                .into(),
        ));
    }
    let normal = polyline_plane_normal(points);
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

/// Resolve every arc segment into the polyline's plane, `None` for a line.
///
/// A bulge that [`cad_geometry::bulge_arc_geometry`] cannot turn into an arc (a
/// full turn or more, a degenerate chord) is a genuinely non-constructible bulge,
/// so it keeps the `offset.bulge_segment` name instead of being straightened.
fn source_arcs(
    points: &[Point3],
    bulges: &[f64],
    normal: Point3,
    dirs: &[Point3],
    tol: f64,
) -> CadResult<Vec<Option<SourceArc>>> {
    let count = points.len();
    let mut arcs = Vec::with_capacity(dirs.len());
    for index in 0..dirs.len() {
        let bulge = bulges.get(index).copied().unwrap_or(0.0);
        if bulge.abs() <= STRAIGHT_BULGE {
            arcs.push(None);
            continue;
        }
        let start = points[index];
        let end = points[(index + 1) % count];
        let Some(SemanticGeometry::Arc {
            center,
            radius,
            start: start_angle,
            sweep,
            ..
        }) = bulge_arc_geometry(start, end, bulge, normal)
        else {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.bulge_segment: LWPOLYLINE segment {index} has bulge {bulge}, which \
                 is not a constructible arc (a full turn or more, or a degenerate chord)"
            )));
        };
        if !radius.is_finite() || radius <= tol || !sweep.is_finite() || sweep.abs() >= FULL_TURN {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.bulge_segment: LWPOLYLINE segment {index} has bulge {bulge}, which \
                 does not describe a finite arc of less than one full turn"
            )));
        }
        arcs.push(Some(SourceArc {
            center,
            radius,
            start: start_angle,
            sweep,
        }));
    }
    Ok(arcs)
}

/// The pick's signed offset value, from the nearest segment (the shared rule).
///
/// The nearest segment is the one with the smallest Euclidean distance to the
/// finite segment (for an arc: the radial difference when the pick's angle is on
/// the span, otherwise the nearer end point), and the returned sign is always the
/// **displacement along `m`** at that segment, so one pick yields one side for the
/// whole polyline.
fn pick_signed_distance(
    points: &[Point3],
    dirs: &[Point3],
    arcs: &[Option<SourceArc>],
    normal: Point3,
    side_point: Point3,
) -> f64 {
    let count = points.len();
    let mut best: Option<(f64, f64)> = None;
    for (index, u) in dirs.iter().enumerate() {
        let start = points[index];
        let end = points[(index + 1) % count];
        let (distance, signed) = match arcs.get(index).copied().flatten() {
            None => line_pick(start, end, *u, normal, side_point),
            Some(arc) => arc_pick(arc, normal, side_point),
        };
        if best.map(|(current, _)| distance < current).unwrap_or(true) {
            best = Some((distance, signed));
        }
    }
    best.map(|(_, signed)| signed).unwrap_or(0.0)
}

/// The pick's distance to one arc and its signed offset value (see the module
/// docs for the `−sign(σ)` derivation).
fn arc_pick(arc: SourceArc, normal: Point3, side_point: Point3) -> (f64, f64) {
    let (ax, ay, _) = arbitrary_axis(normal);
    let offset = sub(side_point, arc.center);
    let (radial_x, radial_y) = (dot(offset, ax), dot(offset, ay));
    let reach = radial_x.hypot(radial_y);
    // The pick's distance to the arc's own circle, signed along `m` at that point.
    let signed = -(reach - arc.radius) * arc.sweep.signum();
    // The tolerance is one world tolerance expressed as an angle on this arc; the
    // axis case (no radial direction at all) is the same predicate at its limit.
    let angle_tolerance = (offset_tolerance() / arc.radius).max(1e-12);
    if !reach.is_finite() || reach <= offset_tolerance() {
        // On the axis there is no radial direction: every point of the arc is the
        // same distance away, so the arc's radius is its distance.
        return (arc.radius, signed);
    }
    if angle_on_arc(
        radial_y.atan2(radial_x),
        arc.start,
        arc.sweep,
        angle_tolerance,
    ) {
        (signed.abs(), signed)
    } else {
        // Off the span: the nearer end point is the nearest arc point (the same
        // rule the LINE arm applies to a pick beyond a segment end).
        let first = arc.point_at(arc.start, ax, ay);
        let last = arc.point_at(arc.start + arc.sweep, ax, ay);
        (
            distance(side_point, first).min(distance(side_point, last)),
            signed,
        )
    }
}

/// One offset segment: a shifted line or a concentric arc.
fn offset_segment(
    index: usize,
    points: &[Point3],
    dirs: &[Point3],
    arcs: &[Option<SourceArc>],
    normal: Point3,
    signed: f64,
    tol: f64,
) -> CadResult<OffsetSegment> {
    match arcs.get(index).copied().flatten() {
        None => {
            // A line moves by the constant signed vector `s·m`: still parallel,
            // still the same distance away.
            let shift = scale(cross(normal, dirs[index]), signed);
            Ok(OffsetSegment::Line {
                start: add(points[index], shift),
                end: add(points[(index + 1) % points.len()], shift),
            })
        }
        Some(arc) => {
            // `R' = R − s·sign(σ)`: the derivation is in the module docs. The
            // centre does not move and the angles are re-derived after the trim.
            let radius = arc.radius - signed * arc.sweep.signum();
            if !radius.is_finite() || radius <= tol {
                return Err(CadError::Unsupported(format!(
                    "OFFSET offset.arc_collapsed: LWPOLYLINE segment {index} (arc radius {}) \
                     would be offset to radius {radius}; the segment is neither deleted nor \
                     blended into its neighbours",
                    arc.radius
                )));
            }
            Ok(OffsetSegment::Arc(SourceArc { radius, ..arc }))
        }
    }
}

/// The open end of one offset segment (no cap, no mitre).
fn open_vertex(segment: &OffsetSegment, ax: Point3, ay: Point3, at_start: bool) -> Point3 {
    match segment {
        OffsetSegment::Line { start, end } => {
            if at_start {
                *start
            } else {
                *end
            }
        }
        OffsetSegment::Arc(arc) => {
            let angle = if at_start {
                arc.start
            } else {
                arc.start + arc.sweep
            };
            arc.point_at(angle, ax, ay)
        }
    }
}

/// The join at source vertex `index` between the two segments either side of it.
fn join_vertex(
    index: usize,
    points: &[Point3],
    dirs: &[Point3],
    offset: &[OffsetSegment],
    normal: Point3,
    signed: f64,
    tol: f64,
) -> CadResult<Point3> {
    let segments = offset.len();
    let previous = (index + segments - 1) % segments;
    let vertex = points[index];
    match (&offset[previous], &offset[index]) {
        // Two offset lines: the analytic miter (exactly the straight arm's point).
        (OffsetSegment::Line { .. }, OffsetSegment::Line { .. }) => {
            miter_vertex(vertex, dirs[previous], dirs[index], normal, signed)
        }
        _ => {
            // A tangency is resolved in closed form, not by the engine: the
            // engine's analytic solve returns the double root of a quadratic whose
            // discriminant is ~0 at a tangency, where a relative error of `eps`
            // becomes `sqrt(eps)` — so the root drifts (measured ~3e-10) and a
            // slightly negative discriminant collapses to "no intersection", i.e. a
            // spurious `join_failed`. A tangent join is the single most common
            // bulged-polyline join there is (every join of a FILLET-rounded outline
            // is tangent), and the touching point itself is exact.
            let candidates = match tangency_point(offset[previous], offset[index], tol) {
                Some(point) => vec![point],
                None => join_candidates(offset[previous], offset[index], normal, tol)?,
            };
            nearest_candidate(candidates, vertex, tol, index)
        }
    }
}

/// The pair's touching point when the two offset carriers are **tangent**.
///
/// `None` when they are not tangent within `tol` (the engine then resolves the
/// transverse case), and also when they are concentric (two distinct circles: no
/// touch; one circle: no unique join). The test is the same distance/radius
/// comparison `cad-geometry`'s own circle solve uses to decide whether a second
/// root exists, so the two never disagree about what "tangent" means.
fn tangency_point(first: OffsetSegment, second: OffsetSegment, tol: f64) -> Option<Point3> {
    match (first, second) {
        (OffsetSegment::Line { start, end }, OffsetSegment::Arc(arc))
        | (OffsetSegment::Arc(arc), OffsetSegment::Line { start, end }) => {
            // The perpendicular foot from the centre onto the offset line.
            let direction = sub(end, start);
            let len = length(direction);
            if !len.is_finite() || len <= 0.0 {
                return None;
            }
            let unit = scale(direction, 1.0 / len);
            let foot = add(start, scale(unit, dot(sub(arc.center, start), unit)));
            if (distance(foot, arc.center) - arc.radius).abs() <= tol {
                Some(foot)
            } else {
                None
            }
        }
        (OffsetSegment::Arc(first_arc), OffsetSegment::Arc(second_arc)) => {
            let between = sub(second_arc.center, first_arc.center);
            let separation = length(between);
            // Concentric (within the tolerance the engine's own circle solve uses
            // for the same test): either two distinct circles that never touch, or
            // one and the same circle, which has no unique join either.
            if !separation.is_finite() || separation <= tol {
                return None;
            }
            let unit = scale(between, 1.0 / separation);
            if (first_arc.radius + second_arc.radius - separation).abs() <= tol {
                // External tangency: the touch point is on the joining ray.
                return Some(add(first_arc.center, scale(unit, first_arc.radius)));
            }
            if ((first_arc.radius - second_arc.radius).abs() - separation).abs() <= tol {
                // Internal tangency: the touch point is on the ray from the
                // enclosing centre through the enclosed one, or the other way round
                // when the first circle is the smaller.
                let side = if first_arc.radius >= second_arc.radius {
                    1.0
                } else {
                    -1.0
                };
                return Some(add(first_arc.center, scale(unit, side * first_arc.radius)));
            }
            None
        }
        (OffsetSegment::Line { .. }, OffsetSegment::Line { .. }) => None,
    }
}

/// Every intersection of the two offset segments' own (unbounded) carriers.
fn join_candidates(
    first: OffsetSegment,
    second: OffsetSegment,
    normal: Point3,
    tol: f64,
) -> CadResult<Vec<Point3>> {
    let (probe_first, probe_second) = match (first, second) {
        (OffsetSegment::Line { start, end }, OffsetSegment::Arc(arc)) => {
            (line_probe(start, end, arc, tol), circle_probe(arc, normal))
        }
        (OffsetSegment::Arc(arc), OffsetSegment::Line { start, end }) => {
            (circle_probe(arc, normal), line_probe(start, end, arc, tol))
        }
        (OffsetSegment::Arc(first_arc), OffsetSegment::Arc(second_arc)) => (
            circle_probe(first_arc, normal),
            circle_probe(second_arc, normal),
        ),
        (OffsetSegment::Line { .. }, OffsetSegment::Line { .. }) => {
            return Err(CadError::Invariant(
                "OFFSET join: a line/line join is the analytic miter, never an intersection query"
                    .into(),
            ));
        }
    };
    DefaultGeometryEngine
        .intersect_local(&probe_first, &probe_second, &TolerancePolicy::default())
        .map_err(|error| {
            CadError::Unsupported(format!(
                "OFFSET offset.join_failed: the two offset segments could not be intersected \
                 ({error})"
            ))
        })
}

/// A full circle probe: never an `Arc`, which would filter the hits by the
/// source span and hide a miter point outside it.
fn circle_probe(arc: SourceArc, normal: Point3) -> SemanticGeometry {
    SemanticGeometry::Circle {
        center: arc.center,
        normal,
        radius: arc.radius,
    }
}

/// The offset line extended well past both ends, so a corner outside the source
/// segment's own span is still reachable (FILLET's own extension rule).
///
/// Any intersection with the circle satisfies `|p − mid| <= R + |c − mid|`, so a
/// probe of exactly that reach contains every candidate; it is measured from the
/// segment's midpoint so both directions are covered.
fn line_probe(start: Point3, end: Point3, arc: SourceArc, tol: f64) -> SemanticGeometry {
    let direction = sub(end, start);
    let len = length(direction);
    let unit = if len.is_finite() && len > 0.0 {
        scale(direction, 1.0 / len)
    } else {
        // Unreachable for a validated source segment; a zero reach is still a
        // legal (degenerate) probe rather than a panic.
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    };
    let middle = scale(add(start, end), 0.5);
    let reach = arc.radius + distance(middle, arc.center) + tol;
    SemanticGeometry::Line {
        start: sub(middle, scale(unit, reach)),
        end: add(middle, scale(unit, reach)),
    }
}

/// The candidate nearest `vertex`; two equally near candidates are ambiguous.
fn nearest_candidate(
    candidates: Vec<Point3>,
    vertex: Point3,
    tol: f64,
    index: usize,
) -> CadResult<Point3> {
    let mut scored: Vec<(f64, Point3)> = candidates
        .into_iter()
        .filter(|point| is_finite_point(*point))
        .map(|point| (distance(point, vertex), point))
        .collect();
    if scored.is_empty() {
        return Err(CadError::Unsupported(format!(
            "OFFSET offset.join_failed: the two offset segments either side of vertex {index} \
             ({:.6}, {:.6}) do not meet",
            vertex.x, vertex.y
        )));
    }
    scored.sort_by(|left, right| left.0.total_cmp(&right.0));
    if scored.len() > 1 && scored[1].0 - scored[0].0 <= tol {
        return Err(CadError::Unsupported(format!(
            "OFFSET offset.join_failed: the two offset segments either side of vertex {index} \
             ({:.6}, {:.6}) meet at two equally near points ({:.9} and {:.9} away), so there is no \
             join to pick",
            vertex.x, vertex.y, scored[0].0, scored[1].0
        )));
    }
    Ok(scored[0].1)
}

/// Re-derive every arc segment's bulge from its trimmed join points.
///
/// The sweep comes from the **centre angles only** (never from a cross product),
/// measured along the traversal direction, so the sign is preserved by
/// construction; the emitted bulge is then verified through
/// [`cad_geometry::bulge_arc_geometry`]. Straight segments keep their stored bulge
/// byte for byte.
fn rederive_bulges(
    bulges: &[f64],
    offset: &[OffsetSegment],
    out: &[Point3],
    normal: Point3,
    tol: f64,
) -> CadResult<Vec<f64>> {
    let (ax, ay, _) = arbitrary_axis(normal);
    let mut out_bulges = bulges.to_vec();
    for (index, segment) in offset.iter().enumerate() {
        let OffsetSegment::Arc(arc) = segment else {
            continue;
        };
        let start = out[index];
        let end = out[(index + 1) % out.len()];
        let angle_tolerance = (tol / arc.radius).max(1e-12);
        let start_angle = arc.angle_of(start, ax, ay);
        let end_angle = arc.angle_of(end, ax, ay);
        // The forward span in the traversal direction: always the same sign as the
        // source sweep, so a reversal cannot slip through.
        let sweep = if arc.sweep >= 0.0 {
            norm_tau(end_angle - start_angle)
        } else {
            -norm_tau(start_angle - end_angle)
        };
        if !sweep.is_finite() || sweep.abs() <= angle_tolerance {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.collapsed: offset arc segment {index} collapses to zero sweep \
                 between its two joins"
            )));
        }
        if sweep.abs() > arc.sweep.abs() + MAX_SWEEP_GROWTH {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.arc_mismatch: offset arc segment {index} would need a {:.6} rad \
                 sweep where its source has {:.6} rad, which means the join wrapped to the far \
                 side of the circle",
                sweep, arc.sweep
            )));
        }
        let bulge = arc_sweep_to_bulge(sweep).ok_or_else(|| {
            CadError::Unsupported(format!(
                "OFFSET offset.arc_mismatch: offset arc segment {index} produced a \
                 non-representable sweep {sweep}"
            ))
        })?;
        require_recovered_arc(index, start, end, bulge, arc, sweep, normal)?;
        if index < out_bulges.len() {
            out_bulges[index] = bulge;
        }
    }
    Ok(out_bulges)
}

/// Verify an emitted bulge against the single `bulge → arc` authority.
fn require_recovered_arc(
    index: usize,
    start: Point3,
    end: Point3,
    bulge: f64,
    arc: &SourceArc,
    sweep: f64,
    normal: Point3,
) -> CadResult<()> {
    let mismatch = || {
        CadError::Unsupported(format!(
            "OFFSET offset.arc_mismatch: the offset arc of segment {index} does not survive the \
             bulge round-trip (bulge {bulge})"
        ))
    };
    let Some(SemanticGeometry::Arc {
        center,
        radius,
        sweep: recovered_sweep,
        ..
    }) = bulge_arc_geometry(start, end, bulge, normal)
    else {
        return Err(mismatch());
    };
    let tolerance = 1e-9 * (1.0 + arc.radius);
    if distance(center, arc.center) > tolerance
        || (radius - arc.radius).abs() > tolerance
        || (recovered_sweep - sweep).abs() > tolerance
    {
        return Err(mismatch());
    }
    Ok(())
}

/// Refuse an offset **line** segment that folded back or shrank away.
///
/// Arc segments are covered by the sweep checks above; a line must keep its source
/// direction (`dot > 0`) and a real length, exactly like the straight arm.
fn require_no_line_collapse(
    out: &[Point3],
    offset: &[OffsetSegment],
    dirs: &[Point3],
    tol: f64,
) -> CadResult<()> {
    for (index, segment) in offset.iter().enumerate() {
        if matches!(segment, OffsetSegment::Arc(_)) {
            continue;
        }
        let next = (index + 1) % out.len();
        let piece = sub(out[next], out[index]);
        let len = length(piece);
        let along = dot(piece, dirs[index]);
        if !len.is_finite() || len <= tol || !along.is_finite() || along <= tol {
            return Err(CadError::Unsupported(format!(
                "OFFSET offset.collapsed: offset segment {index} collapses to zero length or \
                 folds back (it would self-intersect at its vertex)"
            )));
        }
    }
    Ok(())
}

/// An angle normalised into `[0, TAU)`.
fn norm_tau(angle: f64) -> f64 {
    let mut value = angle % FULL_TURN;
    if value < 0.0 {
        value += FULL_TURN;
    }
    value
}

/// Whether `angle` lies on the (possibly negative-sweep) arc.
///
/// `cad-geometry` keeps its own copy `pub(crate)`; this planner intersects
/// `Circle`s only (never `Arc`s), so it needs the predicate locally.
fn angle_on_arc(angle: f64, start: f64, sweep: f64, angle_tolerance: f64) -> bool {
    if sweep.abs() >= FULL_TURN - 1e-9 {
        return true;
    }
    let normalized_start = norm_tau(start);
    let normalized_angle = norm_tau(angle);
    if sweep >= 0.0 {
        norm_tau(normalized_angle - normalized_start) <= sweep + angle_tolerance
    } else {
        norm_tau(normalized_start - normalized_angle) <= -sweep + angle_tolerance
    }
}
