//! The `CAMERA` typed grammar, split out of `arguments.rs` for the code-size
//! gate.
//!
//! The bare `CAMERA` selector starts the two-point eye/target placement capture.
//! `CAMERA TWO-POINT` levels the current perspective camera (two-point
//! perspective, view only); `CAMERA WALK` stays a named refusal because it is a
//! separate tool rather than a second camera mode. Nothing else is admitted.

/// The SketchUp `CAMERA` selector.
pub const CAMERA_SELECTOR: &str = "CAMERA";

/// One admitted `CAMERA` argument: a mode this build names and refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraArgument {
    /// `TWO-POINT` / `TWO-POINT-PERSPECTIVE` / `2POINT`.
    TwoPoint,
    /// `WALK`.
    Walk,
}

/// Parse the argument of a `CAMERA <mode>` line.
pub fn parse_camera_argument(argument: &str) -> Option<CameraArgument> {
    let key = argument.trim().to_ascii_uppercase();
    match key.as_str() {
        "TWO-POINT" | "TWO-POINT-PERSPECTIVE" | "2POINT" => Some(CameraArgument::TwoPoint),
        "WALK" => Some(CameraArgument::Walk),
        _ => None,
    }
}

/// The stable rejection reason for a `CAMERA` line outside the grammar.
pub fn invalid_camera_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (CAMERA takes no argument to place a viewpoint; TWO-POINT and \
         WALK are refused by name)",
        text.trim()
    )
}
