//! f64 geometry; display tolerances never control measurement semantics.
//!
//! Spec v2.0 §16.1: there is no single global epsilon. Operations take the
//! [`TolerancePolicy`] and never confuse display discretisation with the
//! geometric predicates used for closure, area or snapping.
//!
//! Independent display tessellation (spec §10.2) is provided by
//! [`GeometryEngine::tessellate_curve`] and [`mesh`]. Kernel and codec types
//! stay outside this crate.

pub mod area;
pub mod clip;
pub mod mesh;
pub mod nurbs;
pub mod spline;

pub use area::{measure_polygon_area, signed_area, AreaError};
pub use clip::{clip_polyline_to_xy_rect, clip_segment_to_xy_rect};
pub use dash::{dash_polyline, dash_polyline_segments, polyline_length, DashIssue, DashOutcome};
pub use mesh::{compute_vertex_normals, mesh_bounds};
pub use nurbs::{clamped_uniform_knots, NurbsCurve};
pub use spline::{
    chord_parameters, cubic_spline_control_point_count, fit_points_to_cubic_spline,
    FIT_SPLINE_DEGREE, MIN_FIT_POINTS_FOR_CUBIC_SPLINE,
};

pub mod dash;

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
pub use intersect::{infinite_line_intersection, LineIntersection};
pub use tessellate::*;
pub use transform::bulge_arc_geometry;
pub(crate) use transform::*;
pub use vector::*;
pub use workplane::*;

#[cfg(test)]
mod tests;
