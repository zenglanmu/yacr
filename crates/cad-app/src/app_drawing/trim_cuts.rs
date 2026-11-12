//! The cut-resolution helpers shared by TRIM and EXTEND.
//!
//! Split out of `trim_curves.rs` so both files stay inside the code-size limit;
//! the behaviour and the documentation text are the ones `trim_curves` documents:
//! the target's parameter frame, the ordered/deduplicated cut list, the periodic
//! piece selection, the "no unique crossing" overlap guard and the EXTEND probe.
//!
//! Every helper here resolves to plain data, so a plan-level refusal changes
//! nothing; the caller (`plan_trim` / `plan_extend`) owns the honesty notes.

use super::trim_boundary::{note_approximation, BoundarySet};
use super::*;
use cad_geometry::{add, length, scale, sub, DefaultGeometryEngine, GeometryEngine};

const TAU: f64 = std::f64::consts::TAU;

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
pub(super) fn target_frame(
    command: &str,
    curve: &TargetCurve,
    pick: Point3,
    tol: f64,
) -> Result<(f64, f64, f64), String> {
    let span_len = curve.length();
    if !span_len.is_finite() || span_len <= tol {
        return Err(format!("{command} target is degenerate (zero length)"));
    }
    let label = curve.label();
    let Some(u_pick) = curve.project_param(pick, tol) else {
        return Err(format!(
            "{command} pick point is degenerate for the target {label}: it lies on the target's \
             own axis and has no radial direction to project onto the {label}"
        ));
    };
    pick_frame(command, label, span_len, u_pick, tol)
}

/// The pick half of [`target_frame`]: the parameter tolerance and the
/// **out-of-span** refusal, shared by every target kind whose projection is not
/// inherently bounded (a LINE projects onto its infinite line; a SPLINE projects
/// onto the nearest point of its own domain, so it never leaves `[0, 1]`).
///
/// `span_len` must already be finite and above `tol`; `u_pick` is **not** clamped
/// by the caller — a projection outside the target's own span is refused by name,
/// never snapped to the nearest end (that would decide which piece to keep for the
/// user).
pub(super) fn pick_frame(
    command: &str,
    label: &str,
    span_len: f64,
    u_pick: f64,
    tol: f64,
) -> Result<(f64, f64, f64), String> {
    let param_tol = (tol / span_len).max(1e-12);
    if !u_pick.is_finite() || u_pick < -param_tol || u_pick > 1.0 + param_tol {
        return Err(format!(
            "{command} pick projects outside the target {label} (parameter {u_pick:.9}, \
             the target spans [0, 1])"
        ));
    }
    Ok((span_len, param_tol, u_pick.clamp(0.0, 1.0)))
}

/// Every interior cut parameter on the target, sorted and de-duplicated.
pub(super) fn collect_cuts(
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
pub(super) fn periodic_piece(cuts: &[f64], u_pick: f64) -> (f64, f64) {
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
pub(super) fn pair_has_no_unique_crossing(
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
pub(super) fn line_probe(
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
pub(super) fn norm_tau(angle: f64) -> f64 {
    let value = angle % TAU;
    if value < 0.0 {
        value + TAU
    } else {
        value
    }
}
