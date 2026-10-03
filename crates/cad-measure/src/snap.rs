//! Object snap engine (spec §3.3, F06).
//!
//! Snapping is computed from a **local** set of targets the caller has already
//! narrowed with the spatial index; the engine never iterates the drawing.
//! Candidates are filtered against a pick ray (`t >= 0` in front of the origin)
//! and a tolerance expressed in logical pixels, converted to world units with
//! `world_per_px`. Every candidate carries its [`SnapKind`], world point, source
//! entity/sub-element and [`Precision`] so the UI can show it and a measurement
//! can record its provenance.
//!
//! Available snaps: endpoint, midpoint, center, quadrant, perpendicular and
//! local intersection. Curved geometry is handled analytically (a bulge arc is
//! reconstructed from its bulge, an ellipse from its axes); nothing here is
//! derived from the display LOD, so a change of display density cannot change a
//! snap result.

use cad_db::MeasurementRecord;
use cad_domain::*;
use cad_geometry::{
    add, arbitrary_axis, distance, dot, length, normalize, scale, sub, tessellate_spline,
    DefaultGeometryEngine, GeometryEngine, TessellationParams,
};

/// The kind of object snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Center,
    Quadrant,
    Perpendicular,
    LocalIntersection,
}

impl SnapKind {
    /// Stable identifier for diagnostics and UI labels.
    pub fn as_str(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "endpoint",
            SnapKind::Midpoint => "midpoint",
            SnapKind::Center => "center",
            SnapKind::Quadrant => "quadrant",
            SnapKind::Perpendicular => "perpendicular",
            SnapKind::LocalIntersection => "intersection",
        }
    }

    /// Tie-break order: earlier kinds win when distances are equal.
    fn priority(self) -> u8 {
        match self {
            SnapKind::Endpoint => 0,
            SnapKind::Midpoint => 1,
            SnapKind::Center => 2,
            SnapKind::Quadrant => 3,
            SnapKind::Perpendicular => 4,
            SnapKind::LocalIntersection => 5,
        }
    }
}

/// One resolved snap candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapCandidate {
    pub kind: SnapKind,
    pub point: Point3,
    /// Space the candidate's geometry lives in, used for space filtering.
    pub space: SpaceId,
    /// Source entity plus the sub-element the snap came from.
    pub source: SelectionRef,
    /// For an intersection, the second participating entity.
    pub secondary: Option<SelectionRef>,
    /// Precision of the geometry the candidate was derived from.
    pub precision: Precision,
    /// Perpendicular distance from the cursor, in logical pixels.
    pub logical_pixel_distance: f64,
}

/// A local snap target: entity geometry plus its source/precision context.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapTarget {
    pub geometry: SemanticGeometry,
    pub source: SelectionRef,
    pub space: SpaceId,
    pub precision: Precision,
}

impl SnapTarget {
    pub fn new(
        geometry: SemanticGeometry,
        source: SelectionRef,
        space: SpaceId,
        precision: Precision,
    ) -> Self {
        SnapTarget {
            geometry,
            source,
            space,
            precision,
        }
    }
}

/// The provenance of one snapped input point, suitable for a measurement record.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapProvenance {
    pub kind: SnapKind,
    pub source: SelectionRef,
    pub secondary: Option<SelectionRef>,
    pub precision: Precision,
    pub logical_pixel_distance: f64,
}

impl SnapProvenance {
    /// Build provenance from a resolved candidate.
    pub fn from_candidate(candidate: &SnapCandidate) -> Self {
        SnapProvenance {
            kind: candidate.kind,
            source: candidate.source.clone(),
            secondary: candidate.secondary.clone(),
            precision: candidate.precision.clone(),
            logical_pixel_distance: candidate.logical_pixel_distance,
        }
    }
}

/// A measurement together with the snap provenance of each input point.
///
/// [`MeasurementRecord`] itself is owned by the database layer and cannot gain
/// a snap field here; this wrapper carries the provenance beside it so a caller
/// can persist both without losing the record's contract.
#[derive(Debug, Clone, PartialEq)]
pub struct SnappedMeasurement {
    pub record: MeasurementRecord,
    pub snaps: Vec<SnapProvenance>,
}

/// Upper bound on the number of local targets for which pairwise intersection
/// snap is attempted. Point snaps are still produced above it; only the
/// quadratic intersection pass is skipped.
pub const MAX_INTERSECTION_TARGETS: usize = 64;

const TAU: f64 = std::f64::consts::TAU;
const FRAC_PI_2: f64 = std::f64::consts::FRAC_PI_2;
const EPS: f64 = 1e-12;

struct Raw {
    kind: SnapKind,
    point: Point3,
    key: String,
}

/// Compute every snap candidate within tolerance of the pick ray.
///
/// `targets` must already be the **local** subset selected by the caller. The
/// query is rejected (`InvalidInput`) for a non-finite or degenerate ray, a
/// non-positive/non-finite `world_per_px`, or a non-positive tolerance. A
/// parallel ray that never meets `plane` yields no candidates.
pub fn collect_candidates(
    targets: &[SnapTarget],
    ray: &Ray3,
    plane: Option<&WorkPlane>,
    world_per_px: f64,
    space_filter: Option<&SpaceId>,
    policy: &TolerancePolicy,
) -> CadResult<Vec<SnapCandidate>> {
    let dir = unit_direction(ray)?;
    if !world_per_px.is_finite() || world_per_px <= 0.0 {
        return Err(CadError::InvalidInput(
            "snap world_per_px must be finite and positive".to_string(),
        ));
    }
    let tolerance = policy.interaction_logical_pixels * world_per_px;
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(CadError::InvalidInput(
            "snap tolerance must be finite and positive".to_string(),
        ));
    }

    let probe = match plane {
        Some(work) => match ray_plane_point(ray, &dir, work) {
            Some(point) => Some(point),
            // A plane was requested but the ray is parallel to it (or hits it
            // behind the origin): there is no cursor position to snap to.
            None => return Ok(Vec::new()),
        },
        None => None,
    };

    let mut out: Vec<SnapCandidate> = Vec::new();
    for target in targets {
        if let Some(filter) = space_filter {
            if target.space != *filter {
                continue;
            }
        }
        if !cad_geometry::geometry_is_finite(&target.geometry) {
            // A poisoned target cannot contribute a trustworthy snap.
            continue;
        }
        let mut raw: Vec<Raw> = Vec::new();
        generate(&target.geometry, probe, &mut raw, "");
        for r in raw {
            if let Some(candidate) = accept(
                r,
                target,
                None,
                ray,
                &dir,
                probe,
                world_per_px,
                tolerance,
                policy,
            ) {
                out.push(candidate);
            }
        }
    }

    // Local intersections only: pairs within the supplied local set, so this is
    // never an all-pairs pass over the drawing.
    if targets.len() <= MAX_INTERSECTION_TARGETS {
        let engine = DefaultGeometryEngine;
        for i in 0..targets.len() {
            for j in (i + 1)..targets.len() {
                let (a, b) = (&targets[i], &targets[j]);
                if a.space != b.space || !engine_is_local(a, b) {
                    continue;
                }
                if let Some(filter) = space_filter {
                    if a.space != *filter {
                        continue;
                    }
                }
                if !cad_geometry::geometry_is_finite(&a.geometry)
                    || !cad_geometry::geometry_is_finite(&b.geometry)
                {
                    continue;
                }
                let points = engine.intersect_local(&a.geometry, &b.geometry, policy)?;
                for point in points {
                    let raw = Raw {
                        kind: SnapKind::LocalIntersection,
                        point,
                        key: "intersection".to_string(),
                    };
                    if let Some(candidate) = accept(
                        raw,
                        a,
                        Some(b.source.clone()),
                        ray,
                        &dir,
                        probe,
                        world_per_px,
                        tolerance,
                        policy,
                    ) {
                        out.push(candidate);
                    }
                }
            }
        }
    }

    dedup(&mut out);
    out.sort_by(|x, y| {
        x.logical_pixel_distance
            .partial_cmp(&y.logical_pixel_distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.kind.priority().cmp(&y.kind.priority()))
            .then_with(|| {
                x.point
                    .x
                    .partial_cmp(&y.point.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                x.point
                    .y
                    .partial_cmp(&y.point.y)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    Ok(out)
}

/// Best candidate by logical pixel distance, if any.
pub fn best_candidate(
    targets: &[SnapTarget],
    ray: &Ray3,
    plane: Option<&WorkPlane>,
    world_per_px: f64,
    space_filter: Option<&SpaceId>,
    policy: &TolerancePolicy,
) -> CadResult<Option<SnapCandidate>> {
    Ok(
        collect_candidates(targets, ray, plane, world_per_px, space_filter, policy)?
            .into_iter()
            .next(),
    )
}

/// Intersections are only meaningful between polyline-like curves, not against
/// a mesh or opaque payload.
fn engine_is_local(a: &SnapTarget, b: &SnapTarget) -> bool {
    is_intersectable(&a.geometry) && is_intersectable(&b.geometry)
}

fn is_intersectable(geometry: &SemanticGeometry) -> bool {
    use SemanticGeometry as G;
    match geometry {
        G::Line { .. }
        | G::Polyline { .. }
        | G::Circle { .. }
        | G::Arc { .. }
        | G::Ellipse { .. }
        | G::Spline { .. } => true,
        G::Compound(children) => children.iter().any(is_intersectable),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn accept(
    raw: Raw,
    target: &SnapTarget,
    secondary: Option<SelectionRef>,
    ray: &Ray3,
    dir: &Point3,
    probe: Option<Point3>,
    world_per_px: f64,
    tolerance: f64,
    _policy: &TolerancePolicy,
) -> Option<SnapCandidate> {
    if !cad_geometry::is_finite(raw.point) {
        return None;
    }
    let v = sub(raw.point, ray.origin);
    let proj = dot(v, *dir);
    // Only in front of the pick-ray origin.
    if !proj.is_finite() || proj < 0.0 {
        return None;
    }
    let perpendicular = match probe {
        Some(cursor) => distance(raw.point, cursor),
        None => {
            let closest = add(ray.origin, scale(*dir, proj));
            distance(raw.point, closest)
        }
    };
    if !perpendicular.is_finite() || perpendicular > tolerance {
        return None;
    }
    let mut source = target.source.clone();
    if !raw.key.is_empty() {
        source.sub_element = Some(SubElementId {
            source_key: raw.key,
            topology_revision: Revision(0),
        });
    }
    Some(SnapCandidate {
        kind: raw.kind,
        point: raw.point,
        space: target.space.clone(),
        source,
        secondary,
        precision: target.precision.clone(),
        logical_pixel_distance: perpendicular / world_per_px,
    })
}

/// Remove exact duplicates (same kind, point, entity and sub-element).
fn dedup(out: &mut Vec<SnapCandidate>) {
    let mut seen: Vec<(SnapKind, Point3, u128, String)> = Vec::with_capacity(out.len());
    out.retain(|c| {
        let key = (
            c.kind,
            c.point,
            c.source.entity.0,
            c.source
                .sub_element
                .as_ref()
                .map(|s| s.source_key.clone())
                .unwrap_or_default(),
        );
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

fn unit_direction(ray: &Ray3) -> CadResult<Point3> {
    if !cad_geometry::is_finite(ray.origin) || !cad_geometry::is_finite(ray.direction) {
        return Err(CadError::InvalidInput(
            "snap ray has non-finite values".to_string(),
        ));
    }
    let len = length(ray.direction);
    if !len.is_finite() || len < EPS {
        return Err(CadError::InvalidInput(
            "snap ray direction is degenerate".to_string(),
        ));
    }
    Ok(scale(ray.direction, 1.0 / len))
}

/// Ray ∩ plane for a finite, non-degenerate plane, in front of the origin.
fn ray_plane_point(ray: &Ray3, dir: &Point3, plane: &WorkPlane) -> Option<Point3> {
    if !cad_geometry::is_finite(plane.origin)
        || !cad_geometry::is_finite(plane.u)
        || !cad_geometry::is_finite(plane.v)
    {
        return None;
    }
    let normal = cad_geometry::cross(plane.u, plane.v);
    if length(normal) < EPS {
        return None;
    }
    let normal = normalize(normal);
    if !cad_geometry::is_finite(normal) {
        return None;
    }
    let denom = dot(*dir, normal);
    if denom.abs() < EPS {
        return None;
    }
    let t = dot(sub(plane.origin, ray.origin), normal) / denom;
    if !t.is_finite() || t < 0.0 {
        return None;
    }
    Some(add(ray.origin, scale(*dir, t)))
}

// ---- candidate generation -------------------------------------------------

fn generate(geometry: &SemanticGeometry, probe: Option<Point3>, out: &mut Vec<Raw>, prefix: &str) {
    use SemanticGeometry as G;
    match geometry {
        G::Line { start, end } => line_candidates(*start, *end, probe, out, prefix),
        G::Polyline {
            points,
            bulges,
            closed,
        } => polyline_candidates(points, bulges, *closed, probe, out, prefix),
        G::Circle {
            center,
            normal,
            radius,
        } => circle_candidates(*center, *normal, *radius, probe, out, prefix),
        G::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => arc_candidates(
            *center, *normal, *radius, *start, *sweep, probe, out, prefix,
        ),
        G::Ellipse {
            center,
            major_axis,
            ratio,
            start,
            sweep,
            // The minor axis convention is `cross(world_z, major)`; any other
            // ellipse metadata is ignored here.
            ..
        } => ellipse_candidates(
            *center,
            *major_axis,
            *ratio,
            *start,
            *sweep,
            probe,
            out,
            prefix,
        ),
        G::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => spline_candidates(control_points, knots, weights, *degree, probe, out, prefix),
        G::Point(p) => push_point(out, SnapKind::Endpoint, *p, format!("{prefix}point")),
        G::Compound(children) => {
            for (i, child) in children.iter().enumerate() {
                generate(child, probe, out, &format!("{prefix}child:{i}:"));
            }
        }
        G::Mesh(_) | G::Insert { .. } | G::Text { .. } | G::Shape { .. } | G::Opaque { .. } => {}
    }
}

fn push_point(out: &mut Vec<Raw>, kind: SnapKind, point: Point3, key: String) {
    if cad_geometry::is_finite(point) {
        out.push(Raw { kind, point, key });
    }
}

fn midpoint(a: Point3, b: Point3) -> Point3 {
    scale(add(a, b), 0.5)
}

fn line_candidates(a: Point3, b: Point3, probe: Option<Point3>, out: &mut Vec<Raw>, prefix: &str) {
    push_point(out, SnapKind::Endpoint, a, format!("{prefix}vertex:0"));
    push_point(out, SnapKind::Endpoint, b, format!("{prefix}vertex:1"));
    push_point(
        out,
        SnapKind::Midpoint,
        midpoint(a, b),
        format!("{prefix}edge:0"),
    );
    if let Some(p) = probe {
        if let Some(foot) = foot_on_segment(p, a, b) {
            push_point(
                out,
                SnapKind::Perpendicular,
                foot,
                format!("{prefix}edge:0"),
            );
        }
    }
}

fn foot_on_segment(p: Point3, a: Point3, b: Point3) -> Option<Point3> {
    let d = sub(b, a);
    let len2 = dot(d, d);
    if !len2.is_finite() || len2 < EPS {
        return None;
    }
    let t = dot(sub(p, a), d) / len2;
    if !t.is_finite() || !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some(add(a, scale(d, t)))
}

fn polyline_candidates(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    let n = points.len();
    if n == 0 {
        return;
    }
    for (i, p) in points.iter().enumerate() {
        push_point(out, SnapKind::Endpoint, *p, format!("{prefix}vertex:{i}"));
    }
    if n < 2 {
        return;
    }
    let normal = polyline_plane_normal(points);
    let last = if closed { n } else { n - 1 };
    for i in 0..last {
        let a = points[i];
        let b = points[(i + 1) % n];
        let bulge = bulges.get(i).copied().unwrap_or(0.0);
        if bulge.abs() < EPS {
            push_point(
                out,
                SnapKind::Midpoint,
                midpoint(a, b),
                format!("{prefix}edge:{i}"),
            );
            if let Some(p) = probe {
                if let Some(foot) = foot_on_segment(p, a, b) {
                    push_point(
                        out,
                        SnapKind::Perpendicular,
                        foot,
                        format!("{prefix}edge:{i}"),
                    );
                }
            }
        } else if let Some(arc) = bulge_arc_params(a, b, bulge, normal) {
            // The chord midpoint is not on the arc; only the true arc point is.
            push_point(
                out,
                SnapKind::Midpoint,
                arc.point_at(arc.start + arc.sweep * 0.5),
                format!("{prefix}edge:{i}"),
            );
            push_point(
                out,
                SnapKind::Center,
                arc.center,
                format!("{prefix}edge:{i}"),
            );
            for k in 0..4 {
                let q = k as f64 * FRAC_PI_2;
                if arc.in_sweep(q) {
                    push_point(
                        out,
                        SnapKind::Quadrant,
                        arc.point_at(q),
                        format!("{prefix}edge:{i}:q{k}"),
                    );
                }
            }
            if let Some(p) = probe {
                if let Some(point) = arc.nearest(p) {
                    push_point(
                        out,
                        SnapKind::Perpendicular,
                        point,
                        format!("{prefix}edge:{i}"),
                    );
                }
            }
        }
    }
}

fn circle_candidates(
    center: Point3,
    normal: Point3,
    radius: f64,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    if !radius.is_finite() || radius.abs() < EPS {
        return;
    }
    let radius = radius.abs();
    push_point(out, SnapKind::Center, center, format!("{prefix}center"));
    let (ax, ay, _) = arbitrary_axis(normal);
    for (i, axis) in [ax, ay, scale(ax, -1.0), scale(ay, -1.0)]
        .iter()
        .enumerate()
    {
        push_point(
            out,
            SnapKind::Quadrant,
            add(center, scale(*axis, radius)),
            format!("{prefix}quadrant:{i}"),
        );
    }
    if let Some(p) = probe {
        if let Some(point) = nearest_on_circle(p, center, normal, radius) {
            push_point(
                out,
                SnapKind::Perpendicular,
                point,
                format!("{prefix}circle"),
            );
        }
    }
}

fn nearest_on_circle(probe: Point3, center: Point3, normal: Point3, radius: f64) -> Option<Point3> {
    let n = normalize(normal);
    let mut d = sub(probe, center);
    d = sub(d, scale(n, dot(d, n)));
    if length(d) < EPS {
        return None;
    }
    Some(add(center, scale(normalize(d), radius)))
}

#[allow(clippy::too_many_arguments)]
fn arc_candidates(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    if !radius.is_finite() || radius.abs() < EPS || !sweep.is_finite() || sweep.abs() < EPS {
        return;
    }
    let radius = radius.abs();
    let sweep = normalize_sweep(sweep);
    if !start.is_finite() {
        return;
    }
    let (ax, ay, _) = arbitrary_axis(normal);
    let point_at = |t: f64| {
        add(
            center,
            add(scale(ax, t.cos() * radius), scale(ay, t.sin() * radius)),
        )
    };
    let full = sweep.abs() >= TAU - 1e-9;
    let in_sweep = move |theta: f64| {
        full || {
            let t = (theta - start) / sweep;
            (0.0..=1.0).contains(&t)
        }
    };
    push_point(out, SnapKind::Center, center, format!("{prefix}center"));
    push_point(
        out,
        SnapKind::Endpoint,
        point_at(start),
        format!("{prefix}vertex:0"),
    );
    push_point(
        out,
        SnapKind::Endpoint,
        point_at(start + sweep),
        format!("{prefix}vertex:1"),
    );
    push_point(
        out,
        SnapKind::Midpoint,
        point_at(start + sweep * 0.5),
        format!("{prefix}arc"),
    );
    for k in 0..4 {
        let q = k as f64 * FRAC_PI_2;
        if in_sweep(q) {
            push_point(
                out,
                SnapKind::Quadrant,
                point_at(q),
                format!("{prefix}quadrant:{k}"),
            );
        }
    }
    if let Some(p) = probe {
        let d = sub(p, center);
        let angle = dot(d, ay).atan2(dot(d, ax));
        for candidate in [angle, angle + std::f64::consts::PI] {
            if in_sweep(candidate) {
                push_point(
                    out,
                    SnapKind::Perpendicular,
                    point_at(candidate),
                    format!("{prefix}arc"),
                );
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn ellipse_candidates(
    center: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    let major_len = length(major_axis);
    if !major_len.is_finite() || major_len < EPS || !ratio.is_finite() {
        return;
    }
    let ratio = ratio.abs();
    let u = scale(major_axis, 1.0 / major_len);
    let minor_dir = ellipse_minor_dir(u);
    let minor = scale(minor_dir, major_len * ratio);
    let sweep = normalize_sweep(sweep);
    if !start.is_finite() || sweep.abs() < EPS {
        return;
    }
    let point_at = |t: f64| {
        add(
            center,
            add(scale(major_axis, t.cos()), scale(minor, t.sin())),
        )
    };
    let full = sweep.abs() >= TAU - 1e-9;
    let in_sweep = move |theta: f64| {
        full || {
            let t = (theta - start) / sweep;
            (0.0..=1.0).contains(&t)
        }
    };
    push_point(out, SnapKind::Center, center, format!("{prefix}center"));
    push_point(
        out,
        SnapKind::Endpoint,
        point_at(start),
        format!("{prefix}vertex:0"),
    );
    push_point(
        out,
        SnapKind::Endpoint,
        point_at(start + sweep),
        format!("{prefix}vertex:1"),
    );
    push_point(
        out,
        SnapKind::Midpoint,
        point_at(start + sweep * 0.5),
        format!("{prefix}ellipse"),
    );
    for k in 0..4 {
        let q = k as f64 * FRAC_PI_2;
        if in_sweep(q) {
            push_point(
                out,
                SnapKind::Quadrant,
                point_at(q),
                format!("{prefix}quadrant:{k}"),
            );
        }
    }
    if let Some(p) = probe {
        if let Some(t) = nearest_ellipse_param(p, center, major_axis, minor, start, sweep) {
            push_point(
                out,
                SnapKind::Perpendicular,
                point_at(t),
                format!("{prefix}ellipse"),
            );
        }
    }
}

/// Nearest parameter on an elliptic arc to `probe`, by a fixed-resolution scan
/// plus a golden-section refinement. The resolution is independent of the
/// display LOD so the snap does not move when the LOD changes.
fn nearest_ellipse_param(
    probe: Point3,
    center: Point3,
    major: Point3,
    minor: Point3,
    start: f64,
    sweep: f64,
) -> Option<f64> {
    const SAMPLES: usize = 360;
    let point_at = |t: f64| add(center, add(scale(major, t.cos()), scale(minor, t.sin())));
    let dist2 = |t: f64| {
        let d = sub(point_at(t), probe);
        dot(d, d)
    };
    let mut best_t = start;
    let mut best = f64::INFINITY;
    for i in 0..=SAMPLES {
        let t = start + sweep * (i as f64) / (SAMPLES as f64);
        let d = dist2(t);
        if d < best {
            best = d;
            best_t = t;
        }
    }
    let step = sweep / (SAMPLES as f64);
    let lo_bound = start.min(start + sweep);
    let hi_bound = start.max(start + sweep);
    let mut a = (best_t - step).clamp(lo_bound, hi_bound);
    let mut b = (best_t + step).clamp(lo_bound, hi_bound);
    // Golden-section search on the unimodal neighbourhood.
    let gr = (5.0f64.sqrt() - 1.0) / 2.0;
    let mut c = b - gr * (b - a);
    let mut d = a + gr * (b - a);
    for _ in 0..48 {
        if dist2(c) < dist2(d) {
            b = d;
        } else {
            a = c;
        }
        c = b - gr * (b - a);
        d = a + gr * (b - a);
        if (b - a).abs() < EPS {
            break;
        }
    }
    let best_t = 0.5 * (a + b);
    (best_t.is_finite()).then_some(best_t)
}

fn ellipse_minor_dir(major_unit: Point3) -> Point3 {
    let minor = cad_geometry::cross(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        major_unit,
    );
    if length(minor) < 1e-9 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        normalize(minor)
    }
}

fn spline_candidates(
    control_points: &[Point3],
    knots: &[f64],
    weights: &[f64],
    degree: u32,
    probe: Option<Point3>,
    out: &mut Vec<Raw>,
    prefix: &str,
) {
    if control_points.len() < 2 {
        return;
    }
    let params = TessellationParams {
        // The sample count is fixed; `tessellate_spline` ignores the tolerance
        // and uses the source knots, so this is LOD-independent.
        max_segments: 512,
        min_segments: 32,
        ..TessellationParams::default()
    };
    let samples = tessellate_spline(control_points, knots, weights, degree, params);
    if samples.len() < 2 {
        return;
    }
    push_point(
        out,
        SnapKind::Endpoint,
        samples[0],
        format!("{prefix}spline:start"),
    );
    push_point(
        out,
        SnapKind::Endpoint,
        samples[samples.len() - 1],
        format!("{prefix}spline:end"),
    );
    if let Some(mid) = sample_midpoint(&samples) {
        push_point(out, SnapKind::Midpoint, mid, format!("{prefix}spline"));
    }
    if let Some(p) = probe {
        if let Some(near) = samples.iter().copied().min_by(|a, b| {
            distance(*a, p)
                .partial_cmp(&distance(*b, p))
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            push_point(
                out,
                SnapKind::Perpendicular,
                near,
                format!("{prefix}spline"),
            );
        }
    }
}

/// Point at half the polyline arc length.
fn sample_midpoint(samples: &[Point3]) -> Option<Point3> {
    let mut total = 0.0;
    for w in samples.windows(2) {
        total += distance(w[0], w[1]);
    }
    if !total.is_finite() || total < EPS {
        return None;
    }
    let half = total * 0.5;
    let mut walked = 0.0;
    for w in samples.windows(2) {
        let seg = distance(w[0], w[1]);
        if walked + seg >= half {
            let t = if seg > EPS {
                (half - walked) / seg
            } else {
                0.0
            };
            return Some(add(w[0], scale(sub(w[1], w[0]), t)));
        }
        walked += seg;
    }
    samples.last().copied()
}

fn polyline_plane_normal(points: &[Point3]) -> Point3 {
    let world_z = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let n = points.len();
    if n < 3 {
        return world_z;
    }
    let mut acc = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        acc.x += (a.y - b.y) * (a.z + b.z);
        acc.y += (a.z - b.z) * (a.x + b.x);
        acc.z += (a.x - b.x) * (a.y + b.y);
    }
    if length(acc) < EPS {
        world_z
    } else {
        normalize(acc)
    }
}

/// An arc reconstructed from an AutoCAD bulge, in the polyline's own plane.
struct BulgeArc {
    center: Point3,
    ax: Point3,
    ay: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
}

impl BulgeArc {
    fn point_at(&self, t: f64) -> Point3 {
        add(
            self.center,
            add(
                scale(self.ax, t.cos() * self.radius),
                scale(self.ay, t.sin() * self.radius),
            ),
        )
    }

    fn in_sweep(&self, theta: f64) -> bool {
        let t = (theta - self.start) / self.sweep;
        (0.0..=1.0).contains(&t)
    }

    fn nearest(&self, probe: Point3) -> Option<Point3> {
        let d = sub(probe, self.center);
        let angle = dot(d, self.ay).atan2(dot(d, self.ax));
        for candidate in [angle, angle + std::f64::consts::PI] {
            if self.in_sweep(candidate) {
                return Some(self.point_at(candidate));
            }
        }
        None
    }
}

/// Reconstruct a bulge arc (`bulge = tan(theta/4)`) in the plane with normal
/// `plane_normal`, mirroring the polyline tessellation convention.
fn bulge_arc_params(a: Point3, b: Point3, bulge: f64, plane_normal: Point3) -> Option<BulgeArc> {
    let chord = sub(b, a);
    let chord_len = length(chord);
    if !chord_len.is_finite() || chord_len < EPS || !bulge.is_finite() {
        return None;
    }
    let (ax, ay, _) = arbitrary_axis(plane_normal);
    let d2 = [dot(chord, ax), dot(chord, ay)];
    let chord2 = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
    if chord2 < EPS {
        return None;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let half_chord = chord_len * 0.5;
    let sagitta = bulge * half_chord;
    if sagitta.abs() < EPS {
        return None;
    }
    let radius = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta.abs());
    let dir2 = [d2[0] / chord2, d2[1] / chord2];
    let left2 = [-dir2[1], dir2[0]];
    let u2 = if sagitta >= 0.0 {
        left2
    } else {
        [-left2[0], -left2[1]]
    };
    let mid2 = [d2[0] * 0.5, d2[1] * 0.5];
    let center2 = [
        mid2[0] - u2[0] * radius * half.cos(),
        mid2[1] - u2[1] * radius * half.cos(),
    ];
    let center = add(a, add(scale(ax, center2[0]), scale(ay, center2[1])));
    let start = (-center2[1]).atan2(-center2[0]);
    Some(BulgeArc {
        center,
        ax,
        ay,
        radius,
        start,
        sweep: theta,
    })
}

/// Normalise an arc/ellipse sweep to a magnitude no larger than a full turn,
/// keeping its sign.
fn normalize_sweep(sweep: f64) -> f64 {
    if sweep.abs() >= TAU - 1e-9 {
        if sweep < 0.0 {
            -TAU
        } else {
            TAU
        }
    } else {
        sweep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeasurementEngine, MeasurementPoint, MeasurementRequest, MeasurementSpace};
    use cad_db::MeasurementAlgorithm;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn flat(x: f64, y: f64) -> Point3 {
        p(x, y, 0.0)
    }

    fn z_plane() -> WorkPlane {
        WorkPlane {
            origin: p(0.0, 0.0, 0.0),
            u: p(1.0, 0.0, 0.0),
            v: p(0.0, 1.0, 0.0),
        }
    }

    fn source(entity: u128) -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(entity),
            instance: InstancePath(Vec::new()),
            sub_element: None,
        }
    }

    fn target(geometry: SemanticGeometry) -> SnapTarget {
        SnapTarget::new(geometry, source(1), SpaceId::Model, Precision::Analytic)
    }

    fn line(a: (f64, f64), b: (f64, f64)) -> SemanticGeometry {
        SemanticGeometry::Line {
            start: flat(a.0, a.1),
            end: flat(b.0, b.1),
        }
    }

    fn circle(center: (f64, f64), radius: f64) -> SemanticGeometry {
        SemanticGeometry::Circle {
            center: flat(center.0, center.1),
            normal: p(0.0, 0.0, 1.0),
            radius,
        }
    }

    /// Ray straight down onto the z=0 plane through the cursor.
    fn cursor_ray(cx: f64, cy: f64) -> Ray3 {
        Ray3 {
            origin: p(cx, cy, 10.0),
            direction: p(0.0, 0.0, -1.0),
        }
    }

    fn snaps(targets: &[SnapTarget], cx: f64, cy: f64, world_per_px: f64) -> Vec<SnapCandidate> {
        MeasurementEngine::default()
            .snap_targets(
                targets,
                &cursor_ray(cx, cy),
                Some(&z_plane()),
                world_per_px,
                Some(SpaceId::Model),
            )
            .unwrap()
    }

    #[test]
    fn endpoint_snap_lands_on_the_line_end() {
        let t = [target(line((0.0, 0.0), (10.0, 0.0)))];
        let best = MeasurementEngine::default()
            .snap_best(&t, &cursor_ray(0.0, 0.2), Some(&z_plane()), 1.0, None)
            .unwrap()
            .expect("a snap");
        assert_eq!(best.kind, SnapKind::Endpoint);
        assert_eq!(best.point, flat(0.0, 0.0));
        assert_eq!(best.source.entity, EntityId(1));
    }

    #[test]
    fn midpoint_snap_lands_on_the_segment_middle() {
        let t = [target(line((0.0, 0.0), (10.0, 0.0)))];
        let best = MeasurementEngine::default()
            .snap_best(&t, &cursor_ray(5.0, 0.2), Some(&z_plane()), 1.0, None)
            .unwrap()
            .expect("a snap");
        assert_eq!(best.kind, SnapKind::Midpoint);
        assert_eq!(best.point, flat(5.0, 0.0));
    }

    #[test]
    fn center_snap_lands_on_the_circle_center() {
        let t = [target(circle((2.0, 3.0), 5.0))];
        let best = MeasurementEngine::default()
            .snap_best(&t, &cursor_ray(2.1, 3.1), Some(&z_plane()), 1.0, None)
            .unwrap()
            .expect("a snap");
        assert_eq!(best.kind, SnapKind::Center);
        assert_eq!(best.point, flat(2.0, 3.0));
    }

    #[test]
    fn quadrant_snap_lands_on_the_circle_quadrant() {
        let t = [target(circle((0.0, 0.0), 5.0))];
        let all = snaps(&t, 5.0, 0.0, 1.0);
        let quad = all
            .iter()
            .find(|c| c.kind == SnapKind::Quadrant && c.point == flat(5.0, 0.0));
        assert!(quad.is_some(), "{all:?}");
        // At the exact quadrant the perpendicular candidate coincides, and the
        // quadrant kind has the higher priority.
        assert_eq!(all[0].kind, SnapKind::Quadrant);
    }

    #[test]
    fn perpendicular_snap_drops_onto_the_segment() {
        let t = [target(line((0.0, 0.0), (10.0, 0.0)))];
        let best = MeasurementEngine::default()
            .snap_best(&t, &cursor_ray(3.0, 2.0), Some(&z_plane()), 1.0, None)
            .unwrap()
            .expect("a snap");
        assert_eq!(best.kind, SnapKind::Perpendicular);
        assert_eq!(best.point, flat(3.0, 0.0));
    }

    #[test]
    fn perpendicular_is_not_offered_past_a_segment_end() {
        // The cursor is beyond `b`; the perpendicular foot would be behind the
        // segment, so only the endpoint snap is available.
        let t = [target(line((0.0, 0.0), (10.0, 0.0)))];
        let all = snaps(&t, 12.0, 2.0, 2.0);
        assert!(
            all.iter().all(|c| c.kind != SnapKind::Perpendicular),
            "{all:?}"
        );
    }

    #[test]
    fn local_intersection_snap_finds_the_crossing() {
        let a = SnapTarget::new(
            line((0.0, 0.0), (10.0, 10.0)),
            source(1),
            SpaceId::Model,
            Precision::Analytic,
        );
        let b = SnapTarget::new(
            line((0.0, 10.0), (10.0, 0.0)),
            source(2),
            SpaceId::Model,
            Precision::Analytic,
        );
        let all = snaps(&[a, b], 5.1, 5.1, 1.0);
        let hit = all
            .iter()
            .find(|c| c.kind == SnapKind::LocalIntersection)
            .expect("intersection candidate");
        assert_eq!(hit.point, flat(5.0, 5.0));
        assert!(hit.secondary.is_some());
        assert_eq!(hit.secondary.as_ref().unwrap().entity, EntityId(2));
    }

    #[test]
    fn candidate_behind_the_pick_ray_origin_is_not_returned() {
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let behind = SnapTarget::new(
            line((-5.0, 0.0), (-1.0, 0.0)),
            source(1),
            SpaceId::Model,
            Precision::Analytic,
        );
        let found = MeasurementEngine::default()
            .snap_targets(&[behind], &ray, None, 1.0, None)
            .unwrap();
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn pixel_tolerance_scales_with_world_per_px() {
        let t = [target(line((10.0, 0.0), (20.0, 0.0)))];
        // Distance 10 world units from the cursor: at 0.5 world/px the 12 px
        // aperture is 6 units and misses; at 1.0 it is 12 units and hits.
        assert!(snaps(&t, 0.0, 0.0, 0.5).is_empty());
        let all = snaps(&t, 0.0, 0.0, 1.0);
        let endpoint = all
            .iter()
            .find(|c| c.kind == SnapKind::Endpoint && c.point == flat(10.0, 0.0))
            .expect("endpoint within 12 px");
        assert!((endpoint.logical_pixel_distance - 10.0).abs() < 1e-9);
    }

    #[test]
    fn snap_is_independent_of_the_display_pixel_budget() {
        let t = [
            target(line((0.0, 0.0), (10.0, 0.0))),
            target(circle((20.0, 20.0), 3.0)),
        ];
        let fine = TolerancePolicy {
            display_pixels: 0.01,
            ..Default::default()
        };
        let coarse = TolerancePolicy {
            display_pixels: 50.0,
            ..Default::default()
        };
        let ray = cursor_ray(1.0, 0.5);
        let a = collect_candidates(&t, &ray, Some(&z_plane()), 1.0, None, &fine).unwrap();
        let b = collect_candidates(&t, &ray, Some(&z_plane()), 1.0, None, &coarse).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn measurement_is_independent_of_the_display_pixel_budget() {
        let request = MeasurementRequest::from_tapped(
            MeasurementAlgorithm::Distance3d,
            vec![
                MeasurementPoint::model(flat(0.0, 0.0)),
                MeasurementPoint::model(flat(3.0, 4.0)),
            ],
            MeasurementSpace::World3d,
            UnitContext::drawing_units(),
            GeometrySource::UserPoints,
            Precision::Analytic,
        );
        let fine = MeasurementEngine {
            tolerance: TolerancePolicy {
                display_pixels: 0.01,
                ..Default::default()
            },
        };
        let coarse = MeasurementEngine {
            tolerance: TolerancePolicy {
                display_pixels: 50.0,
                ..Default::default()
            },
        };
        assert_eq!(
            fine.measure(&request).unwrap().value,
            coarse.measure(&request).unwrap().value
        );
    }

    #[test]
    fn non_finite_or_degenerate_queries_are_rejected() {
        let t = [target(line((0.0, 0.0), (1.0, 0.0)))];
        let engine = MeasurementEngine::default();
        let zero_dir = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(0.0, 0.0, 0.0),
        };
        assert!(engine
            .snap_targets(&t, &zero_dir, Some(&z_plane()), 1.0, None)
            .is_err());
        let nan_ray = Ray3 {
            origin: p(f64::NAN, 0.0, 0.0),
            direction: p(0.0, 0.0, 1.0),
        };
        assert!(engine
            .snap_targets(&t, &nan_ray, Some(&z_plane()), 1.0, None)
            .is_err());
        let ray = cursor_ray(0.0, 0.0);
        assert!(engine
            .snap_targets(&t, &ray, Some(&z_plane()), 0.0, None)
            .is_err());
        let zero_tol = MeasurementEngine {
            tolerance: TolerancePolicy {
                interaction_logical_pixels: 0.0,
                ..Default::default()
            },
        };
        assert!(zero_tol
            .snap_targets(&t, &ray, Some(&z_plane()), 1.0, None)
            .is_err());
        // A poisoned target contributes nothing rather than a bogus point.
        let bad = SnapTarget::new(
            SemanticGeometry::Line {
                start: p(f64::NAN, 0.0, 0.0),
                end: flat(1.0, 0.0),
            },
            source(1),
            SpaceId::Model,
            Precision::Analytic,
        );
        assert!(engine
            .snap_targets(&[bad], &ray, Some(&z_plane()), 1.0, None)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn snapped_measurement_records_provenance() {
        let engine = MeasurementEngine::default();
        let request = MeasurementRequest::from_tapped(
            MeasurementAlgorithm::Distance3d,
            vec![
                MeasurementPoint::model(flat(0.0, 0.0)),
                MeasurementPoint::model(flat(3.0, 4.0)),
            ],
            MeasurementSpace::World3d,
            UnitContext::drawing_units(),
            GeometrySource::UserPoints,
            Precision::Analytic,
        );
        let snaps = vec![
            SnapProvenance {
                kind: SnapKind::Endpoint,
                source: source(1),
                secondary: None,
                precision: Precision::Analytic,
                logical_pixel_distance: 0.5,
            },
            SnapProvenance {
                kind: SnapKind::LocalIntersection,
                source: source(2),
                secondary: Some(source(3)),
                precision: Precision::Analytic,
                logical_pixel_distance: 0.25,
            },
        ];
        let snapped = engine.measure_snapped(&request, snaps).unwrap();
        assert!((snapped.record.value - 5.0).abs() < 1e-12);
        assert_eq!(snapped.snaps[0].kind, SnapKind::Endpoint);
        assert_eq!(
            snapped.snaps[1].secondary.as_ref().unwrap().entity,
            EntityId(3)
        );
        // A provenance count that does not match the inputs is an error.
        assert!(engine
            .measure_snapped(
                &request,
                vec![SnapProvenance {
                    kind: SnapKind::Endpoint,
                    source: source(1),
                    secondary: None,
                    precision: Precision::Analytic,
                    logical_pixel_distance: 0.0,
                }]
            )
            .is_err());
    }
}
