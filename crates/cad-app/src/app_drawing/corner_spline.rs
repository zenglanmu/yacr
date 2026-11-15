//! SPLINE as a FILLET participant.
//!
//! A SPLINE can be filleted: the construction is the numeric tangent family the
//! ELLIPSE form already uses ([`super::corner_ellipse`]), with the spline as the
//! driver curve, and the trim is an **exact** B-spline sub-curve. The "no
//! control-frame trim" refusal this command used to return was a planner gap, not
//! a model gap: [`cad_geometry::sub_curve`] inserts the tangent parameter as a knot
//! (Boehm, on the homogeneous control points) and truncates the control polygon
//! between the kept ends, so the result is the same curve, same degree, same
//! weights, with the tangent point as an exact endpoint. Nothing is refitted, and
//! no sample becomes a control point.
//!
//! ## Construction (declared v1)
//!
//! The fillet centre is the one-parameter family `C(t) ± R·n(t)` of the picked
//! spline (`n` the unit normal from the exact first derivative). A root of
//! `distance(centre, other) − R` is a circle of radius `R` tangent to both
//! participants. Roots are bracketed by the shared dense parameter scan
//! ([`super::corner_roots`]) and refined there, so the spline form brackets,
//! bisects and polishes exactly like the ellipse form; the nearest-pick candidate
//! wins, and an exact score tie keeps the first candidate, so the winner is
//! deterministic.
//!
//! ## What is trimmed
//!
//! * SPLINE — the exact sub-curve on the pick side of the tangent parameter. A
//!   tangent at the curve's own end leaves the whole curve (no update), and a
//!   kept span shorter than `degree + 1` control points is a named refusal.
//! * LINE — retargeted on the pick side (a segment whose tangent lies outside it
//!   is not a fillet of the picked object: `FILLET.tangent_outside_arc`).
//! * ARC — trimmed at its tangent point on the pick side.
//! * CIRCLE — **never** trimmed (same rule as every other FILLET form).
//! * ELLIPSE — its parametric `start`/`sweep` are rewritten, never re-approximated.
//!
//! ## `R = 0`
//!
//! The sharp corner: the picked pair is trimmed to their intersection nearest the
//! two picks and **no** ARC is created. For a spline↔spline pair the intersection
//! is numeric (the minimum of the point-to-curve distance, refined by a
//! golden-section search and accepted within [`NUMERIC_RELATIVE`] of the control
//! polygon's scale); for a spline against a LINE/CIRCLE/ARC/ELLIPSE it is a root of
//! that participant's signed residual, bisected to the parameter tolerance. No
//! intersection is the named `FILLET.no_corner_intersection` refusal, never an
//! empty success. With trim off the sharp corner would create nothing and is
//! refused (`FILLET.trim_off_no_arc`).
//!
//! ## Refusals (all named, all before the first write)
//!
//! `FILLET.unsupported_plane` (the control polygon is not planar, or the other
//! participant is on another Z), `FILLET.not_coplanar`, `FILLET.no_tangent_arc`
//! (no radius of this size touches both), `FILLET.tangent_outside_arc` (a tangent
//! or pick outside the picked finite object), `FILLET.pick_at_tangent` (the pick
//! selects neither side of the tangent on the spline),
//! `FILLET.pick_outside_spline`, `FILLET.degenerate_spline` (the kept span cannot
//! be represented), `FILLET.no_corner_intersection`, `FILLET.radius_negative`.
//!
//! ## Not implemented
//!
//! AutoCAD's `Multiple` loop and its multi-solution **cycling** between the
//! candidate fillets: each accepted invocation is one pair, one solution (nearest
//! the picks), one transaction / one undo step.

use super::super::helpers::{distance, normalize_signed};
use super::super::*;
use super::corner_geometry::{arc, pt, Curve, GeometryUpdate, TwoLinePlan};
use super::corner_participant::Other;
use super::corner_roots::scan_roots;
use cad_geometry::{add, length, nearest_point_parameter, scale, sub, NurbsCurve};

/// Parameter samples of the tangency/corner scan (the ellipse form's budget).
const SAMPLES: usize = 720;

/// Relative Z tolerance of "one +Z plane".
const PLANE_RELATIVE: f64 = 1e-9;

/// World-space acceptance of a numeric spline↔spline corner, relative to the
/// control polygon's extent. Declared, not a claim of exactness.
const NUMERIC_RELATIVE: f64 = 1e-7;

/// A validated SPLINE participant: the source curve plus the derived quantities
/// the plan needs (the expanded stored form and the control-polygon extent).
#[derive(Debug, Clone)]
pub(super) struct SplineCurve {
    curve: NurbsCurve,
    /// Diagonal of the control-polygon box: the scale of every numeric tolerance.
    extent: f64,
    /// The one Z plane the control polygon lies in.
    z: f64,
}

impl SplineCurve {
    /// Expand and validate a stored SPLINE definition (`knots: []` /
    /// `weights: []` are the documented clamped-uniform / unit-weight shorthands).
    ///
    /// A malformed definition is `InvalidInput` (the stored data is wrong); a
    /// spline that is not planar is `Unsupported` — the whole corner construction
    /// is a `+Z` construction.
    pub(super) fn new(
        degree: u32,
        knots: &[f64],
        control_points: &[Point3],
        weights: &[f64],
    ) -> CadResult<Self> {
        let curve = cad_geometry::expand_spline_source(degree, knots, control_points, weights)?;
        let (t_min, t_max) = curve.domain();
        if !t_min.is_finite() || !t_max.is_finite() || t_max <= t_min {
            return Err(CadError::Unsupported(
                "FILLET.degenerate_spline: the picked SPLINE has no finite parameter domain".into(),
            ));
        }
        let z = curve.control_points()[0].z;
        let spread = curve
            .control_points()
            .iter()
            .map(|point| (point.z - z).abs())
            .fold(0.0_f64, f64::max);
        if !z.is_finite() || !spread.is_finite() || spread > PLANE_RELATIVE * (1.0 + z.abs()) {
            return Err(CadError::Unsupported(format!(
                "FILLET.unsupported_plane: the picked SPLINE control polygon leaves one +Z plane \
                 (z spread {spread}); only planar splines are filleted"
            )));
        }
        let extent = cad_geometry::control_polygon_extent(curve.control_points());
        Ok(Self { curve, extent, z })
    }

    pub(super) fn domain(&self) -> (f64, f64) {
        self.curve.domain()
    }

    pub(super) fn z(&self) -> f64 {
        self.z
    }

    pub(super) fn point(&self, parameter: f64) -> Point3 {
        self.curve.evaluate(parameter)
    }

    /// The unit `+Z`-plane normal from the exact first derivative (`None` where the
    /// tangent has no direction). Its orientation is arbitrary for an open curve;
    /// the caller scans both signs of the offset, so both fillet sides are tried.
    fn normal(&self, parameter: f64) -> Option<Point3> {
        let tangent = self.curve.tangent(parameter);
        let len = (tangent.x * tangent.x + tangent.y * tangent.y).sqrt();
        if !len.is_finite() || len <= 1e-12 {
            return None;
        }
        let normal = pt(-tangent.y / len, tangent.x / len, 0.0);
        is_finite_point(normal).then_some(normal)
    }

    /// The nearest point of the curve and its own parameter.
    pub(super) fn nearest(&self, from: Point3) -> Option<(f64, Point3)> {
        let (parameter, gap) = nearest_point_parameter(&self.curve, from)?;
        if !parameter.is_finite() || !gap.is_finite() {
            return None;
        }
        Some((parameter, self.point(parameter)))
    }

    /// The curve parameter of `point` when it really lies on this curve.
    ///
    /// The bound is a numeric inversion tolerance (the shared inversion is exact to
    /// machine precision, so this only rejects points that are not on the curve).
    fn parameter_of(&self, point: Point3, tol: f64) -> Option<f64> {
        let (parameter, gap) = nearest_point_parameter(&self.curve, point)?;
        let bound = (self.extent * NUMERIC_RELATIVE).max(tol);
        (gap.is_finite() && gap <= bound).then_some(parameter)
    }

    /// Whether the tangent and the pick both resolve onto this curve.
    pub(super) fn accepts(&self, tangent: Point3, pick: Point3, tol: f64) -> bool {
        self.parameter_of(tangent, tol).is_some() && self.parameter_of(pick, tol).is_some()
    }

    /// Trim to the piece on the pick side of the tangent parameter.
    ///
    /// `Ok(None)` means the tangent sits at (or within the world tolerance of) one
    /// of the curve's own ends, so the kept piece is the whole spline. A pick that
    /// projects onto the tangent point selects no side and is refused by name.
    pub(super) fn trim_keep_pick(
        &self,
        tangent_parameter: f64,
        pick: Point3,
        tol: f64,
    ) -> Result<Option<GeometryUpdate>, String> {
        let (t_min, t_max) = self.domain();
        if !tangent_parameter.is_finite() || tangent_parameter < t_min || tangent_parameter > t_max
        {
            return Err(
                "FILLET.degenerate_spline: the tangent parameter leaves the picked SPLINE's knot \
                 domain"
                    .into(),
            );
        }
        // The host resolves the picked entity with its own pickbox and passes the
        // raw click, so a near miss is still a pick of this spline — the same rule
        // TRIM follows for a SPLINE target (`project_param`). Only a curve with no
        // projection at all is refused.
        let (pick_parameter, _) = self.nearest(pick).ok_or_else(|| {
            "FILLET.pick_outside_spline: the pick could not be projected onto the picked SPLINE"
                .to_string()
        })?;
        // Convert the world tolerance into this curve's parameter units at the
        // pick (a spline's parameter is not a length).
        let speed = length(self.curve.tangent(pick_parameter));
        let parameter_tol = if speed.is_finite() && speed > 1e-12 {
            tol / speed
        } else {
            (t_max - t_min) * 1e-9
        };
        if (pick_parameter - tangent_parameter).abs() <= parameter_tol {
            return Err(
                "FILLET.pick_at_tangent: the pick selects neither side of the tangent point on the \
                 picked SPLINE"
                    .into(),
            );
        }
        if tangent_parameter <= t_min + parameter_tol || tangent_parameter >= t_max - parameter_tol
        {
            // The kept side is the whole curve: nothing to rewrite.
            return Ok(None);
        }
        let (kept_start, kept_end) = if pick_parameter < tangent_parameter {
            (t_min, tangent_parameter)
        } else {
            (tangent_parameter, t_max)
        };
        self.sub_curve(kept_start, kept_end)
            .map(|geometry| Some(GeometryUpdate::Spline { geometry }))
    }

    /// The exact `[t0, t1]` B-spline sub-curve in the stored representation.
    fn sub_curve(&self, t0: f64, t1: f64) -> Result<SemanticGeometry, String> {
        let sub = cad_geometry::sub_curve(&self.curve, t0, t1).map_err(|error| {
            format!(
                "FILLET.degenerate_spline: the kept SPLINE sub-curve cannot be extracted: {error}"
            )
        })?;
        Ok(SemanticGeometry::Spline {
            degree: sub.degree as u32,
            knots: sub.knots,
            control_points: sub.control_points,
            weights: sub.weights,
        })
    }
}

/// One candidate fillet circle of a SPLINE plan (same ordering rule as the
/// analytic curve↔curve and ellipse forms).
struct SplineCandidate {
    score: f64,
    feasible: bool,
    centre: Point3,
    driver_parameter: f64,
    other_parameter: f64,
    driver_tangent: Point3,
    other_tangent: Point3,
}

impl SplineCandidate {
    fn outranks(&self, other: &Self) -> bool {
        match (self.feasible, other.feasible) {
            (true, false) => true,
            (false, true) => false,
            _ => self.score < other.score,
        }
    }
}

/// Plan FILLET of a pair whose first or second participant is a SPLINE.
///
/// The spline is always the driver of the numeric tangent family, so the
/// construction is symmetric in pick order; `driver_is_first` maps the two
/// computed tangents back onto the caller's first/second slots.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan_fillet_with_spline(
    first: Curve,
    pick_first: Point3,
    second: Curve,
    pick_second: Point3,
    radius: f64,
    trim: bool,
    tol: f64,
) -> Result<TwoLinePlan, String> {
    if !radius.is_finite() || radius < 0.0 {
        return Err("FILLET.radius_negative: the radius must be finite and non-negative".into());
    }
    let (driver, pick_driver, other_curve, pick_other, driver_is_first) = match (first, second) {
        (Curve::Spline(spline), other) => (spline, pick_first, other, pick_second, true),
        (other, Curve::Spline(spline)) => (spline, pick_second, other, pick_first, false),
        _ => {
            return Err(
                "FILLET.unsupported_pair: the spline form needs a SPLINE participant".into(),
            );
        }
    };
    let other = Other::from_curve(other_curve);
    other.validate(tol)?;
    let (other_low, other_high) = other.z_extent();
    let z_tol = PLANE_RELATIVE * (1.0 + driver.z().abs() + other_low.abs() + other_high.abs());
    if (driver.z() - other_low).abs() > z_tol || (driver.z() - other_high).abs() > z_tol {
        return Err(format!(
            "FILLET.not_coplanar: the SPLINE and the {} are not coplanar (+Z)",
            other.label()
        ));
    }
    // The command dispatch routes every `R = 0` invocation to the sharp-corner
    // form ([`super::corner_sharp`]) before this planner, so a zero radius here is
    // a caller bug; refuse by name instead of running the tangency search with a
    // zero radius (which would report a misleading "no tangent arc").
    if radius == 0.0 {
        return Err(
            "FILLET.radius_zero: the R=0 sharp corner is planned by the sharp-corner form \
             (corner_sharp), not by the spline tangency form"
                .into(),
        );
    }
    let (t_min, t_max) = driver.domain();
    let mut best: Option<SplineCandidate> = None;
    for sign in [1.0f64, -1.0] {
        let residual = |t: f64| spline_residual(&driver, &other, t, sign, radius);
        for t in scan_roots(&residual, t_min, t_max, SAMPLES) {
            let Some(candidate) = spline_candidate(
                &driver,
                &other,
                t,
                sign,
                radius,
                tol,
                pick_driver,
                pick_other,
            ) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|current| candidate.outranks(current))
            {
                best = Some(candidate);
            }
        }
    }
    let Some(chosen) = best else {
        return Err(
            "FILLET.no_tangent_arc: no arc of this radius is tangent to both objects".into(),
        );
    };
    if !chosen.feasible {
        return Err(
            "FILLET.tangent_outside_arc: the chosen fillet touches a picked object outside its \
             span (or the pick is off it); nothing is written"
                .into(),
        );
    }
    let driver_update = if trim {
        driver.trim_keep_pick(chosen.driver_parameter, pick_driver, tol)?
    } else {
        None
    };
    let other_update = if trim {
        other.trim(
            chosen.other_tangent,
            chosen.other_parameter,
            pick_other,
            tol,
        )?
    } else {
        None
    };
    let (first_tangent, second_tangent) = match driver_is_first {
        true => (chosen.driver_tangent, chosen.other_tangent),
        false => (chosen.other_tangent, chosen.driver_tangent),
    };
    let start = (first_tangent.y - chosen.centre.y).atan2(first_tangent.x - chosen.centre.x);
    let end = (second_tangent.y - chosen.centre.y).atan2(second_tangent.x - chosen.centre.x);
    let sweep = normalize_signed(end - start);
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return Err("FILLET.degenerate_arc: the tangent arc has no sweep".into());
    }
    let (first_update, second_update) = match driver_is_first {
        true => (driver_update, other_update),
        false => (other_update, driver_update),
    };
    Ok(TwoLinePlan {
        first: first_update,
        second: second_update,
        inserted: Some(arc(chosen.centre, radius, start, sweep)),
    })
}

/// The signed residual of a candidate fillet centre at spline parameter `t`:
/// `distance-to-other − R` (`None` where the normal or the projection fails).
fn spline_residual(
    driver: &SplineCurve,
    other: &Other,
    t: f64,
    sign: f64,
    radius: f64,
) -> Option<f64> {
    let point = driver.point(t);
    let normal = driver.normal(t)?;
    let centre = add(point, scale(normal, sign * radius));
    let closest = other.closest(centre)?;
    let residual = closest.distance - radius;
    residual.is_finite().then_some(residual)
}

/// Build a candidate from one spline parameter, or `None` when the other curve is
/// not exactly `R` away (the numeric acceptance tolerance).
#[allow(clippy::too_many_arguments)]
fn spline_candidate(
    driver: &SplineCurve,
    other: &Other,
    t: f64,
    sign: f64,
    radius: f64,
    tol: f64,
    pick_driver: Point3,
    pick_other: Point3,
) -> Option<SplineCandidate> {
    let mut parameter = t;
    let mut point = driver.point(parameter);
    let normal = driver.normal(parameter)?;
    let mut centre = add(point, scale(normal, sign * radius));
    let mut closest = other.closest(centre)?;
    let accept = NUMERIC_RELATIVE * (1.0 + centre.x.abs() + centre.y.abs() + radius);
    if (closest.distance - radius).abs() > accept {
        return None;
    }
    // Alternating projection: pull the centre onto the exact `R`-offset of the
    // other curve, then onto the spline's own signed `R`-offset. Near a tangency
    // this converges to machine precision, unlike refining the (there quadratic)
    // residual itself.
    for _ in 0..24 {
        let direction = sub(centre, closest.point);
        let separation = length(direction);
        if !separation.is_finite() || separation <= 1e-12 {
            break;
        }
        centre = add(closest.point, scale(direction, radius / separation));
        let (refined, refined_point) = driver.nearest(centre)?;
        parameter = refined;
        let refined_normal = driver.normal(parameter)?;
        let projected = add(refined_point, scale(refined_normal, sign * radius));
        let moved = distance(projected, centre);
        point = refined_point;
        centre = projected;
        closest = other.closest(centre)?;
        if moved <= 1e-14 * (1.0 + length(centre)) {
            break;
        }
    }
    if (closest.distance - radius).abs() > 1e-9 * (1.0 + radius) {
        return None;
    }
    let start = (point.y - centre.y).atan2(point.x - centre.x);
    let end = (closest.point.y - centre.y).atan2(closest.point.x - centre.x);
    let sweep = normalize_signed(end - start);
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return None;
    }
    let feasible =
        driver.accepts(point, pick_driver, tol) && other.accepts(closest.point, pick_other, tol);
    Some(SplineCandidate {
        score: distance(point, pick_driver) + distance(closest.point, pick_other),
        feasible,
        centre,
        driver_parameter: parameter,
        other_parameter: closest.parameter,
        driver_tangent: point,
        other_tangent: closest.point,
    })
}
