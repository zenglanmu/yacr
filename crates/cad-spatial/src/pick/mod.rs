//! World-space ray hit testing (spec F14 / F05).
//!
//! This is the CPU precise-pick stage that follows the broad phase in
//! [`crate::SpatialIndex`]: given a world [`Ray3`] and a set of
//! [`PickItem`]s (a [`SelectionRef`] plus its geometry and accumulated
//! transform) it returns the **closest** hit and its distance, never the first
//! candidate found.
//!
//! Picking never depends on GPU readback (spec §8.5): it is pure geometry, so
//! it runs identically on the WebGL2 path and in a host without a device.
//!
//! # What is supported
//!
//! * `Line` / `Polyline` / `Point` — analytic segment/point distance;
//! * `Circle` / `Arc` / `Ellipse` / `Spline` / `Text` — discretised through
//!   `cad_geometry::tessellate_geometry` and marked
//!   [`Precision::Approximate`] (see `docs/picking-3d.md`);
//! * `Mesh` — Möller–Trumbore triangle intersection with an explicit
//!   [`BackFacePolicy`];
//! * `Compound` — recurses and keeps the closest child.
//!
//! # What is explicitly *not* supported here
//!
//! * `Opaque` — an unknown payload is [`PickOutcome::Unsupported`]; a caller
//!   must never be handed a fabricated hit for it;
//! * `Insert` — the block graph lives in the database, so expanding an insert
//!   (and attaching its [`InstancePath`]) is the caller's job; a bare insert
//!   here is [`PickOutcome::Unsupported`].
//!
//! Degenerate rays (non-finite or zero-direction) and a non-positive tolerance
//! are refused with [`CadError::InvalidInput`] rather than silently missing.

mod bounds;
mod hit;
mod math;
mod ray;
mod types;

pub use bounds::pick_world_bounds;
pub use hit::{hit_geometry, pick_closest, pick_closest_borrowed};
pub use ray::{ray_segment_closest, ray_triangle, validate_ray};
pub use types::{
    BackFacePolicy, GeometryHit, PickItem, PickOptions, PickOutcome, PickReport, SkippedGeometry,
    MAX_PICK_SEGMENTS, MIN_RAY_DIRECTION, REASON_FACE_SOURCES_ABSENT, REASON_FACE_SOURCE_MISSING,
    REASON_FACE_SOURCE_OUT_OF_RANGE,
};

pub(crate) use bounds::ray_intersects_expanded_bounds;

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    static CULLED_BY_PICK_BOUNDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BYPASS_PICK_BOUNDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PICK_BOUNDS_COMPUTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PICK_NARROW_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
