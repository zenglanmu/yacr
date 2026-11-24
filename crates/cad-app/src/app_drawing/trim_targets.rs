//! The one **TRIM/EXTEND target dispatch**: which target kinds a planner
//! accepts and how each kind reaches its planner.
//!
//! Split out of `trim_extend` (which stays the command owner) so the target
//! policy is a single function instead of an inline `match` per command:
//!
//! * `TRIM` — a HATCH target goes through its boundary loop
//!   ([`super::trim_hatch::plan_trim_hatch`]), an LWPOLYLINE through the shared
//!   polyline chain model ([`super::trim_polyline::plan_trim_polyline`]), and
//!   every other supported kind through [`TargetCurve::new_trim_extend`] +
//!   [`plan_trim`]. The named target gaps ([`trim_target_gap`]) are refused
//!   before either planner runs.
//! * `EXTEND` — an open SPLINE has its own end-tangent continuation
//!   ([`super::extend_spline::plan_extend_spline`]); LINE/ARC/elliptical-arc
//!   targets use [`plan_extend`]; every kind with no endpoint (CIRCLE, full
//!   ELLIPSE) and the named gaps ([`extend_target_gap`], which includes
//!   HATCH/POLYLINE) are refused.
//!
//! `BTRIM`/`BEXTEND` call these same functions, so their target set cannot drift
//! from the TRIM/EXTEND set. Nothing here writes: a plan is returned to the
//! caller, which stages it in one transaction.

use super::extend_spline::plan_extend_spline;
use super::trim_boundary::BoundarySet;
use super::trim_curves::{
    plan_extend, plan_trim, PlanRefusal, TargetCurve, TrimPlan, TrimResolution,
};
use super::trim_hatch::plan_trim_hatch;
use super::trim_polyline::plan_trim_polyline;
use super::*;

/// A resolved target plan plus the label and honesty notes the command turns
/// into diagnostics. `label` is the planner's own lowercase kind name.
pub(crate) struct TargetPlan {
    pub(crate) plan: TrimPlan,
    pub(crate) notes: Vec<String>,
    pub(crate) label: &'static str,
}

/// Plan a TRIM of one target against the boundary set.
///
/// The target kinds are exactly TRIM's own: LINE, ARC, CIRCLE, ELLIPSE (or an
/// elliptical arc), open/closed LWPOLYLINE, SPLINE and — through its boundary
/// loop — HATCH. A kind outside that set is a named capability refusal from
/// [`trim_target_gap`]; every plan-level refusal keeps its
/// [`PlanRefusal`] kind (`InvalidInput` for bad input, `Unsupported` for the
/// closed-SPLINE containment gap).
pub(crate) fn plan_trim_target(
    geometry: &SemanticGeometry,
    pick: Point3,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
) -> CadResult<TargetPlan> {
    if let Some(error) = trim_target_gap(geometry) {
        return Err(error);
    }
    match geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            width,
        } => {
            let mut resolution =
                plan_trim_polyline(points, bulges, *closed, pick, boundaries, policy, tol)
                    .map_err(PlanRefusal::into_cad_error)?;
            // TRIM keeps the clicked piece of the same entity, so it keeps the
            // polyline's uniform width (the shared curve model emits a
            // centreline-only geometry).
            if let TrimPlan::Replace(geometry) = &mut resolution.plan {
                if let SemanticGeometry::Polyline { width: kept, .. } = &mut **geometry {
                    *kept = *width;
                }
            }
            Ok(TargetPlan {
                plan: resolution.plan,
                notes: resolution.notes,
                label: if *closed {
                    "closed polyline"
                } else {
                    "polyline"
                },
            })
        }
        SemanticGeometry::Hatch(hatch) => {
            let resolution = plan_trim_hatch(hatch.as_ref(), pick, boundaries, policy, tol)
                .map_err(PlanRefusal::into_cad_error)?;
            Ok(TargetPlan {
                plan: resolution.plan,
                notes: resolution.notes,
                label: "hatch",
            })
        }
        other => {
            let curve = TargetCurve::new_trim_extend(
                other,
                "TRIM",
                "a LINE, ARC, CIRCLE, ELLIPSE, SPLINE or LWPOLYLINE",
            )?;
            let TrimResolution { plan, notes } = plan_trim(&curve, pick, boundaries, policy, tol)
                .map_err(PlanRefusal::into_cad_error)?;
            Ok(TargetPlan {
                plan,
                notes,
                label: curve.label(),
            })
        }
    }
}

/// Plan an EXTEND of one open target against the boundary set.
///
/// The target kinds are exactly EXTEND's own: LINE, ARC, ELLIPSE (or an
/// elliptical arc) and an open SPLINE. A CIRCLE/full ELLIPSE has no endpoint
/// (refused by the periodic rule), and every other kind is a named capability
/// refusal from [`extend_target_gap`].
pub(crate) fn plan_extend_target(
    geometry: &SemanticGeometry,
    pick: Point3,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
) -> CadResult<TargetPlan> {
    if let Some(error) = extend_target_gap(geometry) {
        return Err(error);
    }
    let curve =
        TargetCurve::new_trim_extend(geometry, "EXTEND", "a LINE, ARC, ELLIPSE, CIRCLE or SPLINE")?;
    if curve.is_periodic() {
        return Err(CadError::Unsupported(format!(
            "EXTEND target {} is closed and has no endpoint; only LINE, ARC, ELLIPTICAL \
             ARC and an open SPLINE are supported",
            geometry_kind(geometry)
        )));
    }
    let (geometry, notes) = match &curve {
        TargetCurve::Spline(spline) => {
            let resolution = plan_extend_spline(spline, pick, boundaries, policy, tol)
                .map_err(PlanRefusal::into_cad_error)?;
            (resolution.geometry, resolution.notes)
        }
        _ => match plan_extend(&curve, pick, boundaries, policy, tol) {
            Ok(resolution) => (resolution.geometry, resolution.notes),
            Err(reason) => return Err(CadError::InvalidInput(reason)),
        },
    };
    Ok(TargetPlan {
        plan: TrimPlan::Replace(Box::new(geometry)),
        notes,
        label: curve.label(),
    })
}

/// The named TRIM **target** gaps: the kinds the shared planners cannot
/// parameterise are refused with a kind-specific `trim.target_<kind>` reason
/// instead of the constructor's generic "not supported" string.
///
/// `None` means "this is a target kind TRIM models" (LINE, ARC, CIRCLE, ELLIPSE,
/// SPLINE, LWPOLYLINE and — through its boundary loop — HATCH), so the caller
/// proceeds to the matching planner. Every arm here is a **capability gap**, not
/// bad input: the caller has already resolved the entity, and the refusal writes
/// nothing.
pub(crate) fn trim_target_gap(geometry: &SemanticGeometry) -> Option<CadError> {
    let reason = match geometry {
        SemanticGeometry::Insert { .. } => {
            "trim.target_insert: TRIM cannot trim an INSERT target (AcDbBlockReference). A \
             reference stores only a placement plus its dynamic parameters; its members are rows \
             of the shared block definition, and this build has no per-instance geometry override. \
             Cutting a member would therefore either rewrite the definition (changing every \
             insertion of that block, nested ones included) or require a new definition plus a \
             per-instance replacement — neither is a TRIM of this reference. Only directly \
             addressable LINE, ARC, CIRCLE, ELLIPSE, LWPOLYLINE, SPLINE or HATCH targets are \
             supported; nothing was written"
        }
        SemanticGeometry::Mesh(_) => {
            "trim.target_mesh: TRIM cannot trim a MESH target: a 3D face has no modeled 2D curve \
             parameterisation; only LINE, ARC, CIRCLE, ELLIPSE, LWPOLYLINE or SPLINE targets are \
             supported"
        }
        SemanticGeometry::Opaque { .. } => {
            "trim.target_opaque: TRIM cannot trim opaque/proxy geometry: it has no semantic form \
             to cut; only LINE, ARC, CIRCLE, ELLIPSE, LWPOLYLINE or SPLINE targets are supported"
        }
        SemanticGeometry::Compound(_) => {
            "trim.target_compound: TRIM cannot trim a Compound target: it has no single \
             owner-level curve to cut; only LINE, ARC, CIRCLE, ELLIPSE, LWPOLYLINE or SPLINE \
             targets are supported"
        }
        _ => return None,
    };
    Some(CadError::Unsupported(reason.into()))
}

/// The named EXTEND **target** gaps (see [`trim_target_gap`]); an open SPLINE is
/// a supported target (its own tangent rule lives in `super::extend_spline`), and
/// a CIRCLE/full ELLIPSE is refused by the closed-target rule instead.
pub(crate) fn extend_target_gap(geometry: &SemanticGeometry) -> Option<CadError> {
    let reason = match geometry {
        SemanticGeometry::Insert { .. } => {
            "extend.target_insert: EXTEND cannot extend an INSERT target (AcDbBlockReference): a \
             block reference stores a placement, not an open curve with ends; only LINE, ARC \
             and elliptical-arc targets are supported"
        }
        SemanticGeometry::Hatch(_) => {
            "extend.target_hatch: EXTEND cannot extend a HATCH target: a hatch's display is \
             derived from its boundary loops and fill, so there is no source curve with ends; \
             only LINE, ARC and elliptical-arc targets are supported"
        }
        SemanticGeometry::Polyline { .. } => {
            "extend.target_polyline: EXTEND cannot extend a POLYLINE target: each end of a \
             multi-segment target would need its own continuation rule and no polyline \
             continuation policy is modeled, so nothing is extrapolated; only LINE, ARC and \
             elliptical-arc targets are supported"
        }
        SemanticGeometry::Mesh(_) => {
            "extend.target_mesh: EXTEND cannot extend a MESH target: a 3D face has no modeled 2D \
             curve parameterisation; only LINE, ARC and elliptical-arc targets are supported"
        }
        SemanticGeometry::Opaque { .. } => {
            "extend.target_opaque: EXTEND cannot extend opaque/proxy geometry: it has no \
             semantic form to extend; only LINE, ARC and elliptical-arc targets are supported"
        }
        SemanticGeometry::Compound(_) => {
            "extend.target_compound: EXTEND cannot extend a Compound target: it has no single \
             owner-level curve with ends; only LINE, ARC and elliptical-arc targets are \
             supported"
        }
        _ => return None,
    };
    Some(CadError::Unsupported(reason.into()))
}
