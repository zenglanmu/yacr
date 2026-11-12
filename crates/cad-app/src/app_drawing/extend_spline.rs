//! SPLINE as an **EXTEND target**: one declared continuation rule.
//!
//! A NURBS curve has no natural continuation, so EXTEND does not pretend to
//! extrapolate the spline's own shape. The rule is explicit and geometric:
//!
//! 1. The pick is **projected** onto the spline and that projection selects the
//!    nearer end (the same `u_pick > 0.5` rule a LINE/ARC uses). Only that end
//!    moves; the other end and every stored control point stay where they are.
//! 2. The curve is extended **along its end tangent** — the unit first derivative
//!    at that end (`NurbsCurve::tangent`, rational-safe) — as a **straight-line
//!    continuation**. The end-tangent ray is intersected with the same
//!    [`BoundarySet`] primitives TRIM/EXTEND already use, and the nearest crossing
//!    strictly **ahead** of the end wins. Every crossing is verified to lie on the
//!    ray itself, so a boundary that merely meets the ray's infinite line, or one
//!    that is not coplanar with the extension (skew in 3D), contributes nothing.
//! 3. The result is the original spline plus **one extra clamped knot span** whose
//!    control polygon is collinear with the tangent: the stored control points and
//!    weights come first, then `p + 1` appended points `E, E + d/p·T, …, E + d·T`
//!    (`E` = the old end, `T` = the unit tangent, `d` = the crossing distance) and
//!    `p + 1` appended copies of the new end knot. This is the same C0
//!    concatenation rule `join_spline` uses (joint multiplicity `degree + 1`), so
//!    the source curve is **unchanged** on its own knot domain, the joint is C0 and
//!    the appended span is G1 (its start derivative is along `T`, the source's own
//!    end tangent). The added segment is a straight tangent extension, **not** a
//!    re-fit, and `degree`/`weights` are preserved.
//!
//! Growing the **start** is the exact mirror of growing the end: the curve is
//! mirrored (reversed control points/weights and reflected knots), the tail is
//! appended at the new end, and the result is mirrored back — so the stored control
//! points reappear verbatim and the same rule applies to both ends.
//!
//! The result is therefore "the old curve, then a straight tangent tail", stated
//! as such in the `drawing.extend` diagnostic. A crossing that only the engine's
//! **sampled** fallback can find (any SPLINE boundary) is reported with the
//! existing approximation note, exactly like a LINE/ARC target.
//!
//! # Refusals (all write nothing)
//!
//! * `extend.spline_closed` — first control point == last (the domain model has no
//!   closed flag, so a closed *frame* is the only signal; a closed curve has no end
//!   to extend, like a CIRCLE);
//! * `extend.spline_unclamped` — the picked end is not clamped
//!   (multiplicity `degree + 1`), so appending a span would change the existing
//!   curve instead of only adding a tail; the definition is refused rather than
//!   silently re-clamped or re-parameterised;
//! * `extend.spline_degenerate_tangent` — a zero-length target, or an end derivative
//!   that is zero/non-finite (there is no direction to continue along);
//! * `extend.spline_already_at_boundary` — a boundary passes through the picked end
//!   and nothing crosses the ray ahead (the extension would be zero-length);
//! * `extend.no_intersection` — no boundary crosses the tangent ray ahead. A
//!   non-coplanar/skew boundary cannot be told apart from a plain miss here, so it
//!   is reported as this same refusal, never as a fabricated crossing;
//! * `extend.spline_extension_failed` — the appended definition would not be a
//!   finite/valid B-spline (f64 sanity; there is **no** arbitrary length cap).
//!
//! The pick's own refusal reuses the shared `pick_frame` wording. A spline's
//! projection is the nearest point of its own domain, so the shared "pick projects
//! outside the target" refusal cannot fire for this kind (a LINE projects onto its
//! infinite line; a spline never leaves `[0, 1]`).

use super::trim_boundary::{note_approximation, BoundarySet};
use super::trim_curves::{ExtendResolution, PlanRefusal, TargetCurve};
use super::trim_cuts::{pair_has_no_unique_crossing, pick_frame};
use super::trim_spline::SplineTarget;
use super::*;
use cad_geometry::{
    add, dot, length, scale, sub, DefaultGeometryEngine, GeometryEngine, NurbsCurve,
};

/// A spline-plan refusal: the stable `extend.*` code plus its detail.
fn refusal(code: &str, detail: impl std::fmt::Display) -> PlanRefusal {
    PlanRefusal::Invalid(format!("{code}: {detail}"))
}

/// Resolve an EXTEND of an open SPLINE target (see the module doc for the rule).
pub(crate) fn plan_extend_spline(
    spline: &SplineTarget,
    pick: Point3,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
) -> Result<ExtendResolution, PlanRefusal> {
    let curve = spline.curve();
    let points = curve.control_points();
    if points.len() >= 2 && distance(points[0], *points.last().expect("checked non-empty")) <= tol {
        return Err(PlanRefusal::Unsupported(format!(
            "extend.spline_closed: EXTEND cannot extend a closed SPLINE target: its first and \
             last control points coincide, so the curve has no end to extend ({} control \
             points); a closed curve is refused rather than opened or extrapolated",
            points.len()
        )));
    }

    // The shared pick refusal, so an ambiguous pick reads exactly like LINE/ARC.
    let span_len = spline.length();
    if !span_len.is_finite() || span_len <= tol {
        return Err(refusal(
            "extend.spline_degenerate_tangent",
            "the target spline is degenerate (zero length), so it has no end tangent",
        ));
    }
    let Some(u_pick) = spline.project_param(pick, tol) else {
        return Err(refusal(
            "extend.spline_degenerate_tangent",
            "EXTEND could not project the pick onto the target spline",
        ));
    };
    let (_, _, u_pick) =
        pick_frame("EXTEND", "spline", span_len, u_pick, tol).map_err(PlanRefusal::Invalid)?;
    let grow_end = u_pick > 0.5;

    // Growing the start is the mirror of growing the end.
    let working = if grow_end {
        curve.clone()
    } else {
        reverse_parts(
            curve.degree(),
            curve.control_points(),
            curve.knots(),
            curve.weights(),
        )
        .map_err(|error| {
            refusal(
                "extend.spline_extension_failed",
                format!("the target spline could not be mirrored: {error}"),
            )
        })?
    };

    let (_, t_max) = working.domain();
    let end = working.evaluate(t_max);
    let raw_tangent = working.tangent(t_max);
    let tangent_len = length(raw_tangent);
    if !raw_tangent.x.is_finite()
        || !raw_tangent.y.is_finite()
        || !raw_tangent.z.is_finite()
        || !tangent_len.is_finite()
        || tangent_len <= tol
    {
        return Err(refusal(
            "extend.spline_degenerate_tangent",
            format!(
                "the picked end's tangent is degenerate (|C'| = {tangent_len:.9}); there is no \
                 direction to continue along"
            ),
        ));
    }
    let tangent = scale(raw_tangent, 1.0 / tangent_len);

    let mut notes = Vec::new();
    let distance = nearest_crossing_ahead(end, tangent, boundaries, policy, tol, &mut notes)?;
    let extended = append_tangent(&working, end, tangent, distance)?;
    // Mirror back so the stored control points (and their order) are the source's.
    let result = if grow_end {
        extended
    } else {
        reverse_parts(
            extended.degree(),
            extended.control_points(),
            extended.knots(),
            extended.weights(),
        )
        .map_err(|error| {
            refusal(
                "extend.spline_extension_failed",
                format!("the extended spline could not be mirrored back: {error}"),
            )
        })?
    };
    notes.push(format!(
        "EXTEND: the SPLINE target grew along its own end tangent by {distance:.6} as a straight \
         tangent continuation (one appended clamped knot span, degree {} kept, {} appended \
         control point(s), C0/G1 at the joint); the source curve is unchanged on its own knot \
         domain and is never refitted",
        curve.degree(),
        curve.degree() + 1
    ));
    Ok(ExtendResolution {
        geometry: SemanticGeometry::Spline {
            degree: result.degree() as u32,
            knots: result.knots().to_vec(),
            control_points: result.control_points().to_vec(),
            weights: result.weights().to_vec(),
        },
        notes,
    })
}

/// The nearest boundary crossing on the end-tangent ray, as a distance from `end`.
///
/// The probe is a segment long enough to contain every boundary (the same reach
/// justification [`super::trim_cuts::line_probe`] uses), and only a hit that is
/// really on the ray (`|perpendicular component| <= tol`) and strictly ahead
/// (`along > tol`) is a candidate. A hit at the end itself is remembered, and only
/// becomes `extend.spline_already_at_boundary` when nothing crosses further ahead;
/// a hit behind the end is not "ahead" and is ignored.
fn nearest_crossing_ahead(
    end: Point3,
    tangent: Point3,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
    notes: &mut Vec<String>,
) -> Result<f64, PlanRefusal> {
    let reach = ray_reach(end, boundaries)?;
    let probe = SemanticGeometry::Line {
        start: end,
        end: add(end, scale(tangent, reach)),
    };
    let probe_curve = TargetCurve::new(&probe, "EXTEND", "a LINE")
        .map_err(|error| refusal("extend.spline_extension_failed", error))?;
    let engine = DefaultGeometryEngine;
    let mut best: Option<f64> = None;
    let mut at_end = false;
    for primitive in boundaries.primitives() {
        if pair_has_no_unique_crossing(&probe_curve, primitive, tol) {
            continue;
        }
        let hits = engine
            .intersect_local(&probe, primitive, policy)
            .map_err(|error| {
                refusal(
                    "extend.no_intersection",
                    format!("EXTEND could not resolve a boundary intersection: {error}"),
                )
            })?;
        let mut used = false;
        for hit in hits {
            let offset = sub(hit, end);
            let along = dot(offset, tangent);
            if !along.is_finite() {
                continue;
            }
            // The hit must lie on the ray, not merely on its infinite line: an
            // off-ray or (in 3D) non-coplanar boundary is not a crossing.
            if length(sub(offset, scale(tangent, along))) > tol {
                continue;
            }
            if along.abs() <= tol {
                at_end = true;
                continue;
            }
            if along < 0.0 {
                continue;
            }
            used = true;
            best = Some(best.map_or(along, |seen| seen.min(along)));
        }
        // Only a pair that really contributed a crossing is reported as sampled.
        if used {
            note_approximation(&probe, primitive, policy, "EXTEND", notes);
        }
    }
    if let Some(distance) = best {
        return Ok(distance);
    }
    if at_end {
        return Err(refusal(
            "extend.spline_already_at_boundary",
            "a boundary passes through the picked end of the SPLINE target and nothing crosses \
             the tangent ray ahead; there is no positive extension to make",
        ));
    }
    Err(refusal(
        "extend.no_intersection",
        "EXTEND found no intersection ahead of the picked end of the SPLINE target on its \
         end-tangent ray",
    ))
}

/// A segment on the ray long enough to contain every boundary.
///
/// Same reach justification as [`super::trim_cuts::line_probe`]: every crossing
/// lies inside the boundary's bounding box, so
/// `reach = (|box_center − end| + diag + 1)·1.5 + 1` covers it with a world unit to
/// spare; overshooting is harmless because only the nearest crossing is kept.
fn ray_reach(end: Point3, boundaries: &BoundarySet) -> Result<f64, PlanRefusal> {
    let mut bounds = cad_geometry::BoundsAccumulator::new();
    for primitive in boundaries.primitives() {
        bounds.add_geometry(primitive);
    }
    let Some(bounds) = bounds.finish() else {
        return Err(refusal(
            "extend.no_intersection",
            "the EXTEND boundary has no finite extent to extend to",
        ));
    };
    let box_center = scale(add(bounds.min, bounds.max), 0.5);
    let diagonal = length(sub(bounds.max, bounds.min));
    Ok((length(sub(box_center, end)) + diagonal + 1.0) * 1.5 + 1.0)
}

/// Append the straight tangent continuation as one extra clamped knot span.
///
/// `E, E + d/p·T, …, E + d·T` is a collinear (`p + 1` point) control polygon, so the
/// appended span is exactly the segment `[E, E + d·T]`; the trailing knot gets
/// multiplicity `degree + 1` so the joint is a proper clamped span boundary (the
/// source is unchanged, the joint is C0 and the tail's start derivative is along
/// `T`). The appended weights are `1.0` (a straight span is weight-invariant); the
/// source weights are untouched.
fn append_tangent(
    curve: &NurbsCurve,
    end: Point3,
    tangent: Point3,
    distance: f64,
) -> Result<NurbsCurve, PlanRefusal> {
    if !distance.is_finite() || distance <= 0.0 {
        return Err(refusal(
            "extend.spline_extension_failed",
            format!("the tangent continuation distance {distance} is not a positive finite value"),
        ));
    }
    let degree = curve.degree();
    let knots = curve.knots();
    let count = curve.control_points().len();
    let (_, t_max) = curve.domain();
    // Refuse rather than reshape: appending a span only adds a tail if the picked
    // end is clamped (multiplicity degree + 1).
    let clamp_slack = t_max.abs().max(1.0) * 1e-12;
    if knots[count..]
        .iter()
        .any(|knot| (knot - t_max).abs() > clamp_slack)
    {
        return Err(refusal(
            "extend.spline_unclamped",
            format!(
                "the picked end of the SPLINE target is not clamped (multiplicity {}), so \
                 appending a knot span would change the existing curve instead of adding a \
                 tangent tail",
                degree + 1
            ),
        ));
    }
    let t_new = t_max + extension_span(knots, count, degree, t_max);
    if !t_new.is_finite() || t_new <= t_max {
        return Err(refusal(
            "extend.spline_extension_failed",
            "the appended knot span is not representable",
        ));
    }

    let mut control_points = curve.control_points().to_vec();
    let mut weights = curve.weights().to_vec();
    for step in 0..=degree {
        let fraction = step as f64 / degree as f64;
        control_points.push(add(end, scale(tangent, distance * fraction)));
        weights.push(1.0);
    }
    let mut new_knots = knots.to_vec();
    new_knots.extend(std::iter::repeat_n(t_new, degree + 1));
    NurbsCurve::new(degree, control_points, new_knots, weights).map_err(|error| {
        refusal(
            "extend.spline_extension_failed",
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
        // One degenerate span only: fall back to the whole domain, and to `1.0` for
        // an all-zero width (NurbsCurve::new has already refused that case).
        None => (t_max - knots[degree]).abs().max(1.0),
    }
}

/// The exact knot mirror of `curve`: reversed control points/weights and
/// `(U₀ + U_last) − U` reflected knots (the same formula `join_spline` uses, so both
/// ends of the curve map to the same geometry with the direction flipped).
fn reverse_parts(
    degree: usize,
    control_points: &[Point3],
    knots: &[f64],
    weights: &[f64],
) -> CadResult<NurbsCurve> {
    let mut points = control_points.to_vec();
    let mut weights = weights.to_vec();
    points.reverse();
    weights.reverse();
    let sum = knots[0] + knots[knots.len() - 1];
    let knots = knots.iter().rev().map(|knot| sum - knot).collect();
    NurbsCurve::new(degree, points, knots, weights)
}
