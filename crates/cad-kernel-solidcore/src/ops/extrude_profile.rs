//! One closed planar profile extruded into a capped solid.
//!
//! The neutral profile is a loop of [`BrepCurve`]s in traversal order: lines,
//! proper arcs and (as the whole loop) one full circle. This module converts it
//! to the kernel's `sweep::Curve` list — lines and arcs stay analytic, so a
//! circle is one exact wall and a polyline's arc segment is one exact cone/cylinder
//! wall, never a fitted spline — and calls `solidcore_ops::sweep::extrude` with a
//! zero draft and both ends capped.
//!
//! # Frame and direction
//!
//! The seam's `direction` is the extrusion vector (any non-zero length, square to
//! the profile plane) and `distance` is positive along it. A caller that wants
//! the other side negates `direction`; the kernel call is never given a negative
//! distance.
//!
//! # Refusals
//!
//! A profile the kernel cannot close or extrude (open loop, self-intersection, a
//! degenerate wall) is `Unsupported` with [`EXTRUDE_PROFILE_FAILED`]; a
//! kernel-integrity failure stays `Failed`; cancellation is
//! [`cad_domain::CadError::Cancelled`] with no product. Nothing is approximated
//! and no empty body is returned.

use std::f64::consts::TAU;

use cad_domain::{BrepCurve, CadError, CadResult, Point3};
use cad_kernel_adapter::{SolidOperationFamily, SolidOutcome};
use solidcore_ops::sweep::{extrude, Curve, DraftCorner};

use super::{body_outcome, unlift, watched};
use crate::error::{self, LiftRefusal};
use crate::record::{cross, dot, length, mul, normalize, sub};
use crate::{EMPTY_RESULT, EXTRUDE_PROFILE_FAILED, LIFT_UNSUPPORTED};

const FAMILY: SolidOperationFamily = SolidOperationFamily::ExtrudeProfile;

/// Extrude `profile` by `distance` along `direction` into a capped solid.
pub(crate) fn extrude_profile(
    profile: &[BrepCurve],
    direction: Point3,
    distance: f64,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> CadResult<SolidOutcome> {
    let curves = match sweep_curves(profile) {
        Ok(curves) => curves,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    let result = watched(cancelled, || {
        extrude(
            &curves,
            [direction.x, direction.y, direction.z],
            distance,
            0.0,
            DraftCorner::Curvature,
            true,
        )
    });
    if cancelled() {
        return Err(CadError::Cancelled);
    }
    match result {
        Ok(body) => finish(&body),
        Err(error) => error::from_extrude_profile(error),
    }
}

/// The neutral loop as the kernel's sweep curves, exactly.
fn sweep_curves(profile: &[BrepCurve]) -> Result<Vec<Curve>, LiftRefusal> {
    let mut out = Vec::with_capacity(profile.len());
    for (index, curve) in profile.iter().enumerate() {
        out.push(match curve {
            BrepCurve::Line { start, end } => Curve::Line {
                start: coords(*start),
                end: coords(*end),
            },
            BrepCurve::Circle {
                center,
                normal,
                u_dir,
                radius,
            } => {
                let axis = unit(*normal, index, "circle normal")?;
                let reference = perpendicular_unit(*u_dir, axis, index, "circle u_dir")?;
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(refused(index, "circle radius must be finite and positive"));
                }
                Curve::Arc {
                    center: coords(*center),
                    axis: coords(axis),
                    reference: coords(reference),
                    radius: *radius,
                    start: 0.0,
                    end: TAU,
                }
            }
            BrepCurve::Arc {
                center,
                normal,
                u_dir,
                radius,
                start,
                end,
            } => {
                let axis = unit(*normal, index, "arc normal")?;
                let reference = perpendicular_unit(*u_dir, axis, index, "arc u_dir")?;
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(refused(index, "arc radius must be finite and positive"));
                }
                let v = cross(axis, reference);
                let a0 = angle_at(*start, *center, reference, v);
                let a1 = angle_at(*end, *center, reference, v);
                let Some((a0, a1)) = arc_angles(a0, a1) else {
                    return Err(refused(index, "arc endpoints are coincident"));
                };
                Curve::Arc {
                    center: coords(*center),
                    axis: coords(axis),
                    reference: coords(reference),
                    radius: *radius,
                    start: a0,
                    end: a1,
                }
            }
            BrepCurve::Unsupported { type_key } => {
                return Err(refused(index, &format!("unsupported curve '{type_key}'")))
            }
        });
    }
    Ok(out)
}

/// The CCW interval `(a0, a1]` about the axis, as the kernel's arc takes it.
///
/// A start equal to the end is a zero-length arc (refused); otherwise the end is
/// lifted by a full turn when the CCW sweep from the start would be non-positive,
/// so `0 < end - start <= 2π`.
fn arc_angles(a0: f64, a1: f64) -> Option<(f64, f64)> {
    if !a0.is_finite() || !a1.is_finite() {
        return None;
    }
    let mut end = a1;
    while end <= a0 {
        end += TAU;
    }
    if (end - a0) <= 1e-12 || (end - a0) > TAU + 1e-9 {
        None
    } else {
        Some((a0, end))
    }
}

fn angle_at(p: Point3, center: Point3, u: Point3, v: Point3) -> f64 {
    let d = sub(p, center);
    dot(d, v).atan2(dot(d, u))
}

fn coords(p: Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

fn unit(v: Point3, index: usize, what: &str) -> Result<Point3, LiftRefusal> {
    if !v.x.is_finite() || !v.y.is_finite() || !v.z.is_finite() || length(v) <= 1e-12 {
        return Err(refused(
            index,
            &format!("{what} must be a finite non-zero vector"),
        ));
    }
    Ok(normalize(v))
}

/// `u_dir` projected square to `axis` and normalised; a parallel `u_dir` has no
/// in-plane reference and is refused.
fn perpendicular_unit(
    u_dir: Point3,
    axis: Point3,
    index: usize,
    what: &str,
) -> Result<Point3, LiftRefusal> {
    let along = dot(u_dir, axis);
    let in_plane = sub(u_dir, mul(axis, along));
    if !in_plane.x.is_finite() || !in_plane.y.is_finite() || !in_plane.z.is_finite() {
        return Err(refused(index, &format!("{what} must be finite")));
    }
    if length(in_plane) <= 1e-12 {
        return Err(refused(
            index,
            &format!("{what} is parallel to the axis, so the arc has no in-plane reference"),
        ));
    }
    Ok(normalize(in_plane))
}

fn refused(index: usize, what: &str) -> LiftRefusal {
    LiftRefusal::new(
        EXTRUDE_PROFILE_FAILED,
        format!("profile curve {index}: {what}"),
    )
}

/// Read the extruded body back into the neutral model.
fn finish(body: &solidcore_ops::Body) -> CadResult<SolidOutcome> {
    let record = match body.to_record() {
        Ok((record, _order)) => record,
        Err(error) => return error::from_ops(error, FAMILY),
    };
    let brep = match unlift(&record) {
        Ok(brep) => brep,
        Err(refusal) => return Ok(error::unsupported(refusal, FAMILY)),
    };
    if brep.face_count() == 0 {
        return Ok(error::failed(
            EMPTY_RESULT,
            "the extrusion produced no face",
        ));
    }
    if brep.shells.len() != 1 {
        return Ok(error::unsupported(
            LiftRefusal::new(
                LIFT_UNSUPPORTED,
                format!(
                    "the extrusion produced {} shells; this seam hands out exactly one closed \
                     shell, and a second one would only be refused by its next lift",
                    brep.shells.len()
                ),
            ),
            FAMILY,
        ));
    }
    Ok(body_outcome(brep))
}
