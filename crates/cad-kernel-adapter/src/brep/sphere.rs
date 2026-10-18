//! Loop-less spherical faces, sampled from their natural parameterisation.

use cad_domain::Point3;

use super::math::{add, frame_from, mul, normalize, sub};
use super::tessellate::{arc_segments, face_normal, FaceFailure, FaceMesh};
use super::types::BrepFace;

pub(super) fn tessellate_sphere(
    face: &BrepFace,
    center: Point3,
    radius: f64,
    u_dir: Point3,
    pole: Point3,
    linear_deflection: f64,
    angular_deflection: f64,
) -> Result<FaceMesh, FaceFailure> {
    if !face.loops.is_empty() {
        return Err(FaceFailure::UnsupportedSurface(
            "sphere with trimming loops".into(),
        ));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(FaceFailure::Degenerate("sphere has no radius".into()));
    }
    let Some((x, y, z)) = frame_from(pole, u_dir) else {
        return Err(FaceFailure::Degenerate("sphere frame is degenerate".into()));
    };
    let n_lon = arc_segments(
        radius,
        std::f64::consts::TAU,
        linear_deflection,
        angular_deflection,
    );
    let n_lat = arc_segments(
        radius,
        std::f64::consts::PI,
        linear_deflection,
        angular_deflection,
    );
    let n_lat = n_lat.max(2);

    let point = |lon: f64, lat: f64| {
        let (sl, cl) = lat.sin_cos();
        add(
            center,
            mul(
                add(
                    mul(x, cl * lon.cos()),
                    add(mul(y, cl * lon.sin()), mul(z, sl)),
                ),
                radius,
            ),
        )
    };
    let normal_at = |p: Point3| normalize(sub(p, center));

    let mut out = FaceMesh::empty();
    let south = point(0.0, -std::f64::consts::FRAC_PI_2);
    let north = point(0.0, std::f64::consts::FRAC_PI_2);
    out.vertices.push(south);
    out.normals
        .push(face_normal(normal_at(south), face.reversed));
    let south_i = 0u32;
    // Interior latitude rings.
    let mut rings: Vec<Vec<u32>> = Vec::new();
    for j in 1..n_lat {
        let lat = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * (j as f64) / (n_lat as f64);
        let mut ring = Vec::with_capacity(n_lon);
        for i in 0..n_lon {
            let lon = std::f64::consts::TAU * (i as f64) / (n_lon as f64);
            let p = point(lon, lat);
            ring.push(out.vertices.len() as u32);
            out.vertices.push(p);
            out.normals.push(face_normal(normal_at(p), face.reversed));
        }
        rings.push(ring);
    }
    out.vertices.push(north);
    out.normals
        .push(face_normal(normal_at(north), face.reversed));
    let north_i = (out.vertices.len() - 1) as u32;

    // South fan.
    for i in 0..n_lon {
        let a = rings[0][i];
        let b = rings[0][(i + 1) % n_lon];
        out.triangles.push([south_i, b, a]);
    }
    // Middle bands.
    for j in 0..rings.len() - 1 {
        for i in 0..n_lon {
            let a = rings[j][i];
            let b = rings[j][(i + 1) % n_lon];
            let c = rings[j + 1][(i + 1) % n_lon];
            let d = rings[j + 1][i];
            out.triangles.push([a, b, c]);
            out.triangles.push([a, c, d]);
        }
    }
    // North fan.
    let last = rings.last().unwrap();
    for i in 0..n_lon {
        let a = last[i];
        let b = last[(i + 1) % n_lon];
        out.triangles.push([a, b, north_i]);
    }

    out.curved = true;
    out.error_bound = Some(radius * (1.0 - (std::f64::consts::PI / n_lat as f64).cos()));
    Ok(out)
}
