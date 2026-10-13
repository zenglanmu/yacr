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
