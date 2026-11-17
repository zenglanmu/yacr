//! Per-face tessellation entry point: the face mesh container, the failure
//! report, tolerance-driven segment counts and the surface dispatch.

use cad_domain::Point3;

use super::cone::tessellate_cone;
use super::cylinder::tessellate_cylinder;
use super::math::{add, dot, frame_from, mul, sub};
use super::planar::tessellate_planar;
use super::sphere::tessellate_sphere;
use super::torus::tessellate_torus;
use super::types::{BrepCurve, BrepFace, BrepLoop, BrepSurface};

/// One face's local tessellation result, before global assembly.
#[derive(Debug, Clone)]
pub(crate) struct FaceMesh {
    pub vertices: Vec<Point3>,
    pub normals: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
    /// Feature/boundary polylines for this face (source loops).
    pub edges: Vec<Vec<Point3>>,
    /// `true` if any part of the face is a curved approximation.
    pub curved: bool,
    /// Chordal error bound for curved faces.
    pub error_bound: Option<f64>,
}

impl FaceMesh {
    pub(crate) fn empty() -> Self {
        FaceMesh {
            vertices: Vec::new(),
            normals: Vec::new(),
            triangles: Vec::new(),
            edges: Vec::new(),
            curved: false,
            error_bound: None,
        }
    }
}

/// Why a face could not be tessellated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FaceFailure {
    UnsupportedSurface(String),
    UnsupportedCurve(String),
    Degenerate(String),
}

impl FaceFailure {
    pub(crate) fn reason(&self) -> String {
        match self {
            FaceFailure::UnsupportedSurface(t) => format!("unsupported surface '{t}'"),
            FaceFailure::UnsupportedCurve(t) => format!("unsupported curve '{t}'"),
            FaceFailure::Degenerate(m) => format!("degenerate face: {m}"),
        }
    }

    pub(crate) fn is_unsupported(&self) -> bool {
        matches!(
            self,
            FaceFailure::UnsupportedSurface(_) | FaceFailure::UnsupportedCurve(_)
        )
    }
}

/// Number of segments for an arc of `sweep` radians on a circle of `radius`.
///
/// Both the chordal deflection and the angular deflection are respected; the
/// result is monotone in `linear_deflection` (smaller deflection → no fewer
/// segments), which is exactly the tolerance-monotonicity contract.
pub(crate) fn arc_segments(
    radius: f64,
    sweep: f64,
    linear_deflection: f64,
    angular_deflection: f64,
) -> usize {
    let sweep = sweep.abs();
    if !sweep.is_finite() || sweep <= 0.0 || !radius.is_finite() || radius <= 0.0 {
        return 3;
    }
    let chord = if linear_deflection < radius {
        let ratio = (1.0 - linear_deflection / radius).clamp(-1.0, 1.0);
        2.0 * ratio.acos()
    } else {
        std::f64::consts::PI
    };
    let n_chord = (sweep / chord.max(1e-9)).ceil();
    let n_ang = (sweep / angular_deflection.max(1e-9)).ceil();
    let n = n_chord.max(n_ang).max(3.0);
    (n as usize).clamp(3, 8192)
}

/// Tessellate one neutral face. Unsupported geometry is an error, never a mesh.
pub(crate) fn tessellate_face(
    face: &BrepFace,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    match &face.surface {
        BrepSurface::Plane {
            origin,
            normal,
            u_dir,
        } => tessellate_planar(
            face,
            *origin,
            *normal,
            *u_dir,
            linear_deflection,
            angular_deflection,
        ),
        BrepSurface::Sphere {
            center,
            radius,
            u_dir,
            pole,
        } => tessellate_sphere(
            face,
            *center,
            *radius,
            *u_dir,
            *pole,
            linear_deflection,
            angular_deflection,
        ),
        BrepSurface::Torus {
            center,
            normal,
            major_radius,
            minor_radius,
            u_dir,
        } => tessellate_torus(
            face,
            *center,
            *normal,
            *major_radius,
            *minor_radius,
            *u_dir,
            linear_deflection,
            angular_deflection,
        ),
        BrepSurface::Cylinder {
            origin,
            axis,
            ref_dir,
            radius,
        } => tessellate_cylinder(
            face,
            *origin,
            *axis,
            *ref_dir,
            *radius,
            linear_deflection,
            angular_deflection,
        ),
        BrepSurface::Cone {
            origin,
            axis,
            ref_dir,
            radius,
            sin_half_angle,
            cos_half_angle,
        } => tessellate_cone(
            face,
            *origin,
            *axis,
            *ref_dir,
            *radius,
            *sin_half_angle,
            *cos_half_angle,
            linear_deflection,
            angular_deflection,
        ),
        // A NURBS face is a form the neutral model carries, but the **neutral**
        // tessellator has no mesher for it. This message is accurate: a mesher
        // exists in this build (`solidcore-tess`, injected by a carrier as a
        // `SolidTessellator`, ADR-0036 P2.1c part 1); it is simply not supplied to
        // this call. Reported as an unsupported **surface** so the body marks this
        // face missing — never a fabricated mesh, never a silent skip.
        BrepSurface::Nurbs { .. } => Err(FaceFailure::UnsupportedSurface(
            "nurbs-face: the neutral tessellator has no NURBS mesher; a solidcore-tess \
             tessellator injected through the seam (P2.1c) draws it"
                .into(),
        )),
        BrepSurface::Unsupported { type_key } => {
            Err(FaceFailure::UnsupportedSurface(type_key.clone()))
        }
    }
}

/// Effective outward normal of a face.
pub(super) fn face_normal(surface_normal: Point3, reversed: bool) -> Point3 {
    if reversed {
        mul(surface_normal, -1.0)
    } else {
        surface_normal
    }
}

/// Sample a loop into a closed 3D ring.
pub(super) fn sample_loop(
    loop_: &BrepLoop,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<Vec<Point3>, FaceFailure> {
    let mut pts = Vec::new();
    for curve in &loop_.edges {
        match curve {
            BrepCurve::Line { start, .. } => pts.push(*start),
            BrepCurve::Circle {
                center,
                normal,
                u_dir,
                radius,
            } => {
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(FaceFailure::Degenerate("circle has no radius".into()));
                }
                let Some((u, v, _)) = frame_from(*normal, *u_dir) else {
                    return Err(FaceFailure::Degenerate("circle frame is degenerate".into()));
                };
                let n = arc_segments(
                    *radius,
                    std::f64::consts::TAU,
                    linear_deflection,
                    angular_deflection,
                );
                for i in 0..n {
                    let t = std::f64::consts::TAU * (i as f64) / (n as f64);
                    pts.push(add(
                        *center,
                        add(mul(u, *radius * t.cos()), mul(v, *radius * t.sin())),
                    ));
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
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(FaceFailure::Degenerate("arc has no radius".into()));
                }
                let Some((u, v, _)) = frame_from(*normal, *u_dir) else {
                    return Err(FaceFailure::Degenerate("arc frame is degenerate".into()));
                };
                let Some(sweep) = arc_sweep(*center, u, v, *start, *end) else {
                    return Err(FaceFailure::Degenerate(
                        "arc does not sweep a proper part of one turn about its normal".into(),
                    ));
                };
                let from = dot(sub(*start, *center), v).atan2(dot(sub(*start, *center), u));
                let n = arc_segments(*radius, sweep, linear_deflection, angular_deflection);
                // The arc contributes its own start point exactly (loops chain by
                // the shared point, not by a re-evaluated one) and stops before its
                // end, which the next curve contributes as its start.
                pts.push(*start);
                for i in 1..n {
                    let t = from + sweep * (i as f64) / (n as f64);
                    pts.push(add(
                        *center,
                        add(mul(u, *radius * t.cos()), mul(v, *radius * t.sin())),
                    ));
                }
            }
            BrepCurve::Unsupported { type_key } => {
                return Err(FaceFailure::UnsupportedCurve(type_key.clone()));
            }
        }
    }
    dedupe_ring(&mut pts);
    Ok(pts)
}

/// The counter-clockwise sweep in `(0, 2π)` from `arc`'s `start` to its `end`
/// about its normal, in the `(u, v)` frame `frame_from` built for that normal.
///
/// `start` fixes the parameter's origin (an arc's `u_dir` is the frame's axis,
/// not the start angle), so both endpoints' angles are read and the sweep is the
/// positive turn from one to the other.
///
/// `None` when the two endpoints describe no proper sub-span (coincident ends, a
/// full turn, or a non-finite angle) — the caller reports that as degenerate
/// rather than guessing a direction.
pub(super) fn arc_sweep(
    center: Point3,
    u: Point3,
    v: Point3,
    start: Point3,
    end: Point3,
) -> Option<f64> {
    let angle = |p: Point3| {
        let d = sub(p, center);
        dot(d, v).atan2(dot(d, u))
    };
    let from = angle(start);
    let to = angle(end);
    if !from.is_finite() || !to.is_finite() {
        return None;
    }
    let delta = to - from;
    let sweep = if delta > 0.0 {
        delta
    } else {
        delta + std::f64::consts::TAU
    };
    (sweep > 0.0 && sweep < std::f64::consts::TAU).then_some(sweep)
}

/// The chordal sagitta bound of an arc sampled at `arc_segments` of `sweep`.
pub(super) fn arc_error_bound(
    radius: f64,
    sweep: f64,
    linear_deflection: f64,
    angular_deflection: f64,
) -> f64 {
    let n = arc_segments(radius, sweep, linear_deflection, angular_deflection).max(3);
    // Half the angle each chord spans: `sweep / n`, halved. A full turn reduces to
    // the familiar `r(1 - cos(π/n))`.
    radius * (1.0 - (0.5 * sweep / n as f64).cos())
}

/// Drop consecutive (and wrap-around) duplicate points from a ring.
fn dedupe_ring(pts: &mut Vec<Point3>) {
    pts.dedup_by(|a, b| {
        let d = sub(*a, *b);
        dot(d, d) <= 1e-18
    });
    while pts.len() >= 2 {
        let d = sub(pts[0], pts[pts.len() - 1]);
        if dot(d, d) <= 1e-18 {
            pts.pop();
        } else {
            break;
        }
    }
}
