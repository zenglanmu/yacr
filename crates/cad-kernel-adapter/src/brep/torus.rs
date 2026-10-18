//! Loop-less torus faces, sampled from their natural parameterisation.

use cad_domain::Point3;

use super::math::{add, frame_from, mul, normalize};
use super::tessellate::{arc_segments, face_normal, FaceFailure, FaceMesh};
use super::types::BrepFace;

#[allow(clippy::too_many_arguments)]
pub(super) fn tessellate_torus(
    face: &BrepFace,
    center: Point3,
    normal: Point3,
    major_radius: f64,
    minor_radius: f64,
    u_dir: Point3,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    if !face.loops.is_empty() {
        return Err(FaceFailure::UnsupportedSurface(
            "torus with trimming loops".into(),
        ));
    }
    if major_radius <= 0.0 || minor_radius <= 0.0 {
        return Err(FaceFailure::Degenerate("torus has no radii".into()));
    }
    let Some((x, y, z)) = frame_from(normal, u_dir) else {
        return Err(FaceFailure::Degenerate("torus frame is degenerate".into()));
    };
    let n_u = arc_segments(
        major_radius + minor_radius,
        std::f64::consts::TAU,
        linear_deflection,
        angular_deflection,
    );
    let n_v = arc_segments(
        minor_radius,
        std::f64::consts::TAU,
        linear_deflection,
        angular_deflection,
    );

    let point = |u: f64, v: f64| {
        let (su, cu) = u.sin_cos();
        let (sv, cv) = v.sin_cos();
        add(
            center,
            add(
                mul(
                    add(mul(x, cu), mul(y, su)),
                    major_radius + minor_radius * cv,
                ),
                mul(z, minor_radius * sv),
            ),
        )
    };
    let normal_at = |u: f64, v: f64| {
        let (su, cu) = u.sin_cos();
        let (sv, cv) = v.sin_cos();
        normalize(add(mul(add(mul(x, cu), mul(y, su)), cv), mul(z, sv)))
    };

    let mut out = FaceMesh::empty();
    for i in 0..n_u {
        for j in 0..n_v {
            let u = std::f64::consts::TAU * (i as f64) / (n_u as f64);
            let v = std::f64::consts::TAU * (j as f64) / (n_v as f64);
            let p = point(u, v);
            out.vertices.push(p);
            out.normals
                .push(face_normal(normal_at(u, v), face.reversed));
        }
    }
    let idx = |i: usize, j: usize| ((i % n_u) * n_v + (j % n_v)) as u32;
    for i in 0..n_u {
        for j in 0..n_v {
            let a = idx(i, j);
            let b = idx(i + 1, j);
            let c = idx(i + 1, j + 1);
            let d = idx(i, j + 1);
            out.triangles.push([a, b, c]);
            out.triangles.push([a, c, d]);
        }
    }
    out.curved = true;
    let err_u = (major_radius + minor_radius) * (1.0 - (std::f64::consts::PI / n_u as f64).cos());
    let err_v = minor_radius * (1.0 - (std::f64::consts::PI / n_v as f64).cos());
    out.error_bound = Some(err_u.max(err_v));
    Ok(out)
}
