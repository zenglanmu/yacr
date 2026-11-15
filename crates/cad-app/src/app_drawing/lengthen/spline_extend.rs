//! `LENGTHEN` an open SPLINE **longer**: one declared continuation policy.
//!
//! A NURBS curve has no natural continuation, so growing one is not derived from
//! the curve's own shape. The policy is explicit and geometric, and it is the
//! same one EXTEND uses for a SPLINE target (`super::super::extend_spline`):
//!
//! 1. The end nearest the pick (or, without a pick, the historical far end) is
//!    the one that moves; every other stored control point, weight and knot stays
//!    where it is.
//! 2. The curve is continued **along its own end tangent** — the unit first
//!    derivative at that end ([`NurbsCurve::tangent`], rational-safe) — as a
//!    **straight line**. The requested growth `d = new − old` is the tail's
//!    length; there is no curvature continuation and no re-fit.
//! 3. The result is the source definition **plus one extra clamped knot span**,
//!    stored as `degree + 1` appended control points `E, E + d/p·T, …, E + d·T`
//!    (`E` = the old end point, `T` = the unit end tangent, all appended weights
//!    `1.0`) and `degree + 1` copies of the new end knot.
//!
//! Growing the **start** is the exact mirror of growing the end: the curve is
//! mirrored (reversed control points/weights and reflected knots), the tail is
//! appended at the new end and the result is mirrored back, so the stored control
//! points reappear verbatim.
//!
//! # What the result is, precisely
//!
//! * The source curve is **unchanged on its own knot domain**: the stored control
//!   points, weights and interior knots are a prefix of the new definition and the
//!   joint knot keeps multiplicity `degree + 1`, so every point of the old span is
//!   reproduced exactly. Nothing is re-interpolated.
//! * The appended span is **exactly** the segment `[E, E + d·T]`: a Bezier span
//!   whose control polygon is collinear and evenly spaced is the straight segment
//!   itself, so the new end point is exactly `E + d·T` and the tail is exactly `d`
//!   long (this is the one length in the operation that is not a sample).
//! * The joint is **C0 and G1** (the tail leaves along the source's own end
//!   tangent) but **not G2**: the tail's curvature is zero, so the extension does
//!   not continue the spline's curvature and does not "look like" the original
//!   curve. That is the policy, not an approximation of a smoother tail.
//! * `degree` and the source weights are preserved; the tail's own weights are
//!   `1.0` (a straight span is weight-invariant).
//!
//! The measured "old" length LENGTHEN reports for a SPLINE is a sampled polyline
//! estimate (`super::super::trim_spline`: chord height `extent × 1e-3`, at most
//! 256 segments), so the requested growth is exact while the resulting **total**
//! length is `old + d` only to that estimate's sampling error. The stored tail is
//! never shorter or longer than `d`.
//!
//! # Refusals (all write nothing)
//!
//! * `lengthen.closed_spline` — first control point == last (the domain model has
//!   no closed flag, so a closed *frame* is the only signal; a closed curve has no
//!   end to move, like a CIRCLE);
//! * `lengthen.spline_unclamped` — the moving end is not clamped (multiplicity
//!   `degree + 1`), so appending a span would change the existing curve instead of
//!   adding a tail; the definition is refused rather than silently re-clamped;
//! * `lengthen.spline_degenerate_tangent` — an end derivative that is
//!   zero/non-finite (there is no direction to continue along);
//! * `lengthen.non_positive` — the requested growth is not a positive finite
//!   value;
//! * `lengthen.spline_extension_failed` — the appended definition would not be a
//!   finite/valid B-spline (f64 sanity; there is **no** arbitrary length cap).

use super::super::trim_curves::cut_tolerance;
use super::refusal;
use cad_domain::{CadResult, Point3, SemanticGeometry};
use cad_geometry::{add, distance, length, scale, NurbsCurve};

/// Whether the stored SPLINE is a closed **frame** (`first == last` control
/// point).
///
/// `SemanticGeometry::Spline` carries no closed flag (`super::super::trim_spline`),
/// so this is the only signal a LENGTHEN can read; a closed curve has no end to
/// move, so it is refused by name instead of being treated as an open curve whose
/// end happens to sit on its start.
pub(super) fn is_closed_frame(curve: &NurbsCurve, tol: f64) -> bool {
    let points = curve.control_points();
    points.len() >= 2 && distance(points[0], *points.last().expect("non-empty")) <= tol
}

/// The outward unit tangent at the curve's start or end: `−C'(t_min)` for the
/// start, `+C'(t_max)` for the end (the direction the curve would grow).
pub(super) fn outward_tangent(curve: &NurbsCurve, at_start: bool, tol: f64) -> CadResult<Point3> {
    let (t_min, t_max) = curve.domain();
    let raw = curve.tangent(if at_start { t_min } else { t_max });
    let tangent_len = length(raw);
    if !raw.x.is_finite()
        || !raw.y.is_finite()
        || !raw.z.is_finite()
        || !tangent_len.is_finite()
        || tangent_len <= tol
    {
        return Err(refusal(
            "lengthen.spline_degenerate_tangent",
            format!(
                "the moving end's tangent is degenerate (|C'| = {tangent_len:.9}); there is no \
                 direction to continue along"
            ),
        ));
    }
    let unit = scale(raw, 1.0 / tangent_len);
    Ok(if at_start { scale(unit, -1.0) } else { unit })
}

/// Grow the SPLINE by `tail` along the moving end's tangent (see the module
/// docs).
pub(super) fn extend_spline(
    curve: &NurbsCurve,
    move_start: bool,
    tail: f64,
) -> CadResult<SemanticGeometry> {
    let tol = cut_tolerance();
    if !tail.is_finite() || tail <= tol {
        return Err(refusal(
            "lengthen.non_positive",
            format!("the SPLINE extension length {tail} is not a positive finite value"),
        ));
    }
    // Growing the start is the mirror of growing the end.
    let working = if move_start {
        reverse_parts(curve)?
    } else {
        curve.clone()
    };
    let (_, t_max) = working.domain();
    let end = working.evaluate(t_max);
    let tangent = outward_tangent(&working, false, tol)?;
    let extended = append_tangent(&working, end, tangent, tail)?;
    let result = if move_start {
        reverse_parts(&extended)?
    } else {
        extended
    };
    Ok(SemanticGeometry::Spline {
        degree: result.degree() as u32,
        knots: result.knots().to_vec(),
        control_points: result.control_points().to_vec(),
        weights: result.weights().to_vec(),
    })
}

/// Append the straight tangent continuation as one extra clamped knot span.
///
/// `E, E + d/p·T, …, E + d·T` is a collinear (`p + 1` point) control polygon, so
/// the appended span is exactly the segment `[E, E + d·T]`; the trailing knot
/// gets multiplicity `degree + 1` so the joint is a proper clamped span boundary
/// (the source is unchanged, the joint is C0 and the tail's start derivative is
/// along `T`).
fn append_tangent(
    curve: &NurbsCurve,
    end: Point3,
    tangent: Point3,
    tail: f64,
) -> CadResult<NurbsCurve> {
    let degree = curve.degree();
    let knots = curve.knots();
    let count = curve.control_points().len();
    let (_, t_max) = curve.domain();
    // Refuse rather than reshape: appending a span only adds a tail if the moving
    // end is clamped (multiplicity degree + 1).
    let clamp_slack = t_max.abs().max(1.0) * 1e-12;
    if knots[count..]
        .iter()
        .any(|knot| (knot - t_max).abs() > clamp_slack)
    {
        return Err(refusal(
            "lengthen.spline_unclamped",
            format!(
                "the moving end of the SPLINE target is not clamped (multiplicity {}), so \
                 appending a knot span would change the existing curve instead of adding a \
                 tangent tail",
                degree + 1
            ),
        ));
    }
    let t_new = t_max + extension_span(knots, count, degree, t_max);
    if !t_new.is_finite() || t_new <= t_max {
        return Err(refusal(
            "lengthen.spline_extension_failed",
            "the appended knot span is not representable",
        ));
    }

    let mut control_points = curve.control_points().to_vec();
    let mut weights = curve.weights().to_vec();
    for step in 0..=degree {
        let fraction = step as f64 / degree as f64;
        control_points.push(add(end, scale(tangent, tail * fraction)));
        weights.push(1.0);
    }
    let mut new_knots = knots.to_vec();
    new_knots.extend(std::iter::repeat_n(t_new, degree + 1));
    NurbsCurve::new(degree, control_points, new_knots, weights).map_err(|error| {
        refusal(
            "lengthen.spline_extension_failed",
            format!("the extended spline is not a valid B-spline curve: {error}"),
        )
    })
}

/// The parameter width of the appended span: the last non-degenerate knot span of
/// the source, so the tail's parameterisation is proportional to the source's own.
/// The shape is independent of this value (the appended control polygon is
/// collinear); it only keeps the knot vector well-scaled.
fn extension_span(knots: &[f64], count: usize, degree: usize, t_max: f64) -> f64 {
    let previous = knots[..count]
        .iter()
        .rev()
        .copied()
        .find(|knot| *knot < t_max);
    match previous {
        Some(previous) => t_max - previous,
        // One degenerate span only: fall back to the whole domain, and to `1.0`
        // for an all-zero width (`NurbsCurve::new` has already refused that case).
        None => (t_max - knots[degree]).abs().max(1.0),
    }
}

/// The exact knot mirror of `curve`: reversed control points/weights and
/// `(U₀ + U_last) − U` reflected knots — the same formula `join_spline` and
/// EXTEND use, so both ends of the curve map to the same geometry with the
/// direction flipped.
fn reverse_parts(curve: &NurbsCurve) -> CadResult<NurbsCurve> {
    let degree = curve.degree();
    let mut points = curve.control_points().to_vec();
    let mut weights = curve.weights().to_vec();
    points.reverse();
    weights.reverse();
    let knots = curve.knots();
    let sum = knots[0] + knots[knots.len() - 1];
    let knots = knots.iter().rev().map(|knot| sum - knot).collect();
    NurbsCurve::new(degree, points, knots, weights).map_err(|error| {
        refusal(
            "lengthen.spline_extension_failed",
            format!("the target spline could not be mirrored: {error}"),
        )
    })
}
