//! The `cad_set_view` wire shape and its `CadSession` entry point.
//!
//! Split out of `wire.rs` / `mod.rs` for the code-size gate. The types are the
//! **wire contract**: exactly one of a preset name or an absolute camera, with
//! the projection as an internally tagged object. The free [`set_view`] function
//! is the admission seam both transports call; [`CadSession::set_view`] applies
//! the already-parsed placement through the normal command funnel.
//!
//! Semantics (mode/projection agreement, camera validation, fitted presets and
//! history) live in `cad-app` (`Application::set_view`) and in the shared
//! `parse_set_view`/`check_set_view` rules; this module only carries the values.
use cad_app::{CommandId, CommandPayload, ViewPlacement};

use super::admission::{check_set_view, parse_set_view, Channel};
use super::wire::{AdmissionTarget, CadCommandAck};
use super::CadSession;

/// One absolute camera placement, as `cad_set_view` carries it.
///
/// `mode` is `"2d"` or `"3d"`; the shared parser enforces the mode/projection
/// agreement (2D ⇒ orthographic, 3D ⇒ perspective) before anything is queued.
/// The nested `projection` is an internally tagged object:
///
/// ```json
/// { "eye": [0, -10, 10], "target": [0, 0, 0], "up": [0, 0, 1],
///   "projection": { "kind": "perspective", "verticalFovRadians": 0.785398 },
///   "mode": "3d" }
/// ```
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetViewCameraArgs {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub up: [f64; 3],
    pub projection: SetViewProjectionArgs,
    pub mode: String,
}

/// Projection half of [`SetViewCameraArgs`].
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SetViewProjectionArgs {
    /// Full vertical field of view, radians.
    Perspective {
        #[serde(rename = "verticalFovRadians")]
        vertical_fov_radians: f64,
    },
    /// World units per logical pixel.
    Orthographic { scale: f64 },
}

/// `cad_set_view`: admit one typed camera placement.
///
/// The request is exactly one of `preset` or `camera`; both forms are parsed and
/// validated **before** the gate, so a malformed pose never enters a queue. The
/// gates are the standard-view ones in the same order (modal popup, CAD mode,
/// queue bound). The desktop queues the placement for its owner tick;
/// `dom-canvas` applies it inline.
pub fn set_view(
    target: &mut dyn AdmissionTarget,
    preset: Option<&str>,
    camera: Option<SetViewCameraArgs>,
) -> Result<CadCommandAck, String> {
    let placement = parse_set_view(preset, camera)?;
    check_set_view(
        target.cad_mode(),
        target.modal_open(),
        target.pending(Channel::SetView),
    )?;
    target.set_view(placement)?;
    Ok(CadCommandAck::accepted())
}

impl CadSession {
    /// Apply one typed camera placement (`cad_set_view`).
    ///
    /// The placement is either a host-fitted preset or an absolute camera. The
    /// shared wire parser already validated its shape; the application validates
    /// the camera again and owns the real refusals (empty active space, a
    /// mode/projection mismatch). A refusal is the session's own error and leaves
    /// the camera unchanged; a success records one `ZOOM PREVIOUS` step unless the
    /// placement is identical to the current view.
    pub fn set_view(&self, placement: ViewPlacement) -> Result<(), String> {
        // The typed channel is the only route that carries a payload; echo the
        // selector the router uses for the bare (payload-refused) name.
        if let Some(name) = super::command_name(CommandId::SetView) {
            self.core.record_command(name);
        }
        self.dispatch_payload(CommandId::SetView, CommandPayload::SetView(placement))
    }
}
