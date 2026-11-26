#![forbid(unsafe_code)]
//! Kernel-free triangle-mesh editing and hole filling (ADR-0061 P-MESH-2/3).
//!
//! This crate is the shared topology/edit layer the ADR places beside the kernel
//! seam. It links **no** modelling kernel and never names one: input and output
//! are [`cad_domain::Mesh`], and every operation is content-in/content-out.
//!
//! What it owns:
//!
//! * position-welded topology census (boundary/non-manifold edges);
//! * face regions addressed by the persisted `Mesh.face_sources` key;
//! * face-region move / press-pull with generated side walls;
//! * triangle subdivision and Laplacian smoothing;
//! * planar boundary-loop hole filling with a per-hole report (P-MESH-3).
//!
//! What it deliberately does **not** do:
//!
//! * recompute normals, UVs, vertex colours, textures or PBR channels (the edit
//!   products drop them rather than keep stale values);
//! * fill a non-planar or self-intersecting hole (it reports the refusal);
//! * turn a mesh into exact B-rep geometry.

mod fill;
mod region;
mod topology;

pub use fill::{fill_holes, require_filled, FillHolesResult, HoleReport};
pub use region::{
    extrude_faces, is_closed, move_edges, move_faces, region_for, report, smooth, subdivide,
};
pub use topology::{
    boundary_loops, classify, edge_for_key, edges, face_region, face_region_triangles,
    face_region_union, newell_normal, region_boundary_loops, weld, FaceRegion, MeshClass, MeshEdge,
    TopologyReport, WeldedMesh,
};

use cad_domain::{CadError, Point3};

/// A stable command-layer code carrying a detail, as `InvalidInput`.
pub(crate) fn invalid(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// A stable command-layer code carrying a detail, as `Unsupported`.
pub(crate) fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

/// A point helper for tests and callers.
pub fn point(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

#[cfg(test)]
mod tests;
