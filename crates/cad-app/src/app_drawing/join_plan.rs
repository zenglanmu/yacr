//! The JOIN planning predicates (source-object and MULTIPLE modes).

use super::super::helpers::normalize_signed;
use super::super::*;
use super::{
    angle_tolerance, chain_pieces, invalid, join_tolerance, piece_from_geometry, unsupported,
};
use crate::command_payload::{JoinMode, JoinOptions};
use cad_geometry::{add, clamped_uniform_knots, dot, length, scale, sub};

const TAU: f64 = std::f64::consts::TAU;

/// One inclusive span along the common line (world units from the reference
/// origin).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    start: f64,
    end: f64,
}

/// The merged geometry a JOIN will insert, resolved before any write.
pub(super) enum Joined {
    Line {
        start: Point3,
        end: Point3,
    },
    Arc {
        center: Point3,
        normal: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Circle {
        center: Point3,
        normal: Point3,
        radius: f64,
    },
    Ellipse {
        center: Point3,
        normal: Point3,
        major_axis: Point3,
        ratio: f64,
        start: f64,
        sweep: f64,
    },
    Polyline {
        points: Vec<Point3>,
        bulges: Vec<f64>,
        closed: bool,
    },
    Spline(SplineCurve),
    /// A geometry a planner already assembled (the mixed NURBS weld).
    Raw(SemanticGeometry),
}

impl Joined {
    pub(super) fn geometry(self) -> SemanticGeometry {
        match self {
            Joined::Line { start, end } => SemanticGeometry::Line { start, end },
            Joined::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            },
            Joined::Circle {
                center,
                normal,
                radius,
            } => SemanticGeometry::Circle {
                center,
                normal,
                radius,
            },
            Joined::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            },
            Joined::Polyline {
                points,
                bulges,
                closed,
            } => SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            },
            Joined::Spline(curve) => curve.geometry(),
            Joined::Raw(geometry) => geometry,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SplineCurve {
    degree: u32,
    knots: Vec<f64>,
    control_points: Vec<Point3>,
    weights: Vec<f64>,
}

impl SplineCurve {
    /// Expand the sanctioned empty-knot/empty-weight shorthand to the stored
    /// clamped-uniform form.
    fn from_geometry(
        degree: u32,
        knots: &[f64],
        control_points: &[Point3],
        weights: &[f64],
    ) -> CadResult<Self> {
        let n = control_points.len();
        let degree = degree.max(1).min(n.saturating_sub(1) as u32);
        if n < degree as usize + 1 {
            return Err(invalid(
                "join.degenerate",
                "a SPLINE has too few control points",
            ));
        }
        let knots = if knots.is_empty() {
            clamped_uniform_knots(n, degree as usize)
        } else {
            knots.to_vec()
        };
        let weights = if weights.is_empty() {
            vec![1.0; n]
        } else {
            weights.to_vec()
        };
        Ok(SplineCurve {
            degree,
            knots,
            control_points: control_points.to_vec(),
            weights,
        })
    }

    fn geometry(self) -> SemanticGeometry {
        SemanticGeometry::Spline {
            degree: self.degree,
            knots: self.knots,
            control_points: self.control_points,
            weights: self.weights,
        }
    }

    fn head(&self) -> Point3 {
        self.control_points[0]
    }

    fn tail(&self) -> Point3 {
        *self.control_points.last().expect("validated non-empty")
    }

    /// Reverse the curve: reversed control points/weights and mirrored knots.
    fn reversed(self) -> Self {
        let mut control_points = self.control_points;
        let mut weights = self.weights;
        control_points.reverse();
        weights.reverse();
        let knots = self.knots;
        let sum = knots[0] + knots[knots.len() - 1];
        let knots = knots.iter().rev().map(|knot| sum - knot).collect();
        SplineCurve {
            degree: self.degree,
            knots,
            control_points,
            weights,
        }
    }

    /// Exact C0 concatenation: `U = U1 ++ shift(U2)[p+1..]` and
    /// `P = P1 ++ P2`, so the join knot has multiplicity `p + 1` and the
    /// control points stay the stored ones.
    fn concat(mut self, other: SplineCurve) -> CadResult<Self> {
        if self.degree != other.degree {
            return Err(unsupported(
                "join.spline_degree",
                format!(
                    "SPLINE degrees {} and {} cannot be one curve",
                    self.degree, other.degree
                ),
            ));
        }
        let p = self.degree as usize;
        let total = self.control_points.len() + other.control_points.len();
        if self.knots.len() != self.control_points.len() + p + 1
            || other.knots.len() != other.control_points.len() + p + 1
        {
            return Err(unsupported(
                "join.spline_clamped",
                "a source SPLINE does not have a clamped knot vector to concatenate",
            ));
        }
        let first = self.knots[0];
        let first_end = self.knots[self.control_points.len()];
        let other_start = other.knots[0];
        let other_end = other.knots[other.control_points.len()];
        if first_end <= first || other_end <= other_start {
            return Err(invalid(
                "join.degenerate",
                "a SPLINE has an empty knot domain",
            ));
        }
        if self.knots[self.control_points.len()..]
            .iter()
            .any(|knot| (knot - first_end).abs() > 1e-12)
            || other.knots[..=p]
                .iter()
                .any(|knot| (knot - other_start).abs() > 1e-12)
        {
            return Err(unsupported(
                "join.spline_clamped",
                "a source SPLINE is not clamped at the junction end",
            ));
        }
        // Normalise both pieces onto [0, 1] and [1, 2].
        let span = first_end - first;
        let mut knots: Vec<f64> = self
            .knots
            .iter()
            .map(|knot| (knot - first) / span)
            .collect();
        let other_span = other_end - other_start;
        let shifted: Vec<f64> = other.knots[p + 1..]
            .iter()
            .map(|knot| 1.0 + (knot - other_start) / other_span)
            .collect();
        knots.extend(shifted);
        debug_assert_eq!(knots.len(), total + p + 1);
        let mut control_points = std::mem::take(&mut self.control_points);
        control_points.extend(other.control_points);
        let mut weights = std::mem::take(&mut self.weights);
        weights.extend(other.weights);
        Ok(SplineCurve {
            degree: self.degree,
            knots,
            control_points,
            weights,
        })
    }
}

/// Resolve the one merged geometry from the sources, or refuse by name.
pub(super) fn plan_join(sources: &[DbEntity], options: JoinOptions) -> CadResult<Joined> {
    for source in sources {
        refuse_unjointable(source)?;
    }
    match options.mode {
        JoinMode::Multiple => plan_multiple(sources, options.close),
        JoinMode::SourceObject => plan_source_object(sources, options.close),
    }
}

/// Objects AutoCAD never joins: CIRCLEs, closed curves and construction lines.
fn refuse_unjointable(source: &DbEntity) -> CadResult<()> {
    let tol = join_tolerance();
    match &source.object.type_key[..] {
        "AcDbXline" | "AcDbRay" => {
            return Err(unsupported(
                "join.construction_line",
                format!(
                    "entity {} is an infinite construction line; JOIN needs finite curves",
                    source.id.0
                ),
            ));
        }
        _ => {}
    }
    match &source.geometry {
        SemanticGeometry::Circle { .. } => Err(invalid(
            "join.closed_object",
            format!(
                "entity {} is a CIRCLE; a closed object cannot be joined",
                source.id.0
            ),
        )),
        SemanticGeometry::Polyline { closed: true, .. } => Err(invalid(
            "join.closed_object",
            format!(
                "entity {} is a closed POLYLINE; a closed object cannot be joined",
                source.id.0
            ),
        )),
        SemanticGeometry::Ellipse { sweep, .. } if sweep.abs() >= TAU - tol => Err(invalid(
            "join.closed_object",
            format!(
                "entity {} is a full ELLIPSE; a closed object cannot be joined",
                source.id.0
            ),
        )),
        SemanticGeometry::Spline { control_points, .. }
            if control_points.len() >= 2
                && distance(
                    control_points[0],
                    *control_points.last().expect("checked len"),
                ) <= tol =>
        {
            Err(invalid(
                "join.closed_object",
                format!(
                    "entity {} is a closed SPLINE (coincident endpoints); a closed object \
                     cannot be joined",
                    source.id.0
                ),
            ))
        }
        _ => Ok(()),
    }
}

fn plan_source_object(sources: &[DbEntity], close: bool) -> CadResult<Joined> {
    match &sources[0].geometry {
        SemanticGeometry::Line { .. } => {
            ensure_all(sources, |geometry| {
                matches!(geometry, SemanticGeometry::Line { .. })
            })?;
            plan_lines(&line_list(sources))
        }
        SemanticGeometry::Arc { .. } => {
            ensure_all(sources, |geometry| {
                matches!(geometry, SemanticGeometry::Arc { .. })
            })?;
            plan_arcs(&arc_list(sources), close)
        }
        SemanticGeometry::Polyline { .. } => plan_polyline_sources(sources, close),
        SemanticGeometry::Ellipse { .. } => {
            ensure_all(sources, |geometry| {
                matches!(geometry, SemanticGeometry::Ellipse { .. })
            })?;
            plan_ellipses(&ellipse_list(sources), close)
        }
        SemanticGeometry::Spline { .. } => {
            ensure_all(sources, |geometry| {
                matches!(geometry, SemanticGeometry::Spline { .. })
            })?;
            plan_splines(sources, close)
        }
        other => Err(unsupported(
            "join.unsupported",
            format!("{} cannot be joined", class_name(other)),
        )),
    }
}

fn ensure_all(sources: &[DbEntity], accepts: impl Fn(&SemanticGeometry) -> bool) -> CadResult<()> {
    if sources.iter().all(|source| accepts(&source.geometry)) {
        Ok(())
    } else {
        Err(invalid(
            "join.mixed_kinds",
            "the first pick's kind is the join source; the other picks must be the same kind \
             (use JOIN MULTIPLE for a mixed selection)",
        ))
    }
}

fn line_list(sources: &[DbEntity]) -> Vec<(Point3, Point3)> {
    sources
        .iter()
        .filter_map(|source| match &source.geometry {
            SemanticGeometry::Line { start, end } => Some((*start, *end)),
            _ => None,
        })
        .collect()
}

type ArcEntry = (Point3, Point3, f64, f64, f64);

fn arc_list(sources: &[DbEntity]) -> Vec<ArcEntry> {
    sources
        .iter()
        .filter_map(|source| match &source.geometry {
            SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => Some((*center, *normal, *radius, *start, *sweep)),
            _ => None,
        })
        .collect()
}

type EllipseEntry = (Point3, Point3, Point3, f64, f64, f64);

fn ellipse_list(sources: &[DbEntity]) -> Vec<EllipseEntry> {
    sources
        .iter()
        .filter_map(|source| match &source.geometry {
            SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => Some((*center, *normal, *major_axis, *ratio, *start, *sweep)),
            _ => None,
        })
        .collect()
}

fn plan_multiple(sources: &[DbEntity], close: bool) -> CadResult<Joined> {
    let has_spline = sources
        .iter()
        .any(|source| matches!(source.geometry, SemanticGeometry::Spline { .. }));
    let has_ellipse = sources
        .iter()
        .any(|source| matches!(source.geometry, SemanticGeometry::Ellipse { .. }));
    let all_lines = sources
        .iter()
        .all(|source| matches!(source.geometry, SemanticGeometry::Line { .. }));
    let all_arcs = sources
        .iter()
        .all(|source| matches!(source.geometry, SemanticGeometry::Arc { .. }));
    let all_ellipses = sources
        .iter()
        .all(|source| matches!(source.geometry, SemanticGeometry::Ellipse { .. }));
    let all_splines = sources
        .iter()
        .all(|source| matches!(source.geometry, SemanticGeometry::Spline { .. }));
    let all_polyline_family = sources.iter().all(|source| {
        matches!(
            source.geometry,
            SemanticGeometry::Line { .. }
                | SemanticGeometry::Arc { .. }
                | SemanticGeometry::Polyline { .. }
        )
    });

    if all_lines && collinear(&line_list(sources)) {
        return plan_lines(&line_list(sources));
    }
    if all_arcs && arcs_share_circle(&arc_list(sources)) {
        return plan_arcs(&arc_list(sources), close);
    }
    if all_ellipses {
        return plan_ellipses(&ellipse_list(sources), close);
    }
    if all_splines {
        return plan_splines(sources, close);
    }
    if has_spline || has_ellipse {
        return super::nurbs::plan_mixed_weld(sources, close);
    }
    if all_polyline_family {
        return plan_polyline_sources(sources, close);
    }
    let kinds: Vec<&str> = sources
        .iter()
        .map(|source| class_name(&source.geometry))
        .collect();
    Err(unsupported(
        "join.unsupported",
        format!("JOIN MULTIPLE cannot weld {}", kinds.join(", ")),
    ))
}

fn collinear(lines: &[(Point3, Point3)]) -> bool {
    let Ok(reference) = longest_line(lines) else {
        return false;
    };
    let direction = sub(reference.1, reference.0);
    let reference_length = length(direction);
    if reference_length <= join_tolerance() {
        return false;
    }
    let unit = scale(direction, 1.0 / reference_length);
    let tol = join_tolerance();
    lines.iter().all(|(start, end)| {
        let chord = sub(*end, *start);
        let chord_length = length(chord);
        if chord_length <= tol {
            return false;
        }
        let along = scale(chord, 1.0 / chord_length);
        let across = sub(along, scale(unit, dot(along, unit)));
        if length(across) > (tol / chord_length).max(1e-12) {
            return false;
        }
        [start, end].iter().all(|point| {
            let offset = sub(**point, reference.0);
            let along = dot(offset, unit);
            length(sub(offset, scale(unit, along))) <= tol
        })
    })
}

fn longest_line(lines: &[(Point3, Point3)]) -> CadResult<(Point3, Point3)> {
    lines
        .iter()
        .max_by(|a, b| {
            length(sub(a.1, a.0))
                .partial_cmp(&length(sub(b.1, b.0)))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
        .ok_or_else(|| invalid("join.needs_two", "no LINE to merge"))
}

fn arcs_share_circle(arcs: &[ArcEntry]) -> bool {
    let tol = join_tolerance();
    let Some((center, normal, radius, _, _)) = arcs.first().copied() else {
        return false;
    };
    arcs.iter()
        .all(|(other_center, other_normal, other_radius, _, _)| {
            distance(center, *other_center) <= tol
                && dot(normal, *other_normal) > 0.0
                && (other_radius.abs() - radius.abs()).abs() <= tol
        })
}

/// Plan a LINE merge: collinear, **gaps allowed**, union of the spans.
fn plan_lines(lines: &[(Point3, Point3)]) -> CadResult<Joined> {
    let tol = join_tolerance();
    let reference = longest_line(lines)?;
    let direction = sub(reference.1, reference.0);
    let reference_length = length(direction);
    if !reference_length.is_finite() || reference_length <= tol {
        return Err(invalid(
            "join.degenerate",
            "the longest selected LINE has no length to merge along",
        ));
    }
    let unit = scale(direction, 1.0 / reference_length);
    let base = reference.0;

    let mut spans: Vec<Span> = Vec::with_capacity(lines.len());
    for (index, (start, end)) in lines.iter().enumerate() {
        for point in [start, end] {
            if !is_finite_point(*point) {
                return Err(invalid(
                    "join.degenerate",
                    format!("LINE {index} has a non-finite endpoint"),
                ));
            }
        }
        let chord = sub(*end, *start);
        let chord_length = length(chord);
        if !chord_length.is_finite() || chord_length <= tol {
            return Err(invalid(
                "join.degenerate",
                format!("LINE {index} has no length"),
            ));
        }
        let along = scale(chord, 1.0 / chord_length);
        let across = sub(along, scale(unit, dot(along, unit)));
        if length(across) > (tol / chord_length).max(1e-12) {
            return Err(invalid(
                "join.not_collinear",
                format!("LINE {index} is not parallel to the merge direction"),
            ));
        }
        let mut projected = [0.0_f64; 2];
        for (slot, point) in [start, end].iter().enumerate() {
            let offset = sub(**point, base);
            let along = dot(offset, unit);
            if length(sub(offset, scale(unit, along))) > tol {
                return Err(invalid(
                    "join.not_collinear",
                    format!("LINE {index} does not lie on the first pick's line"),
                ));
            }
            projected[slot] = along;
        }
        spans.push(Span {
            start: projected[0].min(projected[1]),
            end: projected[0].max(projected[1]),
        });
    }

    let union_start = spans
        .iter()
        .map(|span| span.start)
        .fold(f64::INFINITY, f64::min);
    let union_end = spans
        .iter()
        .map(|span| span.end)
        .fold(f64::NEG_INFINITY, f64::max);
    if union_end - union_start <= tol {
        return Err(invalid(
            "join.degenerate",
            "the selected LINEs have no extent to merge",
        ));
    }
    let start = add(base, scale(unit, union_start));
    let end = add(base, scale(unit, union_end));
    // A LINE is an unordered segment: order the pair ascending so the same picks
    // always produce byte-identical geometry (a declared choice).
    let (start, end) = if (start.x, start.y, start.z) <= (end.x, end.y, end.z) {
        (start, end)
    } else {
        (end, start)
    };
    Ok(Joined::Line { start, end })
}

/// The smallest covering span of `[0, 2π)` intervals, or a full turn.
///
/// `spans` are `(start, length)` with `start ∈ [0, 2π)` and `length ∈ (0, 2π)`.
fn covering_span(spans: &[(f64, f64)]) -> (f64, f64) {
    let tol = 1e-12_f64;
    let mut intervals: Vec<(f64, f64)> = Vec::new();
    for (start, length) in spans {
        let start = normalize_positive(*start);
        let length = length.abs();
        let end = start + length;
        if end <= TAU + tol {
            intervals.push((start, end.min(TAU)));
        } else {
            intervals.push((start, TAU));
            intervals.push((0.0, end - TAU));
        }
    }
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in intervals {
        match merged.last_mut() {
            Some(last) if start <= last.1 + tol => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    if merged.len() == 1 && merged[0].0 <= tol && merged[0].1 >= TAU - tol {
        return (0.0, TAU);
    }
    let mut best_gap = -1.0_f64;
    let mut cover_start = 0.0;
    for index in 0..merged.len() {
        let gap_start = merged[index].1;
        let gap_end = if index + 1 < merged.len() {
            merged[index + 1].0
        } else {
            merged[0].0 + TAU
        };
        let gap = gap_end - gap_start;
        if gap > best_gap {
            best_gap = gap;
            cover_start = gap_end % TAU;
        }
    }
    (cover_start, TAU - best_gap.max(0.0))
}

/// Plan an ARC merge: same centre/normal/radius and direction, **gaps allowed**;
/// `close` turns a full turn into a CIRCLE.
fn plan_arcs(arcs: &[ArcEntry], close: bool) -> CadResult<Joined> {
    let tol = join_tolerance();
    let (center, normal, radius, _start, sweep) = arcs[0];
    if !is_finite_point(center) || !is_finite_point(normal) {
        return Err(invalid("join.degenerate", "the first ARC is not finite"));
    }
    let radius = radius.abs();
    if !radius.is_finite() || radius <= tol {
        return Err(invalid("join.degenerate", "the first ARC has no radius"));
    }
    let angle_tol = angle_tolerance(radius);
    if !sweep.is_finite() || sweep.abs() <= angle_tol {
        return Err(invalid("join.degenerate", "the first ARC has no sweep"));
    }
    let sign = if sweep > 0.0 { 1.0 } else { -1.0 };

    let mut spans: Vec<(f64, f64)> = Vec::with_capacity(arcs.len());
    let mut total_sweep = 0.0_f64;
    for (index, (other_center, other_normal, other_radius, other_start, other_sweep)) in
        arcs.iter().enumerate()
    {
        if !is_finite_point(*other_center) || !is_finite_point(*other_normal) {
            return Err(invalid(
                "join.degenerate",
                format!("ARC {index} is not finite"),
            ));
        }
        if distance(center, *other_center) > tol {
            return Err(invalid(
                "join.not_contiguous",
                format!("ARC {index} has a different centre; only concentric ARCs merge"),
            ));
        }
        let normal_dot = dot(normal, *other_normal);
        let cross = Point3 {
            x: normal.y * other_normal.z - normal.z * other_normal.y,
            y: normal.z * other_normal.x - normal.x * other_normal.z,
            z: normal.x * other_normal.y - normal.y * other_normal.x,
        };
        if normal_dot <= 0.0 || length(cross) > (tol / radius).max(1e-12) {
            return Err(invalid(
                "join.not_contiguous",
                format!("ARC {index} has a different normal; only co-planar ARCs merge"),
            ));
        }
        let other_radius = other_radius.abs();
        if !other_radius.is_finite() || (other_radius - radius).abs() > tol {
            return Err(invalid(
                "join.not_contiguous",
                format!("ARC {index} has a different radius; only equal radii merge"),
            ));
        }
        if !other_sweep.is_finite() || other_sweep.abs() <= angle_tol {
            return Err(invalid(
                "join.degenerate",
                format!("ARC {index} has no sweep"),
            ));
        }
        if other_sweep * sign <= 0.0 {
            return Err(invalid(
                "join.not_contiguous",
                format!("ARC {index} runs the other way; JOIN needs one direction"),
            ));
        }
        total_sweep += other_sweep.abs();
        spans.push((normalize_positive(other_start * sign), other_sweep.abs()));
    }
    if total_sweep > TAU + angle_tol {
        return Err(invalid(
            "join.sweep_overflow",
            format!(
                "the summed sweep {total_sweep} exceeds one full turn; JOIN never wraps an ARC"
            ),
        ));
    }
    let (cover_start, total) = covering_span(&spans);
    if close {
        if total >= TAU - angle_tol {
            return Ok(Joined::Circle {
                center,
                normal,
                radius,
            });
        }
        return Err(invalid(
            "join.not_closed",
            "JOIN CLOSE needs the ARCs to cover a full turn; they leave a gap",
        ));
    }
    Ok(Joined::Arc {
        center,
        normal,
        radius,
        start: normalize_signed(sign * cover_start),
        sweep: sign * total.min(TAU),
    })
}

/// Plan an ELLIPSE merge: same frame axes, **gaps allowed**; `close` requires a
/// full turn (the result stays an ELLIPSE, full sweep).
fn plan_ellipses(ellipses: &[EllipseEntry], close: bool) -> CadResult<Joined> {
    let tol = join_tolerance();
    let (center, normal, major_axis, ratio, _start, sweep) = ellipses[0];
    if !is_finite_point(center) || !is_finite_point(normal) || !is_finite_point(major_axis) {
        return Err(invalid(
            "join.degenerate",
            "the first ELLIPSE is not finite",
        ));
    }
    if !ratio.is_finite() || ratio <= 0.0 {
        return Err(invalid(
            "join.degenerate",
            "the first ELLIPSE has no valid ratio",
        ));
    }
    let major_length = length(major_axis);
    if major_length <= tol {
        return Err(invalid(
            "join.degenerate",
            "the first ELLIPSE has no major axis",
        ));
    }
    let sign = if sweep >= 0.0 { 1.0 } else { -1.0 };
    let mut spans: Vec<(f64, f64)> = Vec::with_capacity(ellipses.len());
    let mut total_sweep = 0.0_f64;
    for (index, (other_center, other_normal, other_major, other_ratio, other_start, other_sweep)) in
        ellipses.iter().enumerate()
    {
        if !is_finite_point(*other_center)
            || !is_finite_point(*other_normal)
            || !is_finite_point(*other_major)
        {
            return Err(invalid(
                "join.degenerate",
                format!("ELLIPSE {index} is not finite"),
            ));
        }
        if distance(center, *other_center) > tol {
            return Err(invalid(
                "join.not_contiguous",
                format!("ELLIPSE {index} has a different centre"),
            ));
        }
        if distance(normal, *other_normal) > tol {
            return Err(invalid(
                "join.not_contiguous",
                format!("ELLIPSE {index} has a different plane normal"),
            ));
        }
        if distance(major_axis, *other_major) > tol {
            return Err(invalid(
                "join.not_contiguous",
                format!("ELLIPSE {index} has a different major axis"),
            ));
        }
        if (ratio - other_ratio).abs() > tol {
            return Err(invalid(
                "join.not_contiguous",
                format!("ELLIPSE {index} has a different minor/major ratio"),
            ));
        }
        if !other_sweep.is_finite() || other_sweep.abs() <= 1e-12 {
            return Err(invalid(
                "join.degenerate",
                format!("ELLIPSE {index} has no sweep"),
            ));
        }
        if other_sweep * sign < 0.0 {
            return Err(invalid(
                "join.not_contiguous",
                format!("ELLIPSE {index} runs the other way; JOIN needs one direction"),
            ));
        }
        total_sweep += other_sweep.abs();
        spans.push((normalize_positive(other_start * sign), other_sweep.abs()));
    }
    if total_sweep > TAU + 1e-9 {
        return Err(invalid(
            "join.sweep_overflow",
            format!(
                "the summed sweep {total_sweep} exceeds one full turn; JOIN never wraps an ELLIPSE"
            ),
        ));
    }
    let (cover_start, total) = covering_span(&spans);
    if close && total < TAU - 1e-9 {
        return Err(invalid(
            "join.not_closed",
            "JOIN CLOSE needs the ELLIPSE arcs to cover a full turn; they leave a gap",
        ));
    }
    Ok(Joined::Ellipse {
        center,
        normal,
        major_axis,
        ratio,
        start: normalize_signed(sign * cover_start),
        sweep: sign * total.min(TAU),
    })
}

/// Plan a POLYLINE source merge: LINEs, ARCs and open POLYLINEs chained
/// end-to-end on one +Z plane.
fn plan_polyline_sources(sources: &[DbEntity], close: bool) -> CadResult<Joined> {
    let mut pieces = Vec::with_capacity(sources.len());
    for source in sources {
        pieces.push(piece_from_geometry(&source.geometry, "join")?);
    }
    let chain = chain_pieces(pieces, "join")?;
    let tol = join_tolerance();
    let z_min = chain
        .points
        .iter()
        .map(|point| point.z)
        .fold(f64::INFINITY, f64::min);
    let z_max = chain
        .points
        .iter()
        .map(|point| point.z)
        .fold(f64::NEG_INFINITY, f64::max);
    if z_max - z_min > tol {
        return Err(unsupported(
            "join.noncoplanar",
            "the selected curves do not lie on one plane; a 3D POLYLINE result is outside \
             this build's scope",
        ));
    }
    let mut points = chain.points;
    let mut bulges = chain.bulges;
    let mut closed = false;
    if close {
        let first = points[0];
        let last = *points.last().expect("a chain has points");
        if distance(first, last) > tol {
            return Err(invalid(
                "join.not_closed",
                "JOIN CLOSE needs the polyline ends to meet; they leave a gap",
            ));
        }
        points.pop();
        if !bulges.is_empty() {
            bulges.pop();
        }
        closed = true;
    }
    Ok(Joined::Polyline {
        points,
        bulges,
        closed,
    })
}

/// Plan a SPLINE merge: contiguous end-to-end, any plane, exact C0
/// concatenation for equal degrees.
fn plan_splines(sources: &[DbEntity], close: bool) -> CadResult<Joined> {
    if close {
        return Err(unsupported(
            "join.close_spline",
            "JOIN CLOSE on SPLINEs is not implemented (the SPLINE model has no closed flag)",
        ));
    }
    let mut pieces: Vec<SplineCurve> = Vec::with_capacity(sources.len());
    for source in sources {
        let SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } = &source.geometry
        else {
            return Err(invalid(
                "join.mixed_kinds",
                "a SPLINE source takes SPLINEs only",
            ));
        };
        pieces.push(SplineCurve::from_geometry(
            *degree,
            knots,
            control_points,
            weights,
        )?);
    }
    let tol = join_tolerance();
    let mut current = pieces.remove(0);
    while !pieces.is_empty() {
        let head = current.head();
        let tail = current.tail();
        let mut matched = false;
        for index in 0..pieces.len() {
            let piece = &pieces[index];
            if distance(piece.head(), tail) <= tol {
                let piece = pieces.remove(index);
                current = current.concat(piece)?;
                matched = true;
                break;
            }
            if distance(piece.tail(), tail) <= tol {
                let piece = pieces.remove(index).reversed();
                current = current.concat(piece)?;
                matched = true;
                break;
            }
            if distance(piece.tail(), head) <= tol {
                let piece = pieces.remove(index);
                current = piece.concat(current)?;
                matched = true;
                break;
            }
            if distance(piece.head(), head) <= tol {
                let piece = pieces.remove(index).reversed();
                current = piece.concat(current)?;
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(invalid(
                "join.not_contiguous",
                "a selected SPLINE does not touch the current spline; JOIN does not bridge gaps",
            ));
        }
    }
    Ok(Joined::Spline(current))
}

/// Normalise an angle into `[0, 2π)`.
fn normalize_positive(angle: f64) -> f64 {
    let value = angle % TAU;
    if value < 0.0 {
        value + TAU
    } else {
        value
    }
}
