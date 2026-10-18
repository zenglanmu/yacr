//! Headless geometry logic for the CAD pipelines (spec §5.2, §8; audit F14).
//!
//! Everything here is pure CPU math with no `wgpu` types, so it can be unit
//! tested on a host without a GPU. The renderer consumes these decisions when it
//! builds vertex/uniform data. In particular this module owns:
//!
//! * the camera projection for the 3D orbit path (`Camera3d::view_projection`),
//! * the mirrored-transform winding decision,
//! * normal repair (`repaired_normals`) for `Mesh` data with missing or
//!   inconsistent normals, and
//! * the per-frame vertex/triangle budget accounting.

use cad_domain::{Mesh, Point3};
use cad_scene::{FrameBudget, FrameUsage, RenderBatch, RenderTopology};

mod camera;
mod draw_order;
mod math;
mod mesh_repair;

#[cfg(test)]
mod tests;

pub use camera::*;
pub use draw_order::*;
pub(crate) use math::*;
pub use mesh_repair::*;
