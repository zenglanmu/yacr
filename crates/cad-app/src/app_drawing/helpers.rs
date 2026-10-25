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
        // A hatch keeps its source type key so EXPLODE's `type_key` refusal and
        // the importer's `AcDbHatch` agree (ADR 0018 D/E).
        SemanticGeometry::Hatch(_) => "AcDbHatch",
        // A dimension keeps its source type key so EXPLODE's `type_key` refusal
        // and the importer's `AcDbDimension` agree (ADR 0017 A.8).
        SemanticGeometry::Dimension(_) => "AcDbDimension",
        // A multileader keeps its source type key so EXPLODE's explicit
        // `type_key` refusal and the importer's `AcDbMultiLeader` agree
        // (ADR 0019 decision G).
        SemanticGeometry::MultiLeader(_) => "AcDbMultiLeader",
    }
}

/// A short, stable kind label for diagnostics (never a UI string).
pub(super) fn geometry_kind(geometry: &SemanticGeometry) -> &'static str {
    class_name(geometry)
}

/// Whether a stored `Text` entity came from MTEXT (`AcDbMText`).
///
/// The string is the **raw** DXF content: only `AcDbMText` interprets `\P` (and
/// accepts line breaks) in the viewport, so the text-edit channels convert
/// newlines to `\P` only for that source class and refuse them for `AcDbText`
/// (ADR-0021 D4).
pub(super) fn is_multiline_text_source(entity: &DbEntity) -> bool {
    entity.object.type_key == text_policy::MTEXT_TYPE_KEY
}

/// The text style `TEXT`/`MTEXT` creation resolves by name before falling back to
/// the documented `StyleId(0)` entry (ADR 0019 F.1).
pub(super) const TEXT_STYLE_NAME: &str = "Standard";

/// Resolve the `(style, font)` pair a **created** text entity stores.
///
/// The single style path for both text-creation commands: `Standard` by name,
/// else the documented `StyleId(0)` fallback, else no resolved row — in which
/// case the style id is `0` and the font key is `None` (the import path's own
/// convention). Nothing is invented: a drawing without the style stores what the
/// fallback actually is.
pub(super) fn resolve_created_text_style(drawing: &DrawingDatabase) -> (StyleId, Option<String>) {
    let resolved = drawing
        .style_by_name(TEXT_STYLE_NAME)
        .or_else(|| drawing.style(StyleId(0)));
    (
        resolved.map(|style| style.id).unwrap_or(StyleId(0)),
        resolved.and_then(|style| style.resource_keys.first().cloned()),
    )
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

/// The default world tolerance used by the OFFSET construction.
fn offset_tolerance() -> f64 {
    let policy = TolerancePolicy::default();
    policy
        .computation_world
        .max(1e-12)
        .max(policy.topology_world)
}

/// The world tolerance shared by the CHAMFER/FILLET corner construction.
pub(super) fn corner_tolerance() -> f64 {
    offset_tolerance()
}

/// The two endpoints of a straight LINE pick, or an explicit `Unsupported` when
/// the picked entity is any other geometry kind.
pub(super) fn straight_line(
    geometry: &SemanticGeometry,
    command: &str,
) -> CadResult<(Point3, Point3)> {
    match geometry {
        SemanticGeometry::Line { start, end } => Ok((*start, *end)),
        other => Err(CadError::Unsupported(format!(
            "{command} needs straight LINE picks; {} is not supported",
            geometry_kind(other)
        ))),
    }
}

/// The resolved geometry of a two-line CHAMFER/FILLET edit: the new endpoints of
/// both picked lines plus the optional new entity.
///
/// `inserted` is `None` for the corner-only shapes (CHAMFER `A = B = 0`, FILLET
/// `R = 0`), which write **no** new entity — the ADR's decision C/B degenerate
/// case. Otherwise it is the joining LINE (CHAMFER) or the tangent ARC (FILLET).
pub(super) struct TwoLineEdit {
    /// The new `(start, end)` of the first-picked line, in its original order.
    pub first: (Point3, Point3),
    /// The new `(start, end)` of the second-picked line, in its original order.
    pub second: (Point3, Point3),
    pub inserted: Option<SemanticGeometry>,
}

/// The corner of the two infinite lines, the pick-side direction on each line,
/// and the surviving endpoint of each segment.
struct CornerFrame {
    corner: Point3,
    /// Unit direction from `corner` toward the first pick side.
    dir1: (f64, f64),
    /// Unit direction from `corner` toward the second pick side.
    dir2: (f64, f64),
    /// The endpoint of the first segment on the pick side (the kept end).
    keep1: Point3,
    keep1_is_start: bool,
    /// Distance from `corner` to `keep1` along `dir1` (strictly positive).
    reach1: f64,
    keep2: Point3,
    keep2_is_start: bool,
    reach2: f64,
}

/// The two straight-line picks (endpoints + raw pick point) a CHAMFER/FILLET
/// plans from, in pick order.
pub(super) struct TwoLineInput {
    /// The first-picked line's two endpoints.
    pub first: (Point3, Point3),
    /// The first raw pick point (selects the pick side on the first line).
    pub pick_first: Point3,
    /// The second-picked line's two endpoints.
    pub second: (Point3, Point3),
    /// The second raw pick point (selects the pick side on the second line).
    pub pick_second: Point3,
}

/// Resolve the corner and the pick side of two straight lines.
///
/// The **pick-side rule** (ADR 0016 decision B/C): each raw pick point is
/// projected onto its infinite line and its sign relative to the corner selects
/// the half that survives — exactly AutoCAD's "select the object closest to where
/// you want the result". A pick that projects onto the corner has no defined
/// side and is an explicit refusal. Parallel/collinear lines (no unique corner)
/// and non-coplanar (`+Z` subset) picks are refused too.
fn corner_frame(input: &TwoLineInput, tol: f64, command: &str) -> Result<CornerFrame, String> {
    let TwoLineInput {
        first: (a1, a2),
        pick_first: p1,
        second: (b1, b2),
        pick_second: p2,
    } = *input;
    if !is_finite_point(a1)
        || !is_finite_point(a2)
        || !is_finite_point(p1)
        || !is_finite_point(b1)
        || !is_finite_point(b2)
        || !is_finite_point(p2)
    {
        return Err(format!("{command} picks must be finite"));
    }
    // Flat `+Z` subset: two coplanar straight lines are a planar construction, so
    // a non-coplanar pair is refused rather than silently projected.
    let z_tol = 1e-9 * (1.0 + a1.z.abs() + a2.z.abs() + b1.z.abs() + b2.z.abs());
    if (a2.z - a1.z).abs() > z_tol || (b1.z - a1.z).abs() > z_tol || (b2.z - a1.z).abs() > z_tol {
        return Err(format!("{command} needs two coplanar (+Z) straight lines"));
    }
    let (u1x, u1y) = unit_xy(a1, a2, tol, command)?;
    let (u2x, u2y) = unit_xy(b1, b2, tol, command)?;
    let corner = match cad_geometry::infinite_line_intersection(a1, a2, b1, b2, tol) {
        cad_geometry::LineIntersection::Point(c) => c,
        cad_geometry::LineIntersection::Parallel | cad_geometry::LineIntersection::Collinear => {
            return Err(format!("{command} needs two non-parallel lines"));
        }
    };
    let s1 = (p1.x - corner.x) * u1x + (p1.y - corner.y) * u1y;
    if !s1.is_finite() || s1.abs() <= tol {
        return Err(format!("{command} pick must be away from the corner"));
    }
    let dir1 = if s1 > 0.0 { (u1x, u1y) } else { (-u1x, -u1y) };
    let s2 = (p2.x - corner.x) * u2x + (p2.y - corner.y) * u2y;
    if !s2.is_finite() || s2.abs() <= tol {
        return Err(format!("{command} pick must be away from the corner"));
    }
    let dir2 = if s2 > 0.0 { (u2x, u2y) } else { (-u2x, -u2y) };
    // The surviving endpoint is the one on the pick side of the corner.
    let along1_start = (a1.x - corner.x) * dir1.0 + (a1.y - corner.y) * dir1.1;
    let along1_end = (a2.x - corner.x) * dir1.0 + (a2.y - corner.y) * dir1.1;
    let keep1_is_start = along1_start >= along1_end;
    let (keep1, reach1) = if keep1_is_start {
        (a1, along1_start)
    } else {
        (a2, along1_end)
    };
    let along2_start = (b1.x - corner.x) * dir2.0 + (b1.y - corner.y) * dir2.1;
    let along2_end = (b2.x - corner.x) * dir2.0 + (b2.y - corner.y) * dir2.1;
    let keep2_is_start = along2_start >= along2_end;
    let (keep2, reach2) = if keep2_is_start {
        (b1, along2_start)
    } else {
        (b2, along2_end)
    };
    Ok(CornerFrame {
        corner,
        dir1,
        dir2,
        keep1,
        keep1_is_start,
        reach1,
        keep2,
        keep2_is_start,
        reach2,
    })
}

/// A unit XY direction from `from` to `to`, or a degeneracy refusal.
fn unit_xy(from: Point3, to: Point3, tol: f64, command: &str) -> Result<(f64, f64), String> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = dx.hypot(dy);
    if !len.is_finite() || len <= tol {
        return Err(format!("{command} needs two non-degenerate straight lines"));
    }
    Ok((dx / len, dy / len))
}

/// The point at `dist` from `corner` along the unit XY direction `dir`.
fn point_from(corner: Point3, dir: (f64, f64), dist: f64) -> Point3 {
    Point3 {
        x: corner.x + dir.0 * dist,
        y: corner.y + dir.1 * dist,
        z: corner.z,
    }
}

/// Rebuild a segment in its original order with the corner-side endpoint replaced
/// by `target` (the kept endpoint is on the pick side and never moves).
fn retarget(keep_is_start: bool, keep: Point3, target: Point3) -> (Point3, Point3) {
    if keep_is_start {
        (keep, target)
    } else {
        (target, keep)
    }
}

/// Plan a CHAMFER of two straight lines (ADR 0016 decision C).
///
/// `distance_a` applies to the **first-picked** line and `distance_b` to the
/// second (AutoCAD's `CHAMFER` order). `A = B = 0` trims/extends both lines to
/// their corner and inserts **no** new entity; otherwise both lines are trimmed
/// to the offset points `corner + distance·dir` and one joining LINE is inserted.
/// Explicit refusals (never a silent no-op): non-finite/negative distances; every
/// [`corner_frame`] refusal; and a distance too long for its line (which would
/// collapse or reverse the retained segment).
pub(super) fn plan_chamfer(
    input: &TwoLineInput,
    distance_a: f64,
    distance_b: f64,
    tol: f64,
) -> Result<TwoLineEdit, String> {
    if !distance_a.is_finite() || distance_a < 0.0 || !distance_b.is_finite() || distance_b < 0.0 {
        return Err("CHAMFER distances must be finite and non-negative".into());
    }
    let frame = corner_frame(input, tol, "CHAMFER")?;
    if distance_a >= frame.reach1 - tol {
        return Err("CHAMFER distance A is too long for the first line".into());
    }
    if distance_b >= frame.reach2 - tol {
        return Err("CHAMFER distance B is too long for the second line".into());
    }
    let t1 = point_from(frame.corner, frame.dir1, distance_a);
    let t2 = point_from(frame.corner, frame.dir2, distance_b);
    let first = retarget(frame.keep1_is_start, frame.keep1, t1);
    let second = retarget(frame.keep2_is_start, frame.keep2, t2);
    let inserted = if distance_a == 0.0 && distance_b == 0.0 {
        None
    } else {
        if distance(t1, t2) <= tol {
            return Err("CHAMFER produces a degenerate joining segment".into());
        }
        Some(SemanticGeometry::Line { start: t1, end: t2 })
    };
    Ok(TwoLineEdit {
        first,
        second,
        inserted,
    })
}

/// Plan a FILLET of two straight lines (ADR 0016 decision B).
///
/// `R = 0` trims/extends both lines to their corner and inserts **no** new entity
/// — the exact result of CHAMFER `0/0` (the ADR names the parity test). `R > 0`
/// inserts a tangent ARC: the centre is at distance `R` from both lines on the
/// pick side, the tangent distance from the corner is `R / tan(θ/2)` (θ the angle
/// between the two pick-side rays), and both lines are trimmed to the tangent
/// points.
///
/// **The centre construction is UNVERIFIED (geometric inference)**: Autodesk
/// publishes no formula for the FILLET centre, so the ADR requires this label and
/// a test pinning the pick-side inference. Explicit refusals: a non-finite or
/// negative radius; every [`corner_frame`] refusal (including **parallel** lines —
/// AutoCAD auto-adjusts the radius, we refuse); and a radius too large for either
/// line.
pub(super) fn plan_fillet(
    input: &TwoLineInput,
    radius: f64,
    tol: f64,
) -> Result<TwoLineEdit, String> {
    if !radius.is_finite() || radius < 0.0 {
        return Err("FILLET radius must be finite and non-negative".into());
    }
    let frame = corner_frame(input, tol, "FILLET")?;
    if radius == 0.0 {
        if frame.reach1 <= tol || frame.reach2 <= tol {
            return Err("FILLET cannot reach the corner (degenerate pick side)".into());
        }
        return Ok(TwoLineEdit {
            first: retarget(frame.keep1_is_start, frame.keep1, frame.corner),
            second: retarget(frame.keep2_is_start, frame.keep2, frame.corner),
            inserted: None,
        });
    }
    // UNVERIFIED (geometric inference): the tangent-arc centre formula below is
    // inferred, not quoted from Autodesk. The tangent distance from the corner is
    // `d = R / tan(θ/2)` and the centre lies on the pick-side bisector at
    // `R / sin(θ/2)` from the corner.
    let dot = (frame.dir1.0 * frame.dir2.0 + frame.dir1.1 * frame.dir2.1).clamp(-1.0, 1.0);
    let theta = dot.acos();
    let half = theta * 0.5;
    let (sin_half, cos_half) = half.sin_cos();
    if !sin_half.is_finite() || sin_half.abs() <= 1e-12 || !cos_half.is_finite() {
        return Err("FILLET needs two lines that meet at a real angle".into());
    }
    let tan_half = sin_half / cos_half;
    if !tan_half.is_finite() || tan_half.abs() <= 1e-12 {
        return Err("FILLET needs two lines that meet at a real angle".into());
    }
    let tangent_distance = radius / tan_half;
    if !tangent_distance.is_finite()
        || tangent_distance >= frame.reach1 - tol
        || tangent_distance >= frame.reach2 - tol
    {
        return Err("FILLET radius is too large for the picked lines".into());
    }
    let t1 = point_from(frame.corner, frame.dir1, tangent_distance);
    let t2 = point_from(frame.corner, frame.dir2, tangent_distance);
    let bisector = (frame.dir1.0 + frame.dir2.0, frame.dir1.1 + frame.dir2.1);
    let bisector_len = bisector.0.hypot(bisector.1);
    if !bisector_len.is_finite() || bisector_len <= 1e-12 {
        return Err("FILLET needs two lines that meet at a real angle".into());
    }
    let centre_distance = radius / sin_half;
    let centre = Point3 {
        x: frame.corner.x + bisector.0 / bisector_len * centre_distance,
        y: frame.corner.y + bisector.1 / bisector_len * centre_distance,
        z: frame.corner.z,
    };
    let start = (t1.y - centre.y).atan2(t1.x - centre.x);
    let end = (t2.y - centre.y).atan2(t2.x - centre.x);
    let sweep = normalize_signed(end - start);
    if !sweep.is_finite() || sweep.abs() <= 1e-12 {
        return Err("FILLET arc is degenerate".into());
    }
    let arc = SemanticGeometry::Arc {
        center: centre,
        normal: Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        radius,
        start,
        sweep,
    };
    Ok(TwoLineEdit {
        first: retarget(frame.keep1_is_start, frame.keep1, t1),
        second: retarget(frame.keep2_is_start, frame.keep2, t2),
        inserted: Some(arc),
    })
}

/// Normalise an angle into `(-π, π]` (the signed minor sweep).
pub(super) fn normalize_signed(angle: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut value = angle % tau;
    if value <= -std::f64::consts::PI {
        value += tau;
    } else if value > std::f64::consts::PI {
        value -= tau;
    }
    value
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
    let mut change_set = drawing.apply_drawing_changes(reason, transaction, changes)?;
    let mut restored: std::collections::BTreeSet<EntityId> = std::collections::BTreeSet::new();
    for patch in patches {
        let (entity, state) = if forward {
            (&patch.after, &patch.after_state)
        } else {
            (&patch.before, &patch.before_state)
        };
        if entity.is_some() {
            if let Some(state) = state {
                drawing.restore_entity_display_state(patch.id, state);
                restored.insert(patch.id);
            }
        }
    }
    // Restoring display attributes is a style change: without this the
    // ChangeSet would describe only the entity row and a consumer would not
    // rebuild the representation (colour/linetype/lineweight/opacity).
    for change in &mut change_set.changes {
        if let cad_db::ObjectChange::Update(object, mask) = change {
            if restored.contains(&EntityId(object.0)) {
                *mask = mask.union(cad_db::ChangeMask::STYLE);
            }
        }
    }
    Ok(change_set)
}

#[cfg(test)]
mod tests {
    use super::{plan_chamfer, plan_fillet, TwoLineInput};
    use cad_domain::{Point3, SemanticGeometry};

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// The L corner: L1 the +x axis, L2 the +y axis, picked on the +x/+y arms.
    fn corner_input() -> TwoLineInput {
        TwoLineInput {
            first: (p(0.0, 0.0), p(10.0, 0.0)),
            pick_first: p(8.0, 0.0),
            second: (p(0.0, 0.0), p(0.0, 10.0)),
            pick_second: p(0.0, 8.0),
        }
    }

    /// The plan-level refusals the session typed channel cannot carry (it refuses
    /// a negative/non-finite value before it can reach the command), so they are
    /// pinned directly on the pure helpers.
    #[test]
    fn plan_refuses_negative_or_non_finite_distance_and_radius() {
        let input = corner_input();
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(
                plan_chamfer(&input, bad, 1.0, 1e-9).is_err(),
                "CHAMFER A = {bad} must be refused"
            );
            assert!(
                plan_chamfer(&input, 1.0, bad, 1e-9).is_err(),
                "CHAMFER B = {bad} must be refused"
            );
            assert!(
                plan_fillet(&input, bad, 1e-9).is_err(),
                "FILLET R = {bad} must be refused"
            );
        }
    }

    #[test]
    fn plan_refuses_a_degenerate_line_and_a_collinear_pair() {
        // A zero-length first line has no direction.
        let degenerate = TwoLineInput {
            first: (p(0.0, 0.0), p(0.0, 0.0)),
            pick_first: p(8.0, 0.0),
            second: (p(0.0, 0.0), p(0.0, 10.0)),
            pick_second: p(0.0, 8.0),
        };
        assert!(plan_chamfer(&degenerate, 1.0, 1.0, 1e-9).is_err());
        // Collinear lines share no unique corner.
        let collinear = TwoLineInput {
            first: (p(0.0, 0.0), p(10.0, 0.0)),
            pick_first: p(8.0, 0.0),
            second: (p(2.0, 0.0), p(3.0, 0.0)),
            pick_second: p(2.5, 0.0),
        };
        assert!(plan_fillet(&collinear, 1.0, 1e-9).is_err());
    }

    #[test]
    fn fillet_zero_radius_equals_chamfer_zero_zero_at_the_plan_level() {
        let input = corner_input();
        let chamfer = plan_chamfer(&input, 0.0, 0.0, 1e-9).unwrap();
        let fillet = plan_fillet(&input, 0.0, 1e-9).unwrap();
        assert_eq!(chamfer.first, fillet.first);
        assert_eq!(chamfer.second, fillet.second);
        assert!(chamfer.inserted.is_none() && fillet.inserted.is_none());
    }

    #[test]
    fn fillet_centre_is_at_the_radius_from_both_lines() {
        let edit = plan_fillet(&corner_input(), 2.0, 1e-9).unwrap();
        match edit.inserted {
            Some(SemanticGeometry::Arc { center, radius, .. }) => {
                assert!((radius - 2.0).abs() < 1e-9);
                // Distance from the centre to the x-axis (L1) and y-axis (L2).
                assert!((center.y.abs() - radius).abs() < 1e-9, "{center:?}");
                assert!((center.x.abs() - radius).abs() < 1e-9, "{center:?}");
            }
            other => panic!("expected a tangent ARC, got {other:?}"),
        }
    }
}
