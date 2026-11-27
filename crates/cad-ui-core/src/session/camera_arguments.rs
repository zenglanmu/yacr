//! `CAMERA <mode>`: the reference tool's other modes, refused by name.
//!
//! The bare selector is a two-point eye/target capture (routed as `BeginDraw`).
//! Two-point perspective and walk mode have no model here, so their typed forms
//! answer with stable markers instead of placing a different camera behind the
//! user's back.

use super::commands::{invalid_camera_argument_reason, parse_camera_argument, CameraArgument};

impl super::CadSession {
    /// Apply one typed `CAMERA <argument>` line.
    pub(super) fn camera_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(mode) = parse_camera_argument(argument) else {
            return self.refuse_argument(invalid_camera_argument_reason(text));
        };
        match mode {
            CameraArgument::TwoPoint => self.refuse_argument(
                "camera.two_point_not_implemented: two-point perspective is not modeled; this \
                 build places one perspective frame with world +Z up"
                    .into(),
            ),
            CameraArgument::Walk => self.refuse_argument(
                "camera.walk_not_implemented: walk mode is not modeled; the WALK tool is still \
                 pending"
                    .into(),
            ),
        }
    }
}
