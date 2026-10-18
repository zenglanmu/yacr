//! Application-side camera, projection and work-plane state (spec F13).
//!
//! This module owns the *model* of how the user observes a drawing: the
//! orthographic/perspective projection, the standard views, orbiting about a
//! target with a near-plane guard, zoom-to-cursor, and the right-handed work
//! plane that planar measurement consumes. It computes pure geometry only — no
//! GPU state and no picking — so hosts and tests can drive it without a device.
//!
//! # Conventions
//!
//! * **Handedness**: right-handed world coordinates `+X` right, `+Y` forward
//!   (plan "up"), `+Z` up. The default 2D view looks down `-Z` with screen
//!   `+right = +X` and screen `+up = +Y`.
//! * **Units**: world coordinates are in drawing units, screen coordinates are
//!   logical pixels with `y` growing **downwards** (the screen/pointer
//!   convention). [`Camera::world_to_screen`] flips the sign accordingly.
//! * **Angles**: all angles are radians unless a function name says otherwise;
//!   a perspective field of view is the full *vertical* angle in radians.
//! * **View space**: `right`, `up`, `forward` are unit vectors with `forward`
//!   pointing from the eye toward the target. `(right, up, -forward)` is a
//!   right-handed basis (`right × up = -forward`); [`Camera::view_basis`]
//!   returns that ordered triple and guarantees orthonormality.
//!
//! The implementation is split across the submodules of this directory; every
//! moved item is re-exported here under its original name, so the module path
//! `crate::camera::…` (and therefore the crate's public surface) is unchanged.

use cad_domain::*;

mod math;
mod plane;
mod types;
mod view;

#[cfg(test)]
mod tests;

pub use math::{add, cross, dot3, length3, normalize3, scale3, sub};
pub use plane::{is_finite_point, orthonormal_work_plane, xy_work_plane};
pub use types::{
    Camera, Camera2dParams, Camera3dParams, Projection, ProjectionKind, StandardView, ViewBasis,
    DEFAULT_ORTHO_SCALE, DEFAULT_PERSPECTIVE_FOV_RADIANS, MAX_FOV_RADIANS, MIN_FOV_RADIANS,
    MIN_ORTHO_SCALE,
};

use math::{
    finite2, orthographic_matrix, perspective_matrix, perspective_near_far, rotate_about_axis,
    rotate_about_z,
};
