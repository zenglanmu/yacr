//! Cylindrical side faces bounded by two full circles, sampled as a strip.

use cad_domain::Point3;

use super::math::{add, dot, frame_from, mul, sub};
use super::tessellate::{arc_segments, face_normal, FaceFailure, FaceMesh};
use super::types::{BrepCurve, BrepFace};

#[allow(clippy::too_many_arguments)]
pub(super) fn tessellate_cylinder(
    face: &BrepFace,
    origin: Point3,
    axis: Point3,
    ref_dir: Point3,
    radius: f64,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    if face.loops.len() != 2 {
        return Err(FaceFailure::UnsupportedSurface(
            "cylinder face without two circular boundaries".into(),
        ));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(FaceFailure::Degenerate("cylinder has no radius".into()));
    }
    let Some((x, y, z)) = frame_from(axis, ref_dir) else {
        return Err(FaceFailure::Degenerate(
            "cylinder frame is degenerate".into(),
        ));
    };
    // Each loop must be a single full circle; its centre gives the axial level.
    let mut levels = Vec::new();
    for loop_ in &face.loops {
        if loop_.edges.len() != 1 {
            return Err(FaceFailure::UnsupportedSurface(
                "cylinder boundary is not a single full circle".into(),
            ));
        }
        match &loop_.edges[0] {
            BrepCurve::Circle { center, .. } => {
                if (dot(sub(*center, origin), z)).is_finite() {
                    levels.push(dot(sub(*center, origin), z));
                } else {
                    return Err(FaceFailure::Degenerate(
                        "cylinder level is not finite".into(),
                    ));
                }
            }
            _ => {
                return Err(FaceFailure::UnsupportedSurface(
                    "cylinder boundary is not a full circle".into(),
                ))
            }
        }
    }
    if (levels[0] - levels[1]).abs() <= 1e-12 {
        return Err(FaceFailure::Degenerate(
            "cylinder boundaries share an axial level".into(),
        ));
    }
    let v0 = levels[0];
    let v1 = levels[1];
    let n = arc_segments(
        radius,
        std::f64::consts::TAU,
        linear_deflection,
        angular_deflection,
    );

    let mut out = FaceMesh::empty();
    for level in [v0, v1] {
        for i in 0..n {
            let t = std::f64::consts::TAU * (i as f64) / (n as f64);
            let radial = add(mul(x, t.cos()), mul(y, t.sin()));
            let p = add(add(origin, mul(z, level)), mul(radial, radius));
            out.vertices.push(p);
            out.normals.push(face_normal(radial, face.reversed));
        }
    }
    let idx = |ring: usize, i: usize| (ring * n + (i % n)) as u32;
    for i in 0..n {
        let a = idx(0, i);
        let b = idx(0, i + 1);
        let c = idx(1, i + 1);
        let d = idx(1, i);
        out.triangles.push([a, b, c]);
        out.triangles.push([a, c, d]);
    }
    out.curved = true;
    out.error_bound = Some(radius * (1.0 - (std::f64::consts::PI / n as f64).cos()));
    Ok(out)
}
