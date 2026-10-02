//! Completeness ranking and geometry transform helpers.

use super::*;

/// Maximum INSERT nesting expanded into display fragments.
pub const MAX_INSTANCE_DEPTH: usize = cad_db::MAX_INSTANCE_DEPTH;

pub(crate) fn completeness_rank(c: &Completeness) -> u8 {
    match c {
        Completeness::Complete => 3,
        Completeness::Unverified => 2,
        Completeness::Partial(_) => 1,
        Completeness::Missing(_) => 0,
    }
}

pub(crate) fn weaker_completeness(a: &Completeness, b: &Completeness) -> Completeness {
    if completeness_rank(a) <= completeness_rank(b) {
        a.clone()
    } else {
        b.clone()
    }
}

pub(crate) fn normalize(v: Point3) -> Point3 {
    let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if len > 1e-12 {
        Point3 {
            x: v.x / len,
            y: v.y / len,
            z: v.z / len,
        }
    } else {
        v
    }
}

/// Conservative upper bound on a transform's linear scale.
pub(crate) fn transform_scale(t: &Transform3) -> f64 {
    let m = &t.matrix;
    let mut sum = 0.0;
    for row in m.iter().take(3) {
        for value in row.iter().take(3) {
            sum += value * value;
        }
    }
    sum.sqrt().max(1e-12)
}

pub(crate) fn transform_direction(t: &Transform3, v: Point3) -> Point3 {
    let m = &t.matrix;
    normalize(Point3 {
        x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    })
}

pub(crate) fn transform_mesh(mesh: &Mesh, t: &Transform3) -> Mesh {
    let vertices: Vec<Point3> = mesh.vertices.iter().map(|p| t.apply_point(*p)).collect();
    let normals = if mesh.normals.len() == mesh.vertices.len() {
        mesh.normals
            .iter()
            .map(|n| transform_direction(t, *n))
            .collect()
    } else {
        mesh.normals.clone()
    };
    Mesh {
        vertices,
        triangles: mesh.triangles.clone(),
        normals,
        face_sources: mesh.face_sources.clone(),
    }
}
