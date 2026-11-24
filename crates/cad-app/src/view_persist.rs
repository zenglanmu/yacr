//! Conversion between the renderer camera (`cad-app`) and the persisted
//! named-view DTO (`cad-domain::view`).
//!
//! The camera types live in this crate because only the renderer uses them; the
//! document stores the renderer-independent [`cad_domain::NamedView`]. There is
//! no second camera model: [`ViewSnapshot`] remains the one capture/restore
//! value, and both directions are pure field maps.

use cad_domain::{CadError, CadResult, NamedView, SpaceId, ViewCamera, ViewProjection};

use crate::camera::{Camera, Projection};
use crate::{ViewMode2d3d, ViewSnapshot};

impl ViewSnapshot {
    /// Build the session snapshot a `VIEW RESTORE` applies.
    ///
    /// Every stored camera is re-validated through the renderer's own projection
    /// constructors; a row that cannot be drawn is a named refusal rather than a
    /// fabricated camera.
    pub(crate) fn from_named_view(view: &NamedView) -> CadResult<Self> {
        let camera = camera_from_view(&view.camera)?;
        camera.validate()?;
        let saved_2d = camera_from_view(&view.saved_2d_camera)?;
        saved_2d.validate()?;
        let view_mode = if view.is_3d {
            ViewMode2d3d::ThreeD { saved_2d }
        } else {
            ViewMode2d3d::TwoD { saved: saved_2d }
        };
        Ok(ViewSnapshot {
            camera,
            view_mode,
            work_plane: view.work_plane,
        })
    }
}

/// Capture a viewport's current view as a persisted row.
pub(crate) fn named_view_from_snapshot(
    name: String,
    view: &ViewSnapshot,
    space: SpaceId,
) -> NamedView {
    NamedView {
        name,
        camera: camera_to_view(view.camera),
        is_3d: matches!(view.view_mode, ViewMode2d3d::ThreeD { .. }),
        saved_2d_camera: camera_to_view(view.view_mode.saved_2d_camera()),
        work_plane: view.work_plane,
        space,
    }
}

fn camera_to_view(camera: Camera) -> ViewCamera {
    ViewCamera {
        eye: camera.eye,
        target: camera.target,
        up: camera.up,
        projection: match camera.projection {
            Projection::Orthographic { scale } => ViewProjection::Orthographic { scale },
            Projection::Perspective {
                vertical_fov_radians,
            } => ViewProjection::Perspective {
                vertical_fov_radians,
            },
        },
    }
}

fn camera_from_view(camera: &ViewCamera) -> CadResult<Camera> {
    let projection = match camera.projection {
        ViewProjection::Orthographic { scale } => Projection::orthographic(scale)
            .map_err(|error| named_error("view.invalid_projection", error))?,
        ViewProjection::Perspective {
            vertical_fov_radians,
        } => Projection::perspective(vertical_fov_radians)
            .map_err(|error| named_error("view.invalid_projection", error))?,
    };
    Ok(Camera {
        eye: camera.eye,
        target: camera.target,
        up: camera.up,
        projection,
    })
}

fn named_error(marker: &str, error: CadError) -> CadError {
    CadError::InvalidInput(format!("{marker}: {error}"))
}
