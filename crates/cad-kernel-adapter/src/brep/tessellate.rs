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
            BrepCurve::Unsupported { type_key } => {
                return Err(FaceFailure::UnsupportedCurve(type_key.clone()))
            }
        }
    }
    dedupe_ring(&mut pts);
    Ok(pts)
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
