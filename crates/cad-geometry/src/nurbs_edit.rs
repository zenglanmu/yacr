//! Exact B-spline surgery for sub-curves, plus the point → parameter inverse.
//!
//! TRIM needs two operations on a NURBS curve that no existing module provided:
//! turning a **point** into a parameter, and turning an interval of that
//! parameter space into the **exact** piece of the curve. Neither may degrade the
//! source: a trimmed SPLINE stays a B-spline of the same degree, with the same
//! (rational) weights up to the knot insertion's own arithmetic — never a
//! polyline, never a refit, and never a control point moved to fake an endpoint.
//!
//! # Parameter space
//!
//! Every parameter here is the curve's **raw knot value**
//! (`[knots[degree], knots[n]]`, see [`NurbsCurve::domain`]); it is never
//! normalised to `[0, 1]` and never rescaled. A caller that speaks normalised
//! parameters (the TRIM plan does) converts with the domain returned by
//! [`NurbsCurve::domain`] itself.
//!
//! # Shorthands
//!
//! [`SemanticGeometry::Spline`](cad_domain::SemanticGeometry::Spline) allows two
//! empty vectors, and they mean exactly what the tessellator already implements
//! (`cad_geometry::tessellate_spline`): `knots: []` is the **clamped uniform**
//! knot vector of `clamped_uniform_knots(n, degree)` and `weights: []` is `1.0`
//! per control point. [`expand_spline_source`] expands the same two shorthands and
//! then validates; it never repairs a *non-empty* knot vector or weight list, so a
//! definition that is malformed rather than shorthand is refused by name.
//!
//! # What "exact" means here
//!
//! [`sub_curve`] inserts the two span ends as knots until each has multiplicity
//! `degree + 1` and then truncates the homogeneous control polygon between them.
//! The insertion is Boehm's algorithm applied to the **homogeneous** control
//! points `(w·x, w·y, w·z, w)`, which is what keeps a rational curve exact: the
//! same linear combination acts on the weights, so the projected curve is the
//! same curve. The result is checked against the source at the two ends and the
//! middle ([`SUB_CURVE_SELF_CHECK_RELATIVE`]) before it is returned — a bug in the
//! bookkeeping shows up as an `Invariant` error rather than as a silently
//! different curve.

use cad_domain::{CadError, CadResult, Point3};

use crate::{clamped_uniform_knots, distance, dot, sub, NurbsCurve, TessellationParams};

/// Chord height of the sampling that **seeds** the point inversion, as a fraction
/// of the control polygon's own extent.
///
/// The seed only has to *bracket* the nearest point: the refinement that follows
/// is a bisection on the exact derivative, so this is deliberately coarse
/// (1/1000 of the curve's own size) to keep the seed cheap on a wiggly spline.
/// A too-coarse seed cannot lose the answer by itself, because the bracket is
/// refined down to [`INVERSION_PARAM_RELATIVE_TOL`] and the bracket ends are kept
/// as candidates too.
const INVERSION_SEED_CHORD_FRACTION: f64 = 1e-3;

/// Segment budget of the seeding tessellation. The seed is `O(n)` either way; the
/// budget keeps a pathological spline from producing a huge sample list before
/// the first distance comparison.
const INVERSION_SEED_MAX_SEGMENTS: usize = 512;

/// Minimum seed samples, so a nearly straight spline still brackets a point that
/// lands between two knot breaks.
const INVERSION_SEED_MIN_SEGMENTS: usize = 16;

/// The inversion's bisection stops once its bracket is this fraction of the knot
/// domain wide. The domain is the raw knot window, so the tolerance is relative
/// to it rather than to `[0, 1]`.
const INVERSION_PARAM_RELATIVE_TOL: f64 = 1e-15;

/// Maximum iterations of the inversion's bisection. Halving from a knot span
/// reaches [`INVERSION_PARAM_RELATIVE_TOL`] long before this; the cap is the
/// guard against a degenerate bracket.
const INVERSION_BISECTION_ITERATIONS: usize = 200;

/// Relative bound of the [`sub_curve`] self-check, against the larger of the
/// control polygon's extent and the largest absolute coordinate.
///
/// The check exists to catch **bookkeeping** bugs (a shifted control point index,
/// a wrong multiplicity), whose error is of the order of the curve's own size; it
/// is not a precision claim about the linear combinations, which carry ordinary
/// `f64` rounding at the coordinate magnitude.
const SUB_CURVE_SELF_CHECK_RELATIVE: f64 = 1e-9;

/// A clamped B-spline sub-curve: the exact form a trimmed SPLINE is replaced by.
///
/// `knots.len() == control_points.len() + degree + 1`, the first and last
/// `degree + 1` knots repeat the span ends (so the curve interpolates them), and
/// the parameter domain is the requested `[t0, t1]`.
#[derive(Debug, Clone, PartialEq)]
pub struct SplineSubCurve {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub control_points: Vec<Point3>,
    pub weights: Vec<f64>,
}

/// Diagonal of the axis-aligned box spanned by `points` (0.0 when empty).
///
/// This mirrors `cad_geometry::engine`'s local scale estimate for a SPLINE (its
/// control-point box), so the tolerances derived from it match the scale the
/// sampled intersection fallback uses for the same curve.
pub fn control_polygon_extent(points: &[Point3]) -> f64 {
    let mut min = Point3 {
        x: f64::INFINITY,
        y: f64::INFINITY,
        z: f64::INFINITY,
    };
    let mut max = Point3 {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
        z: f64::NEG_INFINITY,
    };
    for point in points {
        if !(point.x.is_finite() && point.y.is_finite() && point.z.is_finite()) {
            return 0.0;
        }
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }
    if !min.x.is_finite() {
        return 0.0;
    }
    distance(min, max)
}

/// Largest absolute coordinate of `points` (0.0 when empty).
pub fn max_absolute_coordinate(points: &[Point3]) -> f64 {
    points.iter().fold(0.0f64, |acc, p| {
        acc.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
    })
}

/// Expand the stored SPLINE definition into a validated [`NurbsCurve`].
///
/// The two empty-vector shorthands follow `cad_geometry::tessellate_spline`
/// exactly: `knots: []` becomes `clamped_uniform_knots(n, degree)` and
/// `weights: []` becomes `1.0` per control point (the latter is
/// [`NurbsCurve::new`]'s own rule). A **non-empty** vector is passed through
/// untouched, so a wrong-length, non-monotone or non-finite definition is refused
/// by [`NurbsCurve::new`] instead of being uniformised or repaired — the
/// tessellator's fallback does reinterpret such a definition, and doing that here
/// would silently trim a *different* curve than the one stored.
///
/// Refusals (all `InvalidInput`, all named): no control points, `degree < 1`,
/// fewer than `degree + 1` control points, and any failure of the NURBS
/// validation itself.
pub fn expand_spline_source(
    degree: u32,
    knots: &[f64],
    control_points: &[Point3],
    weights: &[f64],
) -> CadResult<NurbsCurve> {
    let count = control_points.len();
    if count == 0 {
        return Err(CadError::InvalidInput(
            "SPLINE definition has no control points".into(),
        ));
    }
    let degree = degree as usize;
    if degree == 0 {
        return Err(CadError::InvalidInput(
            "SPLINE definition has degree 0, which is not a B-spline curve (degree >= 1)".into(),
        ));
    }
    if count < degree + 1 {
        return Err(CadError::InvalidInput(format!(
            "SPLINE definition needs at least degree+1 = {} control points, has {count}",
            degree + 1
        )));
    }
    let knots = if knots.is_empty() {
        clamped_uniform_knots(count, degree)
    } else {
        knots.to_vec()
    };
    NurbsCurve::new(degree, control_points.to_vec(), knots, weights.to_vec()).map_err(|error| {
        CadError::InvalidInput(format!(
            "SPLINE definition is not a valid B-spline curve: {error}"
        ))
    })
}

/// The curve point nearest to `target`, as `(parameter, distance)`.
///
/// The search is seeded by the adaptive sampling
/// ([`NurbsCurve::discretize_params`], the same refinement `discretize` uses, at
/// [`INVERSION_SEED_CHORD_FRACTION`] of the control polygon's extent), the sample
/// nearest to `target` is bracketed by its two neighbours, and the bracket is
/// refined by bisection on `g(t) = (C(t) − P)·C'(t)` — the exact derivative of the
/// squared distance, zero at the nearest point, with `C'` from
/// [`NurbsCurve::tangent`] (rational-safe). `None` only when the curve has no
/// parameter domain or the sampling yields no bracket.
///
/// `None` never means "clamped to the nearest end": a caller that needs a distance
/// bound applies it itself ([`invert_point`]).
pub fn nearest_point_parameter(curve: &NurbsCurve, target: Point3) -> Option<(f64, f64)> {
    if !(target.x.is_finite() && target.y.is_finite() && target.z.is_finite()) {
        return None;
    }
    let (t_min, t_max) = curve.domain();
    if t_max <= t_min {
        return None;
    }
    let extent = control_polygon_extent(curve.control_points());
    let chord = (extent * INVERSION_SEED_CHORD_FRACTION).max(f64::MIN_POSITIVE);
    let params = TessellationParams {
        tolerance: chord,
        max_segments: INVERSION_SEED_MAX_SEGMENTS,
        min_segments: INVERSION_SEED_MIN_SEGMENTS,
    };
    let samples = curve.discretize_params(chord, &params);
    if samples.len() < 2 {
        return None;
    }
    let mut best_index = 0usize;
    let mut best_distance = f64::INFINITY;
    for (index, (_, point)) in samples.iter().enumerate() {
        let candidate = distance(*point, target);
        if candidate < best_distance {
            best_distance = candidate;
            best_index = index;
        }
    }
    let lower = if best_index == 0 {
        t_min
    } else {
        samples[best_index - 1].0
    };
    let upper = if best_index + 1 == samples.len() {
        t_max
    } else {
        samples[best_index + 1].0
    };
    let refined = if upper > lower {
        refine_minimum(curve, target, lower, upper)
    } else {
        samples[best_index].0
    };
    // The bisection can only report an interior stationary point of the bracket;
    // the bracket ends and the seed are kept as candidates so a minimum sitting
    // exactly on a knot is not missed by a rounding step.
    let mut best = (refined, distance(curve.evaluate(refined), target));
    for candidate in [samples[best_index].0, lower, upper, t_min, t_max] {
        let candidate_distance = distance(curve.evaluate(candidate), target);
        if candidate_distance < best.1 {
            best = (candidate, candidate_distance);
        }
    }
    Some(best)
}

/// The parameter of the curve point nearest to `target`, or `None` when that
/// nearest point is farther than `tolerance` world units.
///
/// The bound is how a caller states what "this point is on the curve" means for
/// its own purpose; nothing is clamped or projected onto the ends. The parameter
/// is the raw knot value (see the module doc).
pub fn invert_point(curve: &NurbsCurve, target: Point3, tolerance: f64) -> Option<f64> {
    let (parameter, distance) = nearest_point_parameter(curve, target)?;
    if distance <= tolerance {
        Some(parameter)
    } else {
        None
    }
}

/// The exact `[t0, t1]` piece of `curve` as a **clamped** B-spline sub-curve.
///
/// Both ends are inserted (Boehm, on the homogeneous control points) until each
/// has multiplicity `degree + 1`, and the knot vector and homogeneous control
/// polygon are then truncated between them. The knot vector length relation
/// (`knots == control points + degree + 1`), the two end multiplicities and the
/// curve itself (at `t0`, `t1` and the middle) are re-validated before the result
/// is returned.
///
/// Refusals, all zero-effect:
///
/// * `t0`/`t1` non-finite, outside the curve's own knot domain, or `t1 <= t0`;
/// * a span too short to keep `degree + 1` control points (the two end
///   multiplicity blocks would overlap) — the "kept span is below the parameter
///   tolerance" case, refused by name rather than returning a degenerate curve;
/// * a homogeneous weight that comes out non-finite or non-positive: the rational
///   arithmetic is then not representable, and normalising it (scaling the
///   weights) would silently change the geometry;
/// * a result whose self-check does not reproduce the source.
pub fn sub_curve(curve: &NurbsCurve, t0: f64, t1: f64) -> CadResult<SplineSubCurve> {
    let (t_min, t_max) = curve.domain();
    if !t0.is_finite() || !t1.is_finite() {
        return Err(CadError::InvalidInput(
            "SPLINE sub-curve span must be finite".into(),
        ));
    }
    if t0 < t_min || t1 > t_max {
        return Err(CadError::InvalidInput(format!(
            "SPLINE sub-curve span [{t0}, {t1}] leaves the curve's own knot domain [{t_min}, {t_max}]"
        )));
    }
    if t1 <= t0 {
        return Err(CadError::InvalidInput(format!(
            "SPLINE sub-curve span [{t0}, {t1}] is empty: the kept span is below the parameter \
             tolerance"
        )));
    }

    let degree = curve.degree();
    let mut knots = curve.knots().to_vec();
    let mut homogeneous = homogeneous_control_points(curve);
    for value in [t0, t1] {
        while knot_multiplicity(&knots, value) < degree + 1 {
            insert_knot_once(&mut knots, &mut homogeneous, degree, value)?;
        }
    }

    let first = knots.iter().position(|knot| *knot == t0).ok_or_else(|| {
        CadError::Invariant(format!(
            "SPLINE sub-curve lost its start knot {t0} after insertion"
        ))
    })?;
    let last = knots.iter().rposition(|knot| *knot == t1).ok_or_else(|| {
        CadError::Invariant(format!(
            "SPLINE sub-curve lost its end knot {t1} after insertion"
        ))
    })?;
    // `first + degree` is the last knot of the start block and `last - degree` the
    // first of the end block, so this is exactly "the blocks do not overlap and at
    // least degree + 1 control points remain".
    if last < first + 2 * degree + 1 {
        return Err(CadError::InvalidInput(format!(
            "SPLINE sub-curve span [{t0}, {t1}] is too short: it would keep fewer than \
             degree+1 = {} control points",
            degree + 1
        )));
    }
    let control_count = last - first - degree;
    let sub_knots: Vec<f64> = knots[first..=last].to_vec();
    if sub_knots.len() != control_count + degree + 1 {
        return Err(CadError::Invariant(format!(
            "SPLINE sub-curve knot count {} does not match {control_count} control points at \
             degree {degree}",
            sub_knots.len()
        )));
    }

    // A polynomial source keeps its `w == 1` channel exactly: the insertion only
    // ever computes convex combinations of ones, so re-stating 1.0 removes the
    // last rounding ulp instead of introducing a rational form that is not there.
    let polynomial = curve.weights().iter().all(|weight| *weight == 1.0);
    let mut control_points = Vec::with_capacity(control_count);
    let mut weights = Vec::with_capacity(control_count);
    for point in &homogeneous[first..first + control_count] {
        let weight = if polynomial { 1.0 } else { point[3] };
        if !weight.is_finite() || weight <= 0.0 {
            return Err(CadError::Unsupported(format!(
                "SPLINE sub-curve weight arithmetic is not representable: a homogeneous weight \
                 came out {weight}; refusing instead of normalising the weights"
            )));
        }
        control_points.push(Point3 {
            x: point[0] / weight,
            y: point[1] / weight,
            z: point[2] / weight,
        });
        weights.push(weight);
    }

    let sub = SplineSubCurve {
        degree,
        knots: sub_knots,
        control_points,
        weights,
    };
    validate_sub_curve(curve, &sub, t0, t1)?;
    Ok(sub)
}

/// Re-validate a sub-curve: both ends must be `degree + 1` copies of the span
/// end, the stored form must be a valid NURBS curve, and that curve must agree
/// with the source at `t0`, the middle and `t1`.
fn validate_sub_curve(
    source: &NurbsCurve,
    sub: &SplineSubCurve,
    t0: f64,
    t1: f64,
) -> CadResult<()> {
    let degree = sub.degree;
    if sub.knots[..degree + 1].iter().any(|knot| *knot != t0)
        || sub.knots[sub.knots.len() - degree - 1..]
            .iter()
            .any(|knot| *knot != t1)
    {
        return Err(CadError::Invariant(
            "SPLINE sub-curve is not clamped at both span ends".into(),
        ));
    }
    let rebuilt = NurbsCurve::new(
        sub.degree,
        sub.control_points.clone(),
        sub.knots.clone(),
        sub.weights.clone(),
    )?;
    let extent = control_polygon_extent(source.control_points());
    let magnitude = extent
        .max(max_absolute_coordinate(source.control_points()))
        .max(1.0);
    let tolerance = magnitude * SUB_CURVE_SELF_CHECK_RELATIVE;
    for parameter in [t0, 0.5 * (t0 + t1), t1] {
        let drift = distance(rebuilt.evaluate(parameter), source.evaluate(parameter));
        if !drift.is_finite() || drift > tolerance {
            return Err(CadError::Invariant(format!(
                "SPLINE sub-curve does not reproduce the source at {parameter}: {drift} > \
                 {tolerance}"
            )));
        }
    }
    Ok(())
}

/// The homogeneous control points `(w·x, w·y, w·z, w)` of `curve`.
fn homogeneous_control_points(curve: &NurbsCurve) -> Vec<[f64; 4]> {
    curve
        .control_points()
        .iter()
        .zip(curve.weights())
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

/// How many times `value` occurs in the knot vector.
fn knot_multiplicity(knots: &[f64], value: f64) -> usize {
    knots.iter().filter(|knot| **knot == value).count()
}

/// One Boehm knot insertion of `value`, in place, on the **homogeneous** control
/// points.
///
/// `knots.len() == homogeneous.len() + degree + 1` must hold on entry and on exit.
///
/// The span is the last index whose knot is `<= value`, scanned over the **whole**
/// knot vector (not just the control-point indices). That is what makes an
/// insertion at the curve's own **domain end** (`value == knots[count]`) correct:
/// the new copy belongs immediately after the existing ones, so the span is that
/// last copy — which for a non-clamped end lies at `count` or beyond, not at
/// `count - 1`. Capping the span at `count - 1` instead produces a curve whose
/// end value changes (verified by the non-clamped tests below), because the
/// endpoint of a non-clamped B-spline is not a control point.
///
/// The index bounds hold by construction: `count` is always inside the
/// multiplicity block of `knots[count]` when `value == knots[count]`, so
/// `span - multiplicity <= count - 1` and both `homogeneous[index]` and
/// `knots[index + degree]` stay in range. The check below refuses rather than
/// panicking if that ever stops being true.
fn insert_knot_once(
    knots: &mut Vec<f64>,
    homogeneous: &mut Vec<[f64; 4]>,
    degree: usize,
    value: f64,
) -> CadResult<()> {
    let count = homogeneous.len();
    debug_assert_eq!(knots.len(), count + degree + 1);
    let mut span = degree;
    while span + 1 < knots.len() && knots[span + 1] <= value {
        span += 1;
    }
    let multiplicity = knot_multiplicity(knots, value);
    if span - multiplicity > count - 1 {
        return Err(CadError::Invariant(format!(
            "SPLINE knot insertion of {value} has span {span} with multiplicity {multiplicity} \
             outside the curve's own indexing"
        )));
    }
    let lower = span as isize - degree as isize;
    let upper = span as isize - multiplicity as isize;
    let mut next: Vec<[f64; 4]> = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let position = index as isize;
        if position <= lower {
            next.push(homogeneous[index]);
        } else if position > upper {
            next.push(homogeneous[index - 1]);
        } else {
            let denominator = knots[index + degree] - knots[index];
            let alpha = if denominator != 0.0 {
                (value - knots[index]) / denominator
            } else {
                0.0
            };
            let mut point = [0.0; 4];
            for channel in 0..4 {
                point[channel] = alpha * homogeneous[index][channel]
                    + (1.0 - alpha) * homogeneous[index - 1][channel];
            }
            next.push(point);
        }
    }
    let mut next_knots = Vec::with_capacity(knots.len() + 1);
    next_knots.extend_from_slice(&knots[..=span]);
    next_knots.push(value);
    next_knots.extend_from_slice(&knots[span + 1..]);
    *knots = next_knots;
    *homogeneous = next;
    Ok(())
}

/// The minimum of `|C(t) − target|` inside `[lower, upper]`, by bisection on
/// `g(t) = (C(t) − P)·C'(t)`.
///
/// `g` changes sign at the minimum and only there inside a bracket that contains
/// one turning point, so bisection is unconditionally stable; no second derivative
/// is needed (the rational `C'` already comes from the source weights). When the
/// distance is monotone over the whole bracket the minimum is an end, which is
/// returned directly — the caller keeps those ends as candidates anyway.
fn refine_minimum(curve: &NurbsCurve, target: Point3, lower: f64, upper: f64) -> f64 {
    let (t_min, t_max) = curve.domain();
    let resolution = (t_max - t_min) * INVERSION_PARAM_RELATIVE_TOL;
    let derivative = |parameter: f64| {
        dot(
            sub(curve.evaluate(parameter), target),
            curve.tangent(parameter),
        )
    };
    let mut lower = lower;
    let mut upper = upper;
    let at_lower = derivative(lower);
    let at_upper = derivative(upper);
    if !at_lower.is_finite() || !at_upper.is_finite() {
        return 0.5 * (lower + upper);
    }
    if at_lower >= 0.0 {
        return lower;
    }
    if at_upper <= 0.0 {
        return upper;
    }
    for _ in 0..INVERSION_BISECTION_ITERATIONS {
        let middle = 0.5 * (lower + upper);
        if upper - lower <= resolution || middle <= lower || middle >= upper {
            break;
        }
        let at_middle = derivative(middle);
        if at_middle.is_finite() && at_middle < 0.0 {
            lower = middle;
        } else {
            upper = middle;
        }
    }
    0.5 * (lower + upper)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// The exact rational quadratic quarter circle used by `nurbs`' own tests.
    fn quarter_circle() -> NurbsCurve {
        NurbsCurve::new(
            2,
            vec![p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0],
        )
        .unwrap()
    }

    /// The quadratic Bézier of the drawing fixtures, weights all one.
    fn quadratic() -> NurbsCurve {
        NurbsCurve::new(
            2,
            vec![p(2.0, -2.0), p(5.0, 4.0), p(8.0, -2.0)],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn sub_curve_of_a_bezier_is_the_exact_half_on_the_requested_span() {
        let c = quadratic();
        let sub = sub_curve(&c, 0.5, 1.0).unwrap();
        assert_eq!(sub.degree, 2);
        // degree + 1 end multiplicities, no interior knot.
        assert_eq!(sub.knots, vec![0.5, 0.5, 0.5, 1.0, 1.0, 1.0]);
        assert_eq!(sub.control_points.len(), 3);
        // de Casteljau at 1/2: the right half is (center, mid(P1,P2), P2).
        let center = p(5.0, 1.0);
        let mid = p(6.5, 1.0);
        assert!(distance(sub.control_points[0], center) < 1e-12);
        assert!(distance(sub.control_points[1], mid) < 1e-12);
        assert!(distance(sub.control_points[2], p(8.0, -2.0)) < 1e-12);
        // A polynomial source stays polynomial: every weight is exactly one.
        assert!(sub.weights.iter().all(|w| *w == 1.0));
    }

    #[test]
    fn sub_curve_of_a_rational_circle_stays_on_the_circle() {
        let c = quarter_circle();
        let sub = sub_curve(&c, 0.35, 0.9).unwrap();
        assert_eq!(sub.degree, 2);
        let rebuilt = NurbsCurve::new(2, sub.control_points, sub.knots, sub.weights).unwrap();
        assert!((rebuilt.domain().0 - 0.35).abs() < 1e-15);
        assert!((rebuilt.domain().1 - 0.9).abs() < 1e-15);
        let mut worst = 0.0f64;
        for index in 0..=200 {
            let parameter = 0.35 + 0.55 * index as f64 / 200.0;
            let point = rebuilt.evaluate(parameter);
            let radius = point.x.hypot(point.y);
            worst = worst.max((radius - 1.0).abs());
            assert!(
                (radius - 1.0).abs() < 1e-12,
                "parameter {parameter} left the unit circle by {}",
                (radius - 1.0).abs()
            );
            assert!(point.x >= -1e-12 && point.y >= -1e-12);
        }
        // Measured on this fixture: worst |P| - 1 over 201 parameters is
        // 3.3e-16, i.e. the knot insertion's own rounding; the bound below keeps
        // a 3x margin while still failing for any surgery mistake (which is
        // O(1e-2) or worse on this curve).
        assert!(worst < 1e-12, "worst radius error {worst:e}");
    }

    #[test]
    fn sub_curve_keeps_the_whole_span_when_asked_for_it() {
        let c = quadratic();
        let sub = sub_curve(&c, 0.0, 1.0).unwrap();
        assert_eq!(sub.control_points, c.control_points());
        assert_eq!(sub.knots, c.knots());
        assert!(sub.weights.iter().all(|w| *w == 1.0));
    }

    #[test]
    fn sub_curve_refuses_an_empty_or_inverted_or_outside_span() {
        let c = quadratic();
        assert!(matches!(
            sub_curve(&c, 0.5, 0.5),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            sub_curve(&c, 0.7, 0.2),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            sub_curve(&c, -0.1, 0.5),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            sub_curve(&c, 0.5, 1.1),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            sub_curve(&c, f64::NAN, 1.0),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn sub_curve_restricts_a_non_clamped_curve_to_the_same_geometry() {
        // A degree-2 curve with a **non-clamped** (uniform, open) knot vector: the
        // ends of the domain are not interpolated and the end multiplicities are
        // one, so the surgery has to insert the span ends itself and clamp.
        let c = NurbsCurve::new(
            2,
            vec![
                p(0.0, 0.0),
                p(1.0, 2.0),
                p(3.0, 3.0),
                p(5.0, 2.0),
                p(6.0, 0.0),
            ],
            vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(c.domain(), (2.0, 5.0));
        let sub = sub_curve(&c, 2.5, 4.5).unwrap();
        // Clamped at both ends of the requested span.
        assert_eq!(sub.knots[..3], [2.5, 2.5, 2.5]);
        assert_eq!(sub.knots[sub.knots.len() - 3..], [4.5, 4.5, 4.5]);
        let rebuilt = NurbsCurve::new(2, sub.control_points, sub.knots, sub.weights).unwrap();
        for index in 0..=50 {
            let parameter = 2.5 + 2.0 * index as f64 / 50.0;
            assert!(
                distance(rebuilt.evaluate(parameter), c.evaluate(parameter)) < 1e-12,
                "parameter {parameter} left the source curve"
            );
        }
    }

    #[test]
    fn sub_curve_clamps_the_whole_domain_of_a_non_clamped_curve() {
        let c = NurbsCurve::new(
            2,
            vec![
                p(0.0, 0.0),
                p(1.0, 2.0),
                p(3.0, 3.0),
                p(5.0, 2.0),
                p(6.0, 0.0),
            ],
            vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0],
            Vec::new(),
        )
        .unwrap();
        let (t_min, t_max) = c.domain();
        let sub = sub_curve(&c, t_min, t_max).unwrap();
        let rebuilt = NurbsCurve::new(2, sub.control_points, sub.knots, sub.weights).unwrap();
        assert_eq!(rebuilt.domain(), (t_min, t_max));
        for index in 0..=50 {
            let parameter = t_min + (t_max - t_min) * index as f64 / 50.0;
            assert!(
                distance(rebuilt.evaluate(parameter), c.evaluate(parameter)) < 1e-12,
                "parameter {parameter} left the source curve"
            );
        }
    }

    #[test]
    fn inversion_round_trips_through_the_curve() {
        let c = quadratic();
        for index in 0..=20 {
            let parameter = index as f64 / 20.0;
            let point = c.evaluate(parameter);
            let found = invert_point(&c, point, 1e-9).unwrap();
            assert!(
                (found - parameter).abs() < 1e-9,
                "parameter {parameter} inverted to {found}"
            );
        }
    }

    #[test]
    fn inversion_rejects_a_point_off_the_curve_instead_of_clamping() {
        let c = quadratic();
        // A point one unit above the curve apex: the nearest curve point is
        // farther than 0.5, so the inversion refuses rather than returning an end.
        assert!(invert_point(&c, p(5.0, 5.0), 0.5).is_none());
        // The same point with a tolerance that covers the miss resolves.
        assert!(invert_point(&c, p(5.0, 5.0), 4.5).is_some());
        // Outside the domain entirely: no clamping into the domain.
        assert!(invert_point(&c, p(-100.0, -100.0), 1e-6).is_none());
    }

    #[test]
    fn expand_spline_source_expands_both_shorthands_and_refuses_malformed_input() {
        let points = vec![p(0.0, 0.0), p(1.0, 1.0), p(2.0, 0.0), p(3.0, 1.0)];
        let curve = expand_spline_source(2, &[], &points, &[]).unwrap();
        assert_eq!(curve.degree(), 2);
        assert_eq!(curve.knots(), clamped_uniform_knots(4, 2).as_slice());
        assert!(curve.weights().iter().all(|w| *w == 1.0));
        assert!(curve.domain().1 > curve.domain().0);
        // A non-empty but malformed knot vector is refused, never reinterpreted.
        assert!(matches!(
            expand_spline_source(2, &[0.0, 1.0], &points, &[]),
            Err(CadError::InvalidInput(_))
        ));
        // A non-empty weight list of the wrong length is refused too.
        assert!(matches!(
            expand_spline_source(2, &[], &points, &[1.0]),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            expand_spline_source(0, &[], &points, &[]),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            expand_spline_source(4, &[], &points, &[]),
            Err(CadError::InvalidInput(_))
        ));
        assert!(matches!(
            expand_spline_source(1, &[], &[], &[]),
            Err(CadError::InvalidInput(_))
        ));
    }
}
