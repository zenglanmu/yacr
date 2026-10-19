//! Candidate acceptance, deduplication and ray/plane helpers for the snap engine.

use cad_domain::*;
use cad_geometry::{add, distance, dot, length, normalize, scale, sub};

use super::{Raw, SnapCandidate, SnapKind, SnapTarget, EPS};

/// Intersections are only meaningful between polyline-like curves, not against
/// a mesh or opaque payload.
pub(super) fn engine_is_local(a: &SnapTarget, b: &SnapTarget) -> bool {
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
        // A dimension is intersectable through its display children, exactly
        // like a compound (ADR 0017 A.8).
        G::Dimension(dimension) => dimension.display().iter().any(is_intersectable),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn accept(
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
pub(super) fn dedup(out: &mut Vec<SnapCandidate>) {
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

pub(super) fn unit_direction(ray: &Ray3) -> CadResult<Point3> {
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
pub(super) fn ray_plane_point(ray: &Ray3, dir: &Point3, plane: &WorkPlane) -> Option<Point3> {
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
