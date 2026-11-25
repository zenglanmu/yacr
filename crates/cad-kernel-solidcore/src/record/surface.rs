//! Upstream surface records → neutral surfaces.
//!
//! Two families map here:
//!
//! - the analytic labels `Plane`, `PlaneThrough`, `Cylinder`, `Cone`, `Sphere`,
//!   `Torus` become the matching [`cad_domain::BrepSurface`] variant, with one
//!   reclassification: a **zero-slope cone is a cylinder** (see "A zero-slope
//!   cone is a cylinder" and the `Cone` arm of [`to_brep`]);
//! - the **form** surfaces `Spline` and `RationalSpline` become
//!   [`cad_domain::BrepSurface::Nurbs`] (a `Spline` with every weight `1.0`). The
//!   net, knots and weights cross the seam bit-for-bit; the degree stays implicit
//!   and is never re-declared here.
//!
//! `BallPatch`, `PlacedRevolution` and `Blend` stay typed refusals — each needs a
//! certification step (a ball patch/blend net against a defining surface) this
//! build does not do, so they are named, not approximated.
//!
//! A face that carries its `nurbs` form on an **analytic label** is mapped from
//! that label (the record guarantees the label is proved against the form for
//! `Cylinder`/`Cone`/`Sphere`/`Torus`), and the form is ignored. That keeps curved
//! primitives analytic downstream (display and export) instead of degrading them
//! to a spline.
//!
//! # A zero-slope cone is a cylinder
//!
//! Older pins wrote a **round fillet** as `Surface::Cone { half_angle: 0.0 }`
//! plus a rational NURBS net (measured: `tests/fillet.rs`, one round on a unit
//! cube's edge); the current pin writes `Surface::Cylinder` directly, and this
//! mapping stays for records produced by the older kernels. A zero-slope cone
//! *is* a cylinder, and mapping it to
//! [`cad_domain::BrepSurface::Cone`] would keep a face this build cannot write
//! back — the record writer needs a slope for a cone label, and the cone's loop
//! mapping refuses a slope-less lateral face — so the round would be lost on the
//! next lift. It is therefore mapped to
//! [`cad_domain::BrepSurface::Cylinder`] with the same fields
//! (origin, `axis = z`, `ref_dir = x`, radius), which the existing trimmed-side
//! path already writes, measures and booleans.
//!
//! The test is **exactly** `half_angle == 0.0`, on purpose: only a bit-exact zero
//! slope is reclassified, and any genuinely conical face (however shallow) keeps
//! its own label and its own slope.
//!
//! The face's own `(u, v)` box is not read here: it is [`cad_domain::BrepFace`]
//! data, carried by the caller.

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
            // An exactly zero slope is a cylinder, and one that keeps the cone label
            // cannot be carried: the record writer needs a slope for a cone label and
            // the cone's loop mapping refuses a slope-less lateral face, so the face
            // (and with it a round fillet from a record written by an older pin,
            // which recorded it exactly this way) would be lost on the next lift.
            // The radius is guarded like the cylinder label it becomes; a genuinely
            // conical face keeps its own label below.
            if *half_angle == 0.0 {
                if !radius_at_origin.is_finite() || *radius_at_origin <= 0.0 {
                    return Err(LiftRefusal::unsupported(format!(
                        "a zero-slope cone (a cylinder) with radius {radius_at_origin} cannot be \
                         mapped; the radius must be finite and positive"
                    )));
                }
                return Ok(BrepSurface::Cylinder {
                    origin,
                    axis: z,
                    ref_dir: x,
                    radius: *radius_at_origin,
                });
            }
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
        Surface::Spline {
            u_knots,
            v_knots,
            points,
        } => Ok(BrepSurface::Nurbs {
            u_knots: u_knots.clone(),
            v_knots: v_knots.clone(),
            points: points
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|p| [p[0], p[1], p[2], 1.0])
                        .collect::<Vec<[f64; 4]>>()
                })
                .collect(),
        }),
        Surface::RationalSpline {
            u_knots,
            v_knots,
            homogeneous,
        } => Ok(BrepSurface::Nurbs {
            u_knots: u_knots.clone(),
            v_knots: v_knots.clone(),
            points: homogeneous.clone(),
        }),
        _other => Err(LiftRefusal::unsupported(format!(
            "a '{key}' surface is not mapped to the neutral B-rep in P1 \
             (plane, cylinder, cone, sphere, torus, spline and rational-spline are)"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LIFT_UNSUPPORTED;

    /// The frame an older pin wrote for a round on a unit cube's edge at x = 0,
    /// y = 1 (see `tests/fillet.rs`, which measures the current shape).
    fn round_frame() -> Frame {
        Frame {
            origin: [0.1, 0.9, -0.5],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [-1.0, 0.0, 0.0],
        }
    }

    /// An older pin's round fillet came back as `Surface::Cone { half_angle: 0.0 }`
    /// plus a rational net; the label alone must map to the cylinder it describes,
    /// or the face would be unwritable (the cone writer needs a slope) and the
    /// round lost.
    #[test]
    fn a_zero_slope_cone_maps_to_the_cylinder_it_is() {
        let mapped = to_brep(&Surface::Cone {
            frame: round_frame(),
            radius_at_origin: 0.1,
            half_angle: 0.0,
        })
        .expect("a zero-slope cone maps");
        let BrepSurface::Cylinder {
            origin,
            axis,
            ref_dir,
            radius,
        } = mapped
        else {
            panic!("a zero-slope cone is a cylinder, got {mapped:?}");
        };
        assert_eq!(
            origin,
            Point3 {
                x: 0.1,
                y: 0.9,
                z: -0.5
            }
        );
        // z = x_axis × y_axis, x the label's x_axis: the label's own frame.
        assert!((axis.x).abs() < 1e-12 && (axis.y).abs() < 1e-12 && (axis.z - 1.0).abs() < 1e-12);
        assert!((ref_dir.y - 1.0).abs() < 1e-12 && (ref_dir.x).abs() < 1e-12);
        assert_eq!(radius, 0.1);
    }

    /// The reclassification is exact, on purpose: a genuinely conical face keeps
    /// its label and its slope, however shallow.
    #[test]
    fn a_shallow_but_non_zero_cone_keeps_its_cone_label() {
        for half_angle in [1e-9, 1e-3, 0.5] {
            let mapped = to_brep(&Surface::Cone {
                frame: round_frame(),
                radius_at_origin: 0.1,
                half_angle,
            })
            .expect("a cone maps");
            let BrepSurface::Cone {
                sin_half_angle,
                cos_half_angle,
                ..
            } = mapped
            else {
                panic!("a cone of half angle {half_angle} must stay a cone, got {mapped:?}");
            };
            assert_eq!(sin_half_angle, half_angle.sin());
            assert_eq!(cos_half_angle, half_angle.cos());
        }
    }

    /// The cylinder a zero-slope cone becomes needs a radius the cylinder label can
    /// carry; a zero or negative one is refused by name, not mapped.
    #[test]
    fn a_zero_slope_cone_without_a_positive_radius_is_refused() {
        for radius in [0.0, -0.1] {
            let refusal = to_brep(&Surface::Cone {
                frame: round_frame(),
                radius_at_origin: radius,
                half_angle: 0.0,
            })
            .expect_err("a non-positive radius is refused");
            assert_eq!(refusal.code, LIFT_UNSUPPORTED);
            assert!(
                refusal.detail.contains("zero-slope cone") && refusal.detail.contains("radius"),
                "{}",
                refusal.detail
            );
        }
    }
}
