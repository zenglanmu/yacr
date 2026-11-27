//! The typed `WALK` / `LOOKAROUND` payload rows.

use cad_domain::Point3;

/// One `WALK` command: walk to a picked ground point, or walk a distance along
/// the current horizontal view direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WalkCommand {
    /// Walk horizontally to this world point (the eye keeps its z).
    To(Point3),
    /// Walk this many model units forward (negative walks backward), along the
    /// horizontal component of the current view direction.
    By(f64),
}

/// One `LOOKAROUND` command: look at a picked point, or turn by yaw/pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LookAroundCommand {
    /// Turn the eye toward this world point (the eye itself does not move).
    At(Point3),
    /// Turn by yaw (about world `+Z`) and pitch (about the camera's right axis),
    /// in degrees.
    By {
        /// Degrees about world `+Z` (positive = turn left, matching `VPOINT`'s
        /// right-handed convention).
        yaw_degrees: f64,
        /// Degrees above/below the current view direction.
        pitch_degrees: f64,
    },
}
