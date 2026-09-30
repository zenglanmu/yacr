//! Mesh helpers over the domain [`cad_domain::Mesh`] (§3.2 B, §10.3 C).

use cad_domain::{Mesh, Point3};

/// Per-vertex normals via area-weighted triangle normals.
///
/// Degenerate (zero-area) triangles contribute nothing rather than NaN.
pub fn compute_vertex_normals(mesh: &Mesh) -> Vec<Point3> {
    let mut normals = vec![
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0
        };
        mesh.vertices.len()
    ];
    for tri in &mesh.triangles {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() {
            continue;
        }
        let a = mesh.vertices[i0];
        let b = mesh.vertices[i1];
        let c = mesh.vertices[i2];
        let n = cross(sub(b, a), sub(c, a));
        if is_finite(n) {
            normals[i0] = add(normals[i0], n);
            normals[i1] = add(normals[i1], n);
            normals[i2] = add(normals[i2], n);
        }
    }
    for n in &mut normals {
        let l = length(*n);
        if l > 1e-12 {
            *n = Point3 {
                x: n.x / l,
                y: n.y / l,
                z: n.z / l,
            };
        }
    }
    normals
}

/// Axis-aligned bounds of a mesh.
pub fn mesh_bounds(mesh: &Mesh) -> Option<(Point3, Point3)> {
    let mut it = mesh.vertices.iter();
    let first = *it.next()?;
    let mut min = first;
    let mut max = first;
    for p in it {
        min = Point3 {
            x: min.x.min(p.x),
            y: min.y.min(p.y),
            z: min.z.min(p.z),
        };
        max = Point3 {
            x: max.x.max(p.x),
            y: max.y.max(p.y),
            z: max.z.max(p.z),
        };
    }
    Some((min, max))
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn length(a: Point3) -> f64 {
    (a.x * a.x + a.y * a.y + a.z * a.z).sqrt()
}

fn is_finite(a: Point3) -> bool {
    a.x.is_finite() && a.y.is_finite() && a.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::SubElementId;

    #[test]
    fn normals_point_along_z_for_ccw_triangle() {
        let mesh = Mesh {
            vertices: vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
            ],
            triangles: vec![[0, 1, 2]],
            normals: Vec::<Point3>::new(),
            face_sources: vec![None::<SubElementId>],
        };
        let n = compute_vertex_normals(&mesh);
        assert!((n[0].z - 1.0).abs() < 1e-9);
    }
}
