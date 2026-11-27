//! `CAMERA <mode>`: two-point perspective and the one remaining named refusal.
//!
//! The bare selector is a two-point eye/target capture (routed as `BeginDraw`).
//! `CAMERA TWO-POINT` dispatches the level-eye two-point camera command (view
//! only; the app owns the geometry); `CAMERA WALK` stays a named refusal because
//! WALK/LOOKAROUND are separate tools, not a second camera mode.

use cad_app::{CommandId, CommandPayload};

use super::commands::{
    invalid_camera_argument_reason, parse_camera_argument, CameraArgument, CAMERA_SELECTOR,
};

impl super::CadSession {
    /// Apply one typed `CAMERA <argument>` line.
    pub(super) fn camera_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(mode) = parse_camera_argument(argument) else {
            return self.refuse_argument(invalid_camera_argument_reason(text));
        };
        match mode {
            CameraArgument::TwoPoint => self.dispatch_typed_argument(
                CAMERA_SELECTOR,
                CommandId::PositionCamera,
                CommandPayload::PositionCameraTwoPoint,
            ),
            CameraArgument::Walk => self.refuse_argument(
                "camera.walk_not_implemented: CAMERA WALK is not a second walk mode; use the WALK \
                 and LOOKAROUND tools"
                    .into(),
            ),
        }
    }
}
