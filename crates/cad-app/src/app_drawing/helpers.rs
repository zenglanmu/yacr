use super::*;

/// A monotonic transaction id for drawing commits.
///
/// Mirrors the layer counter style but lives in the app layer so a
/// command never depends on a host. The counter is process-global and only needs
/// to be unique within a session's history.
pub(crate) fn next_transaction_id() -> TransactionId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    TransactionId(COUNTER.fetch_add(1, Ordering::Relaxed) as u128)
}

/// The stored class name for a semantic geometry, matching the importer's keys.
pub(crate) fn class_name(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "AcDbLine",
        SemanticGeometry::Circle { .. } => "AcDbCircle",
        SemanticGeometry::Polyline { .. } => "AcDbPolyline",
        SemanticGeometry::Arc { .. } => "AcDbArc",
        SemanticGeometry::Ellipse { .. } => "AcDbEllipse",
        SemanticGeometry::Spline { .. } => "AcDbSpline",
        SemanticGeometry::Mesh(_) => "AcDbSubDMesh",
        SemanticGeometry::Insert { .. } => "AcDbBlockReference",
        SemanticGeometry::Text { .. } => "AcDbText",
        SemanticGeometry::Shape { .. } => "AcDbShape",
        SemanticGeometry::Point(_) => "AcDbPoint",
        SemanticGeometry::Image { .. } => "AcDbRasterImage",
        SemanticGeometry::Mask { .. } => "AcDbWipeout",
        SemanticGeometry::Opaque { .. } => "AcDbUnknown",
        SemanticGeometry::Compound(_) => "AcDbCompound",
    }
}

/// A short, stable kind label for diagnostics (never a UI string).
pub(super) fn geometry_kind(geometry: &SemanticGeometry) -> &'static str {
    class_name(geometry)
}

pub(super) fn distance(a: Point3, b: Point3) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

pub(super) fn is_finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// The resolved boundary input for a trim/extend: straight segments plus closed
/// loops. `command` names the caller (TRIM/EXTEND) so an unsupported boundary is
/// reported with the operation that actually refused it.
pub(super) fn collect_boundary_segments(
    geometry: &SemanticGeometry,
    segments: &mut Vec<(Point3, Point3)>,
    closed_loops: &mut Vec<Vec<Point3>>,
    command: &str,
) -> CadResult<()> {
    match geometry {
        SemanticGeometry::Line { start, end } => {
            segments.push((*start, *end));
            Ok(())
        }
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => {
            if points.len() < 2 {
                return Ok(());
            }
            let straight = |i: usize| bulges.get(i).copied().unwrap_or(0.0).abs() <= 1e-12;
            for i in 0..points.len().saturating_sub(1) {
                if !straight(i) {
                    return Err(CadError::Unsupported(format!(
                        "{command} boundary contains a polyline arc segment; only straight \
                         LWPOLYLINE segments are supported"
                    )));
                }
                segments.push((points[i], points[i + 1]));
            }
            if *closed {
                let last = points[points.len() - 1];
                if !straight(points.len() - 1) {
                    return Err(CadError::Unsupported(format!(
                        "{command} boundary contains a closing arc segment; only straight \
                         LWPOLYLINE segments are supported"
                    )));
                }
                segments.push((last, points[0]));
                closed_loops.push(points.clone());
            }
            Ok(())
        }
        other => Err(CadError::Unsupported(format!(
            "{command} boundary {} is not supported; only LINE and straight LWPOLYLINE are",
            geometry_kind(other)
        ))),
    }
}

/// What a resolved trim will do to the target.
#[derive(Clone, Copy)]
pub(super) enum TrimPlan {
    /// The target is fully inside a closed boundary (or reduced to nothing).
    Delete,
    /// Keep the sub-segment from `start` to `end`.
    Keep { start: Point3, end: Point3 },
}

/// Resolve the trim cuts for a LINE target against straight boundary segments.
///
/// Returns `Err(reason)` for the documented degenerate/no-intersection cases so
/// the caller can raise `Unsupported` without touching the database.
pub(super) fn plan_trim(
    a: Point3,
    b: Point3,
    pick: Point3,
    segments: &[(Point3, Point3)],
    closed_loops: &[Vec<Point3>],
) -> Result<TrimPlan, String> {
    let policy = TolerancePolicy::default();
    let tol = policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world);
    let ab = Point3 {
        x: b.x - a.x,
        y: b.y - a.y,
        z: 0.0,
    };
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let len = len2.sqrt();
    if !len.is_finite() || len <= tol {
        return Err("TRIM target is degenerate (zero length)".into());
    }
    let param_tol = (tol / len).max(1e-12);
    let t_pick = ((pick.x - a.x) * ab.x + (pick.y - a.y) * ab.y) / len2;
    if !t_pick.is_finite() || t_pick < -param_tol || t_pick > 1.0 + param_tol {
        return Err("TRIM pick point does not lie on the target line".into());
    }
    let t_pick = t_pick.clamp(0.0, 1.0);

    // Collect interior intersection parameters, de-duplicated within tolerance.
    let mut cuts: Vec<f64> = Vec::new();
    for (c, d) in segments {
        if let Some(t) = segment_intersection_param(a, b, *c, *d, tol) {
            if t > param_tol
                && t < 1.0 - param_tol
                && !cuts
                    .iter()
                    .any(|existing| (existing - t).abs() <= param_tol * 2.0)
            {
                cuts.push(t);
            }
        }
    }
    cuts.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));

    if cuts.is_empty() {
        // No cut: the line is untouched unless it lies wholly inside a closed
        // boundary, in which case TRIM removes it entirely.
        let midpoint = Point3 {
            x: (a.x + b.x) * 0.5,
            y: (a.y + b.y) * 0.5,
            z: 0.0,
        };
        if closed_loops
            .iter()
            .any(|loop_pts| point_in_polygon(midpoint, loop_pts))
        {
            return Ok(TrimPlan::Delete);
        }
        return Err("TRIM found no intersection between the target and the boundary".into());
    }

    // Breakpoints partition [0, 1]; keep the piece containing the pick.
    let mut breaks: Vec<f64> = Vec::with_capacity(cuts.len() + 2);
    breaks.push(0.0);
    breaks.extend(cuts.iter().copied());
    breaks.push(1.0);
    let mut keep = None;
    for window in breaks.windows(2) {
        let (t0, t1) = (window[0], window[1]);
        if t_pick + param_tol >= t0 && t_pick - param_tol <= t1 {
            keep = Some((t0, t1));
            break;
        }
    }
    let Some((t0, t1)) = keep else {
        return Err("TRIM could not determine the retained segment".into());
    };
    // A full-span window means the boundary did not actually cut the target.
    if t0 <= param_tol && t1 >= 1.0 - param_tol {
        return Err("TRIM boundary does not cut the target".into());
    }
    let start = point_at(a, b, t0);
    let end = point_at(a, b, t1);
    if distance(start, end) <= tol {
        return Ok(TrimPlan::Delete);
    }
    Ok(TrimPlan::Keep { start, end })
}

fn point_at(a: Point3, b: Point3, t: f64) -> Point3 {
    Point3 {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        z: a.z + (b.z - a.z) * t,
    }
}

/// 2D parameter of the boundary segment `c→d` cut along the finite target
/// segment `a→b`, or `None` when the crossing is outside `[0, 1]`.
///
/// A thin wrapper over [`line_segment_intersection_param`]: parallel/collinear
/// segments return `None` (reported as an explicit no-intersection by
/// [`plan_trim`]) rather than fabricating an overlap cut, and a crossing beyond
/// the target's own span is filtered out.
fn segment_intersection_param(a: Point3, b: Point3, c: Point3, d: Point3, tol: f64) -> Option<f64> {
    let (t, _u) = line_segment_intersection_param(a, b, c, d, tol)?;
    let len = len2((b.x - a.x, b.y - a.y)).sqrt().max(f64::MIN_POSITIVE);
    let param_tol = (tol / len).max(1e-12);
    if t >= -param_tol && t <= 1.0 + param_tol {
        Some(t.clamp(0.0, 1.0))
    } else {
        None
    }
}

fn len2(v: (f64, f64)) -> f64 {
    v.0 * v.0 + v.1 * v.1
}

/// Default world tolerance used by the OFFSET construction.
fn offset_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}

/// Offset a LINE by the perpendicular distance to `side_point` (3D-exact).
///
/// Returns the parallel line and the offset distance. A degenerate source or a
/// point on the (infinite) line is an explicit refusal; the new line lies on the
/// side the point is on.
pub(super) fn offset_line(
    start: Point3,
    end: Point3,
    side_point: Point3,
) -> CadResult<(SemanticGeometry, f64)> {
    let tol = offset_tolerance();
    let dir = (end.x - start.x, end.y - start.y, end.z - start.z);
    let len = (dir.0 * dir.0 + dir.1 * dir.1 + dir.2 * dir.2).sqrt();
    if !len.is_finite() || len <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET source LINE is degenerate (zero length)".into(),
        ));
    }
    let u = (dir.0 / len, dir.1 / len, dir.2 / len);
    let v = (
        side_point.x - start.x,
        side_point.y - start.y,
        side_point.z - start.z,
    );
    let along = v.0 * u.0 + v.1 * u.1 + v.2 * u.2;
    let perp = (v.0 - along * u.0, v.1 - along * u.1, v.2 - along * u.2);
    let distance = (perp.0 * perp.0 + perp.1 * perp.1 + perp.2 * perp.2).sqrt();
    if !distance.is_finite() || distance <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET distance must be positive; pick a point off the line".into(),
        ));
    }
    let n = (perp.0 / distance, perp.1 / distance, perp.2 / distance);
    Ok((
        SemanticGeometry::Line {
            start: Point3 {
                x: start.x + distance * n.0,
                y: start.y + distance * n.1,
                z: start.z + distance * n.2,
            },
            end: Point3 {
                x: end.x + distance * n.0,
                y: end.y + distance * n.1,
                z: end.z + distance * n.2,
            },
        },
        distance,
    ))
}

/// Offset a CIRCLE so its new radius is `|side_point − centre|`.
///
/// The side is implicit: a point outside the circle grows it, inside shrinks it.
/// A point at the centre, or a point on the circle (a zero offset), is an
/// explicit refusal rather than a silent no-op.
pub(super) fn offset_circle(
    center: Point3,
    normal: Point3,
    radius: f64,
    side_point: Point3,
) -> CadResult<(SemanticGeometry, f64)> {
    let tol = offset_tolerance();
    let dx = side_point.x - center.x;
    let dy = side_point.y - center.y;
    let dz = side_point.z - center.z;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();
    if !distance.is_finite() || distance <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET distance must be positive; pick a point away from the centre".into(),
        ));
    }
    if (distance - radius.abs()).abs() <= tol {
        return Err(CadError::InvalidInput(
            "OFFSET distance must be positive; pick a point off the circle".into(),
        ));
    }
    Ok((
        SemanticGeometry::Circle {
            center,
            normal,
            radius: distance,
        },
        distance,
    ))
}

/// 2D intersection of the infinite line `a→b` with the boundary segment `c→d`.
///
/// Returns `(t, u)` where `a + t * (b - a)` is the crossing on the infinite
/// target line and `u ∈ [0, 1]` places it on the boundary segment. Parallel or
/// collinear segments, and crossings outside the boundary segment, return `None`
/// so [`plan_extend`] never fabricates a reach.
fn line_segment_intersection_param(
    a: Point3,
    b: Point3,
    c: Point3,
    d: Point3,
    tol: f64,
) -> Option<(f64, f64)> {
    let r = (b.x - a.x, b.y - a.y);
    let s = (d.x - c.x, d.y - c.y);
    let denom = r.0 * s.1 - r.1 * s.0;
    let scale = (len2(r).sqrt() * len2(s).sqrt()).max(f64::MIN_POSITIVE);
    if denom.abs() / scale <= tol.max(1e-12) {
        return None;
    }
    let ca = (c.x - a.x, c.y - a.y);
    let t = (ca.0 * s.1 - ca.1 * s.0) / denom;
    let u = (ca.0 * r.1 - ca.1 * r.0) / denom;
    if (-1e-9..=1.0 + 1e-9).contains(&u) {
        Some((t, u.clamp(0.0, 1.0)))
    } else {
        None
    }
}

/// Resolve where to extend a LINE target to the nearest boundary crossing.
///
/// `pick` selects which endpoint grows (the nearer end). Returns the new
/// `(start, end)`; `Err(reason)` for the documented degenerate, already-touching,
/// already-crossing and no-intersection cases so the caller can refuse without
/// touching the database.
pub(super) fn plan_extend(
    a: Point3,
    b: Point3,
    pick: Point3,
    segments: &[(Point3, Point3)],
) -> Result<(Point3, Point3), String> {
    let policy = TolerancePolicy::default();
    let tol = policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world);
    let ab = Point3 {
        x: b.x - a.x,
        y: b.y - a.y,
        z: 0.0,
    };
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let len = len2.sqrt();
    if !len.is_finite() || len <= tol {
        return Err("EXTEND target is degenerate (zero length)".into());
    }
    let param_tol = (tol / len).max(1e-12);
    let t_pick = ((pick.x - a.x) * ab.x + (pick.y - a.y) * ab.y) / len2;
    if !t_pick.is_finite() || t_pick < -param_tol || t_pick > 1.0 + param_tol {
        return Err("EXTEND pick point does not lie on the target line".into());
    }
    // The endpoint nearer the pick grows: `t_pick <= 0.5` extends `start`.
    let grow_start = t_pick <= 0.5;

    let mut touches = false;
    let mut crosses = false;
    // Nearest crossing below `t = 0` (largest t) and above `t = 1` (smallest t).
    let mut beyond_start: Option<f64> = None;
    let mut beyond_end: Option<f64> = None;
    for (c, d) in segments {
        let Some((t, _u)) = line_segment_intersection_param(a, b, *c, *d, tol) else {
            continue;
        };
        if t.abs() <= param_tol || (t - 1.0).abs() <= param_tol {
            touches = true;
        } else if t > param_tol && t < 1.0 - param_tol {
            crosses = true;
        } else if t < 0.0 {
            if beyond_start.is_none_or(|current| t > current) {
                beyond_start = Some(t);
            }
        } else if t > 1.0 && beyond_end.is_none_or(|current| t < current) {
            beyond_end = Some(t);
        }
    }
    // A target that already meets the boundary (at an endpoint or anywhere in
    // its span) is not extendable: refuse rather than push it through an edge it
    // already touches or crosses. Both refusals are unconditional.
    if touches {
        return Err("EXTEND target already touches the boundary".into());
    }
    if crosses {
        return Err("EXTEND target already reaches the boundary".into());
    }
    let chosen = if grow_start { beyond_start } else { beyond_end };
    if let Some(t) = chosen {
        let extended = point_at(a, b, t);
        return Ok(if grow_start {
            (extended, b)
        } else {
            (a, extended)
        });
    }
    Err("EXTEND found no intersection between the target and the boundary".into())
}

/// 2D point-in-polygon (ray casting) on the XY plane.
fn point_in_polygon(p: Point3, polygon: &[Point3]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let n = polygon.len();
    let mut j = n - 1;
    for i in 0..n {
        let pi = polygon[i];
        let pj = polygon[j];
        let crosses = (pi.y > p.y) != (pj.y > p.y);
        if crosses {
            let x_at = (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y) + pi.x;
            if p.x < x_at {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Apply a list of drawing patches to a database (used by undo/redo).
///
/// `forward` selects the `after` side (redo); otherwise the `before` side is
/// applied (undo). This goes through the database's single validated write path
/// so a patch that no longer matches the store is refused, never silently
/// force-written. A re-inserted side also restores its captured display state
/// (render attributes + dynamic-visibility membership), which the delete path
/// prunes and the entity alone does not carry.
pub(crate) fn apply_drawing_patches(
    drawing: &mut DrawingDatabase,
    reason: &str,
    transaction: TransactionId,
    patches: &[DrawingPatch],
    forward: bool,
) -> CadResult<ChangeSet> {
    let changes: Vec<(EntityId, Option<DbEntity>)> = patches
        .iter()
        .map(|p| {
            (
                p.id,
                if forward {
                    p.after.clone()
                } else {
                    p.before.clone()
                },
            )
        })
        .collect();
    let change_set = drawing.apply_drawing_changes(reason, transaction, changes)?;
    for patch in patches {
        let (entity, state) = if forward {
            (&patch.after, &patch.after_state)
        } else {
            (&patch.before, &patch.before_state)
        };
        if entity.is_some() {
            if let Some(state) = state {
                drawing.restore_entity_display_state(patch.id, state);
            }
        }
    }
    Ok(change_set)
}
