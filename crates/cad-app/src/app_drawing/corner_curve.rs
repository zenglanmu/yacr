//! Pure geometry for the FILLET curve↔curve forms (CIRCLE/ARC pairs).
//!
//! Declared from [`super`] with an explicit `#[path]` beside the line forms so
//! neither file outgrows the 1000-line ceiling. Everything here is pure: the
//! command layer resolves the two picks, calls this planner and commits the
//! returned plan in one transaction.
//!
//! ## Construction (declared v1)
//!
//! For two curves with centres `c₁, c₂`, radii `r₁, r₂` and the session fillet
//! radius `R`, a fillet circle tangent to curve *i* has its centre at the signed
//! distance `dᵢ ∈ {rᵢ + R, rᵢ − R}` from `cᵢ` (external / internal tangency).
//! The four sign combinations give four offset circles whose pairwise
//! intersections are the candidate fillet centres. For a candidate, the tangent
//! point on curve *i* is the curve point on the centre line at the signed offset,
//! and the inserted ARC is the **minor arc** between the two tangent points: its
//! signed sweep is the short way round, so `|sweep| ≤ π` holds by construction and
//! the sign follows the solution arrangement (nothing is forced counter-clockwise).
//!
//! The chosen candidate prefers one whose tangent points lie on both picked arcs,
//! then the one whose tangent points are nearest the two picks (AutoCAD's
//! multiple-solution cycling is NOT IMPLEMENTED). Exact score ties keep the first
//! candidate in loop order, and [`circle_circle_intersections`] returns the
//! `−u_y` intersection first (for a `+X` centre baseline that is the `+Y`
//! solution), so the declared winner is deterministic. A candidate whose tangent
//! point falls outside a selected ARC is **never** committed, even with trim off:
//! the named `FILLET.tangent_outside_arc` refusal wins over a write on the wrong
//! side.
//!
//! Only the `+Z` work plane is implemented: a CIRCLE/ARC whose stored `normal` is
//! not `+Z` (tilted or `−Z`) is refused by name (`FILLET.unsupported_plane`)
//! before any candidate is built, because imported angles live in that normal's
//! frame. Differing entity Z planes stay `FILLET.not_coplanar`.
//!
//! A CIRCLE participant is **never trimmed** (matching the line↔circle form); an
//! ARC participant is trimmed at its tangent point on the pick side. Every
//! refusal is resolved before the first write, so a refusal changes nothing.

use super::super::helpers::{distance, is_finite_point, normalize_signed};
use super::super::*;
use super::corner_geometry::{
    arc, arc_position, pt, trim_arc_to, Curve, GeometryUpdate, TwoLinePlan,
};

/// One CIRCLE/ARC participant of a curve↔curve fillet.
#[derive(Debug, Clone, Copy)]
pub(super) struct RoundCurve {
    center: Point3,
    radius: f64,
    /// `Some((start, sweep))` when the participant is a finite ARC.
    arc: Option<(f64, f64)>,
}

impl RoundCurve {
    /// Resolve a FILLET pick to a round participant (`None` for a LINE, which the
    /// line forms own). The caller ([`super::corner_geometry::fillet_curve`]) has
    /// already refused a participant whose stored `normal` is not `+Z`, so the
    /// stored angles here are `+Z` plane angles.
    pub(super) fn resolve(curve: Curve) -> Option<Self> {
        match curve {
            Curve::Circle { center, radius } => Some(Self {
                center,
                radius,
                arc: None,
            }),
            Curve::Arc {
                center,
                radius,
                start,
                sweep,
            } => Some(Self {
                center,
                radius,
                arc: Some((start, sweep)),
            }),
            Curve::Line { .. } => None,
        }
    }
}

/// Refuse a CIRCLE/ARC whose stored `normal` is not the `+Z` work plane.
///
/// Imported circles and arcs keep an arbitrary extrusion normal, and their stored
/// `start`/`sweep` angles are in that normal's frame. Reinterpreting such angles
/// as `+Z` angles would compute tangencies and trims against the wrong points, so
/// a tilted or `−Z` normal is a named refusal, never a silent reinterpretation.
/// Shared by the curve↔curve form and the line↔circle form (both go through
/// [`super::corner_geometry::fillet_curve`]).
pub(super) fn positive_z_normal(normal: &Point3, kind: &str) -> CadResult<()> {
    if !is_finite_point(*normal) || (normal.z - 1.0).abs() > 1e-9 {
        return Err(CadError::Unsupported(format!(
            "FILLET.unsupported_plane: the picked {kind} is not on the +Z work plane \
             (normal=({}, {}, {})); tilted and -Z curves are NOT IMPLEMENTED",
            normal.x, normal.y, normal.z
        )));
    }
    Ok(())
}

/// One candidate fillet circle of a curve↔curve plan.
struct Candidate {
    score: f64,
    feasible: bool,
    centre: Point3,
    first_tangent: Point3,
    second_tangent: Point3,
    start: f64,
    sweep: f64,
}

impl Candidate {
    /// Prefer a candidate whose tangent points lie on both picked arcs, then the
    /// one nearest the two picks. An exact score tie keeps the incumbent (the
    /// first candidate in loop order), so the winner is deterministic.
    fn outranks(&self, other: &Self) -> bool {
        match (self.feasible, other.feasible) {
            (true, false) => true,
            (false, true) => false,
            _ => self.score < other.score,
        }
    }
}

/// Plan FILLET of two CIRCLE/ARC participants (the curve↔curve form).
pub(super) fn plan_fillet_curve_curve(
    first: RoundCurve,
    pick_first: Point3,
    second: RoundCurve,
    pick_second: Point3,
    radius: f64,
    trim: bool,
    tol: f64,
) -> Result<TwoLinePlan, String> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(
            "FILLET.radius_zero: a curve↔curve fillet needs a positive radius (R=0 has no \
             sharp corner between two curves)"
                .into(),
        );
    }
    if !first.radius.is_finite()
        || first.radius <= tol
        || !second.radius.is_finite()
        || second.radius <= tol
    {
        return Err("FILLET.degenerate_circle: a picked curve has no usable radius".into());
    }
    let z_tol = 1e-9 * (1.0 + first.center.z.abs() + second.center.z.abs());
    if (first.center.z - second.center.z).abs() > z_tol {
        return Err("FILLET.not_coplanar: the two curves are not coplanar (+Z)".into());
    }
    let separation = (second.center.x - first.center.x).hypot(second.center.y - first.center.y);
    if !separation.is_finite() {
        return Err("FILLET.non_finite: the two picked curves must be finite".into());
    }
    if separation <= tol {
        return Err("FILLET.concentric_curves: concentric curves have no tangent fillet".into());
    }
    let mut best: Option<Candidate> = None;
    for signed_first in [first.radius + radius, first.radius - radius] {
        for signed_second in [second.radius + radius, second.radius - radius] {
            let distance_first = signed_first.abs();
            let distance_second = signed_second.abs();
            if distance_first <= tol || distance_second <= tol {
                continue;
            }
            let centres = circle_circle_intersections(
                first.center,
                distance_first,
                second.center,
                distance_second,
                tol,
            );
            for centre in centres {
                let (Some(first_tangent), Some(second_tangent)) = (
                    tangent_point(first.center, first.radius, centre, signed_first),
                    tangent_point(second.center, second.radius, centre, signed_second),
                ) else {
                    continue;
                };
                let start = (first_tangent.y - centre.y).atan2(first_tangent.x - centre.x);
                let end = (second_tangent.y - centre.y).atan2(second_tangent.x - centre.x);
                let sweep = normalize_signed(end - start);
                if !sweep.is_finite() || sweep.abs() <= 1e-12 {
                    continue;
                }
                let feasible = curve_accepts(first, first_tangent, pick_first, tol)
                    && curve_accepts(second, second_tangent, pick_second, tol);
                let candidate = Candidate {
                    score: distance(first_tangent, pick_first)
                        + distance(second_tangent, pick_second),
                    feasible,
                    centre,
                    first_tangent,
                    second_tangent,
                    start,
                    sweep,
                };
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
        return Err(
            "FILLET.no_tangent_arc: no arc of this radius is tangent to both objects".into(),
        );
    };
    // Feasibility is enforced independently of trim: with trim off the ARC
    // participants are left untouched, but committing a tangent on a picked ARC's
    // supporting circle outside that arc would still be the wrong fillet.
    if !chosen.feasible {
        return Err(
            "FILLET.tangent_outside_arc: the chosen fillet touches a picked ARC outside its \
             span (or the pick is off the arc); nothing is written"
                .into(),
        );
    }
    let first_update = if trim {
        trim_round(first, chosen.first_tangent, pick_first, tol)?
    } else {
        None
    };
    let second_update = if trim {
        trim_round(second, chosen.second_tangent, pick_second, tol)?
    } else {
        None
    };
    Ok(TwoLinePlan {
        first: first_update,
        second: second_update,
        inserted: Some(arc(chosen.centre, radius, chosen.start, chosen.sweep)),
    })
}

/// Whether `tangent` (and the pick) lie on the participant's arc.
fn curve_accepts(curve: RoundCurve, tangent: Point3, pick: Point3, tol: f64) -> bool {
    let Some((start, sweep)) = curve.arc else {
        return true;
    };
    let tangent_position = arc_position(curve.center, start, sweep, tangent);
    let pick_position = arc_position(curve.center, start, sweep, pick);
    tangent_position > tol
        && tangent_position <= sweep.abs() + 1e-9
        && pick_position <= sweep.abs() + 1e-9
}

/// Trim an ARC participant at its tangent point; a CIRCLE is never trimmed.
fn trim_round(
    curve: RoundCurve,
    tangent: Point3,
    pick: Point3,
    tol: f64,
) -> Result<Option<GeometryUpdate>, String> {
    match curve.arc {
        Some((start, sweep)) => {
            trim_arc_to(curve.center, start, sweep, tangent, pick, tol).map(Some)
        }
        None => Ok(None),
    }
}

/// The tangent point on a circle of `radius` about `center` for a fillet centre
/// at the signed offset `signed_distance` (`+` external, `-` enclosing).
fn tangent_point(
    center: Point3,
    radius: f64,
    fillet_centre: Point3,
    signed_distance: f64,
) -> Option<Point3> {
    let dx = fillet_centre.x - center.x;
    let dy = fillet_centre.y - center.y;
    let separation = dx.hypot(dy);
    if !separation.is_finite()
        || separation <= 1e-12
        || !signed_distance.is_finite()
        || signed_distance == 0.0
    {
        return None;
    }
    let factor = radius / signed_distance;
    let point = pt(center.x + dx * factor, center.y + dy * factor, center.z);
    is_finite_point(point).then_some(point)
}

/// The intersections of two circles in the `+Z` work plane (0, 1 or 2 points).
///
/// The returned order is deterministic: the `−u_y` point comes first, i.e. for a
/// `+X` centre baseline the `+Y` solution is first. Exact score ties in the
/// candidate loop therefore keep that solution.
fn circle_circle_intersections(
    first: Point3,
    first_radius: f64,
    second: Point3,
    second_radius: f64,
    tol: f64,
) -> Vec<Point3> {
    let dx = second.x - first.x;
    let dy = second.y - first.y;
    let separation = dx.hypot(dy);
    if !separation.is_finite() || separation <= tol {
        return Vec::new();
    }
    if separation > first_radius + second_radius + tol
        || separation < (first_radius - second_radius).abs() - tol
    {
        return Vec::new();
    }
    let along = (first_radius * first_radius - second_radius * second_radius
        + separation * separation)
        / (2.0 * separation);
    let height_squared = first_radius * first_radius - along * along;
    let height = if height_squared <= 0.0 {
        if height_squared < -tol * (1.0 + first_radius + second_radius) {
            return Vec::new();
        }
        0.0
    } else {
        height_squared.sqrt()
    };
    if !along.is_finite() || !height.is_finite() {
        return Vec::new();
    }
    let ux = dx / separation;
    let uy = dy / separation;
    let base = pt(first.x + ux * along, first.y + uy * along, first.z);
    if height <= tol {
        return vec![base];
    }
    vec![
        pt(base.x - uy * height, base.y + ux * height, base.z),
        pt(base.x + uy * height, base.y - ux * height, base.z),
    ]
}
