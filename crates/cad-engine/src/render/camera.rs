//! Field-for-field application/render camera mapping and initial fit.

use cad_app::Camera3dParams;
use cad_db::DrawingDatabase;
use cad_domain::Point3;
use cad_render_wgpu::{Camera2d, Camera3d};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BridgeCamera {
    pub center: Point3,
    pub world_per_px: f64,
}

impl Default for BridgeCamera {
    fn default() -> Self {
        Self {
            center: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            world_per_px: 1.0,
        }
    }
}

/// Compute a fit-to-drawing camera from the database bounds.
pub fn fit_camera(database: &DrawingDatabase, logical_size: [f64; 2]) -> BridgeCamera {
    match database.bounds() {
        Some((min, max)) => {
            let ex = (max.x - min.x).max(1e-6);
            let ey = (max.y - min.y).max(1e-6);
            BridgeCamera {
                center: Point3 {
                    x: (min.x + max.x) * 0.5,
                    y: (min.y + max.y) * 0.5,
                    z: 0.0,
                },
                world_per_px: (ex / logical_size[0].max(1.0)).max(ey / logical_size[1].max(1.0))
                    * 1.05,
            }
        }
        None => BridgeCamera::default(),
    }
}

/// Validation belongs to the application; this mapping never repairs a camera.
pub fn camera2d_from_params(params: cad_app::Camera2dParams) -> Camera2d {
    Camera2d {
        center: params.center,
        world_per_px: params.world_per_px,
        z_plane: 0.0,
    }
}

/// Translate an orthographic camera so its canvas rect lands at
/// `canvas_origin_logical` inside a larger render target.
///
/// The renderer maps the camera centre to the **target** centre
/// (`renderer_impl/render_camera.rs`). When the host draws a sub-rect of a
/// full-window surface (Android native-below under the WebView chrome), the
/// camera centre must be shifted by the rect-centre/target-centre delta in
/// world units; `world_per_px` is unchanged, so the zoom fitted to the rect is
/// preserved.
///
/// `target_logical`, `canvas_origin_logical` and `canvas_size_logical` are
/// logical (CSS) pixels; y grows **down** on screen while the camera/world y
/// grows up, so the vertical term is negated. Returns `None` for a degenerate
/// rectangle (non-finite or non-positive sizes) instead of repairing it.
pub fn place_canvas_camera2d(
    camera: Camera2d,
    target_logical: [f64; 2],
    canvas_origin_logical: [f64; 2],
    canvas_size_logical: [f64; 2],
) -> Option<Camera2d> {
    let finite = |v: &f64| v.is_finite();
    if !target_logical.iter().all(finite)
        || !canvas_origin_logical.iter().all(finite)
        || canvas_origin_logical.iter().any(|v| *v < 0.0)
        || !canvas_size_logical.iter().all(finite)
        || target_logical.iter().any(|v| *v <= 0.0)
        || canvas_size_logical.iter().any(|v| *v <= 0.0)
        || !camera.world_per_px.is_finite()
        || camera.world_per_px <= 0.0
        || !camera.center.x.is_finite()
        || !camera.center.y.is_finite()
    {
        return None;
    }
    let offset_x =
        target_logical[0] * 0.5 - (canvas_origin_logical[0] + canvas_size_logical[0] * 0.5);
    let offset_y =
        target_logical[1] * 0.5 - (canvas_origin_logical[1] + canvas_size_logical[1] * 0.5);
    Some(Camera2d {
        center: Point3 {
            x: camera.center.x + offset_x * camera.world_per_px,
            y: camera.center.y - offset_y * camera.world_per_px,
            z: camera.center.z,
        },
        world_per_px: camera.world_per_px,
        z_plane: camera.z_plane,
    })
}

pub fn camera3d_from_params(params: Camera3dParams) -> Camera3d {
    Camera3d {
        eye: params.eye,
        target: params.target,
        up: params.up,
        fov_y: params.fov_y,
        near: params.near,
        far: params.far,
    }
}
