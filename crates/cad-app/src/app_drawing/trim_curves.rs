//! Parametric cut resolution shared by TRIM and EXTEND.
//!
//! One abstraction over two parameter domains: a LINE uses its own `t ∈ [0, 1]`
//! span, and an ARC uses `u ∈ [0, 1]` across its own **signed** sweep (a CIRCLE
//! is an arc with `|sweep| == TAU`, whose parameter space is periodic). TRIM and
//! EXTEND both convert every boundary intersection into that parameter, so there
//! is exactly one cut algorithm per domain instead of one per geometry pair.
//!
//! Cut points come from [`cad_geometry::GeometryEngine::intersect_local`], the
//! same analytic conic intersection object-snap uses; a bulged LWPOLYLINE
//! boundary is exploded into straight LINE pieces and ARC pieces (via
//! [`cad_geometry::bulge_arc_geometry`]) so an LWPOLYLINE cut is analytic too. A
//! SPLINE boundary is kept whole: it is not a conic, so its cuts come from the
//! engine's **sampled** fallback (chord tolerance `extent × 1e-4` of each operand,
//! capped at 256 segments) and are always reported in
//! [`TrimResolution::notes`]/[`ExtendResolution::notes`] rather than presented as
//! exact (see `trim_boundary::note_approximation`).
//!
//! A SPLINE boundary does not take part in the closed-boundary delete rule: a
//! spline carries no closed flag and no boundary plane, so no sound inside test
//! exists. That combination is an explicit `Unsupported` refusal ([`PlanRefusal`]),
//! never a fabricated containment.
//!
//! Two bounds keep the result predictable: a periodic target needs two distinct
//! cuts to leave an arc, and an ARC extends at most **half a turn** (beyond that
//! the crossing is the opposite end's neighbour reached the long way round).
//!
//! The pick itself is **projected** onto the target ([`TargetCurve::project_param`]):
//! the desktop host resolves the target with its own pickbox and passes the raw
//! click (snap-resolved only when object snap is on), so a click a few pixels off
//! the curve is still a pick *of* that entity by construction. Requiring the click
//! to lie on the curve would refuse the very pick the host already resolved. The
//! only pick refusals left are the genuinely ambiguous ones: a projection that
//! falls **outside** the target's own span (never silently clamped — that would
//! decide which piece to keep for the user) and a pick with no radial direction at
//! all on an ARC/CIRCLE (it lies on the target's own axis).
//!
//! Everything here resolves to plain data: the command mutates only after a plan
//! succeeds, and a plan-level refusal changes nothing.

use super::helpers::normalize_signed;
use super::trim_boundary::{note_approximation, BoundarySet};
use super::*;
use cad_geometry::{add, dot, length, scale, sub, DefaultGeometryEngine, GeometryEngine};

const TAU: f64 = std::f64::consts::TAU;

/// The world tolerance the cut predicates use (the computation predicate
/// tolerance, widened to the topology join tolerance).
pub(super) fn cut_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}

/// What a resolved TRIM will do to the target.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TrimPlan {
    /// The target is fully inside a closed boundary (or would be reduced to
    /// nothing): delete it.
    Delete,
    /// Replace the target's geometry with this value. The entity keeps its id,
    /// layer, space and display state.
    Replace(SemanticGeometry),
}

/// A resolved TRIM plus the honesty notes the command turns into diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TrimResolution {
    pub plan: TrimPlan,
    /// One entry per boundary pair whose cut point came from the engine's
    /// **sampled** fallback instead of an analytic solution. Empty when every
    /// cut was analytic; never silently dropped by the caller.
    pub notes: Vec<String>,
}

/// Why a TRIM plan could not be resolved.
///
/// The distinction is kept (rather than a bare `String`) because one refusal is a
/// **capability gap**, not bad input: closed-SPLINE containment. Reporting that as
/// `InvalidInput` would blame the user for a subset the command does not implement.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum PlanRefusal {
    /// Bad or ambiguous input (degenerate target, no intersection, a pick whose
    /// projection falls outside the target's span).
    Invalid(String),
    /// A combination this command does not implement (closed-SPLINE containment).
    Unsupported(String),
}

impl PlanRefusal {
    pub(super) fn into_cad_error(self) -> CadError {
        match self {
            Self::Invalid(reason) => CadError::InvalidInput(reason),
            Self::Unsupported(reason) => CadError::Unsupported(reason),
        }
    }
}

impl From<String> for PlanRefusal {
    fn from(reason: String) -> Self {
        Self::Invalid(reason)
    }
}

impl From<&str> for PlanRefusal {
    fn from(reason: &str) -> Self {
        Self::Invalid(reason.to_string())
    }
}

/// A resolved EXTEND plus the same honesty notes.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ExtendResolution {
    pub geometry: SemanticGeometry,
    pub notes: Vec<String>,
}

/// The target curve's own parameter space.
#[derive(Debug, Clone, Copy)]
pub(super) enum TargetCurve {
    /// `u ∈ [0, 1]` from `start` to `end`.
    Line { start: Point3, end: Point3 },
    /// `u ∈ [0, 1]` from `start` to `start + sweep` on the circle
    /// `(center, normal, radius)`. A full circle is `sweep == ±TAU`.
    Arc {
        center: Point3,
        normal: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

impl TargetCurve {
    /// The supported targets: LINE, ARC and CIRCLE (`supported` names the set in
    /// the refusal so TRIM and EXTEND stay honest about their own subset).
    ///
    /// An ARC whose sweep exceeds one full turn is refused: it has no single
    /// periodic period, so its cuts would fold at the wrong place.
    pub(super) fn new(
        geometry: &SemanticGeometry,
        command: &str,
        supported: &str,
    ) -> CadResult<Self> {
        match geometry {
            SemanticGeometry::Line { start, end } => Ok(Self::Line {
                start: *start,
                end: *end,
            }),
            SemanticGeometry::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => {
                // More than one full turn has no single periodic period, so its
                // cuts would fold at the wrong place: refuse instead of guessing.
                if !sweep.is_finite() || sweep.abs() > TAU + 1e-9 {
                    return Err(CadError::Unsupported(format!(
                        "{command} target ARC sweep must be at most one full turn, got {sweep}"
                    )));
                }
                Ok(Self::Arc {
                    center: *center,
                    normal: *normal,
                    radius: radius.abs(),
                    start: *start,
                    sweep: *sweep,
                })
            }
            SemanticGeometry::Circle {
                center,
                normal,
                radius,
            } => Ok(Self::Arc {
                center: *center,
                normal: *normal,
                radius: radius.abs(),
                start: 0.0,
                sweep: TAU,
            }),
            other => Err(CadError::Unsupported(format!(
                "{command} target must be {supported}; {} is not supported",
                geometry_kind(other)
            ))),
        }
    }

    /// A short lowercase label for plan-level messages ("line", "arc", "circle").
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Line { .. } => "line",
            Self::Arc { .. } if self.is_periodic() => "circle",
            Self::Arc { .. } => "arc",
        }
    }

    /// A full circle has no ends: its parameter space wraps.
    pub(super) fn is_periodic(&self) -> bool {
        matches!(self, Self::Arc { sweep, .. } if sweep.abs() >= TAU - 1e-9)
    }

    /// The target's world length (chord for a line, arc length for an arc).
    pub(super) fn length(&self) -> f64 {
        match self {
            Self::Line { start, end } => length(sub(*end, *start)),
            Self::Arc { radius, sweep, .. } => radius * sweep.abs(),
        }
    }

    /// The finite geometry the trim cuts are resolved against.
    pub(super) fn geometry(&self) -> SemanticGeometry {
        match self {
            Self::Line { start, end } => SemanticGeometry::Line {
                start: *start,
                end: *end,
            },
            Self::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => SemanticGeometry::Arc {
                center: *center,
                normal: *normal,
                radius: *radius,
                start: *start,
                sweep: *sweep,
            },
        }
    }

    /// The point at parameter `u` (extrapolated outside `[0, 1]`).
    pub(super) fn point_at(&self, u: f64) -> Point3 {
        match self {
            Self::Line { start, end } => add(*start, scale(sub(*end, *start), u)),
            Self::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => {
                let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
                let angle = start + sweep * u;
                add(
                    *center,
                    add(
                        scale(ax, radius * angle.cos()),
                        scale(ay, radius * angle.sin()),
                    ),
                )
            }
        }
    }

    /// The parameter of `p` on the target, without clamping or range filtering.
    ///
    /// For a LINE this is the projection onto the segment (any real value); for
    /// an arc it is measured in units of the arc's own sweep, so `u == 1` is the
    /// far end and a point one full turn further away is `u ± TAU/|sweep|`. A
    /// point off the arc's plane is `None` (it cannot lie on the curve).
    ///
    /// This is the **boundary-hit** parameter, where the point really is on the
    /// curve (an off-plane or off-radius hit must not become a cut). The pick uses
    /// [`Self::project_param`] instead, which projects a near miss onto the curve.
    pub(super) fn param_of(&self, p: Point3, tol: f64) -> Option<f64> {
        match self {
            Self::Line { start, end } => {
                let d = sub(*end, *start);
                let len2 = dot(d, d);
                if !len2.is_finite() || len2 <= 0.0 {
                    return None;
                }
                Some(dot(sub(p, *start), d) / len2)
            }
            Self::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => {
                if radius.abs() <= tol || sweep.abs() <= 1e-12 {
                    return None;
                }
                let (ax, ay, n) = cad_geometry::arbitrary_axis(*normal);
                let v = sub(p, *center);
                if dot(v, n).abs() > tol {
                    return None;
                }
                let angle = dot(v, ay).atan2(dot(v, ax));
                let raw = if *sweep >= 0.0 {
                    norm_tau(angle - start)
                } else {
                    -norm_tau(start - angle)
                };
                Some(raw / sweep)
            }
        }
    }

    /// The parameter of the **projection** of `pick` onto the target curve.
    ///
    /// The host resolves the target with its own pickbox and passes the raw click
    /// (snap-resolved only when object snap is on), so a click a few pixels away
    /// from the curve is still a pick *of* this entity by construction. The pick
    /// is therefore projected onto the curve instead of being required to lie on
    /// it:
    ///
    /// * LINE: the perpendicular projection onto the segment's own infinite line;
    /// * ARC/CIRCLE: the radial projection `center + radius * normalize(pick −
    ///   center)` in the target's own plane, i.e. the angle of the in-plane part of
    ///   `pick − center` (its normal component changes the distance, never the
    ///   direction).
    ///
    /// The result is **not** clamped or range filtered; [`target_frame`] checks it
    /// against the target's own span. `None` is the one case with no projection at
    /// all: a degenerate target, or an ARC/CIRCLE pick on the target's own axis
    /// (a **zero** radial direction).
    pub(super) fn project_param(&self, pick: Point3, tol: f64) -> Option<f64> {
        match self {
            Self::Line { start, end } => {
                let d = sub(*end, *start);
                let len2 = dot(d, d);
                if !len2.is_finite() || len2 <= 0.0 {
                    return None;
                }
                Some(dot(sub(pick, *start), d) / len2)
            }
            Self::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => {
                if !radius.is_finite() || *radius <= tol || sweep.abs() <= 1e-12 {
                    return None;
                }
                let (ax, ay, _) = cad_geometry::arbitrary_axis(*normal);
                let v = sub(pick, *center);
                let (radial_x, radial_y) = (dot(v, ax), dot(v, ay));
                // On the target's axis there is no radial direction to project
                // along: refuse rather than invent an angle from noise.
                if radial_x.hypot(radial_y) <= tol {
                    return None;
                }
                let angle = radial_y.atan2(radial_x);
                let raw = if *sweep >= 0.0 {
                    norm_tau(angle - start)
                } else {
                    -norm_tau(start - angle)
                };
                Some(raw / sweep)
            }
        }
    }

    /// The parameter distance of one full turn (the periodic residue), **always
    /// positive**: a point one turn away from `u` is at `u - period` for either
    /// sweep sign, because [`Self::param_of`] measures `u` along the sweep and so
    /// is never negative for a point "behind" the start. `None` for a line.
    pub(super) fn period(&self) -> Option<f64> {
        match self {
            Self::Line { .. } => None,
            Self::Arc { sweep, .. } => Some(TAU / sweep.abs()),
        }
    }

    /// Rebuild the target as the piece `[u0, u0 + fraction]` of its own span.
    ///
    /// An arc reuses the target's centre/normal/radius and recomputes
    /// `start`/`sweep`, so the sweep sign is preserved.
    pub(super) fn with_span(&self, u0: f64, fraction: f64) -> SemanticGeometry {
        match self {
            Self::Line { .. } => SemanticGeometry::Line {
                start: self.point_at(u0),
                end: self.point_at(u0 + fraction),
            },
            Self::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => SemanticGeometry::Arc {
                center: *center,
                normal: *normal,
                radius: *radius,
                start: normalize_signed(start + sweep * u0),
                sweep: sweep * fraction,
            },
        }
    }
}

/// Resolve the trim cuts for a LINE/ARC/CIRCLE target.
///
/// The cuts are ordered in the target's own parameter; the piece containing the
/// **projection** of `pick` is kept. A target with no cut that lies wholly inside
/// a closed boundary is deleted — including a CIRCLE, which has no ends whose
/// parameter range could stand in for the rule.
///
/// A SPLINE boundary only ever contributes sampled cuts; it is deliberately
/// absent from the containment test (no closed flag, no boundary plane), so a
/// no-cut target with a SPLINE in the boundary set is refused as
/// [`PlanRefusal::Unsupported`] instead of being deleted or silently treated as
/// "not inside".
///
/// `Err(reason)` covers the documented degenerate/no-cut/outside-the-span cases so
/// the caller can refuse without touching the database.
pub(super) fn plan_trim(
    curve: &TargetCurve,
    pick: Point3,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
) -> Result<TrimResolution, PlanRefusal> {
    let label = curve.label();
    let (span_len, param_tol, u_pick) = target_frame("TRIM", curve, pick, tol)?;

    let mut notes = Vec::new();
    let cuts = collect_cuts(
        curve, boundaries, policy, tol, param_tol, "TRIM", &mut notes,
    )?;

    // No cut at all is the *containment* question for every target kind, so it is
    // answered before the periodic branch: a CIRCLE fully inside a closed
    // boundary must be deleted, not refused for having too few cuts.
    if cuts.is_empty() {
        if boundaries.contains(curve.point_at(0.5), tol) {
            return Ok(TrimResolution {
                plan: TrimPlan::Delete,
                notes,
            });
        }
        if boundaries.has_spline_boundary() {
            return Err(PlanRefusal::Unsupported(
                "TRIM found no intersection between the target and the boundary, and the \
                 closed-SPLINE delete rule is not supported: a SPLINE boundary carries no \
                 closed flag and no boundary plane, so a target wholly inside a closed spline \
                 cannot be tested for containment; it is refused rather than deleted"
                    .into(),
            ));
        }
        return Err("TRIM found no intersection between the target and the boundary".into());
    }

    let (u0, fraction) = if curve.is_periodic() {
        if cuts.len() < 2 {
            return Err(format!(
                "TRIM of a {label} needs at least two boundary intersections to leave an arc; \
                 found {}",
                cuts.len()
            )
            .into());
        }
        periodic_piece(&cuts, u_pick)
    } else {
        let mut breaks = Vec::with_capacity(cuts.len() + 2);
        breaks.push(0.0);
        breaks.extend(cuts.iter().copied());
        breaks.push(1.0);
        let mut kept = None;
        for window in breaks.windows(2) {
            if u_pick + param_tol >= window[0] && u_pick - param_tol <= window[1] {
                kept = Some((window[0], window[1]));
                break;
            }
        }
        let Some((start, end)) = kept else {
            return Err("TRIM could not determine the retained segment".into());
        };
        (start, end - start)
    };

    let kept_len = fraction * span_len;
    if !kept_len.is_finite() || kept_len <= tol {
        return match curve {
            TargetCurve::Line { .. } => Ok(TrimResolution {
                plan: TrimPlan::Delete,
                notes,
            }),
            _ => Err(format!("TRIM would reduce the target {label} to nothing").into()),
        };
    }
    Ok(TrimResolution {
        plan: TrimPlan::Replace(curve.with_span(u0, fraction)),
        notes,
    })
}

/// Resolve where to extend a LINE/ARC target to the nearest boundary crossing
/// beyond the picked end.
///
/// The **projection** of the pick onto the target selects the nearer end (in the
/// target's own parameter), so "which end did I click" is answered by where the
/// click lands on the curve rather than by its distance to the curve; the target
/// only ever grows: a boundary that touches or crosses the current span is an
/// explicit refusal, as is "nothing lies ahead". An ARC is bounded to **less
/// than half a turn** of travel: a crossing that is only reachable by sweeping
/// most of the circle is the *opposite* end's neighbour reached the long way
/// round, so it is refused by name instead of silently unfolding the arc.
/// `Err(reason)` carries those cases to the caller before any mutation.
pub(super) fn plan_extend(
    curve: &TargetCurve,
    pick: Point3,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
) -> Result<ExtendResolution, String> {
    let (_, param_tol, u_pick) = target_frame("EXTEND", curve, pick, tol)?;
    let grow_end = u_pick > 0.5;

    let engine = DefaultGeometryEngine;
    // A line extends along its own infinite line: the probe is a segment long
    // enough to contain every boundary. An arc extends along its own circle, so
    // the probe is that full circle.
    let probe = match curve {
        TargetCurve::Line { .. } => line_probe(curve, boundaries, tol)?,
        TargetCurve::Arc {
            center,
            normal,
            radius,
            ..
        } => SemanticGeometry::Circle {
            center: *center,
            normal: *normal,
            radius: *radius,
        },
    };

    let period = curve.period();
    let mut notes = Vec::new();
    let mut touches = false;
    let mut crosses = false;
    let mut beyond_start: Option<f64> = None;
    let mut beyond_end: Option<f64> = None;
    for primitive in boundaries.primitives() {
        if pair_has_no_unique_crossing(curve, primitive, tol) {
            continue;
        }
        let hits = engine
            .intersect_local(&probe, primitive, policy)
            .map_err(|error| {
                format!("EXTEND could not resolve a boundary intersection: {error}")
            })?;
        let mut used = false;
        for hit in hits {
            let Some(u) = curve.param_of(hit, tol) else {
                continue;
            };
            used = true;
            // `period` is always positive, so the two representations of one
            // physical crossing are `u` and `u - period`, and each end of a
            // negative-sweep arc still gets its own candidate.
            for candidate in std::iter::once(u).chain(period.map(|shift| u - shift)) {
                if candidate.abs() <= param_tol || (candidate - 1.0).abs() <= param_tol {
                    touches = true;
                } else if candidate > param_tol && candidate < 1.0 - param_tol {
                    crosses = true;
                } else if candidate < 0.0 {
                    beyond_start =
                        Some(beyond_start.map_or(candidate, |seen: f64| seen.max(candidate)));
                } else if candidate > 1.0 {
                    beyond_end =
                        Some(beyond_end.map_or(candidate, |seen: f64| seen.min(candidate)));
                }
            }
        }
        // Only a pair that contributed a crossing is reported as approximate.
        if used {
            note_approximation(&probe, primitive, policy, "EXTEND", &mut notes);
        }
    }
    // A target that already meets the boundary is not extendable: refuse rather
    // than push it through an edge it already touches or crosses.
    if touches {
        return Err("EXTEND target already touches the boundary".into());
    }
    if crosses {
        return Err("EXTEND target already reaches the boundary".into());
    }
    let chosen = if grow_end { beyond_end } else { beyond_start };
    let Some(u) = chosen else {
        return Err("EXTEND found no intersection between the target and the boundary".into());
    };
    // An arc only turns one way; cap the travel at half a turn so the result
    // cannot unfold into an almost-complete circle (see the doc above).
    if let TargetCurve::Arc { sweep, .. } = curve {
        let travel = if grow_end {
            (u - 1.0) * sweep.abs()
        } else {
            -u * sweep.abs()
        };
        if !travel.is_finite() || travel >= std::f64::consts::PI {
            return Err(format!(
                "EXTEND refused: the nearest boundary ahead is {travel:.4} rad away, which is \
                 half a turn or more on the target's own circle"
            ));
        }
    }
    let geometry = match curve {
        TargetCurve::Line { .. } => {
            if grow_end {
                SemanticGeometry::Line {
                    start: curve.point_at(0.0),
                    end: curve.point_at(u),
                }
            } else {
                SemanticGeometry::Line {
                    start: curve.point_at(u),
                    end: curve.point_at(1.0),
                }
            }
        }
        // Same-sign sweep extension: the far end moves by `u` (a fraction of the
        // target's own sweep), the near end by `1 - u`; `u` is outside `[0, 1]`
        // on the grow side, so the arc only ever gets longer.
        TargetCurve::Arc { .. } => {
            if grow_end {
                curve.with_span(0.0, u)
            } else {
                curve.with_span(u, 1.0 - u)
            }
        }
    };
    Ok(ExtendResolution { geometry, notes })
}

/// The target's world length, its parameter tolerance (one world tolerance
/// expressed in parameter units) and the projected pick parameter.
///
/// The pick is **projected** onto the target ([`TargetCurve::project_param`]),
/// not required to lie on it: the host resolves the target with its own pickbox
/// and passes the raw click, so a near miss is still a pick of this entity. The
/// only refusals here are the genuinely ambiguous ones — a degenerate target, a
/// pick with no radial direction at all (an ARC/CIRCLE pick on the target's axis)
/// and a projection that falls **outside** the target's own span. The last one is
/// refused by name instead of being clamped to the nearest end, because clamping
/// would silently decide which piece to keep for the user.
///
/// Boundary hits are deliberately *not* checked this way: under the engine's
/// sampled fallback a hit may legitimately be slightly off the radius.
fn target_frame(
    command: &str,
    curve: &TargetCurve,
    pick: Point3,
    tol: f64,
) -> Result<(f64, f64, f64), String> {
    let span_len = curve.length();
    if !span_len.is_finite() || span_len <= tol {
        return Err(format!("{command} target is degenerate (zero length)"));
    }
    let param_tol = (tol / span_len).max(1e-12);
    let label = curve.label();
    let Some(u_pick) = curve.project_param(pick, tol) else {
        return Err(format!(
            "{command} pick point is degenerate for the target {label}: it lies on the target's \
             own axis and has no radial direction to project onto the {label}"
        ));
    };
    if !u_pick.is_finite() || u_pick < -param_tol || u_pick > 1.0 + param_tol {
        return Err(format!(
            "{command} pick projects outside the target {label} (parameter {u_pick:.9}, \
             the target spans [0, 1])"
        ));
    }
    Ok((span_len, param_tol, u_pick.clamp(0.0, 1.0)))
}

/// Every interior cut parameter on the target, sorted and de-duplicated.
fn collect_cuts(
    curve: &TargetCurve,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
    param_tol: f64,
    command: &str,
    notes: &mut Vec<String>,
) -> Result<Vec<f64>, String> {
    let engine = DefaultGeometryEngine;
    let probe = curve.geometry();
    let periodic = curve.is_periodic();
    let mut cuts: Vec<f64> = Vec::new();
    for primitive in boundaries.primitives() {
        if pair_has_no_unique_crossing(curve, primitive, tol) {
            continue;
        }
        let hits = engine
            .intersect_local(&probe, primitive, policy)
            .map_err(|error| {
                format!("{command} could not resolve a boundary intersection: {error}")
            })?;
        let mut used = false;
        for hit in hits {
            let Some(u) = curve.param_of(hit, tol) else {
                continue;
            };
            // A periodic target has no ends, so every cut is interior; the seam
            // is folded onto `u == 0`.
            let u = if periodic {
                u.rem_euclid(1.0)
            } else {
                if u <= param_tol || u >= 1.0 - param_tol {
                    continue;
                }
                u
            };
            // On a periodic parameter space the seam is a single point: two
            // crossings at `u ≈ 0` and `u ≈ 1⁻` are the *same* cut, so the
            // distance must wrap (otherwise an honest "too few cuts" refusal turns
            // into a nearly-complete arc).
            let duplicate = cuts.iter().any(|seen| {
                let distance = (seen - u).abs();
                let distance = if periodic {
                    distance.min(1.0 - distance)
                } else {
                    distance
                };
                distance <= param_tol * 2.0
            });
            if duplicate {
                continue;
            }
            cuts.push(u);
            used = true;
        }
        // Report the approximation only for a pair that really produced a cut:
        // a non-analytic pair that misses the target says nothing about this one.
        if used {
            note_approximation(&probe, primitive, policy, command, notes);
        }
    }
    cuts.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    Ok(cuts)
}

/// The periodic piece of `[0, 1)` that contains `u_pick`, as `(start, length)`.
fn periodic_piece(cuts: &[f64], u_pick: f64) -> (f64, f64) {
    let count = cuts.len();
    let next = cuts.iter().position(|cut| *cut > u_pick);
    let (u0, u1) = match next {
        Some(0) | None => (cuts[count - 1], cuts[0]),
        Some(index) => (cuts[index - 1], cuts[index]),
    };
    let fraction = if u1 > u0 { u1 - u0 } else { u1 + 1.0 - u0 };
    (u0, fraction)
}

/// Whether the pair overlaps instead of crossing: a LINE target cut by a
/// parallel/collinear LINE boundary has no unique crossing point, so the
/// analytic "intersection" would be an arbitrary point on the overlap.
fn pair_has_no_unique_crossing(
    curve: &TargetCurve,
    primitive: &SemanticGeometry,
    tol: f64,
) -> bool {
    let TargetCurve::Line { start, end } = curve else {
        return false;
    };
    let SemanticGeometry::Line {
        start: other_start,
        end: other_end,
    } = primitive
    else {
        return false;
    };
    !matches!(
        cad_geometry::infinite_line_intersection(*start, *end, *other_start, *other_end, tol),
        cad_geometry::LineIntersection::Point(_)
    )
}

/// A segment on the target's own infinite line that contains every boundary.
///
/// Reach justification (the probe must not miss a far boundary, so it is
/// deliberately generous). Every crossing of the infinite target line with a
/// boundary lies inside that boundary's bounding box, and for any point `p` of the
/// box `|p − a| ≤ diag/2 + |box_center − mid| + len/2` (and the same from `b`).
/// `reach = (|box_center − mid| + diag + len)·1.5 + 1.0` covers that bound with at
/// least one world unit to spare, which is far larger than the ≤ `0.1` chord by
/// which `BoundsAccumulator`'s coarse sampling can under-estimate a bulge apex.
/// Overshooting is harmless: only the nearest crossing beyond the picked end is
/// kept.
fn line_probe(
    curve: &TargetCurve,
    boundaries: &BoundarySet,
    tol: f64,
) -> Result<SemanticGeometry, String> {
    let TargetCurve::Line { start, end } = curve else {
        return Err("EXTEND line probe needs a LINE target".into());
    };
    let (a, b) = (*start, *end);
    let direction = sub(b, a);
    let len = length(direction);
    if !len.is_finite() || len <= tol {
        return Err("EXTEND target is degenerate (zero length)".into());
    }
    let unit = scale(direction, 1.0 / len);
    let mut bounds = cad_geometry::BoundsAccumulator::new();
    for primitive in boundaries.primitives() {
        bounds.add_geometry(primitive);
    }
    let Some(bounds) = bounds.finish() else {
        return Err("EXTEND boundary has no finite extent to extend to".into());
    };
    let box_center = scale(add(bounds.min, bounds.max), 0.5);
    let diagonal = length(sub(bounds.max, bounds.min));
    let midpoint = scale(add(a, b), 0.5);
    let reach = (length(sub(box_center, midpoint)) + diagonal + len) * 1.5 + 1.0;
    Ok(SemanticGeometry::Line {
        start: sub(a, scale(unit, reach)),
        end: add(b, scale(unit, reach)),
    })
}

/// Angle normalised into `[0, TAU)`.
fn norm_tau(angle: f64) -> f64 {
    let value = angle % TAU;
    if value < 0.0 {
        value + TAU
    } else {
        value
    }
}
