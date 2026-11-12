//! The exact rational-Bezier weld behind `JOIN MULTIPLE`'s mixed-type result.
//!
//! `JOIN MULTIPLE` on a selection that mixes a SPLINE with LINEs, ARCs and open
//! POLYLINEs converts **every** participant into an exact NURBS piece and welds
//! them into one clamped `AcDbSpline`:
//!
//! | source | exact representation |
//! | --- | --- |
//! | LINE | degree-1 clamped Bezier (the chord) |
//! | ARC | rational quadratic Bezier spans (`w = cos(Δ/2)`), split into ≤ 90° spans |
//! | ELLIPSE (elliptical arc) | the affine image of the ARC construction (same `w = cos(Δ/2)`, midpoint scaled by the major/minor axes) |
//! | POLYLINE | one degree-1 span per straight segment, one rational quadratic span per bulged segment |
//! | SPLINE | its own clamped B-spline, decomposed into Bezier spans by exact knot insertion |
//!
//! Every span is then elevated (homogeneous rational Bezier elevation, exact) to
//! the selection's common degree — the largest source degree — and the whole
//! chain is assembled as one clamped B-spline whose interior knots have
//! multiplicity `degree + 1` (an exact C0 weld of the spans). The result keeps
//! the arcs' rational weights, so a welded arc is still the same circle to the
//! last bit of the representation.
//!
//! ## Scope
//!
//! * Every source kind JOIN supports (`LINE`, `ARC`, `ELLIPSE` elliptical arc,
//!   `POLYLINE`, `SPLINE`) converts exactly; an ellipse is the affine image of
//!   the circular rational-quadratic construction, so no sampling or refit is
//!   involved.
//! * `JOIN CLOSE` on a mixed weld is refused (`join.close_spline`), matching the
//!   pure-SPLINE mode (the model has no closed spline flag).
//! * Pieces must touch end-to-end (within [`super::join_tolerance`]); JOIN never
//!   bridges a gap. As in the pure-SPLINE mode the result may be 3D — only the
//!   POLYLINE result keeps the `join.noncoplanar` scope boundary.
//!
//! Nothing here writes: the plan is resolved before the single transaction in
//! [`super::Application::join_entities`].

use super::super::*;
use super::plan::Joined;
use super::{angle_tolerance, invalid, join_tolerance, unsupported};
use cad_geometry::{
    add, arbitrary_axis, bulge_arc_geometry, clamped_uniform_knots, cross, is_xy_parallel, length,
    normalize, scale,
};

/// One rational Bezier span: `points.len() == weights.len()`, degree `len - 1`.
#[derive(Debug, Clone)]
struct Span {
    points: Vec<Point3>,
    weights: Vec<f64>,
}

impl Span {
    fn degree(&self) -> usize {
        self.points.len() - 1
    }

    fn head(&self) -> Point3 {
        self.points[0]
    }

    fn tail(&self) -> Point3 {
        *self.points.last().expect("a span has points")
    }

    fn reversed(self) -> Self {
        let mut points = self.points;
        points.reverse();
        let mut weights = self.weights;
        weights.reverse();
        Span { points, weights }
    }

    /// Exact rational Bezier degree elevation to `degree` (which must be at
    /// least this span's own degree).
    fn elevated(&self, degree: usize) -> Span {
        let mut hom = homogeneous(&self.points, &self.weights);
        while hom.len() - 1 < degree {
            hom = elevate_once(&hom);
        }
        dehomogenize(&hom)
    }
}

/// The homogeneous control points `wᵢ·(x, y, z, 1)` of one rational span.
fn homogeneous(points: &[Point3], weights: &[f64]) -> Vec<[f64; 4]> {
    points
        .iter()
        .zip(weights)
        .map(|(point, weight)| {
            [
                point.x * weight,
                point.y * weight,
                point.z * weight,
                *weight,
            ]
        })
        .collect()
}

/// Divide homogeneous control points back into Euclidean points + weights.
fn dehomogenize(hom: &[[f64; 4]]) -> Span {
    let mut points = Vec::with_capacity(hom.len());
    let mut weights = Vec::with_capacity(hom.len());
    for entry in hom {
        let weight = entry[3];
        if weight.abs() <= 1e-300 {
            points.push(Point3 {
                x: entry[0],
                y: entry[1],
                z: entry[2],
            });
            weights.push(1.0);
        } else {
            points.push(Point3 {
                x: entry[0] / weight,
                y: entry[1] / weight,
                z: entry[2] / weight,
            });
            weights.push(weight);
        }
    }
    Span { points, weights }
}

/// One exact rational Bezier degree-elevation step (homogeneous form).
fn elevate_once(hom: &[[f64; 4]]) -> Vec<[f64; 4]> {
    let degree = hom.len() - 1;
    let mut elevated = Vec::with_capacity(hom.len() + 1);
    for index in 0..=degree + 1 {
        let alpha = index as f64 / (degree + 1) as f64;
        let mut entry = [0.0_f64; 4];
        if index > 0 {
            for (slot, value) in entry.iter_mut().zip(hom[index - 1]) {
                *slot += alpha * value;
            }
        }
        if index <= degree {
            for (slot, value) in entry.iter_mut().zip(hom[index]) {
                *slot += (1.0 - alpha) * value;
            }
        }
        elevated.push(entry);
    }
    elevated
}

/// One LINE as a degree-1 span.
fn line_span(start: Point3, end: Point3) -> CadResult<Span> {
    if !is_finite_point(start) || !is_finite_point(end) {
        return Err(invalid(
            "join.degenerate",
            "a LINE has a non-finite endpoint",
        ));
    }
    if distance(start, end) <= join_tolerance() {
        return Err(invalid("join.degenerate", "a LINE has no length to join"));
    }
    Ok(Span {
        points: vec![start, end],
        weights: vec![1.0, 1.0],
    })
}

/// One ARC as exact rational quadratic spans (each at most a quarter turn).
fn arc_spans(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
) -> CadResult<Vec<Span>> {
    let radius = radius.abs();
    if !is_finite_point(center) || !is_finite_point(normal) {
        return Err(invalid("join.degenerate", "an ARC is not finite"));
    }
    if !radius.is_finite() || radius <= join_tolerance() {
        return Err(invalid("join.degenerate", "an ARC has no radius"));
    }
    if !sweep.is_finite() || sweep.abs() <= angle_tolerance(radius) {
        return Err(invalid("join.degenerate", "an ARC has no sweep"));
    }
    if sweep.abs() > std::f64::consts::TAU + angle_tolerance(radius) {
        return Err(unsupported(
            "join.sweep_overflow",
            "an ARC of more than one full turn has no single exact rational span chain",
        ));
    }
    let (ax, ay, _) = arbitrary_axis(normal);
    let parts = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil() as usize;
    let parts = parts.clamp(1, 4);
    let step = sweep / parts as f64;
    let point = |angle: f64| {
        add(
            center,
            add(
                scale(ax, radius * angle.cos()),
                scale(ay, radius * angle.sin()),
            ),
        )
    };
    let mut spans = Vec::with_capacity(parts);
    for index in 0..parts {
        let first = start + step * index as f64;
        let middle = first + step * 0.5;
        let weight = (step * 0.5).cos();
        if weight <= 1e-12 {
            return Err(invalid(
                "join.degenerate",
                "an ARC span has no finite rational weight",
            ));
        }
        // The middle control point is the intersection of the end tangents,
        // `r / cos(Δ/2)` from the centre along the mid-angle.
        let apex = add(
            center,
            add(
                scale(ax, radius / weight * middle.cos()),
                scale(ay, radius / weight * middle.sin()),
            ),
        );
        spans.push(Span {
            points: vec![point(first), apex, point(first + step)],
            weights: vec![1.0, weight, 1.0],
        });
    }
    Ok(spans)
}

/// One ELLIPSE (elliptical arc) as exact rational quadratic spans.
///
/// The ellipse is the affine image `P(a) = c + M·cos a + N·sin a` of the unit
/// circle (`M` = the stored major-radius vector, `N = cross(normal, M)·ratio`).
/// Affine maps commute with the rational Bezier construction, so the circular
/// spans map directly: the same weights `[1, cos(Δ/2), 1]` and the middle
/// control point `c + (M·cos(mid) + N·sin(mid)) / cos(Δ/2)`. The result is the
/// stored ellipse to the last bit — no sampling, no refit.
fn ellipse_spans(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
) -> CadResult<Vec<Span>> {
    if !is_finite_point(center) || !is_finite_point(normal) || !is_finite_point(major_axis) {
        return Err(invalid("join.degenerate", "an ELLIPSE is not finite"));
    }
    let major_length = length(major_axis);
    if !major_length.is_finite() || major_length <= join_tolerance() {
        return Err(invalid("join.degenerate", "an ELLIPSE has no major axis"));
    }
    // No stored ellipse uses a negative ratio (`join_plan` and the representation
    // both reject it); a non-positive ratio has no second axis to span.
    if !ratio.is_finite() || ratio <= 0.0 {
        return Err(invalid(
            "join.degenerate",
            "an ELLIPSE has no valid minor/major ratio",
        ));
    }
    let normal = normalize(normal);
    if length(cross(normal, major_axis)) <= join_tolerance() {
        return Err(invalid(
            "join.degenerate",
            "an ELLIPSE has no second axis (its normal is parallel to the major axis)",
        ));
    }
    if !start.is_finite() {
        return Err(invalid(
            "join.degenerate",
            "an ELLIPSE has a non-finite start angle",
        ));
    }
    let angle_tol = angle_tolerance(major_length);
    if !sweep.is_finite() || sweep.abs() <= angle_tol {
        return Err(invalid("join.degenerate", "an ELLIPSE has no sweep"));
    }
    if sweep.abs() > std::f64::consts::TAU + angle_tol {
        return Err(unsupported(
            "join.sweep_overflow",
            "an ELLIPSE arc of more than one full turn has no single exact rational span chain",
        ));
    }
    let minor_axis = scale(cross(normal, major_axis), ratio);
    let parts = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil() as usize;
    let parts = parts.clamp(1, 4);
    let step = sweep / parts as f64;
    let point = |angle: f64| {
        add(
            center,
            add(
                scale(major_axis, angle.cos()),
                scale(minor_axis, angle.sin()),
            ),
        )
    };
    let mut spans = Vec::with_capacity(parts);
    for index in 0..parts {
        let first = start + step * index as f64;
        let middle = first + step * 0.5;
        let weight = (step * 0.5).cos();
        if weight <= 1e-12 {
            return Err(invalid(
                "join.degenerate",
                "an ELLIPSE span has no finite rational weight",
            ));
        }
        // The middle control point is the affine image of the circular tangent
        // intersection: the axes' mid-angle vector scaled by `1 / cos(Δ/2)`.
        let apex = add(
            center,
            scale(
                add(
                    scale(major_axis, middle.cos()),
                    scale(minor_axis, middle.sin()),
                ),
                1.0 / weight,
            ),
        );
        spans.push(Span {
            points: vec![point(first), apex, point(first + step)],
            weights: vec![1.0, weight, 1.0],
        });
    }
    Ok(spans)
}

/// An open POLYLINE as straight degree-1 spans and exact rational arc spans.
fn polyline_spans(points: &[Point3], bulges: &[f64], closed: bool) -> CadResult<Vec<Span>> {
    if closed {
        return Err(invalid(
            "join.closed_object",
            "a closed polyline cannot be joined into another polyline",
        ));
    }
    if points.len() < 2 {
        return Err(invalid(
            "join.degenerate",
            "a polyline needs at least two vertices",
        ));
    }
    if !points.iter().all(|point| is_finite_point(*point)) {
        return Err(invalid(
            "join.degenerate",
            "a polyline has a non-finite vertex",
        ));
    }
    let segments = points.len() - 1;
    if bulges.len() < segments.saturating_sub(1) {
        return Err(unsupported(
            "join.unsupported",
            format!(
                "a polyline with {} vertices and {} bulges is not addressable",
                points.len(),
                bulges.len()
            ),
        ));
    }
    let bulge_at = |index: usize| bulges.get(index).copied().unwrap_or(0.0);
    if (0..segments).any(|index| bulge_at(index).abs() > 1e-12) && !is_xy_parallel(points) {
        return Err(unsupported(
            "join.noncoplanar",
            "a bulged polyline must lie in the +Z plane; its arcs have no plane otherwise",
        ));
    }
    let plane_normal = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let mut spans = Vec::with_capacity(segments);
    for index in 0..segments {
        let (start, end) = (points[index], points[index + 1]);
        let bulge = bulge_at(index);
        if !bulge.is_finite() {
            return Err(invalid(
                "join.degenerate",
                format!("polyline segment {index} has a non-finite bulge"),
            ));
        }
        if bulge.abs() <= 1e-12 {
            spans.push(line_span(start, end)?);
            continue;
        }
        let Some(SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start: angle,
            sweep,
        }) = bulge_arc_geometry(start, end, bulge, plane_normal)
        else {
            return Err(invalid(
                "join.degenerate",
                format!("polyline segment {index} has a bulge with no arc"),
            ));
        };
        spans.extend(arc_spans(center, normal, radius, angle, sweep)?);
    }
    Ok(spans)
}

/// One SPLINE decomposed into exact Bezier spans (knot insertion, no refit).
fn spline_spans(
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
) -> CadResult<Vec<Span>> {
    let count = control_points.len();
    if count < 2 {
        return Err(invalid(
            "join.degenerate",
            "a SPLINE has too few control points",
        ));
    }
    if !control_points.iter().all(|point| is_finite_point(*point)) {
        return Err(invalid(
            "join.degenerate",
            "a SPLINE has a non-finite control point",
        ));
    }
    let degree = (degree.max(1) as usize).min(count - 1);
    let mut knots = if knots.is_empty() {
        clamped_uniform_knots(count, degree)
    } else {
        knots.to_vec()
    };
    let weights = if weights.is_empty() {
        vec![1.0; count]
    } else {
        weights.to_vec()
    };
    if weights.len() != count {
        return Err(unsupported(
            "join.spline_clamped",
            "a source SPLINE has a weight count that does not match its control points",
        ));
    }
    if knots.len() != count + degree + 1 {
        return Err(unsupported(
            "join.spline_clamped",
            "a source SPLINE has a knot count that does not match its degree",
        ));
    }
    if !knots.iter().all(|knot| knot.is_finite()) {
        return Err(invalid("join.degenerate", "a SPLINE has a non-finite knot"));
    }
    if knots.windows(2).any(|pair| pair[1] < pair[0]) {
        return Err(invalid(
            "join.degenerate",
            "a SPLINE knot vector is not non-decreasing",
        ));
    }
    if weights
        .iter()
        .any(|weight| !weight.is_finite() || *weight <= 0.0)
    {
        return Err(invalid(
            "join.degenerate",
            "a SPLINE has a non-finite or non-positive weight",
        ));
    }
    let first = knots[degree];
    let last = knots[count];
    if last <= first {
        return Err(invalid(
            "join.degenerate",
            "a SPLINE has an empty knot domain",
        ));
    }
    let epsilon = 1e-12;
    if knots[..=degree]
        .iter()
        .any(|knot| (knot - first).abs() > epsilon)
        || knots[count..]
            .iter()
            .any(|knot| (knot - last).abs() > epsilon)
    {
        return Err(unsupported(
            "join.spline_clamped",
            "a source SPLINE is not clamped at both ends; the exact weld needs clamped pieces",
        ));
    }
    // Bezier decomposition: raise every interior knot to multiplicity degree + 1
    // with exact knot insertion (Boehm), which never changes the curve.
    let mut hom = homogeneous(control_points, &weights);
    let mut values: Vec<f64> = Vec::new();
    for knot in &knots {
        if *knot > first && *knot < last && !values.contains(knot) {
            values.push(*knot);
        }
    }
    for value in values {
        loop {
            let multiplicity = knots.iter().filter(|knot| **knot == value).count();
            if multiplicity > degree {
                break;
            }
            insert_knot(&mut knots, &mut hom, degree, value);
        }
    }
    let stride = degree + 1;
    if !hom.len().is_multiple_of(stride) {
        return Err(unsupported(
            "join.spline_clamped",
            "a source SPLINE could not be decomposed into Bezier spans",
        ));
    }
    Ok(hom.chunks(stride).map(dehomogenize).collect())
}

/// Boehm's single knot insertion into a rational B-spline (homogeneous data).
///
/// `value` must already be a knot; one more copy is inserted, so a curve with
/// interior multiplicity `m` gains one control point and stays the same curve.
fn insert_knot(knots: &mut Vec<f64>, hom: &mut Vec<[f64; 4]>, degree: usize, value: f64) {
    let span = knots
        .iter()
        .rposition(|knot| *knot <= value)
        .expect("an inserted knot comes from the vector");
    let multiplicity = knots[..=span]
        .iter()
        .filter(|knot| (**knot - value).abs() <= 1e-12)
        .count();
    let mut inserted = Vec::with_capacity(hom.len() + 1);
    for index in 0..hom.len() + 1 {
        if index + degree <= span {
            inserted.push(hom[index]);
        } else if index >= span + 1 - multiplicity {
            inserted.push(hom[index - 1]);
        } else {
            let denominator = knots[index + degree] - knots[index];
            let alpha = if denominator.abs() > 1e-300 {
                (value - knots[index]) / denominator
            } else {
                0.0
            };
            let mut entry = [0.0_f64; 4];
            for slot in 0..4 {
                entry[slot] = alpha * hom[index][slot] + (1.0 - alpha) * hom[index - 1][slot];
            }
            inserted.push(entry);
        }
    }
    knots.insert(span + 1, value);
    *hom = inserted;
}

/// Chain the per-source span lists end-to-end (either direction), tolerance
/// `join_tolerance`, never bridging a gap.
fn chain_spans(mut chains: Vec<Vec<Span>>) -> CadResult<Vec<Span>> {
    let tol = join_tolerance();
    let mut current = chains.remove(0);
    while !chains.is_empty() {
        let head = current[0].head();
        let tail = current.last().expect("a chain has spans").tail();
        let mut matched = false;
        for index in 0..chains.len() {
            let chain = &chains[index];
            let chain_head = chain[0].head();
            let chain_tail = chain.last().expect("a chain has spans").tail();
            if distance(chain_head, tail) <= tol {
                current.extend(chains.remove(index));
                matched = true;
                break;
            }
            if distance(chain_tail, tail) <= tol {
                let chain = reversed_chain(chains.remove(index));
                current.extend(chain);
                matched = true;
                break;
            }
            if distance(chain_tail, head) <= tol {
                let mut chain = chains.remove(index);
                chain.extend(current);
                current = chain;
                matched = true;
                break;
            }
            if distance(chain_head, head) <= tol {
                let mut chain = reversed_chain(chains.remove(index));
                chain.extend(current);
                current = chain;
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(invalid(
                "join.not_contiguous",
                "a selected object does not touch the rest of the mixed weld; JOIN does not \
                 bridge gaps",
            ));
        }
    }
    Ok(current)
}

/// Reverse a span chain: the span order and every span's own direction.
fn reversed_chain(mut chain: Vec<Span>) -> Vec<Span> {
    chain.reverse();
    chain.into_iter().map(Span::reversed).collect()
}

/// Assemble the elevated spans into one clamped B-spline.
///
/// Interior junctions get knot multiplicity `degree + 1` and keep the shared
/// endpoint control point on both sides, so the welded curve is exactly the
/// concatenation of its spans (C0 at the junctions, which is what a weld is).
fn assemble(spans: &[Span], degree: usize) -> SemanticGeometry {
    let mut control_points = Vec::new();
    let mut weights = Vec::new();
    for span in spans {
        control_points.extend(span.points.iter().copied());
        weights.extend(span.weights.iter().copied());
    }
    let count = spans.len();
    let mut knots = Vec::with_capacity(control_points.len() + degree + 1);
    knots.extend(std::iter::repeat_n(0.0, degree + 1));
    for junction in 1..count {
        knots.extend(std::iter::repeat_n(junction as f64, degree + 1));
    }
    knots.extend(std::iter::repeat_n(count as f64, degree + 1));
    SemanticGeometry::Spline {
        degree: degree as u32,
        knots,
        control_points,
        weights,
    }
}

/// Resolve the mixed-type `JOIN MULTIPLE` selection into one exact SPLINE.
///
/// Called from [`super::plan`]'s MULTIPLE classification when the selection
/// contains a SPLINE or an ELLIPSE (the all-polyline-family mixes keep their
/// POLYLINE result).
pub(super) fn plan_mixed_weld(sources: &[DbEntity], close: bool) -> CadResult<Joined> {
    if close {
        return Err(unsupported(
            "join.close_spline",
            "JOIN CLOSE on a mixed SPLINE weld is not implemented (the SPLINE model has no \
             closed flag)",
        ));
    }
    let mut chains = Vec::with_capacity(sources.len());
    for source in sources {
        let spans = match &source.geometry {
            SemanticGeometry::Line { start, end } => vec![line_span(*start, *end)?],
            SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => arc_spans(*center, *normal, *radius, *start, *sweep)?,
            SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => ellipse_spans(*center, *normal, *major_axis, *ratio, *start, *sweep)?,
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
                ..
            } => polyline_spans(points, bulges, *closed)?,
            SemanticGeometry::Spline {
                degree,
                knots,
                control_points,
                weights,
            } => spline_spans(*degree, knots, control_points, weights)?,
            other => {
                return Err(unsupported(
                    "join.multiple_spline_mix",
                    format!(
                        "{} cannot be welded into an exact SPLINE",
                        class_name(other)
                    ),
                ));
            }
        };
        if spans.is_empty() {
            return Err(invalid(
                "join.degenerate",
                format!("entity {} has no curve to weld", source.id.0),
            ));
        }
        chains.push(spans);
    }
    let chain = chain_spans(chains)?;
    let degree = chain.iter().map(Span::degree).max().unwrap_or(1).max(1);
    let elevated: Vec<Span> = chain.iter().map(|span| span.elevated(degree)).collect();
    Ok(Joined::Raw(assemble(&elevated, degree)))
}
