//! Upstream surface records → neutral analytic surfaces.
//!
//! Only the analytic set the neutral model can evaluate is mapped: `Plane`,
//! `PlaneThrough`, `Cylinder`, `Cone`, `Sphere`, `Torus`. Everything else
//! (`Spline`, `RationalSpline`, `BallPatch`, `PlacedRevolution`, `Blend`) is a
//! typed refusal — phase P2 adds a NURBS variant to the neutral model first.
//!
//! A face that carries its `nurbs` form is mapped from its **analytic label**
//! (the record guarantees the label is proved against the form for
//! `Cylinder`/`Cone`/`Sphere`/`Torus`). That keeps curved primitives analytic
//! downstream (display and export) instead of degrading them to a spline this
//! build cannot evaluate.

use cad_domain::{BrepSurface, Point3};
use solidcore_record::brep::{Frame, Surface};

use super::units::point;
use super::{cross, is_axis, mul, normalize, sub};
use crate::error::LiftRefusal;

/// The record's own name for a surface, used in refusal details.
pub(crate) fn type_key(surface: &Surface) -> &'static str {
    match surface {
        Surface::Plane { .. } => "plane",
        Surface::PlaneThrough { .. } => "plane-through",
        Surface::Cylinder { .. } => "cylinder",
        Surface::Cone { .. } => "cone",
        Surface::Sphere { .. } => "sphere",
        Surface::Torus { .. } => "torus",
        Surface::BallPatch { .. } => "ball-patch",
        Surface::Spline { .. } => "spline",
        Surface::RationalSpline { .. } => "rational-spline",
        Surface::PlacedRevolution { .. } => "placed-revolution",
        Surface::Blend { .. } => "blend",
    }
}

/// The orthonormal frame a record `Frame` describes: origin plus the right-handed
/// `z = x × y`. `None` when the stored axes are degenerate (the record's own
/// validation leaves orthonormality to the kernel, so this is a real case).
fn frame(value: &Frame) -> Option<(Point3, Point3, Point3)> {
    let origin = point(value.origin);
    let x = point(value.x_axis);
    let y = point(value.y_axis);
    let z = cross(x, y);
    if !is_axis(x) || !is_axis(y) || !is_axis(z) {
        return None;
    }
    Some((origin, normalize(x), normalize(z)))
}

/// Map one record surface to its neutral analytic form.
pub(crate) fn to_brep(surface: &Surface) -> Result<BrepSurface, LiftRefusal> {
    let key = type_key(surface);
    match surface {
        Surface::Plane {
            origin,
            u_axis,
            v_axis,
        } => {
            let normal = cross(point(*u_axis), point(*v_axis));
            if !is_axis(normal) {
                return Err(LiftRefusal::unsupported(
                    "a plane record with parallel axes cannot be mapped",
                ));
            }
            Ok(BrepSurface::Plane {
                origin: point(*origin),
                normal: normalize(normal),
                u_dir: normalize(point(*u_axis)),
            })
        }
        Surface::PlaneThrough {
            origin,
            u_point,
            v_point,
        } => {
            let origin = point(*origin);
            let u = sub(point(*u_point), origin);
            let v = sub(point(*v_point), origin);
            let normal = cross(u, v);
            if !is_axis(u) || !is_axis(normal) {
                return Err(LiftRefusal::unsupported(
                    "a plane-through record with coincident points cannot be mapped",
                ));
            }
            Ok(BrepSurface::Plane {
                origin,
                normal: normalize(normal),
                u_dir: normalize(u),
            })
        }
        Surface::Cylinder { frame: f, radius } => {
            let (origin, x, z) = frame(f).ok_or_else(|| {
                LiftRefusal::unsupported("a cylinder record with a degenerate frame")
            })?;
            Ok(BrepSurface::Cylinder {
                origin,
                axis: z,
                ref_dir: x,
                radius: *radius,
            })
        }
        Surface::Cone {
            frame: f,
            radius_at_origin,
            half_angle,
        } => {
            let (origin, x, z) = frame(f)
                .ok_or_else(|| LiftRefusal::unsupported("a cone record with a degenerate frame"))?;
            if !half_angle.is_finite() || half_angle.abs() >= std::f64::consts::FRAC_PI_2 {
                return Err(LiftRefusal::unsupported(format!(
                    "a cone record with half angle {half_angle} cannot be mapped"
                )));
            }
            Ok(BrepSurface::Cone {
                origin,
                axis: z,
                ref_dir: x,
                radius: *radius_at_origin,
                sin_half_angle: half_angle.sin(),
                cos_half_angle: half_angle.cos(),
            })
        }
        Surface::Sphere { frame: f, radius } => {
            let (center, x, z) = frame(f).ok_or_else(|| {
                LiftRefusal::unsupported("a sphere record with a degenerate frame")
            })?;
            Ok(BrepSurface::Sphere {
                center,
                radius: *radius,
                u_dir: x,
                pole: z,
            })
        }
        Surface::Torus {
            frame: f,
            major_radius,
            minor_radius,
            tube_start,
        } => {
            // The neutral torus samples its tube from the outer equator; a record
            // that starts the tube on a quarter line is a different
            // parameterisation we cannot carry yet.
            if tube_start.abs() > 1e-9 {
                return Err(LiftRefusal::unsupported(format!(
                    "a torus record whose tube starts at {tube_start} rad instead of 0 is not \
                     mapped in P1"
                )));
            }
            let (center, x, z) = frame(f).ok_or_else(|| {
                LiftRefusal::unsupported("a torus record with a degenerate frame")
            })?;
            Ok(BrepSurface::Torus {
                center,
                normal: z,
                major_radius: *major_radius,
                minor_radius: *minor_radius,
                u_dir: x,
            })
        }
        _other => Err(LiftRefusal::unsupported(format!(
            "a '{key}' surface is not mapped to the neutral B-rep in P1 \
             (only plane, cylinder, cone, sphere and torus are)"
        ))),
    }
}

/// A record plane as the orthonormal `u_axis`/`v_axis` pair the kernel demands.
///
/// The kernel refuses a plane whose axes are not orthonormal — the one failure
/// the P1 experiment hit — so the record frame is derived here: `u` is the
/// caller's hint projected into the plane, `v = n × u`. A hint that vanishes or
/// lies along the normal is replaced by an arbitrary perpendicular, never by a
/// skewed axis.
pub(crate) fn record_plane(
    origin: Point3,
    normal: Point3,
    u_hint: Point3,
) -> Result<Surface, LiftRefusal> {
    let n = normalize(normal);
    if !is_axis(n) {
        return Err(LiftRefusal::placement(
            "a face with a degenerate plane normal",
        ));
    }
    let mut u = sub(u_hint, mul(n, super::dot(u_hint, n)));
    if !is_axis(u) {
        // 0.1 at the smallest component of `n`, projected and normalised, gives
        // an in-plane axis that is never parallel to `n`.
        let seed = if n.x.abs() <= n.y.abs() && n.x.abs() <= n.z.abs() {
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            }
        } else if n.y.abs() <= n.z.abs() {
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            }
        } else {
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            }
        };
        u = sub(seed, mul(n, super::dot(seed, n)));
    }
    let u = normalize(u);
    if !is_axis(u) {
        return Err(LiftRefusal::placement(
            "a face with no usable in-plane axis",
        ));
    }
    let v = cross(n, u);
    Ok(Surface::Plane {
        origin: super::units::record_point(origin),
        u_axis: super::units::record_point(u),
        v_axis: super::units::record_point(v),
    })
}
