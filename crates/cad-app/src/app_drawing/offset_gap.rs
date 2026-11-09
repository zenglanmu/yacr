//! The `OFFSETGAPTYPE` fill geometry (fillet / chamfer) for polyline offsets.
//!
//! Split out of the two offset arms so all three files stay inside the code-size
//! limit. Every joint is first resolved as the sharp analytic miter (the arms'
//! own `join_vertex` / `miter_vertex`); a gap exists only when that miter requires
//! **both** offset segments to be extended (the miter lies past the previous
//! segment's end and behind the next segment's start). Where the segments already
//! cross (a concave corner) there is no gap to fill and the miter is kept.
//!
//! * `OFFSETGAPTYPE` 2 (`Chamfer`): a straight segment joins the two untrimmed
//!   offset segment ends.
//! * `OFFSETGAPTYPE` 1 (`Fillet`): a tangent arc of radius equal to the offset
//!   distance, centred on the **source vertex**. Both offset segment ends lie at
//!   exactly the offset distance from that vertex (`p = vertex + s·m` for a line,
//!   `|R′ − R| = |s|` for a concentric arc), so the circle centred there passes
//!   through both ends and is tangent to both carriers — the exact fillet AutoCAD
//!   draws. The emitted bulge is verified through the single `bulge → arc`
//!   authority before it is accepted.
//!
//! Straight silence is never invented: a real gap under `Sharp` is the ordinary
//! miter, and the pre-existing collapse/ambiguity refusals are unchanged.

use super::helpers::offset_tolerance;
use super::offset_polyline::{miter_vertex, require_no_closed_self_intersection};
use super::offset_polyline_bulge::{
    arc_segment_bulge, join_vertex, open_vertex, OffsetSegment, SourceArc,
};
use super::*;
use cad_geometry::{add, bulge_arc_geometry, cross, dot, length, scale, sub};

/// The vertices contributed by one polyline joint under a gap type.
///
/// [`OffsetGapType::Sharp`] (and every joint whose offset segments already cross)
/// contributes the single analytic miter point. A filled gap
/// ([`OffsetGapType::Fillet`] / [`OffsetGapType::Chamfer`]) contributes the two
/// offset segment ends joined by one extra segment: the tangent fillet arc
/// (`bulge != 0`) or the chamfer line (`bulge == 0`).
#[derive(Debug, Clone, Copy)]
pub(super) enum JointVertices {
    Miter(Point3),
    Gap {
        first: Point3,
        second: Point3,
        bulge: f64,
    },
}

/// Append a joint's vertices, returning the `(first, last)` output indices.
pub(super) fn push_joint(
    out: &mut Vec<Point3>,
    out_bulges: &mut Vec<f64>,
    joint: JointVertices,
) -> (usize, usize) {
    let first = out.len();
    match joint {
        JointVertices::Miter(point) => {
            out.push(point);
            out_bulges.push(0.0);
        }
        JointVertices::Gap {
            first: a,
            second: b,
            bulge,
        } => {
            out.push(a);
            out_bulges.push(bulge);
            out.push(b);
            out_bulges.push(0.0);
        }
    }
    (first, out.len() - 1)
}

/// Write each source segment's bulge onto the edge its joint vertices span.
///
/// `joint_first[i]` / `joint_last[i]` are the output indices of the first and
/// last vertex of the joint at source vertex `i`; the edge from
/// `joint_last[i]` to `joint_first[i + 1]` is what source segment `i` became, so
/// the bulge `source_bulge(i, start, end)` (re-derived from the actual trimmed
/// ends) belongs on `joint_last[i]`'s outgoing edge.
pub(super) fn finish_gapped(
    points: Vec<Point3>,
    mut bulges: Vec<f64>,
    joint_first: &[usize],
    joint_last: &[usize],
    segments: usize,
    closed: bool,
    source_bulge: impl Fn(usize, Point3, Point3) -> CadResult<f64>,
) -> CadResult<(Vec<Point3>, Vec<f64>)> {
    for index in 0..segments {
        let next = if closed {
            (index + 1) % segments
        } else {
            index + 1
        };
        let start_point = points[joint_last[index]];
        let end_point = points[joint_first[next]];
        let bulge = source_bulge(index, start_point, end_point)?;
        bulges[joint_last[index]] = bulge;
    }
    Ok((points, bulges))
}

/// The straight-only path under `OFFSETGAPTYPE` 1/2.
#[allow(clippy::too_many_arguments)]
pub(super) fn offset_straight_with_gap(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    gap: OffsetGapType,
    dirs: &[Point3],
    offsets: &[Point3],
    normal: Point3,
    signed: f64,
    distance: f64,
    tol: f64,
) -> CadResult<(SemanticGeometry, f64)> {
    let count = points.len();
    let segments = dirs.len();
    let mut out: Vec<Point3> = Vec::new();
    let mut out_bulges: Vec<f64> = Vec::new();
    let mut joint_first = vec![0usize; count];
    let mut joint_last = vec![0usize; count];
    if closed {
        for index in 0..count {
            let joint = straight_joint(index, points, dirs, offsets, normal, signed, gap, tol)?;
            let (first, last) = push_joint(&mut out, &mut out_bulges, joint);
            joint_first[index] = first;
            joint_last[index] = last;
        }
    } else {
        // The two open ends are not joints: they move with their own segment,
        // exactly like the sharp path (no end cap is invented).
        out.push(add(points[0], offsets[0]));
        out_bulges.push(0.0);
        joint_first[0] = 0;
        joint_last[0] = 0;
        for index in 1..count - 1 {
            let joint = straight_joint(index, points, dirs, offsets, normal, signed, gap, tol)?;
            let (first, last) = push_joint(&mut out, &mut out_bulges, joint);
            joint_first[index] = first;
            joint_last[index] = last;
        }
        out.push(add(points[count - 1], offsets[segments - 1]));
        out_bulges.push(0.0);
        let last_index = out.len() - 1;
        joint_first[count - 1] = last_index;
        joint_last[count - 1] = last_index;
    }
    let (out, out_bulges) = finish_gapped(
        out,
        out_bulges,
        &joint_first,
        &joint_last,
        segments,
        closed,
        |index, _, _| Ok(bulges.get(index).copied().unwrap_or(0.0)),
    )?;
    require_no_gapped_line_collapse(&out, &joint_first, &joint_last, dirs, closed, tol)?;
    let geometry = SemanticGeometry::Polyline {
        points: out,
        bulges: out_bulges,
        closed,
    };
    if closed {
        require_no_closed_self_intersection(&geometry, normal, tol)?;
    }
    Ok((geometry, distance))
}

/// The joint at source vertex `index` for the straight-only arm.
///
/// The miter point is always computed first: it is the result for
/// [`OffsetGapType::Sharp`], and its position relative to the two untrimmed
/// offset segment ends decides whether there is a gap at all. The offset lines
/// cross (a trim) unless the miter lies past the end of the previous segment and
/// behind the start of the next one; only a real gap is filled, so a concave
/// corner keeps the miter instead of gaining an invented arc/line.
#[allow(clippy::too_many_arguments)]
fn straight_joint(
    index: usize,
    points: &[Point3],
    dirs: &[Point3],
    offsets: &[Point3],
    normal: Point3,
    signed: f64,
    gap: OffsetGapType,
    tol: f64,
) -> CadResult<JointVertices> {
    let segments = dirs.len();
    let previous = (index + segments - 1) % segments;
    let vertex = points[index];
    let miter = miter_vertex(vertex, dirs[previous], dirs[index], normal, signed)?;
    if gap == OffsetGapType::Sharp {
        return Ok(JointVertices::Miter(miter));
    }
    let first = add(vertex, offsets[previous]);
    let second = add(vertex, offsets[index]);
    let along_previous = dot(sub(miter, first), dirs[previous]);
    let along_next = dot(sub(miter, second), dirs[index]);
    if along_previous <= tol || along_next >= -tol {
        return Ok(JointVertices::Miter(miter));
    }
    match gap {
        OffsetGapType::Sharp => Ok(JointVertices::Miter(miter)),
        OffsetGapType::Chamfer => Ok(JointVertices::Gap {
            first,
            second,
            bulge: 0.0,
        }),
        OffsetGapType::Fillet => {
            let bulge = fillet_bulge(first, second, vertex, normal)?;
            Ok(JointVertices::Gap {
                first,
                second,
                bulge,
            })
        }
    }
}

/// The bulge of the `OFFSETGAPTYPE` 1 fillet arc that fills one gap.
///
/// Both offset segment ends lie at exactly the offset distance from the source
/// vertex: `p = vertex + s·m` for a line and `|R′ − R| = |s|` for an arc
/// (`R′ = R − s·sign(σ)`), so the circle **centred at the source vertex** with
/// radius `|s|` passes through both and is tangent to both carriers there. That
/// circle's arc is the fillet AutoCAD's gap type 1 draws; the sweep is the signed
/// in-plane angle between the two radius vectors (always the minor arc, so a gap
/// is never filled with the complementary major arc). The emitted bulge is
/// verified through the single `bulge → arc` authority before it is returned.
pub(super) fn fillet_bulge(
    first: Point3,
    second: Point3,
    vertex: Point3,
    normal: Point3,
) -> CadResult<f64> {
    let a = sub(first, vertex);
    let b = sub(second, vertex);
    let radius = length(a);
    let sweep = dot(cross(a, b), normal).atan2(dot(a, b));
    if !sweep.is_finite() || !radius.is_finite() || radius <= offset_tolerance() {
        return Err(CadError::Unsupported(
            "OFFSET offset.gap_type: the fillet gap has no finite radius or sweep at its vertex"
                .into(),
        ));
    }
    let bulge = (sweep / 4.0).tan();
    let tolerance = 1e-9 * (1.0 + radius);
    let recovered = match bulge_arc_geometry(first, second, bulge, normal) {
        Some(SemanticGeometry::Arc {
            center, radius: r, ..
        }) => Some((center, r)),
        _ => None,
    };
    match recovered {
        Some((center, r))
            if distance(center, vertex) <= tolerance && (r - radius).abs() <= tolerance =>
        {
            Ok(bulge)
        }
        _ => Err(CadError::Unsupported(
            "OFFSET offset.gap_type: the fillet arc does not survive the bulge round-trip (it is \
             not the circle centred on the source vertex at the offset distance)"
                .into(),
        )),
    }
}

/// Refuse a gapped offset whose source line segment folded back or vanished.
///
/// The line segment is now the edge from its joint's last vertex to the next
/// joint's first vertex (a fillet/chamfer inserts vertices, so the one-vertex-per
/// -segment check of the sharp path no longer applies). Arc segments are covered
/// by their own sweep checks.
fn require_no_gapped_line_collapse(
    out: &[Point3],
    joint_first: &[usize],
    joint_last: &[usize],
    dirs: &[Point3],
    closed: bool,
    tol: f64,
) -> CadResult<()> {
    for (index, u) in dirs.iter().enumerate() {
        let next = if closed {
            (index + 1) % dirs.len()
        } else {
            index + 1
        };
        let segment = sub(out[joint_first[next]], out[joint_last[index]]);
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

/// The exact arc-segment arm under `OFFSETGAPTYPE` 1/2.
///
/// Every joint is the same `join_vertex` miter the sharp path uses (so a concave
/// corner keeps the trim), except where the two offset segments genuinely do not
/// meet: there the gap is filled with the fillet arc or the chamfer line, exactly
/// as in the straight arm. Arc segments keep their concentric offset and are
/// trimmed to the gap's tangent/end point, and their bulges are re-derived from
/// the actual trimmed ends through [`arc_segment_bulge`], so a moved join point
/// can never silently change an arc into a different circle.
#[allow(clippy::too_many_arguments)]
pub(super) fn gapped_arc_ring(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    gap: OffsetGapType,
    dirs: &[Point3],
    offset: &[OffsetSegment],
    normal: Point3,
    signed: f64,
    tol: f64,
    ax: Point3,
    ay: Point3,
) -> CadResult<SemanticGeometry> {
    let count = points.len();
    let segments = offset.len();
    let mut out: Vec<Point3> = Vec::new();
    let mut out_bulges: Vec<f64> = Vec::new();
    let mut joint_first = vec![0usize; count];
    let mut joint_last = vec![0usize; count];
    if closed {
        for index in 0..count {
            let joint = arc_joint(
                index, points, dirs, offset, normal, signed, gap, tol, ax, ay,
            )?;
            let (first, last) = push_joint(&mut out, &mut out_bulges, joint);
            joint_first[index] = first;
            joint_last[index] = last;
        }
    } else {
        // The two open ends move with their own segment and are never filled.
        out.push(open_vertex(&offset[0], ax, ay, true));
        out_bulges.push(0.0);
        joint_first[0] = 0;
        joint_last[0] = 0;
        for index in 1..count - 1 {
            let joint = arc_joint(
                index, points, dirs, offset, normal, signed, gap, tol, ax, ay,
            )?;
            let (first, last) = push_joint(&mut out, &mut out_bulges, joint);
            joint_first[index] = first;
            joint_last[index] = last;
        }
        out.push(open_vertex(&offset[segments - 1], ax, ay, false));
        out_bulges.push(0.0);
        let last_index = out.len() - 1;
        joint_first[count - 1] = last_index;
        joint_last[count - 1] = last_index;
    }
    let (out, out_bulges) = finish_gapped(
        out,
        out_bulges,
        &joint_first,
        &joint_last,
        segments,
        closed,
        |index, start, end| match &offset[index] {
            OffsetSegment::Arc(arc) => arc_segment_bulge(index, arc, start, end, normal, tol),
            OffsetSegment::Line { .. } => Ok(bulges.get(index).copied().unwrap_or(0.0)),
        },
    )?;
    require_no_line_collapse_gapped(&out, offset, &joint_first, &joint_last, dirs, closed, tol)?;
    let geometry = SemanticGeometry::Polyline {
        points: out,
        bulges: out_bulges,
        closed,
    };
    if closed {
        super::offset_polyline::require_no_closed_self_intersection(&geometry, normal, tol)?;
    }
    Ok(geometry)
}

/// The joint at source vertex `index` for the arc-aware arm.
#[allow(clippy::too_many_arguments)]
fn arc_joint(
    index: usize,
    points: &[Point3],
    dirs: &[Point3],
    offset: &[OffsetSegment],
    normal: Point3,
    signed: f64,
    gap: OffsetGapType,
    tol: f64,
    ax: Point3,
    ay: Point3,
) -> CadResult<JointVertices> {
    let segments = offset.len();
    let previous = (index + segments - 1) % segments;
    let vertex = points[index];
    let first = segment_end_point(&offset[previous], ax, ay);
    let second = segment_start_point(&offset[index], ax, ay);
    // A gap is a local **divergence**, decided from the two traversal tangents
    // alone (never from an intersection, which legitimately does not exist at a
    // gap): offsetting to the outer side of a turn (`signed · turn < 0`) leaves a
    // gap, offsetting to the inner side crosses the carriers and the miter trims.
    // The tangents are the source segments' own tangents at the vertex — a
    // concentric/shifted offset preserves them — so the sign test is exact for
    // line/line, line/arc and arc/arc joints alike.
    let diverges = if gap == OffsetGapType::Sharp {
        false
    } else {
        let previous_direction = segment_end_direction(&offset[previous], ax, ay);
        let next_direction = segment_start_direction(&offset[index], ax, ay);
        let turn = dot(cross(previous_direction, next_direction), normal)
            .atan2(dot(previous_direction, next_direction));
        turn.is_finite() && signed * turn < 0.0
    };
    if !diverges {
        let miter = join_vertex(index, points, dirs, offset, normal, signed, tol)?;
        return Ok(JointVertices::Miter(miter));
    }
    match gap {
        OffsetGapType::Sharp => {
            // Unreachable by the `diverges` guard; keeps the match total.
            let miter = join_vertex(index, points, dirs, offset, normal, signed, tol)?;
            Ok(JointVertices::Miter(miter))
        }
        OffsetGapType::Chamfer => Ok(JointVertices::Gap {
            first,
            second,
            bulge: 0.0,
        }),
        OffsetGapType::Fillet => {
            let bulge = fillet_bulge(first, second, vertex, normal)?;
            Ok(JointVertices::Gap {
                first,
                second,
                bulge,
            })
        }
    }
}

/// The untrimmed start point of one offset segment.
fn segment_start_point(segment: &OffsetSegment, ax: Point3, ay: Point3) -> Point3 {
    match segment {
        OffsetSegment::Line { start, .. } => *start,
        OffsetSegment::Arc(arc) => arc.point_at(arc.start, ax, ay),
    }
}

/// The untrimmed end point of one offset segment.
fn segment_end_point(segment: &OffsetSegment, ax: Point3, ay: Point3) -> Point3 {
    match segment {
        OffsetSegment::Line { end, .. } => *end,
        OffsetSegment::Arc(arc) => arc.point_at(arc.start + arc.sweep, ax, ay),
    }
}

/// The unit traversal direction at one offset segment's start.
fn segment_start_direction(segment: &OffsetSegment, ax: Point3, ay: Point3) -> Point3 {
    match segment {
        OffsetSegment::Line { start, end } => unit_or_zero(sub(*end, *start)),
        OffsetSegment::Arc(arc) => arc_tangent(arc, arc.start, ax, ay),
    }
}

/// The unit traversal direction at one offset segment's end.
fn segment_end_direction(segment: &OffsetSegment, ax: Point3, ay: Point3) -> Point3 {
    match segment {
        OffsetSegment::Line { start, end } => unit_or_zero(sub(*end, *start)),
        OffsetSegment::Arc(arc) => arc_tangent(arc, arc.start + arc.sweep, ax, ay),
    }
}

/// The unit tangent of an arc at one angle, in its stored traversal direction.
fn arc_tangent(arc: &SourceArc, angle: f64, ax: Point3, ay: Point3) -> Point3 {
    let tangent = add(scale(ax, -angle.sin()), scale(ay, angle.cos()));
    if arc.sweep < 0.0 {
        scale(tangent, -1.0)
    } else {
        tangent
    }
}

/// A unit vector, or zero for a degenerate input (unreachable for a validated
/// offset segment; a zero direction simply makes the gap test false).
fn unit_or_zero(vector: Point3) -> Point3 {
    let len = length(vector);
    if len.is_finite() && len > 0.0 {
        scale(vector, 1.0 / len)
    } else {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

/// Refuse a gapped offset whose offset **line** segment folded back or vanished.
fn require_no_line_collapse_gapped(
    out: &[Point3],
    offset: &[OffsetSegment],
    joint_first: &[usize],
    joint_last: &[usize],
    dirs: &[Point3],
    closed: bool,
    tol: f64,
) -> CadResult<()> {
    for (index, segment) in offset.iter().enumerate() {
        if matches!(segment, OffsetSegment::Arc(_)) {
            continue;
        }
        let next = if closed {
            (index + 1) % offset.len()
        } else {
            index + 1
        };
        let piece = sub(out[joint_first[next]], out[joint_last[index]]);
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
