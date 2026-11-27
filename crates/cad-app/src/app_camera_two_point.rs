//! `CAMERA TWO-POINT`: level the current perspective camera (ADR-0082).
//!
//! Split out of `app_view.rs` for the code-size gate. Two-point perspective keeps
//! vertical edges vertical by making the view direction horizontal; this build
//! applies the level-eye form and keeps the viewport's lens (no lens shift).

use super::*;
use crate::app_view::{camera_refusal, view_outcome};
use crate::camera::normalize3;

impl Application {
    /// `CAMERA TWO-POINT`: level the current perspective camera.
    ///
    /// Two-point perspective keeps vertical edges vertical by making the view
    /// direction horizontal (up is world `+Z`), instead of tilting the camera.
    /// This build applies the **level-eye** form and keeps the viewport's lens:
    /// it does not add SketchUp's lens shift for a pitched camera. The 2D camera
    /// is saved like any other switch to 3D, and the move records a
    /// `ZOOM PREVIOUS` snapshot; the document is never written.
    ///
    /// Refusals: `camera.two_point_requires_perspective` (plan/orthographic),
    /// `camera.two_point_degenerate` (the current look is vertical, so no
    /// horizontal direction exists) and the camera-frame validation. A camera
    /// that is already level answers `camera.two_point_unchanged` without a
    /// snapshot.
    pub(crate) fn two_point_camera(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let viewport = self.viewport_mut(command)?;
        let camera = viewport.camera;
        if camera.projection.is_orthographic() {
            return Err(camera_refusal(
                "camera.two_point_requires_perspective",
                "two-point perspective needs a perspective view; place the camera first (CAMERA) \
                 or switch to a perspective view",
            ));
        }
        let forward = Point3 {
            x: camera.target.x - camera.eye.x,
            y: camera.target.y - camera.eye.y,
            z: camera.target.z - camera.eye.z,
        };
        let horizontal = Point3 {
            x: forward.x,
            y: forward.y,
            z: 0.0,
        };
        let up = Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        if forward.z.abs() < 1e-9 && camera.up == up {
            return Ok(view_outcome(
                "camera.two_point_unchanged",
                "CAMERA TWO-POINT: the camera is already level; nothing changed".into(),
            ));
        }
        let Some(unit) = normalize3(horizontal) else {
            return Err(camera_refusal(
                "camera.two_point_degenerate",
                "the camera looks straight up or down, so it has no horizontal direction to level",
            ));
        };
        let distance = crate::camera::length3(horizontal);
        let target = Point3 {
            x: camera.eye.x + unit.x * distance,
            y: camera.eye.y + unit.y * distance,
            z: camera.eye.z,
        };
        let next = Camera {
            eye: camera.eye,
            target,
            up,
            projection: camera.projection,
        };
        next.validate().map_err(|error| {
            CadError::InvalidInput(format!(
                "camera.invalid_frame: the levelled camera is degenerate ({error})"
            ))
        })?;
        viewport.push_view_snapshot();
        viewport.camera = next;
        let saved = viewport.view_mode.saved_2d_camera();
        viewport.view_mode = ViewMode2d3d::ThreeD { saved_2d: saved };
        Ok(view_outcome(
            "camera.two_point",
            "CAMERA TWO-POINT: levelled the eye so vertical edges stay vertical (the lens is \
             unchanged; a pitched two-point camera would need a lens shift)"
                .into(),
        ))
    }
}
