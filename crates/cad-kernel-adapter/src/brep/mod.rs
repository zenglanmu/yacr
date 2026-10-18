//! Neutral B-rep data and its tessellation.
//!
//! This module owns the *only* representation of imported solid geometry that
//! crosses the kernel seam. It is plain data: no ACIS handle, no parser and no
//! acadrust type appears here. The importer (`cad-import-acadrust`, the sole
//! acadrust consumer) lifts a SAT/SAB record graph into these types; this
//! module turns them into a [`cad_domain::Mesh`](crate::TessellationMesh).
//!
//! # Supported subset (honest)
//!
//! * Planar faces bounded by straight and/or full-circular loops, including
//!   inner loops (holes), are triangulated exactly.
//! * Sphere and torus surfaces with an unbounded (loop-less) face are sampled
//!   from their natural parameterisation.
//! * Cylindrical side faces bounded by two full circles are sampled as a strip.
//! * Every other supporting surface, every partial/elliptical curve and every
//!   degenerate loop is reported as an unsupported *face*, never replaced by a
//!   fabricated mesh.
//!
//! The default [`crate::NoKernelTessellator`] still refuses all payloads; a
//! caller opts into this subset explicitly with [`crate::BrepTessellator`].
//!
//! The implementation is split across the submodules of this directory
//! (`types`, `math`, `tessellate`, `planar`, `sphere`, `torus`, `cylinder`,
//! `cone`, `report`). Every moved item is re-exported here under its original
//! name, so the module path `crate::brep::…` (and therefore the crate's public
//! surface) is unchanged.

mod cone;
mod cylinder;
mod math;
mod planar;
mod report;
mod sphere;
mod tessellate;
mod torus;
mod types;

#[cfg(test)]
mod tests;

pub use report::{count_open_edges, mesh_area, mesh_is_finite};
pub use types::{BrepCurve, BrepData, BrepFace, BrepLoop, BrepPlacement, BrepShell, BrepSurface};

// Path-preserving re-exports: the public surface above keeps its visibility,
// and every crate-private helper that used to live directly under
// `crate::brep` keeps resolving at that same path. Re-exporting an item no
// caller currently names is exactly what `unused_imports` flags, so each
// crate-private group carries an explicit allowance; their definitions keep the
// visibility they had before the split.
#[allow(unused_imports)]
pub(crate) use math::{add, cross, dot, length, mul, normalize, sub};
#[allow(unused_imports)]
pub(crate) use report::polygon_is_finite;
#[allow(unused_imports)]
pub(crate) use tessellate::{arc_segments, tessellate_face, FaceFailure, FaceMesh};
