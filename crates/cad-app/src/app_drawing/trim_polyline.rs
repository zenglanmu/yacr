//! TRIM of a POLYLINE (LWPOLYLINE) **target**.
//!
//! A polyline is a chain of LINE/ARC segments, so it reuses the one parameter
//! model BREAK/BREAKATPOINT/DIVIDE already share ([`PolylineCurve`]): `u ∈ [0,
//! segments]`, one unit per segment. There is deliberately **no second splitter**:
//! the kept piece is a `PolylineCurve::sub_curve` span of the source, which returns
//! the exact surviving vertices and bulges. A bulged segment is a circular arc, and
//! `sub_curve` re-derives each sub-bulge with `tan(sweep / 4)`, so a bulge is never
//! flattened to a chord.
//!
//! Cuts come from the same [`BoundarySet`] primitives the LINE/ARC/CIRCLE/
//! ELLIPSE/SPLINE targets use, and each segment is cut on its own parameter
//! ([`TargetCurve`] over that segment's LINE or exact bulge ARC): the analytic
//! conic solver object snap uses, not a polyline tessellation. A hit at a shared
//! vertex is reported by both neighbouring segments at the identical global
//! parameter and is de-duplicated once.
//!
//! The **kept** piece is the one containing the pick's projection — the project's
//! TRIM contract for every target kind (see `docs/drawing-edit.md` §3 and
//! `plan_trim`): TRIM keeps the clicked window and discards the rest. A twice-cut
//! open polyline therefore keeps one of its three windows as a single open
//! polyline (the target id, layer, space and display state are preserved; no new
//! entity is created), and a twice-cut closed polyline keeps the window containing
//! the pick as one open polyline that starts and ends at the two cuts (its
//! direction/vertex order is preserved). A closed polyline needs two cuts, exactly
//! like a CIRCLE, because one cut cannot separate a window from a ring. When there
//! are no cuts at all and the target lies wholly inside a closed boundary it is
//! deleted.
//!
//! Refusals (they write nothing): `trim.polyline_no_intersection`,
//! `trim.polyline_pick_outside`, `trim.polyline_degenerate` and
//! `trim.polyline_nonplanar_bulge`. A pick is **projected** onto the polyline (the
//! host resolved the entity with its own pickbox), and only a projection beyond an
//! open polyline's own ends is refused, never silently clamped.
//!
//! `SemanticGeometry::Polyline` stores no segment widths, so a wide polyline cannot
//! reach TRIM today and no width refusal is emitted; a future width field must add
//! the reserved `trim.polyline_width_unsupported` refusal here instead of trimming
//! a wide polyline as its centreline.

use super::polyline_curve::PolylineCurve;
use super::trim_boundary::{note_approximation, BoundarySet};
use super::trim_curves::{PlanRefusal, TargetCurve, TrimPlan, TrimResolution};
use super::trim_cuts::pair_has_no_unique_crossing;
use super::*;
use cad_geometry::{DefaultGeometryEngine, GeometryEngine};

/// A polyline-plan refusal: the stable `trim.polyline_*` code plus its detail.
fn refusal(code: &str, detail: impl std::fmt::Display) -> PlanRefusal {
    PlanRefusal::Invalid(format!("{code}: {detail}"))
}

/// Resolve a TRIM of a POLYLINE target, or refuse without touching the database.
///
/// See the module doc for the kept window, the exact surviving-bulge rule and the
/// refusal codes.
pub(crate) fn plan_trim_polyline(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    pick: Point3,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
) -> Result<TrimResolution, PlanRefusal> {
    let curve = validate(points, bulges, closed)?;
    let total = curve.total_length();
    if !total.is_finite() || total <= tol {
        return Err(refusal(
            "trim.polyline_degenerate",
            "the target polyline has no measurable length",
        ));
    }
    let segments = curve.segment_count();
    let n = segments as f64;
    // One world tolerance expressed in the polyline's own parameter units (one
    // unit per segment, of mean length `total / segments`).
    let param_tol = (tol * n / total).max(1e-12);
    let u_pick = project_pick(&curve, pick, tol, param_tol)?;

    let mut notes = Vec::new();
    let cuts = collect_cuts(&curve, boundaries, policy, tol, param_tol, &mut notes)?;

    // No cut at all is the containment question: a polyline wholly inside a closed
    // boundary is deleted before any "no intersection" refusal, exactly like the
    // other target kinds.
    if cuts.is_empty() {
        let probe = curve.point_at_distance(total * 0.5);
        if boundaries.contains(probe, tol) {
            return Ok(TrimResolution {
                plan: TrimPlan::Delete,
                notes,
            });
        }
        if boundaries.has_spline_boundary() {
            return Err(PlanRefusal::Unsupported(
                "TRIM found no intersection between the polyline target and the boundary, and \
                 the closed-SPLINE delete rule is not supported: a SPLINE boundary carries no \
                 closed flag and no boundary plane, so a polyline wholly inside a closed spline \
                 cannot be tested for containment; it is refused rather than deleted"
                    .into(),
            ));
        }
        return Err(refusal(
            "trim.polyline_no_intersection",
            "TRIM found no intersection between the polyline target and the boundary",
        ));
    }

    // `window` is the single kept window in the polyline's own parameter, in the
    // original travel direction: an open polyline is bounded by `0`/`n`, a closed
    // one wraps one full period past the cut where the kept window starts.
    let (start, end) = kept_window(&cuts, u_pick, closed, n, param_tol)?;
    if span_length(&curve, start, end) <= tol {
        return Err(refusal(
            "trim.polyline_degenerate",
            format!(
                "TRIM would reduce the target polyline to nothing: the kept span [{start}, {end}] \
                 is below the tolerance"
            ),
        ));
    }
    let geometry = curve.sub_curve(start, end).map_err(|error| {
        refusal(
            "trim.polyline_degenerate",
            format!("the kept polyline span [{start}, {end}] could not be emitted: {error}"),
        )
    })?;
    Ok(TrimResolution {
        plan: TrimPlan::Replace(geometry),
        notes,
    })
}

/// Validate and normalise the stored polyline through the **shared** model, so
/// TRIM reads bulges exactly like BREAK/DIVIDE (`0.0` = straight segment).
///
/// The two named cases are checked first, so the shared validator's own reasons
/// cannot become the user-visible code: non-finite vertices/bulges and fewer than
/// two vertices are `trim.polyline_degenerate`, and a bulged polyline that is not
/// `+Z` planar is `trim.polyline_nonplanar_bulge` (the bulge arc has no single
/// in-plane centre otherwise).
fn validate(points: &[Point3], bulges: &[f64], closed: bool) -> Result<PolylineCurve, PlanRefusal> {
    if points.len() < 2 {
        return Err(refusal(
            "trim.polyline_degenerate",
            "a polyline needs at least two vertices",
        ));
    }
    if points.iter().any(|point| !is_finite_point(*point))
        || bulges.iter().any(|bulge| !bulge.is_finite())
    {
        return Err(refusal(
            "trim.polyline_degenerate",
            "polyline vertices and bulges must be finite",
        ));
    }
    // One bulge per segment; a missing trailing entry is a straight segment, the
    // same reading the boundary collector and the tessellator use.
    let segments = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let mut normalised = bulges.to_vec();
    normalised.truncate(points.len());
    if normalised.len() < segments {
        normalised.resize(segments, 0.0);
    }
    let has_bulge = normalised.iter().any(|bulge| bulge.abs() > 1e-9);
    if has_bulge && !cad_geometry::is_xy_parallel(points) {
        return Err(refusal(
            "trim.polyline_nonplanar_bulge",
            "a bulged target polyline must be +Z planar to resolve its arc segments",
        ));
    }
    PolylineCurve::new(points, &normalised, closed, "trim").map_err(|error| {
        refusal(
            "trim.polyline_degenerate",
            format!("the target polyline has no usable parameterisation: {error}"),
        )
    })
}

/// The parameter of the **projection** of `pick`, in the polyline's own
/// `u ∈ [0, segments]` domain (wrapped for a closed polyline).
///
/// The pick is projected onto the nearest segment (the host resolved the target
/// with its own pickbox, so a near miss is still a pick of this entity). Unlike a
/// LINE target, an open polyline's parameter is clamped per segment by the shared
/// model, so the raw (unclamped) projection is read here to tell a pick beyond an
/// end apart from a legitimate off-curve pick: only the former is
/// `trim.polyline_pick_outside`, never a silent snap to the end vertex.
fn project_pick(
    curve: &PolylineCurve,
    pick: Point3,
    tol: f64,
    param_tol: f64,
) -> Result<f64, PlanRefusal> {
    let n = curve.segment_count() as f64;
    let mut best: Option<(f64, f64)> = None;
    for segment in 0..curve.segment_count() {
        let Some(geometry) = curve.segment_geometry(segment) else {
            continue;
        };
        let segment_curve = segment_curve(&geometry)?;
        let Some(local) = segment_curve.project_param(pick, tol) else {
            continue;
        };
        if !local.is_finite() {
            continue;
        }
        let clamped = local.clamp(0.0, 1.0);
        let distance = cad_geometry::distance(segment_curve.point_at(clamped), pick);
        if !distance.is_finite() {
            continue;
        }
        if best.is_none_or(|(_, best_distance)| distance < best_distance) {
            best = Some((segment as f64 + local, distance));
        }
    }
    let Some((raw, _)) = best else {
        return Err(refusal(
            "trim.polyline_degenerate",
            "the pick has no projection onto any polyline segment",
        ));
    };
    if curve.is_closed() {
        return Ok(raw.rem_euclid(n));
    }
    if raw < -param_tol || raw > n + param_tol {
        return Err(refusal(
            "trim.polyline_pick_outside",
            format!(
                "the pick projects outside the open polyline (parameter {raw:.9}, the target \
                 spans [0, {n}])"
            ),
        ));
    }
    Ok(raw.clamp(0.0, n))
}

/// Every boundary cut on the polyline, sorted in the polyline parameter and
/// de-duplicated (a shared vertex is one cut, not two).
fn collect_cuts(
    curve: &PolylineCurve,
    boundaries: &BoundarySet,
    policy: &TolerancePolicy,
    tol: f64,
    param_tol: f64,
    notes: &mut Vec<String>,
) -> Result<Vec<f64>, PlanRefusal> {
    let engine = DefaultGeometryEngine;
    let n = curve.segment_count() as f64;
    let closed = curve.is_closed();
    let mut cuts: Vec<f64> = Vec::new();
    for segment in 0..curve.segment_count() {
        let Some(geometry) = curve.segment_geometry(segment) else {
            continue;
        };
        let segment_curve = segment_curve(&geometry)?;
        let segment_length = curve.segment_length(segment);
        if !segment_length.is_finite() || segment_length <= tol {
            continue;
        }
        let local_tol = (tol / segment_length).max(1e-12);
        for primitive in boundaries.primitives() {
            if pair_has_no_unique_crossing(&segment_curve, primitive, tol) {
                continue;
            }
            let hits = engine
                .intersect_local(&geometry, primitive, policy)
                .map_err(|error| {
                    refusal(
                        "trim.polyline_degenerate",
                        format!("TRIM could not resolve a boundary intersection: {error}"),
                    )
                })?;
            let mut used = false;
            for hit in hits {
                let Some(local) = segment_curve.param_of(hit, tol) else {
                    continue;
                };
                // A hit outside this segment's own span (the other branch of a
                // bulge's circle, or beyond a straight segment) cannot cut it.
                if local < -local_tol || local > 1.0 + local_tol {
                    continue;
                }
                let local = local.clamp(0.0, 1.0);
                let u = if closed {
                    (segment as f64 + local).rem_euclid(n)
                } else {
                    // Only an interior cut can separate anything from an open
                    // polyline: a hit at its own start/end vertex removes nothing
                    // (the same rule every other target kind uses).
                    let u = segment as f64 + local;
                    if u <= param_tol || u >= n - param_tol {
                        continue;
                    }
                    u
                };
                if cuts.iter().any(|seen| {
                    let distance = (seen - u).abs();
                    let distance = if closed {
                        distance.min(n - distance)
                    } else {
                        distance
                    };
                    distance <= param_tol
                }) {
                    continue;
                }
                cuts.push(u);
                used = true;
            }
            // Only a pair that really contributed a cut is reported as sampled.
            if used {
                note_approximation(&geometry, primitive, policy, "TRIM", notes);
            }
        }
    }
    cuts.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    Ok(cuts)
}

/// The one window that is kept: the piece containing the **projection** of the
/// pick, exactly like `plan_trim`'s open branch for the single-curve targets.
///
/// For an open polyline the ends are `0`/`n`, so the windows are the pieces
/// between consecutive cuts and the pick selects one of them. For a closed
/// polyline the parameter wraps: the pick's window is `[start, start + length]`,
/// where `end` may run one full period past `n`. A closed polyline needs two cuts
/// for the window to be a proper piece of the ring (the same rule a CIRCLE keeps).
fn kept_window(
    cuts: &[f64],
    u_pick: f64,
    closed: bool,
    n: f64,
    param_tol: f64,
) -> Result<(f64, f64), PlanRefusal> {
    if closed {
        if cuts.len() < 2 {
            return Err(PlanRefusal::Invalid(format!(
                "TRIM of a closed polyline needs at least two boundary intersections to leave \
                 an open piece; found {}",
                cuts.len()
            )));
        }
        let next = cuts.iter().position(|cut| *cut > u_pick);
        return Ok(match next {
            Some(0) | None => (cuts[cuts.len() - 1], cuts[0] + n),
            Some(index) => (cuts[index - 1], cuts[index]),
        });
    }

    let mut breaks: Vec<f64> = Vec::with_capacity(cuts.len() + 2);
    breaks.push(0.0);
    breaks.extend(cuts.iter().copied());
    breaks.push(n);
    for window in breaks.windows(2) {
        if u_pick + param_tol >= window[0] && u_pick - param_tol <= window[1] {
            return Ok((window[0], window[1]));
        }
    }
    Err(refusal(
        "trim.polyline_degenerate",
        "TRIM could not determine the retained piece of the polyline target",
    ))
}

/// The exact arc length of the polyline span `[u_start, u_end]`: a segment's
/// length is linear in its local parameter (a chord for a straight segment, an
/// arc's `radius · |sweep|` for a bulge), so no sampling is involved.
fn span_length(curve: &PolylineCurve, u_start: f64, u_end: f64) -> f64 {
    if !u_start.is_finite() || !u_end.is_finite() || u_end <= u_start {
        return 0.0;
    }
    let segments = curve.segment_count();
    if segments == 0 {
        return 0.0;
    }
    let first = u_start.floor().max(0.0) as usize;
    let last = u_end.ceil().max(0.0) as usize;
    let mut total = 0.0;
    for global in first..last {
        let segment = if curve.is_closed() {
            global % segments
        } else if global < segments {
            global
        } else {
            break;
        };
        let lo = (u_start - global as f64).max(0.0);
        let hi = (u_end - global as f64).min(1.0);
        if hi > lo {
            total += curve.segment_length(segment) * (hi - lo);
        }
    }
    total
}

/// The segment's own [`TargetCurve`] (a LINE or the exact bulge ARC).
///
/// An unresolvable segment is a capability failure of the source data, reported as
/// `trim.polyline_degenerate` rather than silently skipped: a skipped segment would
/// move the cuts and keep the wrong piece.
fn segment_curve(geometry: &SemanticGeometry) -> Result<TargetCurve, PlanRefusal> {
    TargetCurve::new(geometry, "TRIM", "a LINE or ARC").map_err(|error| {
        refusal(
            "trim.polyline_degenerate",
            format!("a polyline segment has no cuttable parameterisation: {error}"),
        )
    })
}
