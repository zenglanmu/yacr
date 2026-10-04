//! Pure geometric computations backing the measurement algorithms.
//!
//! Distances, angles and area projection are evaluated here without any
//! dependency on the database, so the space policy lives in [`crate::space`]
//! and the orchestration lives in [`crate::engine`]. Pick-ray helpers used by
//! snapping are included because they are the same kind of geometry.

use cad_domain::*;

use crate::space::MeasurementSpace;

/// Unit pick-ray direction, or `None` when the ray is unusable.
pub(crate) fn unit_direction(ray: &Ray3) -> Option<Point3> {
    let d = ray.direction;
    if !d.x.is_finite() || !d.y.is_finite() || !d.z.is_finite() {
        return None;
    }
    if !ray.origin.x.is_finite() || !ray.origin.y.is_finite() || !ray.origin.z.is_finite() {
        return None;
    }
    let l = length(d);
    if !l.is_finite() || l < 1e-12 {
        return None;
    }
    let dir = scale(d, 1.0 / l);
    if !dir.x.is_finite() || !dir.y.is_finite() || !dir.z.is_finite() {
        return None;
    }
    Some(dir)
}

/// Perpendicular distance from a point to the pick ray, or `None` when the point
/// is behind the ray origin (negative ray parameter).
pub(crate) fn point_ray_distance(dir: &Point3, origin: Point3, p: Point3) -> Option<f64> {
    let v = sub(p, origin);
    let proj = dot(v, *dir);
    if !proj.is_finite() || proj < 0.0 {
        return None;
    }
    let closest = scale(*dir, proj);
    let d = length(sub(v, closest));
    if d.is_finite() {
        Some(d)
    } else {
        None
    }
}

pub(crate) fn two(points: &[Point3]) -> CadResult<[Point3; 2]> {
    if points.len() != 2 {
        return Err(CadError::InvalidInput(
            "distance needs exactly two points".to_string(),
        ));
    }
    Ok([points[0], points[1]])
}

fn distance(a: Point3, b: Point3) -> f64 {
    length(sub(a, b))
}

/// 3D distance, but still governed by the space policy: 3D distance is not
/// defined on the 2D paper sheet. A viewport maps paper to model first, so its
/// points are already model-space by the time they get here.
pub(crate) fn distance_with_space(
    a: Point3,
    b: Point3,
    space: &MeasurementSpace,
) -> CadResult<f64> {
    match space {
        MeasurementSpace::World3d
        | MeasurementSpace::Plane(_)
        | MeasurementSpace::ViewportModel { .. } => Ok(distance(a, b)),
        MeasurementSpace::Paper(_) => Err(CadError::Unsupported(
            "3D distance is not defined in paper space; use a planar paper distance".into(),
        )),
    }
}

/// Distance projected onto the effective measurement plane.
pub(crate) fn distance_in_plane(
    a: Point3,
    b: Point3,
    plane: &WorkPlane,
    tol: f64,
) -> CadResult<f64> {
    let d = sub(b, a);
    let nx = cross(plane.u, plane.v);
    let ln = length(nx);
    if !ln.is_finite() || ln < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    // Remove the plane-normal component so this is a true in-plane distance.
    let n = scale(nx, 1.0 / ln);
    let dn = dot(d, n);
    let planar = sub(d, scale(n, dn));
    Ok(length(planar))
}

/// Angle at `vertex`, measured in the plane defined by the space.
pub(crate) fn angle_at_vertex(
    a: Point3,
    vertex: Point3,
    b: Point3,
    space: &MeasurementSpace,
    plane: Option<&WorkPlane>,
    tol: f64,
) -> CadResult<f64> {
    let v1 = sub(a, vertex);
    let v2 = sub(b, vertex);
    // Project the arms into the measurement plane when one exists; otherwise
    // (3D) use the raw vectors.
    let (w1, w2) = match space {
        MeasurementSpace::Plane(_) | MeasurementSpace::ViewportModel { .. } => {
            let plane = plane.ok_or_else(|| {
                CadError::InvalidInput("angle needs a defined measurement plane".to_string())
            })?;
            let n = normalized_normal(plane, tol)?;
            (sub(v1, scale(n, dot(v1, n))), sub(v2, scale(n, dot(v2, n))))
        }
        MeasurementSpace::World3d => (v1, v2),
        MeasurementSpace::Paper(_) => {
            return Err(CadError::Unsupported(
                "angle is not defined in paper space; use model space for angles".into(),
            ));
        }
    };
    let u1 = normalized_angle_arm(w1, tol)?;
    let u2 = normalized_angle_arm(w2, tol)?;
    let c = dot(u1, u2);
    if !c.is_finite() {
        return Err(CadError::InvalidInput("angle is not finite".to_string()));
    }
    Ok(c.clamp(-1.0, 1.0).acos().to_degrees())
}

/// Normalize without squaring large coordinates or requiring the full arm
/// length to fit in an f64. Keep the degeneracy threshold in world units.
fn normalized_angle_arm(arm: Point3, tol: f64) -> CadResult<Point3> {
    if !arm.x.is_finite() || !arm.y.is_finite() || !arm.z.is_finite() {
        return Err(CadError::InvalidInput(
            "angle arm is not finite".to_string(),
        ));
    }
    let magnitude = arm.x.abs().max(arm.y.abs()).max(arm.z.abs());
    if magnitude == 0.0 {
        return Err(CadError::InvalidInput(
            "angle has a degenerate arm".to_string(),
        ));
    }
    let scaled = Point3 {
        x: arm.x / magnitude,
        y: arm.y / magnitude,
        z: arm.z / magnitude,
    };
    let norm = scaled.x.hypot(scaled.y).hypot(scaled.z);
    if !norm.is_finite() || magnitude < tol / norm {
        return Err(CadError::InvalidInput(
            "angle has a degenerate arm".to_string(),
        ));
    }
    Ok(Point3 {
        x: scaled.x / norm,
        y: scaled.y / norm,
        z: scaled.z / norm,
    })
}

/// Normalized plane normal, rejecting a degenerate basis.
fn normalized_normal(plane: &WorkPlane, tol: f64) -> CadResult<Point3> {
    let n = cross(plane.u, plane.v);
    let l = length(n);
    if !l.is_finite() || l < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    Ok(scale(n, 1.0 / l))
}

/// Project points onto the measurement work plane's 2D coordinates.
///
/// Audit B24: the plane must be finite and orthogonal. Coplanarity is always
/// checked using the *normalised* normal, so the test is unit-correct whatever
/// the plane basis scale (a viewport's model plane is scaled by the drawing
/// ratio). Coordinates are projected onto the orthonormal basis, so the
/// resulting area is in world/model units.
pub(crate) fn project_to_plane(
    points: &[Point3],
    plane: &WorkPlane,
    tol: f64,
) -> CadResult<Vec<Point3>> {
    let u = check_plane_basis(plane, tol)?;
    let v = normalized_normalized(plane.v, tol)?;
    let n = cross(u, v);
    let ln = length(n);
    if !ln.is_finite() || ln < self_epsilon(tol) {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    let n = scale(n, 1.0 / ln);
    for p in points {
        let d = sub(*p, plane.origin);
        let off = dot(d, n);
        if off.abs() > self_epsilon(tol) {
            return Err(CadError::InvalidInput(
                "area ring is not coplanar with the measurement plane".to_string(),
            ));
        }
    }
    let coords = points
        .iter()
        .map(|p| {
            let d = sub(*p, plane.origin);
            Point3 {
                x: dot(d, u),
                y: dot(d, v),
                z: 0.0,
            }
        })
        .collect::<Vec<_>>();
    if coords.iter().any(|c| !c.x.is_finite() || !c.y.is_finite()) {
        return Err(CadError::InvalidInput(
            "area projection is not finite".to_string(),
        ));
    }
    Ok(coords)
}

/// Coplanarity epsilon derived from a topology tolerance; never below 1e-9.
fn self_epsilon(tol: f64) -> f64 {
    let t = tol.max(1e-9);
    if t.is_finite() {
        t
    } else {
        1e-9
    }
}

/// Validate a non-degenerate, finite basis vector and return its unit form.
fn check_plane_basis(plane: &WorkPlane, tol: f64) -> CadResult<Point3> {
    for p in [plane.origin, plane.u, plane.v] {
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return Err(CadError::InvalidInput(
                "measurement plane is not finite".to_string(),
            ));
        }
    }
    let lu = length(plane.u);
    let lv = length(plane.v);
    if !lu.is_finite() || !lv.is_finite() || lu < tol || lv < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    let n = cross(plane.u, plane.v);
    let ln = length(n);
    if !ln.is_finite() || ln < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    let ortho = dot(plane.u, plane.v).abs() / (lu * lv);
    if !ortho.is_finite() || ortho > 1e-6 {
        return Err(CadError::InvalidInput(
            "measurement plane is skewed (u and v are not orthogonal)".to_string(),
        ));
    }
    Ok(scale(plane.u, 1.0 / lu))
}

/// Normalise a basis vector, requiring it to be finite and non-degenerate.
fn normalized_normalized(v: Point3, tol: f64) -> CadResult<Point3> {
    let l = length(v);
    if !l.is_finite() || l < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    Ok(scale(v, 1.0 / l))
}

pub(crate) fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}
fn scale(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}
fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}
fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}
