//! Parametric cut resolution shared by TRIM and EXTEND.
//!
//! One abstraction over the parameter domains: a LINE uses its own `t ∈ [0, 1]`
//! span, an ARC uses `u ∈ [0, 1]` across its own **signed** sweep (a CIRCLE is an
//! arc with `|sweep| == TAU`, whose parameter space is periodic), and an ELLIPSE
//! uses `u ∈ [0, 1]` across its own eccentric-angle sweep (a full ELLIPSE is
//! periodic in exactly the same sense). TRIM and EXTEND both convert every
//! boundary intersection into that parameter, so there is exactly one cut
//! algorithm per domain instead of one per geometry pair.
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
//! cuts to leave an arc, and an ARC/ELLIPSE extends at most (but not including) a
//! **full turn** — the far end follows the target's own signed sweep, so a result
//! that reached a whole turn would be the closed curve, which has no ends.
//!
//! The pick itself is **projected** onto the target ([`TargetCurve::project_param`]):
//! the desktop host resolves the target with its own pickbox and passes the raw
//! click (snap-resolved only when object snap is on), so a click a few pixels off
//! the curve is still a pick *of* that entity by construction. Requiring the click
//! to lie on the curve would refuse the very pick the host already resolved. The
//! only pick refusals left are the genuinely ambiguous ones: a projection that
//! falls **outside** the target's own span (never silently clamped — that would
//! decide which piece to keep for the user) and a pick with no in-plane direction
//! at all on an ARC/CIRCLE/ELLIPSE (it lies on the target's own axis or centre).
//!
//! Everything here resolves to plain data: the command mutates only after a plan
//! succeeds, and a plan-level refusal changes nothing.

use super::helpers::normalize_signed;
use super::trim_boundary::{note_approximation, BoundarySet};
// The cut-resolution helpers live in the sibling `trim_cuts` module (same
// behaviour); `norm_tau` is re-exported because BREAK imports it from here.
pub(super) use super::trim_cuts::norm_tau;
use super::trim_cuts::{
    collect_cuts, line_probe, pair_has_no_unique_crossing, periodic_piece, target_frame,
};
use super::trim_spline::SplineTarget;
use super::*;
use cad_geometry::{
    add, cross, dot, length, normalize, scale, sub, DefaultGeometryEngine, GeometryEngine,
};

const TAU: f64 = std::f64::consts::TAU;

/// The world tolerance the cut predicates use (the computation predicate
/// tolerance, widened to the topology join tolerance).
pub(crate) fn cut_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}

/// What a resolved TRIM will do to the target.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TrimPlan {
    /// The target is fully inside a closed boundary (or would be reduced to
    /// nothing): delete it.
    Delete,
    /// Replace the target's geometry with this value. The entity keeps its id,
    /// layer, space and display state.
    Replace(SemanticGeometry),
}

/// A resolved TRIM plus the honesty notes the command turns into diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TrimResolution {
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
pub(crate) enum PlanRefusal {
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
pub(crate) struct ExtendResolution {
    pub geometry: SemanticGeometry,
    pub notes: Vec<String>,
}

/// The target curve's own parameter space.
#[derive(Debug, Clone)]
pub(crate) enum TargetCurve {
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
    /// `u ∈ [0, 1]` from `start` to `start + sweep` on the ellipse
    /// `(center, normal, major_axis, ratio)`: the point at angle `θ` is
    /// `center + cos θ·major_axis + sin θ·minor`, with the domain's own minor
    /// vector `cross(normal_unit, major_unit)·|major_axis|·|ratio|`. A full
    /// ellipse is `sweep == ±TAU`.
    ///
    /// TRIM/EXTEND build this variant (see [`Self::new_trim_extend`]); the other
    /// [`TargetCurve`] users keep their own kind list, so adding it here cannot
    /// silently widen BREAK/DIVIDE/LENGTHEN/TREX/EXTRIM.
    Ellipse {
        center: Point3,
        normal: Point3,
        major_axis: Point3,
        ratio: f64,
        start: f64,
        sweep: f64,
    },
    /// `u ∈ [0, 1]` across the raw knot domain, from `knots[degree]` to
    /// `knots[n]`. Only TRIM builds this variant; EXTEND refuses a SPLINE by name
    /// before reaching [`Self::build`] (see
    /// [`super::trim_spline::no_continuation_semantics`]).
    Spline(SplineTarget),
}

impl TargetCurve {
    /// The supported targets: LINE, ARC and CIRCLE, plus SPLINE (`supported` names
    /// the set in the refusal so TRIM and EXTEND stay honest about their own
    /// subset). EXTEND refuses a SPLINE before reaching this constructor, because
    /// its reason is about continuation semantics rather than about a kind list.
    ///
    /// An ARC whose sweep exceeds one full turn is refused: it has no single
    /// periodic period, so its cuts would fold at the wrong place.
    ///
    /// An ELLIPSE is **not** accepted here: this constructor is the shared one for
    /// every [`TargetCurve`] consumer (BREAK/DIVIDE/LENGTHEN/TREX/EXTRIM), and
    /// widening their kind list is not part of this lane. TRIM/EXTEND use
    /// [`Self::new_trim_extend`].
    pub(crate) fn new(
        geometry: &SemanticGeometry,
        command: &str,
        supported: &str,
    ) -> CadResult<Self> {
        Self::build(geometry, command, supported, false)
    }

    /// The TRIM/EXTEND constructor: [`Self::new`] **plus** an `ELLIPSE` (and
    /// elliptical arc) target.
    ///
    /// An ellipse is a genuinely parameterised conic in this domain
    /// (`center`/`normal`/`major_axis`/`ratio`/`start`/`sweep`), so a cut is a
    /// parameter value and the kept piece is the same ellipse with a new
    /// `start`/`sweep` — no refit, no polyline, no new variant.
    pub(crate) fn new_trim_extend(
        geometry: &SemanticGeometry,
        command: &str,
        supported: &str,
    ) -> CadResult<Self> {
        Self::build(geometry, command, supported, true)
    }

    fn build(
        geometry: &SemanticGeometry,
        command: &str,
        supported: &str,
        accept_ellipse: bool,
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
            SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } if accept_ellipse => {
                if !sweep.is_finite() || sweep.abs() > TAU + 1e-9 {
                    return Err(CadError::Unsupported(format!(
                        "{command} target ELLIPSE sweep must be at most one full turn, got {sweep}"
                    )));
                }
                if ellipse_frame(*normal, *major_axis, *ratio).is_none() {
                    return Err(CadError::Unsupported(format!(
                        "{command} target ELLIPSE is degenerate: its stored major axis or ratio \
                         has no parameterisation"
                    )));
                }
                Ok(Self::Ellipse {
                    center: *center,
                    normal: *normal,
                    major_axis: *major_axis,
                    ratio: *ratio,
                    start: *start,
                    sweep: *sweep,
                })
            }
            SemanticGeometry::Spline {
                degree,
                knots,
                control_points,
                weights,
            } => Ok(Self::Spline(SplineTarget::new(
                *degree,
                knots,
                control_points,
                weights,
            )?)),
            other => Err(CadError::Unsupported(format!(
                "{command} target must be {supported}; {} is not supported",
                geometry_kind(other)
            ))),
        }
    }

    /// A short lowercase label for plan-level messages ("line", "arc", "circle",
    /// "ellipse", "spline").
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Line { .. } => "line",
            Self::Arc { .. } if self.is_periodic() => "circle",
            Self::Arc { .. } => "arc",
            Self::Ellipse { .. } if self.is_periodic() => "ellipse",
            Self::Ellipse { .. } => "elliptical arc",
            Self::Spline(_) => "spline",
        }
    }

    /// A full circle has no ends: its parameter space wraps.
    ///
    /// A SPLINE is never periodic here: the stored geometry carries no closed flag,
    /// so a closed spline is treated as the open curve its knot domain describes
    /// (its seam is its start/end, not a wrap).
    pub(super) fn is_periodic(&self) -> bool {
        matches!(
            self,
            Self::Arc { sweep, .. } | Self::Ellipse { sweep, .. }
                if sweep.abs() >= TAU - 1e-9
        )
    }

    /// The target's world length (chord for a line, arc length for an arc, a
    /// sampled chord length for a spline or an ellipse).
    pub(super) fn length(&self) -> f64 {
        match self {
            Self::Line { start, end } => length(sub(*end, *start)),
            Self::Arc { radius, sweep, .. } => radius * sweep.abs(),
            // Sampled: an ellipse has no closed-form arc length. It is used only
            // for the parameter tolerance and the degenerate-span guard, never
            // reported as a measurement.
            Self::Ellipse { .. } => {
                const SEGMENTS: usize = 256;
                let mut total = 0.0;
                let mut previous = self.point_at(0.0);
                for index in 1..=SEGMENTS {
                    let point = self.point_at(index as f64 / SEGMENTS as f64);
                    total += length(sub(point, previous));
                    previous = point;
                }
                total
            }
            Self::Spline(spline) => spline.length(),
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
            Self::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => SemanticGeometry::Ellipse {
                center: *center,
                normal: *normal,
                major_axis: *major_axis,
                ratio: *ratio,
                start: *start,
                sweep: *sweep,
            },
            // The shorthand-expanded form, so the intersector never has to decide
            // what `knots: []` means: that decision is made once, by the target.
            Self::Spline(spline) => spline.geometry(),
        }
    }

    /// The point at parameter `u` (extrapolated outside `[0, 1]` for a LINE/ARC/
    /// ELLIPSE; a SPLINE has no continuation, so it is clamped to the knot
    /// domain).
    pub(crate) fn point_at(&self, u: f64) -> Point3 {
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
            Self::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => {
                let angle = start + sweep * u;
                let minor = ellipse_minor(*normal, *major_axis, *ratio);
                add(
                    *center,
                    add(scale(*major_axis, angle.cos()), scale(minor, angle.sin())),
                )
            }
            Self::Spline(spline) => spline.point_at(u),
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
    /// A SPLINE is inverted exactly, within the target's own sampled-cut bound; a
    /// hit farther away than that is `None`.
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
            Self::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => {
                if sweep.abs() <= 1e-12 {
                    return None;
                }
                let normal = normalize(*normal);
                let v = sub(p, *center);
                if dot(v, normal).abs() > tol {
                    return None;
                }
                let (major_len, minor_unit, minor_len) =
                    ellipse_frame(normal, *major_axis, *ratio)?;
                let major_unit = scale(*major_axis, 1.0 / major_len);
                let angle = (dot(v, minor_unit) / minor_len).atan2(dot(v, major_unit) / major_len);
                let raw = if *sweep >= 0.0 {
                    norm_tau(angle - start)
                } else {
                    -norm_tau(start - angle)
                };
                Some(raw / sweep)
            }
            Self::Spline(spline) => spline.param_of(p, tol),
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
    ///   direction);
    /// * ELLIPSE: the same in-plane direction read in the ellipse's own
    ///   `(major, minor)` frame (`atan2` of the two normalised components), which
    ///   is exactly the eccentric angle the ellipse is parameterised by;
    /// * SPLINE: the nearest point of the curve (the only projection it has). No
    ///   distance bound applies, for the same reason it applies to neither of the
    ///   other two: the pick was already resolved onto this entity by the host.
    ///
    /// The result is **not** clamped or range filtered; [`target_frame`] checks it
    /// against the target's own span. `None` is the one case with no projection at
    /// all: a degenerate target, or an ARC/CIRCLE/ELLIPSE pick with a **zero**
    /// in-plane direction (it lies on the target's axis / centre).
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
            Self::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => {
                if sweep.abs() <= 1e-12 {
                    return None;
                }
                let (major_len, minor_unit, minor_len) =
                    ellipse_frame(*normal, *major_axis, *ratio)?;
                let major_unit = scale(*major_axis, 1.0 / major_len);
                let v = sub(pick, *center);
                let (major_component, minor_component) = (
                    dot(v, major_unit) / major_len,
                    dot(v, minor_unit) / minor_len,
                );
                // At the centre there is no in-plane direction to project along:
                // refuse rather than invent an angle from noise.
                if major_component.hypot(minor_component) <= tol {
                    return None;
                }
                let angle = minor_component.atan2(major_component);
                let raw = if *sweep >= 0.0 {
                    norm_tau(angle - start)
                } else {
                    -norm_tau(start - angle)
                };
                Some(raw / sweep)
            }
            Self::Spline(spline) => spline.project_param(pick, tol),
        }
    }

    /// The parameter distance of one full turn (the periodic residue), **always
    /// positive**: a point one turn away from `u` is at `u - period` for either
    /// sweep sign, because [`Self::param_of`] measures `u` along the sweep and so
    /// is never negative for a point "behind" the start. `None` for a line and for
    /// a spline (neither has a periodic parameter space here).
    pub(super) fn period(&self) -> Option<f64> {
        match self {
            Self::Line { .. } | Self::Spline(_) => None,
            Self::Arc { sweep, .. } | Self::Ellipse { sweep, .. } => Some(TAU / sweep.abs()),
        }
    }

    /// Rebuild the target as the piece `[u0, u0 + fraction]` of its own span.
    ///
    /// An arc or an ellipse reuses the target's centre/normal/axes and recomputes
    /// `start`/`sweep`, so the sweep sign is preserved. A spline becomes the exact
    /// sub-curve of the source on that span (same degree, same weights, both ends
    /// clamped); a span that cannot keep `degree + 1` control points is refused by
    /// name instead of yielding a degenerate curve.
    pub(super) fn with_span(&self, u0: f64, fraction: f64) -> Result<SemanticGeometry, String> {
        match self {
            Self::Line { .. } => Ok(SemanticGeometry::Line {
                start: self.point_at(u0),
                end: self.point_at(u0 + fraction),
            }),
            Self::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => Ok(SemanticGeometry::Arc {
                center: *center,
                normal: *normal,
                radius: *radius,
                start: normalize_signed(start + sweep * u0),
                sweep: sweep * fraction,
            }),
            Self::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => Ok(SemanticGeometry::Ellipse {
                center: *center,
                normal: *normal,
                major_axis: *major_axis,
                ratio: *ratio,
                start: normalize_signed(start + sweep * u0),
                sweep: sweep * fraction,
            }),
            Self::Spline(spline) => spline.with_span(u0, fraction),
        }
    }
}

/// Resolve the trim cuts for a LINE/ARC/CIRCLE/ELLIPSE/SPLINE target.
///
/// The cuts are ordered in the target's own parameter; the piece containing the
/// **projection** of `pick` is kept. A target with no cut that lies wholly inside
/// a closed boundary is deleted — including a CIRCLE/ELLIPSE, which has no ends
/// whose parameter range could stand in for the rule.
///
/// A SPLINE boundary only ever contributes sampled cuts; it is deliberately
/// absent from the containment test (no closed flag, no boundary plane), so a
/// no-cut target with a SPLINE in the boundary set is refused as
/// [`PlanRefusal::Unsupported`] instead of being deleted or silently treated as
/// "not inside".
///
/// `Err(reason)` covers the documented degenerate/no-cut/outside-the-span cases so
/// the caller can refuse without touching the database.
pub(crate) fn plan_trim(
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
            _ => Err(format!(
                "TRIM would reduce the target {label} to nothing: the kept span is below the \
                 parameter tolerance"
            )
            .into()),
        };
    }
    let geometry = curve.with_span(u0, fraction)?;
    // The target-side honesty note: a SPLINE is replaced by an exact sub-curve whose
    // *cut parameters* came from the sampled intersection. It is reported next to
    // the per-pair approximation notes, not instead of them.
    if let TargetCurve::Spline(spline) = curve {
        spline.note_exact_sub_curve_on_sampled_cuts(&mut notes);
    }
    Ok(TrimResolution {
        plan: TrimPlan::Replace(geometry),
        notes,
    })
}

/// Resolve where to extend a LINE/ARC/ELLIPSE target to the nearest boundary
/// crossing beyond the picked end.
///
/// The **projection** of the pick onto the target selects the nearer end (in the
/// target's own parameter), so "which end did I click" is answered by where the
/// click lands on the curve rather than by its distance to the curve; the target
/// only ever grows: a boundary that touches or crosses the current span is an
/// explicit refusal, as is "nothing lies ahead". An ARC or an ELLIPSE is bounded
/// to **less than one full turn** of travel: the target runs along its own
/// circle/ellipse, so a crossing is reachable from the picked end only within one
/// turn, and a result that reached a whole turn would be the closed curve, which
/// has no ends (it is refused by name instead of silently closing the target).
/// `Err(reason)` carries those cases to the caller before any mutation.
pub(crate) fn plan_extend(
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
    // the probe is that full circle. A spline has no probe at all: EXTEND refuses
    // it by name before the plan is reached, and says so here too instead of
    // inventing a continuation curve.
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
        // An elliptical target extends along its own ellipse: the probe is the
        // closed ellipse (the same stored frame, one full turn).
        TargetCurve::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            ..
        } => SemanticGeometry::Ellipse {
            center: *center,
            normal: *normal,
            major_axis: *major_axis,
            ratio: *ratio,
            start: 0.0,
            sweep: TAU,
        },
        TargetCurve::Spline(_) => return Err(super::trim_spline::no_continuation_semantics()),
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
    // An arc/ellipse only turns one way; cap the travel at one full turn so the
    // result can never close on itself (a closed curve has no ends; see the doc
    // above). Beyond one turn the same crossing is reached by wrapping, so the
    // cap is exactly the parameter period.
    if let TargetCurve::Arc { sweep, .. } | TargetCurve::Ellipse { sweep, .. } = curve {
        let travel = if grow_end {
            (u - 1.0) * sweep.abs()
        } else {
            -u * sweep.abs()
        };
        if !travel.is_finite() || travel >= TAU - 1e-9 {
            return Err(format!(
                "EXTEND refused: the nearest boundary ahead is {travel:.4} rad away, which is a \
                 full turn or more on the target's own curve (the result would be the closed \
                 curve, which has no ends)"
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
        // on the grow side, so the arc/ellipse only ever gets longer.
        TargetCurve::Arc { .. } | TargetCurve::Ellipse { .. } => {
            if grow_end {
                curve.with_span(0.0, u)?
            } else {
                curve.with_span(u, 1.0 - u)?
            }
        }
        TargetCurve::Spline(_) => return Err(super::trim_spline::no_continuation_semantics()),
    };
    Ok(ExtendResolution { geometry, notes })
}

/// The ellipse's **minor** semi-axis vector: `cross(normal_unit, major_unit) ·
/// |major_axis| · |ratio|`, exactly the domain's own rule (`minor =
/// cross(normal, major_axis)`), so the parameterisation here is the one the
/// tessellator and the object-snap code use.
///
/// A degenerate frame (a non-finite/zero major axis, a non-positive ratio, or a
/// normal parallel to the major axis) yields the zero vector; every caller runs
/// [`ellipse_frame`] first, which refuses that case.
fn ellipse_minor(normal: Point3, major_axis: Point3, ratio: f64) -> Point3 {
    match ellipse_frame(normal, major_axis, ratio) {
        Some((_, minor_unit, minor_len)) => scale(minor_unit, minor_len),
        None => Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    }
}

/// The ellipse frame as `(major length, minor unit vector, minor length)`, or
/// `None` when the stored definition has no parameterisation.
///
/// The checks are the ones a real ellipse needs: a positive finite major axis, a
/// positive finite ratio, a finite normal that is not parallel to the major axis
/// (`cross` must not vanish) and a finite minor length. Nothing is clamped and no
/// axis is invented.
fn ellipse_frame(normal: Point3, major_axis: Point3, ratio: f64) -> Option<(f64, Point3, f64)> {
    let major_len = length(major_axis);
    if !major_len.is_finite() || major_len <= 1e-12 || !ratio.is_finite() || ratio <= 0.0 {
        return None;
    }
    let normal_unit = normalize(normal);
    if !normal_unit.x.is_finite() || !normal_unit.y.is_finite() || !normal_unit.z.is_finite() {
        return None;
    }
    let major_unit = scale(major_axis, 1.0 / major_len);
    let minor = cross(normal_unit, major_unit);
    let minor_unit_len = length(minor);
    if !minor_unit_len.is_finite() || minor_unit_len <= 1e-9 {
        return None;
    }
    let minor_len = major_len * ratio;
    if !minor_len.is_finite() || minor_len <= 0.0 {
        return None;
    }
    Some((major_len, scale(minor, 1.0 / minor_unit_len), minor_len))
}
