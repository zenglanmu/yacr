//! Mesh-level measurements and diagnostics used by the tessellator and the
//! contract tests.

use cad_domain::Mesh;

use super::math::{cross, finite, length, sub};

/// Triangle area of the assembled mesh, for tests and reporting.
pub fn mesh_area(mesh: &Mesh) -> f64 {
    let mut area = 0.0;
    for t in &mesh.triangles {
        let a = mesh.vertices[t[0] as usize];
        let b = mesh.vertices[t[1] as usize];
        let c = mesh.vertices[t[2] as usize];
        area += length(cross(sub(b, a), sub(c, a))) * 0.5;
    }
    area
}

/// Count of positional edges not shared by exactly two facets (open or
/// non-manifold) after welding coincident vertices. A watertight shell reports
/// zero; this is the check `BrepTessellator` uses to decide `Partial`.
pub fn count_open_edges(mesh: &Mesh) -> usize {
    crate::brep_tessellator::detect_open_edges(&mesh.vertices, &mesh.triangles).len()
}

/// Whether every finite coordinate is present (defensive, for diagnostics).
pub fn mesh_is_finite(mesh: &Mesh) -> bool {
    mesh.vertices.iter().all(|p| finite(*p)) && mesh.normals.iter().all(|p| finite(*p))
}

fn finite2(p: [f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

/// Kept for symmetry with `mesh_is_finite`; the 2D path is internal.
#[allow(dead_code)]
pub(crate) fn polygon_is_finite(p: &[[f64; 2]]) -> bool {
    p.iter().all(|q| finite2(*q))
}
