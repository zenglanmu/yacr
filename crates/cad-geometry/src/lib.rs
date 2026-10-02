//! f64 geometry; display tolerances never control measurement semantics.
//!
//! Spec v2.0 §16.1: there is no single global epsilon. Operations take the
//! [`TolerancePolicy`] and never confuse display discretisation with the
//! geometric predicates used for closure, area or snapping.
//!
//! The OpenCADStudio tessellation path (spec §10.2) is realised by
//! [`GeometryEngine::tessellate_curve`] and [`mesh`]. Kernel and codec types
//! stay outside this crate.

pub mod area;
pub mod clip;
pub mod mesh;
pub mod nurbs;

pub use area::{measure_polygon_area, signed_area, AreaError};
pub use clip::{clip_polyline_to_xy_rect, clip_segment_to_xy_rect};
pub use dash::{dash_polyline, polyline_length, DashIssue, DashOutcome};
pub use hatch::{
    fill_rings, gradient_vertex_colors, normalize_stops, pattern_polylines, sample_gradient,
    simplify, triangulate, FillError, FillMesh, GradientDef, GradientKind, GradientStop, Loop,
    PatternLine, MAX_FILL_POINTS, MAX_FILL_TRIANGLES,
};
pub use mesh::{compute_vertex_normals, mesh_bounds};
pub use nurbs::{clamped_uniform_knots, NurbsCurve};

pub mod dash;

pub mod hatch;

use cad_domain::*;
mod bounds;
mod engine;
mod intersect;
mod tessellate;
mod transform;
mod vector;
mod workplane;

pub use bounds::*;
pub use engine::*;
pub(crate) use intersect::*;
pub use tessellate::*;
pub(crate) use transform::*;
pub use vector::*;
pub use workplane::*;

#[cfg(test)]
mod tests;
