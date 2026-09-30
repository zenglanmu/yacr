//! Triangle mesh helpers (spec v2.0 §3.2 B, §10.3 C).
//!
//! This is the interchange shape a [`crate::tessellate`] or kernel adapter
//! produces. Kernel-specific face/body types stay in `cad-kernel-adapter`.

use glam::DVec3;

/// A triangle mesh in world coordinates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mesh {
    pub positions: Vec<DVec3>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.len() < 3
    }
}

/// Per-vertex normals via area-weighted triangle normals.
///
/// Degenerate triangles (zero area) contribute nothing rather than NaN.
pub fn compute_triangle_normals(mesh: &Mesh) -> Vec<DVec3> {
    let mut normals = vec![DVec3::ZERO; mesh.positions.len()];
    for tri in mesh.indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= mesh.positions.len() || i1 >= mesh.positions.len() || i2 >= mesh.positions.len() {
            continue;
        }
        let a = mesh.positions[i0];
        let b = mesh.positions[i1];
        let c = mesh.positions[i2];
        let n = (b - a).cross(c - a); // magnitude ∝ area, which weights correctly
        if n.is_finite() {
            normals[i0] += n;
            normals[i1] += n;
            normals[i2] += n;
        }
    }
    for n in &mut normals {
        let len = n.length();
        if len > 1e-12 {
            *n /= len;
        }
    }
    normals
}

/// Axis-aligned bounds of a mesh.
pub fn mesh_bounds(mesh: &Mesh) -> Option<(DVec3, DVec3)> {
    let mut it = mesh.positions.iter();
    let first = *it.next()?;
    let mut min = first;
    let mut max = first;
    for p in it {
        min = min.min(*p);
        max = max.max(*p);
    }
    Some((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normals_point_along_z_for_ccw_triangle() {
        let mesh = Mesh {
            positions: vec![DVec3::ZERO, DVec3::X, DVec3::Y],
            indices: vec![0, 1, 2],
        };
        let n = compute_triangle_normals(&mesh);
        assert!((n[0] - DVec3::Z).length() < 1e-9);
    }

    #[test]
    fn degenerate_triangle_does_not_produce_nan() {
        let mesh = Mesh { positions: vec![DVec3::ZERO, DVec3::X], indices: vec![0, 0, 1] };
        let n = compute_triangle_normals(&mesh);
        assert!(n.iter().all(|v| v.is_finite()));
    }
}
