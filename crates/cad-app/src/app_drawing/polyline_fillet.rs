//! The AutoCAD **Polyline mode** of FILLET and CHAMFER (`FILLET POLYLINE` /
//! `CHAMFER POLYLINE`) on one selected LWPOLYLINE.
//!
//! ## Why a selector-based command
//!
//! AutoCAD reaches this mode through the `Polyline` sub-option of the FILLET /
//! CHAMFER object prompt. The fixed-step capture machine cannot express "pick the
//! first object **or** the Polyline option", so the mode is a separate
//! selection-based command: the selector is the whole input
//! (`CommandPayload::None`), exactly like `JOIN`/`MATCHPROP`.
//!
//! ## Contract (AutoCAD 2026)
//!
//! * **Every junction where two straight segments meet** is filleted/chamfered.
//! * A **single intervening arc segment** between two straight segments is
//!   removed and replaced by the corner of the two straight segments' lines (the
//!   arc's two vertices disappear). Two or more consecutive arc segments are a
//!   named `Unsupported` (`polyline_arc_chain`).
//! * **Too-short segments are left unmodified**: a corner whose inset does not
//!   fit its segment (or would overlap a neighbour) is skipped, never refused and
//!   never clamped. The greedy order is the polyline's own vertex order.
//! * **Trim on** (the default) rewrites the polyline itself: the chamfer becomes a
//!   polyline segment, the fillet a polyline arc segment (one `bulge`). **Trim
//!   off** leaves the polyline byte-identical and inserts the joining LINE/ARC
//!   entities instead (separate model-space entities in the same transaction).
//! * **FILLET `R = 0`** and **CHAMFER `A = B = 0`** are the intersect-only form:
//!   the straight segments are trimmed/extended to their corner and no joining
//!   segment is created.
//! * **Open polylines are closed**: the start and end segments get the same
//!   corner treatment (their intersection) and the result is `closed = true`. If
//!   that corner does not fit, the polyline stays open and the corner is counted
//!   as skipped (no silent truncation).
//! * **Closed polylines** are supported (every wrap-around junction is treated
//!   exactly like an interior one).
//! * `Multiple` and the in-command `Undo` sub-option need the interactive option
//!   loop this build does not have; one accepted invocation is one transaction /
//!   one undo step and the global `UNDO` reverses it.
//!
//! The construction itself is the two-line one: at a corner with `u_in` the unit
//! direction `corner → previous` and `u_out` the unit direction
//! `corner → next`, `φ = acos(u_in · u_out)` is the interior angle. FILLET uses
//! `d = R / tan(φ/2)` and a tangent arc of radius `R`; CHAMFER uses `A` along the
//! incoming arm and `B` along the outgoing arm (our declared orientation rule,
//! since AutoCAD's polyline prompt order is not reproduced here); with the stored
//! ANGLE method the outgoing distance is derived per vertex as
//! `B = A·sin α / sin(φ + α)`, exactly like the two-line angle method. The arc
//! sweep is derived from the centre angles only and converted with the single
//! `sweep → bulge` authority (`arc_sweep_to_bulge`); a non-representable sweep is
//! a named refusal.
//!
//! ## Refusals (all named, all before any write)
//!
//! | code | meaning |
//! | --- | --- |
//! | `<c>.polyline_invalid_payload` | a payload was supplied (the selection is the input) |
//! | `<c>.polyline_selection` | the selection is not exactly one entity |
//! | `<c>.polyline_not_addressable` | another document, block-instance member or sub-element |
//! | `<c>.polyline_target_missing` | the referenced entity does not exist |
//! | `<c>.polyline_not_polyline` | the pick is not an LWPOLYLINE |
//! | `<c>.polyline_too_few_vertices` | fewer than three vertices |
//! | `<c>.polyline_degenerate` | a non-finite or zero-length vertex pair |
//! | `<c>.polyline_not_planar` | the vertices are not `+Z` coplanar |
//! | `<c>.polyline_arc_chain` | two consecutive arc segments (only one intervening arc is supported) |
//! | `<c>.polyline_radius_negative` | FILLET radius `< 0` |
//! | `<c>.polyline_distance_negative` | CHAMFER distance `< 0` |
//! | `<c>.polyline_angle_out_of_range` | the stored CHAMFER angle is outside `0..180` degrees |
//! | `<c>.polyline_arc_sweep` | a corner produced a non-representable arc sweep |
//! | `<c>.polyline_parameter_missing` | the session typed channel holds no finite number |
//! | `<c>.polyline_nothing_to_modify` | trim on: no corner could be applied |
//! | `<c>.polyline_no_join` | trim off: no joining entity would be created |
//!
//! `<c>` is `fillet` or `chamfer`. Capability gaps (not a polyline, not planar,
//! arc chain) are [`CadError::Unsupported`]; a supported shape whose numbers do
//! not work is [`CadError::InvalidInput`]. Viewer mode is refused by the command
//! layer ([`CommandId::requires_work_mode`]).
//!
//! ## Declared deviations
//!
//! * AutoCAD's radius auto-reduction is replaced by "skip the corner" (the
//!   documented 2026 behaviour for too-short segments).
//! * CHAMFER orientation is our rule (`A` incoming, `B` outgoing), asserted by a
//!   test.
//! * The trim-off joining entities follow the **active layer** (the same declared
//!   rule the two-line commands use).

use super::helpers::{corner_tolerance, distance, normalize_signed};
use super::*;
use cad_geometry::{arc_sweep_to_bulge, is_xy_parallel};

/// The corner operation a polyline-mode FILLET/CHAMFER applies at every eligible
/// junction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PolylineCornerSpec {
    /// FILLET with radius `>= 0` (`0` = sharp corner).
    Fillet { radius: f64, trim: bool },
    /// CHAMFER with `a` along the incoming arm and `b` along the outgoing arm
    /// (our declared orientation rule); `0/0` = intersect-only. When `angle` is
    /// `Some`, the per-vertex outgoing distance is derived from the local corner
    /// angle like the two-line angle method (`b = a·sin α / sin(φ + α)`) and the
    /// stored `b` is unused.
    Chamfer {
        a: f64,
        b: f64,
        angle: Option<f64>,
        trim: bool,
    },
}

impl PolylineCornerSpec {
    fn stem(self) -> &'static str {
        match self {
            Self::Fillet { .. } => "fillet",
            Self::Chamfer { .. } => "chamfer",
        }
    }

    fn command_name(self) -> &'static str {
        match self {
            Self::Fillet { .. } => "FILLET POLYLINE",
            Self::Chamfer { .. } => "CHAMFER POLYLINE",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Fillet { .. } => "fillet polyline",
            Self::Chamfer { .. } => "chamfer polyline",
        }
    }

    fn diagnostic_code(self) -> &'static str {
        match self {
            Self::Fillet { .. } => "drawing.fillet_polyline",
            Self::Chamfer { .. } => "drawing.chamfer_polyline",
        }
    }

    fn trim(self) -> bool {
        match self {
            Self::Fillet { trim, .. } | Self::Chamfer { trim, .. } => trim,
        }
    }
}

/// The result of one polyline corner plan.
pub(crate) struct PolylinePlan {
    /// The rewritten vertices (trim on).
    pub points: Vec<Point3>,
    /// One bulge per vertex; the trailing entry of an open polyline is unused.
    pub bulges: Vec<f64>,
    pub closed: bool,
    /// Joining entities to insert (trim off only).
    pub inserted: Vec<SemanticGeometry>,
    pub cornered: usize,
    pub skipped: usize,
}

/// A `<stem>.polyline_<code>: <detail>` refusal string (the planner's currency).
fn refusal(stem: &str, code: &str, detail: impl std::fmt::Display) -> String {
    format!("{stem}.polyline_{code}: {detail}")
}

/// One polyline segment.
#[derive(Debug, Clone, Copy)]
struct Seg {
    start: Point3,
    end: Point3,
    length: f64,
    bulge: f64,
    straight: bool,
}

/// One eligible junction, resolved to the two tangent points and the join.
struct Candidate {
    left: usize,
    right: usize,
    /// The arc segment removed by this corner, if any.
    inter: Option<usize>,
    t_left: Point3,
    t_right: Point3,
    /// Bulge of the joining segment (`0.0` for a chamfer/intersect-only).
    bulge: f64,
    /// The joining entity a trim-off rewrite inserts (`None` when degenerate).
    join: Option<SemanticGeometry>,
    /// How much of the left segment's `end` endpoint the corner consumes.
    consume_left: f64,
    /// How much of the right segment's `start` endpoint the corner consumes.
    consume_right: f64,
    /// This candidate closes an open polyline (start/end junction).
    closing: bool,
}

/// Plan the polyline rewrite: apply the corner operation at every eligible
/// junction of `points`/`bulges`.
///
/// Returns the rewritten polyline for trim on, or the joining entities for trim
/// off. Every refusal is a `String` naming its code and the plan is
/// **all-or-nothing**: a refusal means the caller writes nothing. The plan is
/// pure — no database, no session, no history.
pub(crate) fn plan_polyline_corners(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    spec: PolylineCornerSpec,
    tol: f64,
) -> Result<PolylinePlan, String> {
    let stem = spec.stem();
    validate_spec(spec, stem)?;
    let count = points.len();
    if count < 3 {
        return Err(refusal(
            stem,
            "too_few_vertices",
            format!("a polyline needs at least three vertices, got {count}"),
        ));
    }
    if !points.iter().all(|p| is_finite_point(*p)) {
        return Err(refusal(
            stem,
            "degenerate",
            "the polyline has a non-finite vertex",
        ));
    }
    if !is_xy_parallel(points) {
        return Err(refusal(
            stem,
            "not_planar",
            "the polyline vertices are not coplanar in a plane parallel to world XY",
        ));
    }
    let seg_count = if closed { count } else { count - 1 };
    let mut segs: Vec<Seg> = Vec::with_capacity(seg_count);
    for index in 0..seg_count {
        let start = points[index];
        let end = points[(index + 1) % count];
        let length = distance(start, end);
        if !length.is_finite() || length <= tol {
            return Err(refusal(
                stem,
                "degenerate",
                format!(
                    "vertices {index} and {} coincide (a zero-length segment has no direction)",
                    (index + 1) % count
                ),
            ));
        }
        let bulge = bulges.get(index).copied().unwrap_or(0.0);
        if !bulge.is_finite() {
            return Err(refusal(
                stem,
                "degenerate",
                format!("segment {index} has a non-finite bulge"),
            ));
        }
        segs.push(Seg {
            start,
            end,
            length,
            bulge,
            straight: bulge.abs() <= tol,
        });
    }
    for index in 0..seg_count {
        let next = if closed {
            (index + 1) % seg_count
        } else {
            index + 1
        };
        if next < seg_count && !segs[index].straight && !segs[next].straight {
            return Err(refusal(
                stem,
                "arc_chain",
                format!("segments {index} and {next} are both arcs; only a single intervening arc is supported"),
            ));
        }
    }

    // Every eligible junction, in the polyline's own order.
    let mut candidates = Vec::new();
    let in_range = |index: usize| closed || index < seg_count;
    for left in 0..seg_count {
        if !segs[left].straight {
            continue;
        }
        let adjacent = left + 1;
        if in_range(adjacent) {
            let right = adjacent % seg_count;
            if segs[right].straight {
                if let Some(candidate) =
                    build_candidate(&segs, left, right, None, false, spec, tol)?
                {
                    candidates.push(candidate);
                }
            }
        }
        let middle = left + 1;
        let far = left + 2;
        if in_range(middle) && in_range(far) {
            let mid = middle % seg_count;
            let right = far % seg_count;
            if !segs[mid].straight && segs[right].straight {
                if let Some(candidate) =
                    build_candidate(&segs, left, right, Some(mid), false, spec, tol)?
                {
                    candidates.push(candidate);
                }
            }
        }
    }
    if !closed {
        // Start/end junction: the start and end segments are joined by the
        // corner of their infinite lines and the result closes the polyline.
        let left = seg_count - 1;
        if segs[left].straight && segs[0].straight {
            if let Some(candidate) = build_candidate(&segs, left, 0, None, true, spec, tol)? {
                candidates.push(candidate);
            }
        }
    }

    if !spec.trim() {
        let inserted = candidates
            .iter()
            .filter_map(|candidate| candidate.join.clone())
            .collect();
        return Ok(PolylinePlan {
            points: points.to_vec(),
            bulges: bulges.to_vec(),
            closed,
            inserted,
            cornered: candidates.iter().filter(|c| c.join.is_some()).count(),
            skipped: candidates.iter().filter(|c| c.join.is_none()).count(),
        });
    }

    // Trim on: greedy per-segment budget (too-short segments stay unmodified).
    let mut start_pt: Vec<Point3> = segs.iter().map(|seg| seg.start).collect();
    let mut end_pt: Vec<Point3> = segs.iter().map(|seg| seg.end).collect();
    let mut removed = vec![false; seg_count];
    let mut join_bulge = vec![0.0; seg_count];
    let mut start_used = vec![0.0; seg_count];
    let mut end_used = vec![0.0; seg_count];
    let mut cornered = 0usize;
    let mut skipped = 0usize;
    let mut closing_accepted = false;
    for candidate in &candidates {
        let fits = start_used[candidate.left] + end_used[candidate.left] + candidate.consume_left
            <= segs[candidate.left].length - tol
            && start_used[candidate.right] + end_used[candidate.right] + candidate.consume_right
                <= segs[candidate.right].length - tol;
        if !fits {
            skipped += 1;
            continue;
        }
        end_pt[candidate.left] = candidate.t_left;
        start_pt[candidate.right] = candidate.t_right;
        end_used[candidate.left] += candidate.consume_left.max(0.0);
        start_used[candidate.right] += candidate.consume_right.max(0.0);
        if let Some(middle) = candidate.inter {
            removed[middle] = true;
        }
        join_bulge[candidate.left] = candidate.bulge;
        cornered += 1;
        if candidate.closing {
            closing_accepted = true;
        }
    }
    let (points_out, bulges_out) = assemble(&segs, &start_pt, &end_pt, &removed, &join_bulge, tol);
    Ok(PolylinePlan {
        points: points_out,
        bulges: bulges_out,
        closed: closed || closing_accepted,
        inserted: Vec::new(),
        cornered,
        skipped,
    })
}

/// Walk the kept segments and assemble the rewritten vertices/bulges.
fn assemble(
    segs: &[Seg],
    start_pt: &[Point3],
    end_pt: &[Point3],
    removed: &[bool],
    join_bulge: &[f64],
    tol: f64,
) -> (Vec<Point3>, Vec<f64>) {
    let same = |a: Point3, b: Point3| distance(a, b) <= tol;
    let mut points: Vec<Point3> = Vec::new();
    let mut bulges: Vec<f64> = Vec::new();
    let mut previous: Option<usize> = None;
    for (index, seg) in segs.iter().enumerate() {
        if removed[index] {
            continue;
        }
        let start = start_pt[index];
        match previous {
            None => {
                points.push(start);
                bulges.push(seg.bulge);
            }
            Some(prev) => {
                if same(*points.last().unwrap_or(&start), start) {
                    // Shared vertex (a skipped corner): the previous point's
                    // outgoing segment is this one.
                    if let Some(last) = bulges.last_mut() {
                        *last = seg.bulge;
                    }
                } else {
                    // The accepted corner opens a gap: the previous point's
                    // outgoing segment is the joining segment.
                    if let Some(last) = bulges.last_mut() {
                        *last = join_bulge[prev];
                    }
                    points.push(start);
                    bulges.push(seg.bulge);
                }
            }
        }
        let end = end_pt[index];
        if !same(start, end) {
            points.push(end);
            bulges.push(0.0);
        }
        previous = Some(index);
    }
    if points.len() > 1 && same(*points.last().unwrap(), points[0]) {
        // A closed polyline's walk returns to its first vertex: drop the
        // duplicate (its outgoing bulge is the last segment's, already set).
        points.pop();
        bulges.pop();
    } else if let Some(prev) = previous {
        // The final point is a distinct vertex: its outgoing segment is the
        // accepted start/end corner's join (or the unused trailing bulge of an
        // open polyline, which is the zero default).
        if let Some(last) = bulges.last_mut() {
            *last = join_bulge[prev];
        }
    }
    (points, bulges)
}

/// Resolve one junction into its tangent points and joining geometry.
fn build_candidate(
    segs: &[Seg],
    left: usize,
    right: usize,
    inter: Option<usize>,
    closing: bool,
    spec: PolylineCornerSpec,
    tol: f64,
) -> Result<Option<Candidate>, String> {
    let stem = spec.stem();
    let left_seg = segs[left];
    let right_seg = segs[right];
    let corner = match cad_geometry::infinite_line_intersection(
        left_seg.start,
        left_seg.end,
        right_seg.start,
        right_seg.end,
        tol,
    ) {
        cad_geometry::LineIntersection::Point(corner) => corner,
        // Parallel/collinear segments have no corner: nothing to modify.
        cad_geometry::LineIntersection::Parallel | cad_geometry::LineIntersection::Collinear => {
            return Ok(None);
        }
    };
    let unit = |from: Point3, to: Point3| -> Option<(f64, f64)> {
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let length = dx.hypot(dy);
        if !length.is_finite() || length <= tol {
            None
        } else {
            Some((dx / length, dy / length))
        }
    };
    let dir_left = unit(corner, left_seg.start);
    let dir_right = unit(corner, right_seg.end);
    let (Some(dir_left), Some(dir_right)) = (dir_left, dir_right) else {
        return Ok(None);
    };
    let dot = (dir_left.0 * dir_right.0 + dir_left.1 * dir_right.1).clamp(-1.0, 1.0);
    let phi = dot.acos();
    let (t_left, t_right, bulge, join) = match spec {
        PolylineCornerSpec::Fillet { radius, .. } => {
            if radius == 0.0 {
                (corner, corner, 0.0, None)
            } else {
                let half = phi * 0.5;
                let (sin_half, cos_half) = half.sin_cos();
                if !sin_half.is_finite() || sin_half <= tol || (std::f64::consts::PI - phi) <= tol {
                    // Collinear arms (a straight-through vertex or a spike) have
                    // no corner: leave the vertex unmodified.
                    return Ok(None);
                }
                let tangent = radius * cos_half / sin_half;
                if !tangent.is_finite() || tangent <= tol {
                    return Ok(None);
                }
                let t1 = Point3 {
                    x: corner.x + dir_left.0 * tangent,
                    y: corner.y + dir_left.1 * tangent,
                    z: corner.z,
                };
                let t2 = Point3 {
                    x: corner.x + dir_right.0 * tangent,
                    y: corner.y + dir_right.1 * tangent,
                    z: corner.z,
                };
                let bisector = (dir_left.0 + dir_right.0, dir_left.1 + dir_right.1);
                let length = bisector.0.hypot(bisector.1);
                if !length.is_finite() || length <= tol {
                    return Ok(None);
                }
                let centre_distance = radius / sin_half;
                let centre = Point3 {
                    x: corner.x + bisector.0 / length * centre_distance,
                    y: corner.y + bisector.1 / length * centre_distance,
                    z: corner.z,
                };
                let start = (t1.y - centre.y).atan2(t1.x - centre.x);
                let end = (t2.y - centre.y).atan2(t2.x - centre.x);
                let sweep = normalize_signed(end - start);
                let Some(bulge) = arc_sweep_to_bulge(sweep) else {
                    return Err(refusal(
                        stem,
                        "arc_sweep",
                        format!("a corner produced the non-representable sweep {sweep}"),
                    ));
                };
                let join = SemanticGeometry::Arc {
                    center: centre,
                    normal: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 1.0,
                    },
                    radius,
                    start,
                    sweep,
                };
                (t1, t2, bulge, Some(join))
            }
        }
        PolylineCornerSpec::Chamfer { a, b, angle, .. } => {
            let b = match angle {
                Some(alpha) if alpha > 0.0 => {
                    let denominator = (phi + alpha).sin();
                    if !denominator.is_finite() || denominator <= tol {
                        return Ok(None);
                    }
                    a * alpha.sin() / denominator
                }
                _ => b,
            };
            if a <= tol && b <= tol {
                (corner, corner, 0.0, None)
            } else {
                let t1 = Point3 {
                    x: corner.x + dir_left.0 * a,
                    y: corner.y + dir_left.1 * a,
                    z: corner.z,
                };
                let t2 = Point3 {
                    x: corner.x + dir_right.0 * b,
                    y: corner.y + dir_right.1 * b,
                    z: corner.z,
                };
                if distance(t1, t2) <= tol {
                    return Ok(None);
                }
                (
                    t1,
                    t2,
                    0.0,
                    Some(SemanticGeometry::Line { start: t1, end: t2 }),
                )
            }
        }
    };
    // How far the tangent points sit inside the two segments, measured from the
    // junction endpoints. The left segment joins at its `end`, the right at its
    // `start`; a negative inset is an extension (allowed only for intersect-only).
    let offset = |anchor: Point3, point: Point3, segment: Seg| {
        let (dx, dy) = (
            segment.end.x - segment.start.x,
            segment.end.y - segment.start.y,
        );
        let length = dx.hypot(dy);
        if !length.is_finite() || length <= 0.0 {
            return f64::NAN;
        }
        ((point.x - anchor.x) * dx + (point.y - anchor.y) * dy) / length
    };
    let left_position = offset(left_seg.start, t_left, left_seg);
    let right_position = offset(right_seg.start, t_right, right_seg);
    let consume_left = left_seg.length - left_position;
    let consume_right = right_position;
    let intersect_only = matches!(spec, PolylineCornerSpec::Fillet { radius, .. } if radius == 0.0)
        || matches!(spec, PolylineCornerSpec::Chamfer { a, b, angle, .. }
            if a == 0.0 && (angle.is_none() || angle == Some(0.0)) && b == 0.0);
    let fits_left =
        left_position <= left_seg.length + tol && (intersect_only || left_position >= -tol);
    let fits_right =
        right_position <= right_seg.length + tol && (intersect_only || right_position >= -tol);
    if !fits_left || !fits_right {
        // A t point off the finite segment: leave this junction unmodified.
        return Ok(None);
    }
    Ok(Some(Candidate {
        left,
        right,
        inter,
        t_left,
        t_right,
        bulge,
        join,
        consume_left: consume_left.max(0.0),
        consume_right: consume_right.max(0.0),
        closing,
    }))
}

/// The spec's own number validation (the meaning is per command).
fn validate_spec(spec: PolylineCornerSpec, stem: &str) -> Result<(), String> {
    match spec {
        PolylineCornerSpec::Fillet { radius, .. } => {
            if !radius.is_finite() || radius < 0.0 {
                return Err(refusal(
                    stem,
                    "radius_negative",
                    format!("the FILLET radius must be finite and non-negative, got {radius}"),
                ));
            }
        }
        PolylineCornerSpec::Chamfer { a, b, angle, .. } => {
            if !a.is_finite() || !b.is_finite() || a < 0.0 || b < 0.0 {
                return Err(refusal(
                    stem,
                    "distance_negative",
                    format!(
                        "the CHAMFER distances must be finite and non-negative, got A={a}, B={b}"
                    ),
                ));
            }
            if let Some(angle) = angle {
                if !angle.is_finite() || angle < 0.0 || angle >= std::f64::consts::PI {
                    return Err(refusal(
                        stem,
                        "angle_out_of_range",
                        format!("the CHAMFER angle must be in 0..180 degrees, got {angle} rad"),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Test-only parity reference: the plan the **two-line** `plan_fillet` builds for
/// the same two arms (the polyline corner is the same geometry).
#[cfg(test)]
pub(crate) type TwoLineFilletPlan = ((Point3, Point3), (Point3, Point3), Option<SemanticGeometry>);

#[cfg(test)]
pub(crate) fn two_line_fillet_plan(
    first: (Point3, Point3),
    pick_first: Point3,
    second: (Point3, Point3),
    pick_second: Point3,
    radius: f64,
    tol: f64,
) -> Option<TwoLineFilletPlan> {
    let input = super::helpers::TwoLineInput {
        first,
        pick_first,
        second,
        pick_second,
    };
    super::helpers::plan_fillet(&input, radius, tol)
        .ok()
        .map(|edit| (edit.first, edit.second, edit.inserted))
}

/// Test-only: the runtime corner tolerance the command hands the planner.
#[cfg(test)]
pub(crate) fn polyline_corner_tolerance() -> f64 {
    corner_tolerance()
}

impl Application {
    /// `FILLET POLYLINE`: round every eligible junction of the one selected
    /// polyline with the session `filletRadius`.
    pub(crate) fn fillet_polyline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let modes = session.drafting_extras.corner_modes();
        let spec = PolylineCornerSpec::Fillet {
            radius: polyline_parameter(session, ToolParameter::FilletRadius, "fillet")?,
            trim: modes.trim(),
        };
        self.polyline_corner_edit(session, command, spec)
    }

    /// `CHAMFER POLYLINE`: chamfer every eligible junction with the session
    /// `chamferDistanceA`/`B` (or the angle method's stored angle).
    pub(crate) fn chamfer_polyline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let modes = session.drafting_extras.corner_modes();
        let a = polyline_parameter(session, ToolParameter::ChamferDistanceA, "chamfer")?;
        let (b, angle) = match modes.chamfer_method() {
            crate::session_state::ChamferMethod::Distance => (
                polyline_parameter(session, ToolParameter::ChamferDistanceB, "chamfer")?,
                None,
            ),
            // Applied per vertex: `b = a·sin α / sin(φ + α)` with the local
            // corner angle `φ`, exactly like the two-line angle method.
            crate::session_state::ChamferMethod::Angle => {
                (0.0, Some(modes.chamfer_angle_radians()))
            }
        };
        let spec = PolylineCornerSpec::Chamfer {
            a,
            b,
            angle,
            trim: modes.trim(),
        };
        self.polyline_corner_edit(session, command, spec)
    }

    /// The shared polyline corner edit: resolve the single selection, plan the
    /// rewrite **before any write**, then commit one transaction / one undo record.
    fn polyline_corner_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
        spec: PolylineCornerSpec,
    ) -> CadResult<CommandOutcome> {
        let stem = spec.stem();
        let command_name = spec.command_name();
        if !matches!(command.payload, CommandPayload::None) {
            return Err(invalid(
                stem,
                "invalid_payload",
                format!("{command_name} takes no payload; the selected polyline is the input"),
            ));
        }
        let references = session.selection.refs();
        let [reference] = references else {
            return Err(invalid(
                stem,
                "selection",
                format!(
                    "{command_name} modifies exactly one polyline, got {} selected",
                    references.len()
                ),
            ));
        };
        if reference.document != command.document {
            return Err(invalid(
                stem,
                "not_addressable",
                format!(
                    "the selected ref for entity {} belongs to another document",
                    reference.entity.0
                ),
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(CadError::Unsupported(refusal(
                stem,
                "not_addressable",
                format!(
                    "{command_name} needs a model-space polyline with an addressable entity \
                     identity; entity {} is referenced through a block instance or a sub-element",
                    reference.entity.0
                ),
            )));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let entity = candidate.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                stem,
                "target_missing",
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        let (points, bulges, closed) = match &entity.geometry {
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
                ..
            } => (points.clone(), bulges.clone(), *closed),
            other => {
                return Err(CadError::Unsupported(refusal(
                    stem,
                    "not_polyline",
                    format!(
                        "{command_name} needs an LWPOLYLINE pick; {} is not supported",
                        class_name(other)
                    ),
                )));
            }
        };
        let tol = corner_tolerance();
        let plan =
            plan_polyline_corners(&points, &bulges, closed, spec, tol).map_err(plan_error)?;
        if plan.cornered == 0 {
            return Err(invalid(
                stem,
                if spec.trim() {
                    "nothing_to_modify"
                } else {
                    "no_join"
                },
                format!(
                    "{command_name}: no eligible junction could be modified ({} skipped)",
                    plan.skipped
                ),
            ));
        }
        let entity_id = entity.id;
        let transaction = next_transaction_id();
        let (candidate_drawing, changes, objects) = if spec.trim() {
            let mut after = entity.clone();
            after.geometry = SemanticGeometry::Polyline {
                points: plan.points.clone(),
                bulges: plan.bulges.clone(),
                closed: plan.closed,
                width: 0.0,
            };
            let change_set = {
                let mut staged = candidate.begin_drawing_transaction(spec.label(), transaction)?;
                staged.update_entity(after.clone())?;
                staged.commit()?
            };
            (candidate, change_set, vec![ObjectId(entity_id.0)])
        } else {
            let inserted_entities: Vec<(EntityId, DbEntity)> = plan
                .inserted
                .iter()
                .map(|geometry| {
                    let draw_order = candidate
                        .model_space()
                        .iter()
                        .map(|entity| entity.draw_order)
                        .max()
                        .map(|max| max.saturating_add(1))
                        .unwrap_or(0);
                    let new_id = candidate.allocate_entity_id();
                    let new_entity = DbEntity {
                        object: cad_db::DbObject {
                            id: ObjectId(new_id.0),
                            type_key: class_name(geometry).to_string(),
                            revision: Revision(0),
                            source_handle: None,
                        },
                        id: new_id,
                        layer: session.active_layer,
                        space: SpaceId::Model,
                        geometry: geometry.clone(),
                        draw_order,
                    };
                    (new_id, new_entity)
                })
                .collect();
            let change_set = {
                let mut staged = candidate.begin_drawing_transaction(spec.label(), transaction)?;
                for (_, entity) in &inserted_entities {
                    staged.insert_entity(entity.clone())?;
                }
                staged.commit()?
            };
            let objects = inserted_entities
                .iter()
                .map(|(id, _)| ObjectId(id.0))
                .collect();
            (candidate, change_set, objects)
        };
        // Re-read the committed entities so undo patches store exact values.
        let mut patches = Vec::new();
        if spec.trim() {
            let stored = candidate_drawing
                .entity(entity_id)
                .cloned()
                .ok_or_else(|| {
                    CadError::Invariant(format!(
                        "{command_name}: the polyline vanished after commit"
                    ))
                })?;
            patches.push(drawing_patch(entity_id, Some(entity.clone()), Some(stored)));
        } else {
            for object in &objects {
                let stored = candidate_drawing
                    .entity(EntityId(object.0))
                    .cloned()
                    .ok_or_else(|| {
                        CadError::Invariant(format!(
                            "{command_name}: inserted entity vanished after commit"
                        ))
                    })?;
                patches.push(drawing_patch(EntityId(object.0), None, Some(stored)));
            }
        }
        self.publish_candidate(
            document_id,
            candidate_drawing,
            DrawingUndoRecord {
                transaction: changes.transaction,
                label: spec.label().to_string(),
                patches,
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        let detail = match spec {
            PolylineCornerSpec::Fillet { radius, .. } => format!("R={radius}"),
            PolylineCornerSpec::Chamfer { a, b, angle, .. } => match angle {
                Some(angle) => format!("A={a}, angle={} deg", angle.to_degrees()),
                None => format!("A={a}, B={b}"),
            },
        };
        Ok(CommandOutcome {
            objects,
            changes: Some(Arc::new(changes)),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity_id.0)),
                code: spec.diagnostic_code().into(),
                message: format!(
                    "{command_name}: {detail} at {} junction(s), {} skipped, {} joining \
                     entit(ies) inserted (trim={})",
                    plan.cornered,
                    plan.skipped,
                    plan.inserted.len(),
                    spec.trim()
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Read one session tool parameter as a finite number, or refuse by name.
fn polyline_parameter(
    session: &SessionState,
    parameter: ToolParameter,
    stem: &str,
) -> CadResult<f64> {
    match session.tool_parameter(parameter) {
        ToolValue::Number(value) if value.is_finite() => Ok(value),
        other => Err(invalid(
            stem,
            "parameter_missing",
            format!(
                "session parameter '{}' holds no finite number ({:?})",
                parameter.key(),
                other
            ),
        )),
    }
}

/// An `<stem>.polyline_<code>: <detail>` [`CadError::InvalidInput`].
fn invalid(stem: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(refusal(stem, code, detail))
}

/// Classify a planner refusal: the capability gaps stay unsupported, a supported
/// shape whose numbers do not work is `InvalidInput`.
fn plan_error(message: String) -> CadError {
    if message.contains(".polyline_not_planar") || message.contains(".polyline_arc_chain") {
        CadError::Unsupported(message)
    } else {
        CadError::InvalidInput(message)
    }
}
