//! Measurement and snapping.
//!
//! Spec v2.0 §3.3: results carry their input points, unit context, algorithm and
//! precision — never just a formatted string. 2D planar and 3D spatial distance
//! are modelled separately, non-coplanar and self-intersecting area inputs are
//! rejected, and snap tolerance is defined in logical pixels.
//!
//! Audit B24 closed here:
//!
//! * [`MeasurementSpace::World3d`] is a **3D** space and may not be used for the
//!   planar algorithms ([`cad_db::MeasurementAlgorithm::Distance2d`],
//!   [`cad_db::MeasurementAlgorithm::PolylineLength`],
//!   [`cad_db::MeasurementAlgorithm::PlanarPolygonArea`]); those require an explicit
//!   [`MeasurementSpace::Plane`] (or a verified paper/viewport transform).
//! * Area projects a closed ring onto the *measurement work plane* and checks
//!   orthogonality, finiteness and coplanarity with the normalised normal before
//!   computing the signed area. A skewed or degenerate plane, a non-coplanar
//!   ring or a non-finite projection is reported, never silently flattened to 0.
//! * Snap candidates are constrained by a pick-ray parameter `t >= 0` and a
//!   unit-direction contract, so geometry *behind* the camera is not snapped.
//! * Every algorithm checks the space policy up front. Paper space measures the
//!   sheet directly; a viewport model measurement needs a **valid inverse**
//!   transform and is explicitly disabled without one (F04), never guessed from
//!   paper pixels. Inputs from different spaces may not be mixed.
//! * Sizes are checked against the measurement plane's unit scale so extremely
//!   large finite coordinates fail with an explicit error instead of overflowing
//!   to `inf`.

mod algorithms;
mod engine;
mod space;

pub mod snap;
pub use snap::{
    best_candidate, collect_candidates, SnapCandidate, SnapKind, SnapProvenance, SnapTarget,
    SnappedMeasurement, MAX_INTERSECTION_TARGETS,
};

pub use engine::*;
pub use space::*;

#[cfg(test)]
mod tests;
