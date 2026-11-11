//! Pure geometry for FILLET with an ELLIPSE participant.
//!
//! Declared with an explicit `#[path]` beside the analytic curve↔curve and
//! line↔circle planners so no file outgrows the 1000-line ceiling. Everything
//! here is pure: the command layer resolves the picks, calls
//! [`plan_fillet_with_ellipse`] and commits the returned plan in one transaction.
//!
//! ## Construction (declared v1)
//!
//! The fillet centre is the one-parameter family `P(θ) ± R·n(θ)` of the picked
//! ellipse. A root of `distance(centre, other) − R` is a circle of radius `R`
//! tangent to both participants. Roots are bracketed by a dense parameter scan:
//! a sign change is bisected, and a **tangential** solution (where the residual
//! only touches zero) is located by Newton on `residual′ = 0`, which is
//! transverse and therefore accurate to machine precision. An alternating
//! projection then polishes both tangency points exactly. The candidate nearest
//! the two picks wins; an exact score tie keeps the first candidate, so the
//! winner is deterministic.
//!
//! Only the `+Z` work plane is implemented (the ellipse frame is built from the
//! stored normal). The elliptical arc is trimmed by rewriting its parametric
//! `start`/`sweep`; a CIRCLE is never trimmed, a LINE is retargeted on the pick
//! side and an ARC is trimmed at the tangent point. `R = 0` is a named refusal
//! (the sharp-corner form is only implemented for line/line and line/circle).
//!
//! **Declared rule**: a LINE participant's tangent must lie on the picked finite
//! segment (in both trim modes), and an ARC/elliptical-arc participant's tangent
//! must lie on its picked span. A candidate that would touch a carrier outside
//! the picked object is never committed (`FILLET.tangent_outside_arc`), matching
//! the analytic curve↔curve rule.

use super::super::helpers::{distance, is_finite_point, normalize_signed};
use super::super::*;
use super::corner_geometry::{
    arc, arc_position, retarget_pick_side, trim_arc_to, Curve, GeometryUpdate, TwoLinePlan,
};
use cad_geometry::{add, dot, length, scale, sub};

/// One candidate fillet circle of an ellipse plan (the same ordering rule as
/// the analytic curve↔curve form).
struct EllipseCandidate {
    score: f64,
    feasible: bool,
    centre: Point3,
    first_tangent: Point3,
    second_tangent: Point3,
}

impl EllipseCandidate {
    fn outranks(&self, other: &Self) -> bool {
        match (self.feasible, other.feasible) {
            (true, false) => true,
            (false, true) => false,
            _ => self.score < other.score,
        }
    }
}

/// One +Z ELLIPSE participant of a FILLET.
///
/// The stored `start`/`sweep` are the ellipse's **parametric (eccentric)**
/// angles, exactly as the domain stores them, so a trim is an angle rewrite of
/// the same entity (never a re-approximation).
#[derive(Debug, Clone, Copy)]
pub(super) struct EllipseCurve {
    pub center: Point3,
    /// Unit major-axis direction in the `+Z` plane.
    pub major: Point3,
    /// Unit minor-axis direction (`cross(+Z, major)`).
    pub minor: Point3,
    /// Semi-major length.
    pub a: f64,
    /// Semi-minor length.
    pub b: f64,
    pub start: f64,
    pub sweep: f64,
}

impl EllipseCurve {
    /// The point at parametric angle `theta`.
    fn point(self, theta: f64) -> Point3 {
        add(
            self.center,
            add(
                scale(self.major, self.a * theta.cos()),
                scale(self.minor, self.b * theta.sin()),
            ),
        )
    }

    /// The outward unit normal at parametric angle `theta` (`None` if it has no
    /// direction, which cannot happen for a validated `a, b > tol`).
    fn normal(self, theta: f64) -> Option<Point3> {
        let normal = add(
            scale(self.major, self.b * theta.cos()),
            scale(self.minor, self.a * theta.sin()),
        );
        let len = length(normal);
        (len.is_finite() && len > 1e-12).then(|| scale(normal, 1.0 / len))
    }

    /// The parametric angle of a point nominally on the ellipse.
    fn theta_of(self, point: Point3) -> Option<f64> {
        if self.a <= 0.0 || self.b <= 0.0 {
            return None;
        }
        let offset = sub(point, self.center);
        let theta = (dot(offset, self.minor) / self.b).atan2(dot(offset, self.major) / self.a);
        theta.is_finite().then_some(theta)
    }

    /// The along-sweep position of `theta` from `start` (`0..2π`).
    fn position(self, theta: f64) -> f64 {
        let sign = self.sweep.signum();
        let mut value = (theta - self.start) * sign % std::f64::consts::TAU;
        if value < 0.0 {
            value += std::f64::consts::TAU;
        }
        value
    }

    fn span(self) -> f64 {
        self.sweep.abs()
    }
}

/// One resolved FILLET participant for the numeric ellipse forms.
#[derive(Debug, Clone, Copy)]
enum Participant {
    Line {
        start: Point3,
        end: Point3,
    },
    Round {
        center: Point3,
        radius: f64,
        arc: Option<(f64, f64)>,
    },
    Ellipse(EllipseCurve),
}

/// The closest point of a participant's supporting curve.
struct ClosestPoint {
    distance: f64,
    point: Point3,
    /// The participant's own parameter at `point` (line projection / angle /
    /// ellipse eccentric angle), used by the tangency polish.
    parameter: f64,
}

impl Participant {
    fn from_curve(curve: Curve) -> Self {
        match curve {
            Curve::Line { start, end } => Self::Line { start, end },
            Curve::Circle { center, radius } => Self::Round {
                center,
                radius,
                arc: None,
            },
            Curve::Arc {
                center,
                radius,
                start,
                sweep,
            } => Self::Round {
                center,
                radius,
                arc: Some((start, sweep)),
            },
            Curve::Ellipse(ellipse) => Self::Ellipse(ellipse),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Line { .. } => "LINE",
            Self::Round { arc: None, .. } => "CIRCLE",
            Self::Round { arc: Some(_), .. } => "ARC",
            Self::Ellipse(_) => "ELLIPSE",
        }
    }

    /// The z range this participant occupies (a line must be planar in XY).
    fn z_range(self) -> (f64, f64) {
        match self {
            Self::Line { start, end } => (start.z.min(end.z), start.z.max(end.z)),
            Self::Round { center, .. } => (center.z, center.z),
            Self::Ellipse(ellipse) => (ellipse.center.z, ellipse.center.z),
        }
    }

    fn validate(self, tol: f64) -> Result<(), String> {
        match self {
            Self::Line { start, end } => {
                if !is_finite_point(start) || !is_finite_point(end) {
                    return Err("FILLET.non_finite: the picked object must be finite".into());
                }
                if distance(start, end) <= tol {
                    return Err("FILLET.degenerate_line: the picked object has no direction".into());
                }
                Ok(())
            }
            Self::Round { radius, .. } => {
                if !radius.is_finite() || radius <= tol {
                    return Err("FILLET.degenerate_circle: the picked curve has no radius".into());
                }
                Ok(())
            }
            Self::Ellipse(ellipse) => {
                if !ellipse.a.is_finite()
                    || !ellipse.b.is_finite()
                    || ellipse.a <= tol
                    || ellipse.b <= tol
                {
                    return Err(
                        "FILLET.degenerate_ellipse: the picked ellipse has no usable axes".into(),
                    );
                }
                Ok(())
            }
        }
    }

    /// The closest point on the **supporting** curve (an arc's span is checked by
    /// [`Self::accepts`], not here).
    fn closest(self, from: Point3) -> Option<ClosestPoint> {
        match self {
            Self::Line { start, end } => {
                let direction = sub(end, start);
                let len = length(direction);
                if !len.is_finite() || len <= 1e-12 {
                    return None;
                }
                let unit = scale(direction, 1.0 / len);
                let along = dot(sub(from, start), unit);
                let point = add(start, scale(unit, along));
                Some(ClosestPoint {
                    distance: distance(point, from),
                    point,
                    parameter: along,
                })
            }
            Self::Round {
                center,
                radius,
                arc: _,
            } => {
                let offset = sub(from, center);
                let len = length(offset);
                if !len.is_finite() || len <= 1e-12 {
                    return None;
                }
                let point = add(center, scale(offset, radius / len));
                Some(ClosestPoint {
                    distance: distance(point, from),
                    point,
                    parameter: offset.y.atan2(offset.x),
                })
            }
            Self::Ellipse(ellipse) => closest_on_ellipse(ellipse, from),
        }
    }

    /// Whether a tangent point (and the pick) lies on the finite participant.
    fn accepts(self, tangent: Point3, pick: Point3, tol: f64) -> bool {
        match self {
            Self::Line { start, end } => {
                let direction = sub(end, start);
                let len = length(direction);
                if !len.is_finite() || len <= tol {
                    return false;
                }
                let unit = scale(direction, 1.0 / len);
                let tangent_along = dot(sub(tangent, start), unit);
                let pick_along = dot(sub(pick, start), unit);
                tangent_along > tol
                    && tangent_along <= len + 1e-9
                    && pick_along > tol
                    && pick_along <= len + 1e-9
            }
            Self::Round {
                center,
                radius,
                arc,
            } => {
                let Some((start, sweep)) = arc else {
                    return true;
                };
                let tangent_position = arc_position(center, start, sweep, tangent);
                let pick_position = arc_position(center, start, sweep, pick);
                tangent_position > tol
                    && tangent_position <= sweep.abs() + 1e-9
                    && pick_position <= sweep.abs() + 1e-9
                    && (radius - distance(tangent, center)).abs() <= 1e-7 * (1.0 + radius)
            }
            Self::Ellipse(ellipse) => ellipse_accepts(ellipse, tangent, pick, tol),
        }
    }

    /// Trim the participant at `tangent` on the pick side.
    fn trim(
        self,
        tangent: Point3,
        pick: Point3,
        tol: f64,
    ) -> Result<Option<GeometryUpdate>, String> {
        match self {
            Self::Line { start, end } => {
                retarget_pick_side((start, end), pick, tangent, tol, "FILLET").map(Some)
            }
            Self::Round {
                center,
                arc: Some((start, sweep)),
                ..
            } => trim_arc_to(center, start, sweep, tangent, pick, tol).map(Some),
            Self::Round { arc: None, .. } => Ok(None),
            Self::Ellipse(ellipse) => trim_ellipse(ellipse, tangent, pick, tol).map(Some),
        }
    }
}

/// Whether the tangent and pick both lie on the picked ellipse arc span.
fn ellipse_accepts(ellipse: EllipseCurve, tangent: Point3, pick: Point3, tol: f64) -> bool {
    let (Some(tangent_theta), Some(pick_theta)) =
        (ellipse.theta_of(tangent), ellipse.theta_of(pick))
    else {
        return false;
    };
    let span = ellipse.span();
    let tangent_position = ellipse.position(tangent_theta);
    let pick_position = ellipse.position(pick_theta);
    tangent_position > tol && tangent_position <= span + 1e-9 && pick_position <= span + 1e-9
}

/// Trim an elliptical arc to the tangent parameter on the pick side.
fn trim_ellipse(
    ellipse: EllipseCurve,
    tangent: Point3,
    pick: Point3,
    tol: f64,
) -> Result<GeometryUpdate, String> {
    let tangent_theta = ellipse.theta_of(tangent).ok_or_else(|| {
        "FILLET.degenerate_ellipse: the tangent point has no ellipse parameter".to_string()
    })?;
    let pick_theta = ellipse.theta_of(pick).ok_or_else(|| {
        "FILLET.pick_outside_ellipse: the pick has no ellipse parameter".to_string()
    })?;
    let span = ellipse.span();
    let tangent_position = ellipse.position(tangent_theta);
    let pick_position = ellipse.position(pick_theta);
    if tangent_position > span + 1e-9 {
        return Err(
            "FILLET.tangent_outside_ellipse: the tangent point does not lie on the picked \
             elliptical arc"
                .into(),
        );
    }
    if pick_position > span + 1e-9 {
        return Err(
            "FILLET.pick_outside_ellipse: the pick does not lie on the picked elliptical arc"
                .into(),
        );
    }
    if tangent_position <= tol {
        return Err(
            "FILLET.degenerate_ellipse: the tangent point is the elliptical arc's own start".into(),
        );
    }
    let sign = ellipse.sweep.signum();
    if pick_position <= tangent_position {
        let sweep = tangent_position * sign;
        if sweep.abs() <= tol {
            return Err("FILLET.degenerate_ellipse: the trim leaves no elliptical arc".into());
        }
        Ok(GeometryUpdate::Ellipse {
            start: ellipse.start,
            sweep,
        })
    } else {
        let sweep = (span - tangent_position) * sign;
        if sweep.abs() <= tol {
            return Err(
                "FILLET.degenerate_ellipse: the trim leaves a near-zero remainder of the picked \
                 elliptical arc"
                    .into(),
            );
        }
        Ok(GeometryUpdate::Ellipse {
            start: ellipse.start + tangent_position * sign,
            sweep,
        })
    }
}

/// The closest point of the supporting ellipse, by bounded numeric minimisation.
fn closest_on_ellipse(ellipse: EllipseCurve, from: Point3) -> Option<ClosestPoint> {
    let samples = 256usize;
    let sweep = ellipse.sweep;
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return None;
    }
    let mut best_theta = ellipse.start;
    let mut best = f64::INFINITY;
    for index in 0..samples {
        let theta = ellipse.start + sweep * (index as f64) / (samples as f64);
        let candidate = distance(ellipse.point(theta), from);
        if candidate < best {
            best = candidate;
            best_theta = theta;
        }
    }
    let step = sweep.abs() / samples as f64;
    let mut low = best_theta - step;
    let mut high = best_theta + step;
    for _ in 0..120 {
        let third = (high - low) / 3.0;
        let lower = low + third;
        let upper = high - third;
        if distance(ellipse.point(lower), from) < distance(ellipse.point(upper), from) {
            high = upper;
        } else {
            low = lower;
        }
    }
    let theta = refine_closest_parameter(ellipse, from, 0.5 * (low + high))?;
    let point = ellipse.point(theta);
    Some(ClosestPoint {
        distance: distance(point, from),
        point,
        parameter: theta,
    })
}

/// Newton polish of the closest-point parameter.
///
/// Minimising the distance itself is quadratic near the minimum, so comparing
/// distances only locates θ to ~√ε. The orthogonality condition
/// `⟨P(θ) − from, P′(θ)⟩ = 0` is **transverse** at the minimum, so its Newton
/// step recovers θ to machine precision.
fn refine_closest_parameter(ellipse: EllipseCurve, from: Point3, mut theta: f64) -> Option<f64> {
    for _ in 0..12 {
        let point = ellipse.point(theta);
        let (sin, cos) = theta.sin_cos();
        let tangent = add(
            scale(ellipse.major, -ellipse.a * sin),
            scale(ellipse.minor, ellipse.b * cos),
        );
        let curvature = add(
            scale(ellipse.major, -ellipse.a * cos),
            scale(ellipse.minor, -ellipse.b * sin),
        );
        let offset = sub(point, from);
        let value = dot(offset, tangent);
        let derivative = dot(tangent, tangent) + dot(offset, curvature);
        if !derivative.is_finite() || derivative.abs() <= 1e-18 {
            break;
        }
        let step = value / derivative;
        if !step.is_finite() {
            break;
        }
        theta -= step;
        if step.abs() <= 1e-15 * (1.0 + theta.abs()) {
            break;
        }
    }
    theta.is_finite().then_some(theta)
}

/// The signed residual of a candidate fillet centre at ellipse parameter
/// `theta`: `distance-to-other − R` (`None` where the normal is undefined).
fn ellipse_residual(
    ellipse: EllipseCurve,
    other: Participant,
    theta: f64,
    sign: f64,
    radius: f64,
) -> Option<f64> {
    let point = ellipse.point(theta);
    let normal = ellipse.normal(theta)?;
    let centre = add(point, scale(normal, sign * radius));
    let closest = other.closest(centre)?;
    let residual = closest.distance - radius;
    residual.is_finite().then_some(residual)
}

/// Run a FILLET whose picked pair contains at least one ELLIPSE.
///
/// The fillet centre is the one-parameter family `P(θ) + s·R·n(θ)`; a root of
/// `distance(centre, other) − R` is a circle of radius `R` tangent to both
/// participants. Roots are bracketed on a dense parameter scan (sign changes
/// and tangential minima) and bisected/refined, so the constructions are
/// numeric but deterministic; the candidate nearest the two picks wins, exactly
/// like the analytic curve↔curve form.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan_fillet_with_ellipse(
    first: Curve,
    pick_first: Point3,
    second: Curve,
    pick_second: Point3,
    radius: f64,
    trim: bool,
    tol: f64,
) -> Result<TwoLinePlan, String> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(
            "FILLET.radius_zero: an ellipse fillet needs a positive radius (the R=0 sharp \
             corner is only implemented for line/line and line/circle)"
                .into(),
        );
    }
    let (driver, pick_driver, other_curve, pick_other, driver_is_first) = match (first, second) {
        (Curve::Ellipse(ellipse), other) => (ellipse, pick_first, other, pick_second, true),
        (other, Curve::Ellipse(ellipse)) => (ellipse, pick_second, other, pick_first, false),
        _ => {
            return Err("FILLET.unsupported_pair: no ELLIPSE participant".into());
        }
    };
    let driver_participant = Participant::Ellipse(driver);
    let other = Participant::from_curve(other_curve);
    driver_participant.validate(tol)?;
    other.validate(tol)?;
    let (driver_low, driver_high) = driver_participant.z_range();
    let (other_low, other_high) = other.z_range();
    let z_tol = 1e-9 * (1.0 + driver_low.abs() + driver_high.abs() + other_low.abs());
    if (driver_low - other_low).abs() > z_tol
        || (driver_high - other_high).abs() > z_tol
        || (driver_low - driver_high).abs() > z_tol
        || (other_low - other_high).abs() > z_tol
    {
        return Err(format!(
            "FILLET.not_coplanar: the {} and the {} are not coplanar (+Z)",
            driver_participant.label(),
            other.label()
        ));
    }
    let mut best: Option<EllipseCandidate> = None;
    let mut considered = false;
    const SAMPLES: usize = 720;
    for sign in [1.0f64, -1.0] {
        let mut values: Vec<(f64, f64)> = Vec::with_capacity(SAMPLES + 1);
        for index in 0..=SAMPLES {
            let theta = driver.start + driver.sweep * (index as f64) / (SAMPLES as f64);
            let residual = ellipse_residual(driver, other, theta, sign, radius).unwrap_or(f64::NAN);
            values.push((theta, residual));
        }
        let mut roots: Vec<f64> = Vec::new();
        for window in values.windows(2) {
            let (low_theta, low) = window[0];
            let (high_theta, high) = window[1];
            if !low.is_finite() || !high.is_finite() {
                continue;
            }
            if low == 0.0 && high == 0.0 {
                roots.push(low_theta);
                continue;
            }
            if low * high < 0.0 {
                if let Some(root) =
                    bisect_residual(driver, other, sign, radius, low_theta, high_theta)
                {
                    roots.push(root);
                }
            }
        }
        // A tangential (double) root does not change sign; refine every local
        // minimum of |residual| and accept it only when it actually vanishes.
        for index in 1..values.len() - 1 {
            let (low_theta, low) = values[index - 1];
            let (_, middle) = values[index];
            let (high_theta, high) = values[index + 1];
            if !low.is_finite() || !middle.is_finite() || !high.is_finite() {
                continue;
            }
            if middle.abs() <= low.abs() && middle.abs() <= high.abs() {
                if let Some(root) =
                    refine_minimum(driver, other, sign, radius, low_theta, high_theta)
                {
                    roots.push(root);
                }
            }
        }
        for theta in roots {
            if let Some(candidate) = ellipse_candidate(
                driver,
                other,
                theta,
                sign,
                radius,
                tol,
                pick_driver,
                pick_other,
            ) {
                considered = true;
                if best
                    .as_ref()
                    .is_none_or(|current| candidate.outranks(current))
                {
                    best = Some(candidate);
                }
            }
        }
    }
    let Some(chosen) = best else {
        return Err(if considered {
            "FILLET.tangent_outside_arc: the chosen fillet touches a picked curve outside its \
             span (or the pick is off it); nothing is written"
                .into()
        } else {
            "FILLET.no_tangent_arc: no arc of this radius is tangent to both objects".into()
        });
    };
    if !chosen.feasible {
        return Err(
            "FILLET.tangent_outside_arc: the chosen fillet touches a picked curve outside its \
             span (or the pick is off it); nothing is written"
                .into(),
        );
    }
    let first_update = if trim {
        match driver_is_first {
            true => driver_participant.trim(chosen.first_tangent, pick_driver, tol)?,
            false => other.trim(chosen.second_tangent, pick_other, tol)?,
        }
    } else {
        None
    };
    let second_update = if trim {
        match driver_is_first {
            true => other.trim(chosen.second_tangent, pick_other, tol)?,
            false => driver_participant.trim(chosen.first_tangent, pick_driver, tol)?,
        }
    } else {
        None
    };
    let (first_tangent, second_tangent) = match driver_is_first {
        true => (chosen.first_tangent, chosen.second_tangent),
        false => (chosen.second_tangent, chosen.first_tangent),
    };
    let start = (first_tangent.y - chosen.centre.y).atan2(first_tangent.x - chosen.centre.x);
    let end = (second_tangent.y - chosen.centre.y).atan2(second_tangent.x - chosen.centre.x);
    let sweep = normalize_signed(end - start);
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return Err("FILLET.degenerate_arc: the tangent arc has no sweep".into());
    }
    Ok(TwoLinePlan {
        first: first_update,
        second: second_update,
        inserted: Some(arc(chosen.centre, radius, start, sweep)),
    })
}

/// Build a candidate from one ellipse parameter, or `None` when the other curve
/// is not exactly `R` away (the numeric acceptance tolerance).
#[allow(clippy::too_many_arguments)]
fn ellipse_candidate(
    driver: EllipseCurve,
    other: Participant,
    theta: f64,
    sign: f64,
    radius: f64,
    tol: f64,
    pick_driver: Point3,
    pick_other: Point3,
) -> Option<EllipseCandidate> {
    let mut point = driver.point(theta);
    let normal = driver.normal(theta)?;
    let mut centre = add(point, scale(normal, sign * radius));
    let mut closest = other.closest(centre)?;
    let accept = 1e-7 * (1.0 + centre.x.abs() + centre.y.abs() + radius);
    if (closest.distance - radius).abs() > accept {
        return None;
    }
    // Alternating projection polish: pull the centre onto the exact `R`-offset of
    // the other curve, then onto the ellipse's own signed `R`-offset. Near a
    // tangency this converges to machine precision, unlike refining the (there
    // quadratic) residual itself, whose root position is limited to ~√ε.
    for _ in 0..24 {
        let direction = sub(centre, closest.point);
        let separation = length(direction);
        if !separation.is_finite() || separation <= 1e-12 {
            break;
        }
        centre = add(closest.point, scale(direction, radius / separation));
        let nearest = closest_on_ellipse(driver, centre)?;
        let normal = driver.normal(nearest.parameter)?;
        let projected = add(nearest.point, scale(normal, sign * radius));
        let moved = distance(projected, centre);
        point = nearest.point;
        centre = projected;
        closest = other.closest(centre)?;
        if moved <= 1e-14 * (1.0 + length(centre)) {
            break;
        }
    }
    if (closest.distance - radius).abs() > 1e-9 * (1.0 + radius) {
        return None;
    }
    // A zero-sweep candidate is degenerate; the final plan recomputes the
    // initial/final tangent angles in first/second pick order.
    let start = (point.y - centre.y).atan2(point.x - centre.x);
    let end = (closest.point.y - centre.y).atan2(closest.point.x - centre.x);
    let sweep = normalize_signed(end - start);
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return None;
    }
    let feasible = ellipse_accepts(driver, point, pick_driver, tol)
        && other.accepts(closest.point, pick_other, tol);
    Some(EllipseCandidate {
        score: distance(point, pick_driver) + distance(closest.point, pick_other),
        feasible,
        centre,
        first_tangent: point,
        second_tangent: closest.point,
    })
}

/// Bisect a sign-changing residual bracket.
fn bisect_residual(
    driver: EllipseCurve,
    other: Participant,
    sign: f64,
    radius: f64,
    low: f64,
    high: f64,
) -> Option<f64> {
    let residual =
        |theta: f64| ellipse_residual(driver, other, theta, sign, radius).unwrap_or(f64::NAN);
    let (mut low, mut high) = (low, high);
    let mut low_value = residual(low);
    let high_value = residual(high);
    if !low_value.is_finite() || !high_value.is_finite() {
        return None;
    }
    for _ in 0..80 {
        let middle = 0.5 * (low + high);
        let middle_value = residual(middle);
        if !middle_value.is_finite() {
            return None;
        }
        if middle_value == 0.0 {
            return Some(middle);
        }
        if low_value * middle_value < 0.0 {
            high = middle;
        } else {
            low = middle;
            low_value = middle_value;
        }
    }
    Some(0.5 * (low + high))
}

/// Refine a local minimum of `|residual|` and accept it when it vanishes.
fn refine_minimum(
    driver: EllipseCurve,
    other: Participant,
    sign: f64,
    radius: f64,
    low: f64,
    high: f64,
) -> Option<f64> {
    let residual =
        |theta: f64| ellipse_residual(driver, other, theta, sign, radius).unwrap_or(f64::NAN);
    // A tangential solution is a **minimum** of the residual: the signed
    // residual itself is quadratic there, so solving `residual = 0` locates θ
    // only to ~√ε. Newton on its derivative (`residual′ = 0`) is transverse and
    // recovers the tangency to machine precision; the residual is then checked
    // against the geometric tolerance.
    let mut theta = 0.5 * (low + high);
    for _ in 0..20 {
        let step_size = 1e-6 * (1.0 + theta.abs());
        let middle = residual(theta);
        let plus = residual(theta + step_size);
        let minus = residual(theta - step_size);
        if !middle.is_finite() || !plus.is_finite() || !minus.is_finite() {
            break;
        }
        let first = (plus - minus) / (2.0 * step_size);
        let second = (plus - 2.0 * middle + minus) / (step_size * step_size);
        if !first.is_finite() || !second.is_finite() || second <= 0.0 {
            break;
        }
        let step = first / second;
        if !step.is_finite() {
            break;
        }
        theta -= step;
        if step.abs() <= 1e-15 * (1.0 + theta.abs()) {
            break;
        }
    }
    let minimum = residual(theta);
    let point = driver.point(theta);
    let accept = 1e-9 * (1.0 + point.x.abs() + point.y.abs() + radius);
    (minimum.is_finite() && minimum.abs() <= accept).then_some(theta)
}
