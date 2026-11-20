//! FILLET `R = 0`: the sharp corner, for every planar pair this build can intersect.
//!
//! Declared from [`super`] with an explicit `#[path]`. Pure: no database, no
//! session, no history — the command layer resolves the two picks, calls
//! [`plan_zero_radius`] and commits the returned plan in one transaction.
//!
//! ## The contract
//!
//! `R = 0` has no arc: the picked pair is trimmed to the intersection **nearest the
//! two picks** and nothing is inserted. Each participant is then cut by its own
//! documented trim rule on the pick side (the `R > 0` rule, unchanged):
//!
//! * LINE — trimmed **or extended** to the corner (the line/line form's rule);
//! * ARC — trimmed at the corner on the pick side;
//! * ELLIPSE / elliptical arc — `start`/`sweep` rewritten, never re-approximated;
//! * SPLINE — the exact B-spline sub-curve (`cad_geometry::sub_curve`);
//! * CIRCLE — **never trimmed**. See "Circles" below.
//!
//! Nearest-pick score is `|corner − pick₁| + |corner − pick₂|`, the same rule the
//! curve↔curve and ellipse forms use for their candidates. An exact tie keeps the
//! first intersection in a deterministic order.
//!
//! ## Where the intersections come from
//!
//! Two sources, both already used by other commands:
//!
//! * **LINE/CIRCLE/ARC pairs** — [`cad_geometry::DefaultGeometryEngine`]'s
//!   `intersect_local`, the same call TRIM/EXTEND make: analytic conic/conic and
//!   conic/segment solutions (`cad_geometry::intersect`), so the corner is exact
//!   and an ARC/ARC or LINE/ARC pair only reports crossings inside both stored
//!   spans. The LINE side is first widened to a probe on its own infinite line
//!   (the reach rule of `trim_cuts::line_probe`, restated in [`line_probe`] because
//!   FILLET has one other object instead of a boundary set), because the `R = 0`
//!   contract is "trimmed **or extended** until they intersect".
//! * **Pairs containing an ELLIPSE or SPLINE** — the same one-parameter residual
//!   scan the `R > 0` numeric forms use ([`super::corner_roots`] + the shared
//!   participant's `signed` carrier function), driven by the ellipse/spline's own
//!   parameter. This is deliberate: `cad_geometry`'s analytic conic intersection has
//!   no ellipse↔(circle/arc/ellipse) arm, so the engine would answer those pairs
//!   with its documented **sampled** fallback (chord-height accurate), while the
//!   residual scan bisects to the parameter tolerance on the driver curve. A
//!   SPLINE↔SPLINE pair has no signed carrier on either side, so its crossings are
//!   the minima of the point-to-curve distance, refined by a golden-section search
//!   and accepted within [`NUMERIC_RELATIVE`] of the point scale (declared numeric,
//!   not exact — the same statement the spline form makes).
//!
//! ## Circles
//!
//! A CIRCLE is a **closed** curve: it has no ends, so cutting it at **one** point is
//! the identity (the pick-side piece is the whole circle, `sweep = 2π`) — the same
//! reason TRIM refuses to leave an arc when a closed target has fewer than two cuts.
//! An `R > 0` fillet therefore meets a circle tangentially and leaves it whole, and
//! a circle paired with a line/arc/ellipse/spline at `R = 0` is likewise untouched:
//! the corner is written on the other object.
//!
//! A pair of **two circles** is the one case that gives a circle *two* crossings, and
//! then the pick-side piece between them is a genuine arc. That case goes through
//! TRIM's own two-cut plan — `TargetCurve`'s CIRCLE → full-turn-ARC target,
//! `plan_trim` with its `periodic_piece` pick-side rule and `with_span` rewrite —
//! and **both** circles are replaced by arcs in place
//! ([`GeometryUpdate::CircleToArc`]): the entity keeps its centre/normal/radius, id,
//! layer, space, draw order and source `type_key`, exactly the in-place CIRCLE→ARC
//! change TRIM already makes. Tangent circles (one crossing) and disjoint circles
//! have no pick-side piece and keep named refusals
//! (`FILLET.tangent_circles` / `FILLET.no_corner_intersection`).
//!
//! ## Refusals (all named, all before the first write)
//!
//! `FILLET.trim_off_no_arc` (with trim off the sharp corner creates nothing),
//! `FILLET.not_coplanar`, `FILLET.no_corner_intersection` (the picked objects do not
//! meet; a SPLINE is never extended past its knot domain and an ARC/ELLIPSE is
//! never extended either), `FILLET.tangent_circles` (one crossing: a single cut
//! leaves a closed circle whole), `FILLET.circle_arc_conversion` (the CIRCLE→ARC
//! rewrite itself refused), `FILLET.pick_at_tangent` (the pick selects no side of
//! the corner), `FILLET.tangent_outside_arc` / `FILLET.pick_outside_arc` /
//! `FILLET.pick_outside_ellipse` (a picked span that cannot host the corner),
//! `FILLET.degenerate_spline`, `FILLET.no_trim_at_zero_radius`.

use super::super::helpers::distance;
use super::super::*;
use super::corner_ellipse::{trim_ellipse, EllipseCurve};
use super::corner_geometry::{
    pt, retarget_pick_side, trim_arc_to, Curve, GeometryUpdate, TwoLinePlan,
};
use super::corner_participant::Other;
use super::corner_roots::{minimise_scalar, scan_roots};
use super::corner_spline::SplineCurve;
use cad_geometry::{add, length, scale, sub, DefaultGeometryEngine, GeometryEngine};

/// Parameter samples of the profile corner scan (the `R > 0` forms' budget).
const SAMPLES: usize = 720;

/// Golden-section iterations of the spline↔spline corner search.
const CORNER_ITERATIONS: usize = 200;

/// Relative acceptance of a numeric spline↔spline corner (declared, not exact).
const NUMERIC_RELATIVE: f64 = 1e-7;

/// Plan the `R = 0` sharp corner of two picked objects.
///
/// Returns the two pick-side trims and **no** inserted entity; each refusal is
/// named and leaves the drawing untouched.
pub(super) fn plan_zero_radius(
    first: Curve,
    pick_first: Point3,
    second: Curve,
    pick_second: Point3,
    trim: bool,
    tol: f64,
) -> Result<TwoLinePlan, String> {
    if !trim {
        return Err(
            "FILLET.trim_off_no_arc: with trim off the R=0 sharp corner creates nothing".into(),
        );
    }
    // A profiled participant drives the residual scan (spline first, so a
    // spline↔ellipse pair is driven by the spline exactly like `R > 0`).
    match (first, second) {
        (Curve::Spline(spline), other) => profile_corner(
            Profile::Spline(spline),
            pick_first,
            other,
            pick_second,
            tol,
            true,
        ),
        (other, Curve::Spline(spline)) => profile_corner(
            Profile::Spline(spline),
            pick_second,
            other,
            pick_first,
            tol,
            false,
        ),
        (Curve::Ellipse(ellipse), other) => profile_corner(
            Profile::Ellipse(ellipse),
            pick_first,
            other,
            pick_second,
            tol,
            true,
        ),
        (other, Curve::Ellipse(ellipse)) => profile_corner(
            Profile::Ellipse(ellipse),
            pick_second,
            other,
            pick_first,
            tol,
            false,
        ),
        (first, second) => conic_corner(first, pick_first, second, pick_second, tol),
    }
}

/// The profiled driver of the corner scan.
enum Profile {
    Ellipse(EllipseCurve),
    Spline(SplineCurve),
}

impl Profile {
    fn label(&self) -> &'static str {
        match self {
            Self::Ellipse(_) => "ELLIPSE",
            Self::Spline(_) => "SPLINE",
        }
    }

    fn point(&self, parameter: f64) -> Point3 {
        match self {
            Self::Ellipse(ellipse) => ellipse.point(parameter),
            Self::Spline(spline) => spline.point(parameter),
        }
    }

    fn z(&self) -> f64 {
        match self {
            Self::Ellipse(ellipse) => ellipse.center.z,
            Self::Spline(spline) => spline.z(),
        }
    }

    /// The driver's own parameter interval, low → high (an ellipse may store a
    /// negative sweep, so the interval is ordered before the scan).
    fn parameter_range(&self) -> (f64, f64) {
        match self {
            Self::Ellipse(ellipse) => {
                let (start, end) = (ellipse.start, ellipse.start + ellipse.sweep);
                if start <= end {
                    (start, end)
                } else {
                    (end, start)
                }
            }
            Self::Spline(spline) => {
                let (t_min, t_max) = spline.domain();
                (t_min, t_max)
            }
        }
    }

    /// The doc comment of a refusal names the driver kind.
    fn trim(
        &self,
        parameter: f64,
        point: Point3,
        pick: Point3,
        tol: f64,
    ) -> Result<Option<GeometryUpdate>, String> {
        match self {
            Self::Ellipse(ellipse) => trim_ellipse(*ellipse, point, pick, tol).map(Some),
            Self::Spline(spline) => spline.trim_keep_pick(parameter, pick, tol),
        }
    }
}

/// One resolved corner of the sharp-corner search.
struct Corner {
    profile_parameter: f64,
    other_parameter: f64,
    point: Point3,
    score: f64,
}

/// Plan `R = 0` for a pair with an ELLIPSE or SPLINE participant.
fn profile_corner(
    profile: Profile,
    pick_profile: Point3,
    other_curve: Curve,
    pick_other: Point3,
    tol: f64,
    profile_is_first: bool,
) -> Result<TwoLinePlan, String> {
    let other = Other::from_curve(other_curve);
    other.validate(tol)?;
    let (other_low, other_high) = other.z_extent();
    let z_tol = 1e-9 * (1.0 + profile.z().abs() + other_low.abs() + other_high.abs());
    if (profile.z() - other_low).abs() > z_tol || (profile.z() - other_high).abs() > z_tol {
        return Err(format!(
            "FILLET.not_coplanar: the {} and the {} are not coplanar (+Z)",
            profile.label(),
            other.label()
        ));
    }
    let (low, high) = profile.parameter_range();
    let score = |point: Point3| distance(point, pick_profile) + distance(point, pick_other);
    let mut corners: Vec<Corner> = Vec::new();
    if matches!(other, Other::Spline(_)) {
        // Neither side has a signed carrier: a crossing is a zero of the
        // (non-negative) point-to-curve distance, i.e. a local minimum that touches
        // zero. Sample, then refine the bracket by golden section.
        for parameter in distance_corner_minima(&profile, &other, low, high) {
            let point = profile.point(parameter);
            let Some(closest) = other.closest(point) else {
                continue;
            };
            let accept = numeric_acceptance(point, tol);
            if !closest.distance.is_finite() || closest.distance > accept {
                continue;
            }
            push_corner(
                &mut corners,
                Corner {
                    profile_parameter: parameter,
                    other_parameter: closest.parameter,
                    point,
                    score: score(point),
                },
                accept,
            );
        }
    } else {
        let residual = |parameter: f64| other.signed(profile.point(parameter));
        for parameter in scan_roots(&residual, low, high, SAMPLES) {
            let point = profile.point(parameter);
            // An ARC/ELLIPSE must be met inside its stored span; a LINE may be
            // extended to the corner, and a CIRCLE/SPLINE is its whole carrier.
            if !other.on_carrier_span(point, tol) {
                continue;
            }
            let lead = residual(parameter).map(f64::abs).unwrap_or(f64::INFINITY);
            if !lead.is_finite() || lead > numeric_acceptance(point, tol) {
                continue;
            }
            let Some(closest) = other.closest(point) else {
                continue;
            };
            push_corner(
                &mut corners,
                Corner {
                    profile_parameter: parameter,
                    other_parameter: closest.parameter,
                    point,
                    score: score(point),
                },
                numeric_acceptance(point, tol),
            );
        }
    }
    let Some(chosen) = corners.into_iter().min_by(score_order) else {
        return Err(format!(
            "FILLET.no_corner_intersection: the R=0 sharp corner needs the picked objects to \
             intersect, and the {} and the {} do not (FILLET trims or extends a LINE only; an \
             arc, ellipse or spline is never extended past its stored span)",
            profile.label(),
            other.label()
        ));
    };
    let profile_update = profile.trim(chosen.profile_parameter, chosen.point, pick_profile, tol)?;
    let other_update = other.trim(chosen.point, chosen.other_parameter, pick_other, tol)?;
    if profile_update.is_none() && other_update.is_none() {
        return Err(format!(
            "FILLET.no_trim_at_zero_radius: the R=0 corner meets the {} at its own end and the {} \
             is never trimmed, so nothing would be written",
            profile.label(),
            other.label()
        ));
    }
    let (first, second) = if profile_is_first {
        (profile_update, other_update)
    } else {
        (other_update, profile_update)
    };
    Ok(TwoLinePlan {
        first,
        second,
        inserted: None,
    })
}

/// Plan `R = 0` for a LINE/CIRCLE/ARC pair through the shared geometry engine.
fn conic_corner(
    first: Curve,
    pick_first: Point3,
    second: Curve,
    pick_second: Point3,
    tol: f64,
) -> Result<TwoLinePlan, String> {
    let first_geometry = carrier(&first)?;
    let second_geometry = carrier(&second)?;
    let (first_low, first_high) = conic_z_extent(&first);
    let (second_low, second_high) = conic_z_extent(&second);
    let z_tol = 1e-9 * (1.0 + first_low.abs() + first_high.abs() + second_low.abs());
    if (first_low - second_low).abs() > z_tol || (first_high - second_high).abs() > z_tol {
        return Err(format!(
            "FILLET.not_coplanar: the {} and the {} are not coplanar (+Z)",
            first.label(),
            second.label()
        ));
    }
    // The LINE side is probed on its own infinite line so the corner may be
    // reached by an extension, exactly like the line/line form.
    let first_probe = match &first {
        Curve::Line { start, end } => line_probe(*start, *end, &second_geometry, tol)?,
        _ => first_geometry.clone(),
    };
    let second_probe = match &second {
        Curve::Line { start, end } => line_probe(*start, *end, &first_geometry, tol)?,
        _ => second_geometry,
    };
    let engine = DefaultGeometryEngine;
    let policy = TolerancePolicy::default();
    let hits = engine
        .intersect_local(&first_probe, &second_probe, &policy)
        .map_err(|error| {
            format!(
                "FILLET.intersection_failed: the picked objects could not be intersected: {error}"
            )
        })?;
    // Two CIRCLEs are the one pair whose *neither* object has a single-cut trim
    // form: a closed curve cut once is still the whole curve. They do have **two**
    // crossings, and the pick-side piece between them is the two-cut arc TRIM
    // already leaves on a CIRCLE target, so that path is reused below.
    if matches!(first, Curve::Circle { .. }) && matches!(second, Curve::Circle { .. }) {
        let finite = hits.iter().filter(|hit| is_finite_point(**hit)).count();
        return circle_pair_corner(&first, pick_first, &second, pick_second, finite);
    }
    let score = |point: Point3| distance(point, pick_first) + distance(point, pick_second);
    let Some(point) = hits
        .into_iter()
        .filter(|hit| is_finite_point(*hit))
        .min_by(|a, b| {
            score(*a)
                .partial_cmp(&score(*b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    else {
        return Err(format!(
            "FILLET.no_corner_intersection: the R=0 sharp corner needs the picked objects to \
             intersect, and the {} and the {} do not (FILLET trims or extends a LINE only; an \
             arc, ellipse or spline is never extended past its stored span)",
            first.label(),
            second.label()
        ));
    };
    let first_update = conic_trim(&first, point, pick_first, tol)?;
    let second_update = conic_trim(&second, point, pick_second, tol)?;
    if first_update.is_none() && second_update.is_none() {
        return Err(
            "FILLET.no_trim_at_zero_radius: the R=0 sharp corner would rewrite neither picked \
             object, so nothing would be written"
                .into(),
        );
    }
    Ok(TwoLinePlan {
        first: first_update,
        second: second_update,
        inserted: None,
    })
}

/// Plan the `R = 0` corner of two CIRCLEs.
///
/// Both are closed curves, so one cut point is the identity — but two intersecting
/// circles give **two** crossing points, and the piece between them that contains
/// the pick is a genuine arc. That is exactly the two-cut case TRIM resolves for a
/// CIRCLE target, so the conversion is **not** re-implemented here: each circle is
/// handed to [`TargetCurve::new_trim_extend`] (which stores a CIRCLE as its
/// full-turn ARC target), the other circle becomes the `BoundarySet`, and
/// [`plan_trim`] — with its `periodic_piece` pick-side rule and `with_span`
/// CIRCLE→ARC rewrite — returns the arc. Only the start/sweep it produces are
/// carried back, as [`GeometryUpdate::CircleToArc`]; the entity keeps its own
/// centre/normal/radius, id, layer, space, draw order and source `type_key`.
///
/// `crossings` is the engine's intersection count, so the two refusal cases are
/// named before any planning: nothing (`FILLET.no_corner_intersection`) and a
/// single tangency (`FILLET.tangent_circles`, where one cut point would leave both
/// circles whole).
fn circle_pair_corner(
    first: &Curve,
    pick_first: Point3,
    second: &Curve,
    pick_second: Point3,
    crossings: usize,
) -> Result<TwoLinePlan, String> {
    let (first_geometry, second_geometry) = (carrier(first)?, carrier(second)?);
    if crossings == 0 {
        return Err(
            "FILLET.no_corner_intersection: the R=0 sharp corner needs the two picked circles to \
             intersect, and these circles are disjoint"
                .into(),
        );
    }
    if crossings == 1 {
        return Err(
            "FILLET.tangent_circles: the two picked circles are tangent (a single intersection \
             point); one cut point leaves a closed circle whole, so R=0 has no corner to write \
             (a non-zero radius inserts a tangent arc instead)"
                .into(),
        );
    }
    let first_update = cut_circle(&first_geometry, pick_first, &second_geometry)?;
    let second_update = cut_circle(&second_geometry, pick_second, &first_geometry)?;
    Ok(TwoLinePlan {
        first: Some(first_update),
        second: Some(second_update),
        inserted: None,
    })
}

/// Cut one CIRCLE at its two crossings with `boundary`, keeping the pick side.
///
/// This is TRIM's own plan, unmodified: `plan_trim` projects the crossings onto the
/// circle's parameter, keeps the periodic piece containing the pick's projection
/// and rewrites the entity geometry with `with_span`. The result must be an ARC —
/// anything else is reported instead of being written.
fn cut_circle(
    target: &SemanticGeometry,
    pick: Point3,
    boundary: &SemanticGeometry,
) -> Result<GeometryUpdate, String> {
    let policy = TolerancePolicy::default();
    let curve = TargetCurve::new_trim_extend(target, "FILLET", "a CIRCLE")
        .map_err(|error| format!("FILLET.circle_arc_conversion: {error}"))?;
    let mut boundaries = BoundarySet::default();
    collect_boundaries(boundary, &mut boundaries, &policy, "FILLET")
        .map_err(|error| format!("FILLET.circle_arc_conversion: {error}"))?;
    if boundaries.is_empty() {
        return Err(
            "FILLET.no_corner_intersection: the R=0 corner needs two crossings, and the other \
             picked object carried no boundary geometry to cut against"
                .into(),
        );
    }
    let resolution = plan_trim(&curve, pick, &boundaries, &policy, cut_tolerance())
        .map_err(|reason| format!("FILLET.circle_arc_conversion: {}", reason.into_cad_error()))?;
    match resolution.plan {
        TrimPlan::Replace(geometry) => match *geometry {
            SemanticGeometry::Arc { start, sweep, .. } => {
                Ok(GeometryUpdate::CircleToArc { start, sweep })
            }
            other => Err(format!(
                "FILLET.circle_arc_conversion: the two-cut CIRCLE target came back as {}",
                class_name(&other)
            )),
        },
        TrimPlan::Delete => Err(
            "FILLET.no_corner_intersection: the R=0 corner needs two crossings, and this picked \
             CIRCLE would be reduced to nothing"
                .into(),
        ),
    }
}

/// The finite carrier geometry a conic participant is intersected as.
fn carrier(curve: &Curve) -> Result<SemanticGeometry, String> {
    match curve {
        Curve::Line { start, end } => Ok(SemanticGeometry::Line {
            start: *start,
            end: *end,
        }),
        Curve::Circle { center, radius } => Ok(SemanticGeometry::Circle {
            center: *center,
            normal: pt(0.0, 0.0, 1.0),
            radius: *radius,
        }),
        Curve::Arc {
            center,
            radius,
            start,
            sweep,
        } => Ok(SemanticGeometry::Arc {
            center: *center,
            normal: pt(0.0, 0.0, 1.0),
            radius: *radius,
            start: *start,
            sweep: *sweep,
        }),
        Curve::Ellipse(_) | Curve::Spline(_) => Err(format!(
            "FILLET.unsupported_pair: the conic sharp-corner path needs LINE/CIRCLE/ARC, got {}",
            curve.label()
        )),
    }
}

fn conic_z_extent(curve: &Curve) -> (f64, f64) {
    match curve {
        Curve::Line { start, end } => (start.z.min(end.z), start.z.max(end.z)),
        Curve::Circle { center, .. } | Curve::Arc { center, .. } => (center.z, center.z),
        Curve::Ellipse(ellipse) => (ellipse.center.z, ellipse.center.z),
        Curve::Spline(spline) => (spline.z(), spline.z()),
    }
}

/// Trim one conic participant at the corner (a CIRCLE is never trimmed).
fn conic_trim(
    curve: &Curve,
    corner: Point3,
    pick: Point3,
    tol: f64,
) -> Result<Option<GeometryUpdate>, String> {
    match curve {
        Curve::Line { start, end } => {
            retarget_pick_side((*start, *end), pick, corner, tol, "FILLET").map(Some)
        }
        Curve::Arc {
            center,
            start,
            sweep,
            ..
        } => trim_arc_to(*center, *start, *sweep, corner, pick, tol).map(Some),
        Curve::Circle { .. } => Ok(None),
        Curve::Ellipse(_) | Curve::Spline(_) => Err(format!(
            "FILLET.unsupported_pair: the conic sharp-corner path cannot trim {}",
            curve.label()
        )),
    }
}

/// A segment on the line's own infinite line, long enough to contain `other`.
///
/// The reach rule is `trim_cuts::line_probe`'s (every crossing with `other` lies
/// inside its bounding box, and `(|box_center − mid| + diag + len)·1.5 + 1.0` covers
/// that with a world unit to spare); it is restated here because FILLET has one
/// other object instead of a TRIM/EXTEND boundary set. Overshooting is harmless:
/// only a real crossing of the other object survives the intersection call.
fn line_probe(
    start: Point3,
    end: Point3,
    other: &SemanticGeometry,
    tol: f64,
) -> Result<SemanticGeometry, String> {
    let direction = sub(end, start);
    let len = length(direction);
    if !len.is_finite() || len <= tol {
        return Err("FILLET.degenerate_line: the picked LINE has no direction".into());
    }
    let unit = scale(direction, 1.0 / len);
    let mut bounds = cad_geometry::BoundsAccumulator::new();
    bounds.add_geometry(other);
    let bounds = bounds.finish().ok_or_else(|| {
        "FILLET.no_corner_intersection: the other picked object has no finite extent".to_string()
    })?;
    let box_center = scale(add(bounds.min, bounds.max), 0.5);
    let diagonal = length(sub(bounds.max, bounds.min));
    let midpoint = scale(add(start, end), 0.5);
    let reach = (length(sub(box_center, midpoint)) + diagonal + len) * 1.5 + 1.0;
    Ok(SemanticGeometry::Line {
        start: sub(start, scale(unit, reach)),
        end: add(end, scale(unit, reach)),
    })
}

/// The parameter candidates of a spline↔spline corner: local minima of the
/// point-to-curve distance, refined by golden section.
fn distance_corner_minima(profile: &Profile, other: &Other, low: f64, high: f64) -> Vec<f64> {
    if !low.is_finite() || !high.is_finite() || high <= low {
        return Vec::new();
    }
    let distance_at = |parameter: f64| {
        let point = profile.point(parameter);
        other
            .closest(point)
            .map(|closest| closest.distance)
            .unwrap_or(f64::INFINITY)
    };
    let span = high - low;
    let values: Vec<f64> = (0..=SAMPLES)
        .map(|index| distance_at(low + span * (index as f64) / (SAMPLES as f64)))
        .collect();
    let mut minima = Vec::new();
    for index in 1..values.len().saturating_sub(1) {
        let (previous, middle, next) = (values[index - 1], values[index], values[index + 1]);
        if !middle.is_finite() || middle > previous || middle > next {
            continue;
        }
        let bracket_low = low + span * ((index - 1) as f64) / (SAMPLES as f64);
        let bracket_high = low + span * ((index + 1) as f64) / (SAMPLES as f64);
        minima.push(minimise_scalar(
            &distance_at,
            bracket_low,
            bracket_high,
            CORNER_ITERATIONS,
        ));
    }
    minima
}

/// World acceptance of a numeric corner, relative to the point's own scale.
fn numeric_acceptance(point: Point3, tol: f64) -> f64 {
    (NUMERIC_RELATIVE * (1.0 + point.x.abs() + point.y.abs() + point.z.abs())).max(tol)
}

/// Order two corners by their picks score, keeping the first on an exact tie.
fn score_order(a: &Corner, b: &Corner) -> std::cmp::Ordering {
    a.score
        .partial_cmp(&b.score)
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Keep a corner unless the same point was already recorded within `accept`.
fn push_corner(corners: &mut Vec<Corner>, corner: Corner, accept: f64) {
    if corners
        .iter()
        .any(|existing| distance(existing.point, corner.point) <= accept.max(1e-12))
    {
        return;
    }
    corners.push(corner);
}
