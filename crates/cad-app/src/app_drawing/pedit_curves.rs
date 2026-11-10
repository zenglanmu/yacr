//! The pure `PEDIT` curve helpers (split out of `pedit.rs` for the code-size
//! limit).
//!
//! Everything here is a planner: it reads a [`SemanticGeometry`] and returns the
//! geometry an option would write, or a named refusal. Nothing writes, and the
//! module is not a second option surface — `pedit.rs` owns the selection rules,
//! the transaction and the undo record.

use super::super::helpers::normalize_signed;
use super::super::join::join_tolerance;
use super::super::*;
use super::{invalid, unsupported};
use cad_geometry::{
    add, arbitrary_axis, arc_sweep_to_bulge, clamped_uniform_knots, is_xy_parallel, length, scale,
    sub, tessellate_spline, TessellationParams,
};

/// The point at `angle` on a circle/arc in its own plane.
pub(super) fn arc_point(center: Point3, normal: Point3, radius: f64, angle: f64) -> Point3 {
    let (ax, ay, _) = arbitrary_axis(normal);
    add(
        center,
        add(
            scale(ax, radius * angle.cos()),
            scale(ay, radius * angle.sin()),
        ),
    )
}

/// Convert one LINE/ARC/SPLINE to a polyline (the `PEDITACCEPT` answer `true`).
pub(super) fn convert_to_polyline(geometry: &SemanticGeometry) -> CadResult<SemanticGeometry> {
    let tol = join_tolerance();
    match geometry {
        SemanticGeometry::Line { start, end } => {
            if distance(*start, *end) <= tol {
                return Err(invalid("convert_degenerate", "the LINE has no length"));
            }
            Ok(SemanticGeometry::Polyline {
                points: vec![*start, *end],
                bulges: vec![0.0],
                closed: false,
            })
        }
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let radius = radius.abs();
            if !is_finite_point(*center) || !is_finite_point(*normal) {
                return Err(invalid("convert_degenerate", "the ARC is not finite"));
            }
            if radius <= tol || !sweep.is_finite() || sweep.abs() <= 1e-12 {
                return Err(invalid(
                    "convert_degenerate",
                    "the ARC has no radius or sweep",
                ));
            }
            let Some(bulge) = arc_sweep_to_bulge(*sweep) else {
                return Err(unsupported(
                    "convert_sweep",
                    "an ARC of one full turn or more cannot be one polyline bulge segment",
                ));
            };
            if geometry_normal(*normal).z < 1.0 - 1e-9 {
                return Err(unsupported(
                    "convert_plane",
                    "an ARC outside the +Z plane cannot become a bulged polyline",
                ));
            }
            Ok(SemanticGeometry::Polyline {
                points: vec![
                    arc_point(*center, *normal, radius, *start),
                    arc_point(*center, *normal, radius, start + sweep),
                ],
                bulges: vec![bulge],
                closed: false,
            })
        }
        SemanticGeometry::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            if control_points.len() < 2 {
                return Err(invalid(
                    "convert_degenerate",
                    "the SPLINE has fewer than two control points",
                ));
            }
            let extent = control_extent(control_points).max(1e-9);
            let samples = tessellate_spline(
                control_points,
                knots,
                weights,
                *degree,
                TessellationParams {
                    tolerance: (extent * 1e-4).max(1e-9),
                    max_segments: 4096,
                    min_segments: 8,
                },
            );
            if samples.len() < 2 {
                return Err(invalid(
                    "convert_degenerate",
                    "the SPLINE produced fewer than two polyline vertices",
                ));
            }
            let segments = samples.len() - 1;
            Ok(SemanticGeometry::Polyline {
                points: samples,
                bulges: vec![0.0; segments],
                closed: false,
            })
        }
        other => Err(unsupported(
            "not_polyline",
            format!(
                "PEDIT needs a POLYLINE selection; {} cannot be converted",
                class_name(other)
            ),
        )),
    }
}

/// `normal` normalised, with a documented +Z fallback for a zero vector.
pub(super) fn geometry_normal(normal: Point3) -> Point3 {
    let length = length(normal);
    if length <= 1e-12 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        scale(normal, 1.0 / length)
    }
}

/// The largest coordinate extent of a control polygon.
pub(super) fn control_extent(points: &[Point3]) -> f64 {
    let mut min = points[0];
    let mut max = points[0];
    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    (max.x - min.x).max(max.y - min.y).max(max.z - min.z)
}

/// Build the tangent-continuous fit bulges (one arc per original segment).
pub(super) fn fit_bulges(points: &[Point3]) -> CadResult<Vec<f64>> {
    let n = points.len();
    if n < 2 {
        return Err(invalid(
            "too_few_vertices",
            "FIT needs at least two vertices",
        ));
    }
    if !is_xy_parallel(points) {
        return Err(unsupported(
            "fit_plane",
            "FIT needs a polyline in the +Z plane to resolve its arc segments",
        ));
    }
    let tol = join_tolerance();
    let mut bulges = Vec::with_capacity(n - 1);
    let mut delta = 0.0_f64;
    let mut previous: Option<f64> = None;
    for index in 0..n - 1 {
        let chord = sub(points[index + 1], points[index]);
        if length(chord) <= tol {
            return Err(invalid(
                "fit_degenerate",
                format!("segment {index} has no length to fit an arc through"),
            ));
        }
        let alpha = chord.y.atan2(chord.x);
        if let Some(previous) = previous {
            // end tangent (previous) = start tangent (this segment)
            delta = normalize_signed(alpha - previous - delta);
        }
        if delta.abs() >= std::f64::consts::FRAC_PI_2 - 1e-9 {
            return Err(unsupported(
                "fit_tangent",
                format!(
                    "segment {index} needs a half-turn-or-more arc to stay tangent-continuous; \
                     one bulge segment cannot represent it"
                ),
            ));
        }
        bulges.push((delta * 0.5).tan());
        previous = Some(alpha);
    }
    Ok(bulges)
}

/// Build the tangent-continuous fit bulges of a **closed** ring (one arc per
/// segment, including the closing one).
///
/// The cyclic tangent system is `δᵢ + δᵢ₊₁ = turnᵢ` for every vertex, with the
/// turns taken from a consistent unwrapping of the chords. For an odd vertex
/// count the system has exactly one solution; for an even count it is solvable
/// only when the alternating turn residual vanishes, and then one parameter is
/// free — it is fixed by minimising the largest `|δ|` (the most even arcs).
pub(super) fn fit_bulges_closed(points: &[Point3]) -> CadResult<Vec<f64>> {
    let n = points.len();
    if n < 3 {
        return Err(invalid(
            "too_few_vertices",
            format!("FIT on a closed polyline needs at least three vertices, got {n}"),
        ));
    }
    if !is_xy_parallel(points) {
        return Err(unsupported(
            "fit_plane",
            "FIT needs a polyline in the +Z plane to resolve its arc segments",
        ));
    }
    let tol = join_tolerance();
    let mut alpha = Vec::with_capacity(n);
    for index in 0..n {
        let chord = sub(points[(index + 1) % n], points[index]);
        if length(chord) <= tol {
            return Err(invalid(
                "fit_degenerate",
                format!("segment {index} has no length to fit an arc through"),
            ));
        }
        alpha.push(chord.y.atan2(chord.x));
    }
    // `turnᵢ = αᵢ₊₁ − αᵢ` (indices modulo n). Folding the residual to the
    // nearest full turn into the closing turn makes the loop close exactly
    // instead of leaving a rounding-sized tangent kink at the first vertex.
    let mut turns: Vec<f64> = (0..n)
        .map(|index| normalize_signed(alpha[(index + 1) % n] - alpha[index]))
        .collect();
    let total: f64 = turns.iter().sum();
    let winding = (total / std::f64::consts::TAU).round();
    turns[n - 1] -= total - winding * std::f64::consts::TAU;
    // δₖ = aₖ + (−1)ᵏ t: a₀ = 0, aₖ = turnₖ₋₁ − aₖ₋₁.
    let mut a = vec![0.0_f64; n];
    for index in 1..n {
        a[index] = turns[index - 1] - a[index - 1];
    }
    let t = if n % 2 == 1 {
        // The closing equation `δₙ₋₁ + δ₀ = turnₙ₋₁` pins `t` for an odd ring.
        (turns[n - 1] - a[n - 1]) * 0.5
    } else {
        if normalize_signed(a[n - 1] - turns[n - 1]).abs() > 1e-9 {
            return Err(unsupported(
                "fit_closure",
                "an even-segment closed ring carries a tangent-continuous single-arc-per-segment \
                 fit only when its alternating turn residual vanishes; this ring cannot close",
            ));
        }
        minimax_turn(&a)
    };
    let mut bulges = Vec::with_capacity(n);
    for (index, a_turn) in a.iter().enumerate() {
        let sign = if index.is_multiple_of(2) { 1.0 } else { -1.0 };
        let delta = a_turn + sign * t;
        if delta.abs() >= std::f64::consts::FRAC_PI_2 - 1e-9 {
            return Err(unsupported(
                "fit_tangent",
                format!(
                    "segment {index} needs a half-turn-or-more arc to stay tangent-continuous; \
                     one bulge segment cannot represent it"
                ),
            ));
        }
        bulges.push((delta * 0.5).tan());
    }
    Ok(bulges)
}

/// The free parameter of an even-segment closed fit: minimise the largest
/// segment turn `max |aₖ + (−1)ᵏ t|`.
///
/// That is a convex piecewise-linear function of `t`, so its minimum is at a
/// kink: a segment turn reaching zero or two segment turns crossing. Every such
/// candidate is evaluated and the smallest `|t|` among the exact minima wins, so
/// the same ring always yields the same fit.
fn minimax_turn(a: &[f64]) -> f64 {
    let n = a.len();
    let sign = |index: usize| if index.is_multiple_of(2) { 1.0 } else { -1.0 };
    let worst = |t: f64| {
        (0..n)
            .map(|index| (a[index] + sign(index) * t).abs())
            .fold(0.0, f64::max)
    };
    let mut candidates: Vec<f64> = Vec::with_capacity(n * (2 * n + 1));
    for (index, turn) in a.iter().enumerate() {
        // A kink where this segment's turn is zero.
        candidates.push(-turn * sign(index));
        for other in 0..n {
            for sense in [-1.0_f64, 1.0] {
                // A kink where `turn(index) == sense * turn(other)`.
                let denominator = sign(index) - sense * sign(other);
                if denominator.abs() > 1e-12 {
                    candidates.push((sense * a[other] - a[index]) / denominator);
                }
            }
        }
    }
    let mut best_t = 0.0_f64;
    let mut best = worst(0.0);
    for t in candidates {
        let value = worst(t);
        if value < best - 1e-12 || ((value - best).abs() <= 1e-12 && t.abs() < best_t.abs()) {
            best = value;
            best_t = t;
        }
    }
    best_t
}

/// Reverse a spline: reversed control frame and mirrored knot vector.
pub(super) fn reverse_spline(
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
) -> SemanticGeometry {
    let n = control_points.len();
    let mut points = control_points.to_vec();
    points.reverse();
    let mut weights = weights.to_vec();
    weights.reverse();
    let knots = if knots.is_empty() {
        clamped_uniform_knots(n, degree as usize)
    } else {
        let sum = knots[0] + knots[knots.len() - 1];
        knots.iter().rev().map(|knot| sum - knot).collect()
    };
    SemanticGeometry::Spline {
        degree,
        knots,
        control_points: points,
        weights,
    }
}

/// The polyline a `PEDIT`-written SPLINE restores to, or `None` when the spline
/// is not exactly one of this module's `SPLINE` forms.
///
/// Returns `(points, bulges, closed)`. The closed form is checked first: its
/// control frame repeats the first vertex, so a unit-weight clamped-uniform
/// frame that ends where it starts is read as the closed ring (the SPLINE model
/// has no periodic flag — a declared ambiguity, documented in the module docs).
pub(super) fn pedit_spline_restore(
    degree: &u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
) -> Option<(Vec<Point3>, Vec<f64>, bool)> {
    let n = control_points.len();
    if n < 2 || weights.len() != n || weights.iter().any(|weight| (*weight - 1.0).abs() > 1e-12) {
        return None;
    }
    let unit_clamped = |expected_degree: u32, count: usize| {
        *degree == expected_degree
            && knots == clamped_uniform_knots(count, expected_degree as usize).as_slice()
    };
    let distinct = n - 1;
    let closed_degree = 3.min(distinct.saturating_sub(1)) as u32;
    if distinct >= 3
        && distance(control_points[0], control_points[n - 1]) <= join_tolerance()
        && unit_clamped(closed_degree, n)
    {
        return Some((
            control_points[..distinct].to_vec(),
            vec![0.0; distinct],
            true,
        ));
    }
    if unit_clamped(3.min(n - 1) as u32, n) {
        return Some((
            control_points.to_vec(),
            vec![0.0; n.saturating_sub(1)],
            false,
        ));
    }
    None
}
