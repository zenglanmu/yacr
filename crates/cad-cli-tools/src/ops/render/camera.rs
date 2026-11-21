//! `render` camera construction: the unchanged 2D plan fit and the optional 3D
//! view.
//!
//! Split out of `ops/render.rs` (file-size ceiling) with no behaviour change:
//! [`FittedCameras`] derives both fitting inputs from a single batch walk, and
//! [`render_frame`] picks the renderer entry point (`render` vs `render_3d`) and
//! reports the stable `camera` object of the result JSON.

use super::*;
use cad_render_wgpu::{Camera2d, Camera3d, FrameStats, RenderTarget, Renderer};

/// The fitting inputs of one frame, derived from the drawable batches only.
pub(super) struct FittedCameras {
    /// The unchanged 2D plan camera. `None` when the batches have no drawable
    /// XY extent (e.g. a conic-only scene).
    pub(super) plan: Option<Camera2d>,
    /// The full 3D bounds `(min, max)` over batch vertices and conic extents.
    /// `None` when no finite 3D point exists.
    pub(super) bounds3: Option<(Point3, Point3)>,
}

/// Widen an optional 3D axis-aligned bounds with one point.
///
/// A non-finite coordinate is skipped rather than propagated: an all-non-finite
/// scene keeps `None`, and the 3D path maps that onto `Camera::enter_3d(None,
/// ..)`'s documented empty-model fallback instead of deriving a NaN camera.
fn extend_bounds(bounds: &mut Option<(Point3, Point3)>, x: f64, y: f64, z: f64) {
    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
        return;
    }
    *bounds = Some(match *bounds {
        None => (Point3 { x, y, z }, Point3 { x, y, z }),
        Some((min, max)) => (
            Point3 {
                x: min.x.min(x),
                y: min.y.min(y),
                z: min.z.min(z),
            },
            Point3 {
                x: max.x.max(x),
                y: max.y.max(y),
                z: max.z.max(z),
            },
        ),
    });
}

/// Fit the 2D plan camera and the 3D bounds from what is actually drawn.
///
/// The 2D fit is unchanged (XY only, `local_origin + vertex`) and stays
/// pixel-identical. The z extent is accumulated alongside it so a 3D view can
/// frame the same batches as a solid; a non-finite z only drops the 3D bound,
/// never the 2D fit. Analytic conics carry no vertices and contribute their
/// world-space AABB instead of invented tessellation.
pub(super) fn fit_cameras(delta: &cad_scene::SceneDelta, width: u32, height: u32) -> FittedCameras {
    let mut fit: Option<(f64, f64, f64, f64)> = None; // min_x, min_y, max_x, max_y
    let mut bounds3: Option<(Point3, Point3)> = None;
    for batch in &delta.added {
        let ox = batch.local_origin.x;
        let oy = batch.local_origin.y;
        let oz = batch.local_origin.z;
        for vertex in &batch.vertices {
            let x = ox + vertex[0] as f64;
            let y = oy + vertex[1] as f64;
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            fit = Some(match fit {
                None => (x, y, x, y),
                Some((min_x, min_y, max_x, max_y)) => {
                    (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                }
            });
            // A batch vertex is `local_origin + vertex`; scene packing keeps a
            // mesh's own first-vertex origin in `local_origin`, so z is as real
            // as x/y.
            extend_bounds(&mut bounds3, x, y, oz + vertex[2] as f64);
        }
        for conic in &batch.conics {
            if let Some((min, max)) = conic.world_aabb() {
                extend_bounds(&mut bounds3, min[0], min[1], min[2]);
                extend_bounds(&mut bounds3, max[0], max[1], max[2]);
            }
        }
    }

    let plan = fit.map(|(min_x, min_y, max_x, max_y)| {
        let span_x = (max_x - min_x).abs();
        let span_y = (max_y - min_y).abs();
        let world_per_px = (span_x / (width as f64 * 0.9))
            .max(span_y / (height as f64 * 0.9))
            .max(1e-9);
        Camera2d {
            center: Point3 {
                x: (min_x + max_x) / 2.0,
                y: (min_y + max_y) / 2.0,
                z: 0.0,
            },
            world_per_px,
            z_plane: 0.0,
        }
    });

    FittedCameras { plan, bounds3 }
}

/// Render one frame with the requested camera and report it as `camera` JSON.
///
/// `None` is the unchanged 2D plan frame. A requested view builds the same
/// camera state the app's 3D entry point uses: the exact eye direction is
/// preserved (eye and target are placed along it), `enter_3d` re-fits the
/// distance to the 3D bounds at the requested field of view, and the frame is
/// rendered as a perspective projection.
pub(super) fn render_frame(
    renderer: &mut Renderer,
    target: &RenderTarget,
    requested_view: Option<RequestedView>,
    plan: Option<Camera2d>,
    bounds3: Option<(Point3, Point3)>,
    fov_degrees: Option<f64>,
) -> Result<(FrameStats, serde_json::Value), CliError> {
    match (requested_view, plan) {
        (None, Some(plan)) => {
            let frame = renderer.render(plan, target).map_err(render_error)?;
            // The 2D camera is an orthographic centre, not a look-at pair: report
            // that centre once instead of inventing an eye at a made-up height.
            let centre = [plan.center.x, plan.center.y, plan.center.z];
            Ok((
                frame,
                serde_json::json!({
                    "mode": "2d",
                    "view": serde_json::Value::Null,
                    "eye": centre,
                    "target": centre,
                    "fov_deg": serde_json::Value::Null,
                }),
            ))
        }
        (None, None) => {
            // Unreachable: `render_document` refuses a scene with neither a 2D
            // fit nor 3D bounds before creating a device.
            Err(CadError::InvalidInput("no drawable geometry to render".into()).into())
        }
        (Some(view), _) => {
            let fov_degrees = fov_degrees.unwrap_or(DEFAULT_RENDER_FOV_DEGREES);
            let fov_radians = fov_degrees.to_radians();
            // An absent extent (no finite vertex z, no conic) is `enter_3d`'s
            // documented empty-model case, not a NaN camera.
            let target_point = bounds3
                .map(|(min, max)| Point3 {
                    x: (min.x + max.x) / 2.0,
                    y: (min.y + max.y) / 2.0,
                    z: (min.z + max.z) / 2.0,
                })
                .unwrap_or(Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                });
            let direction = view.direction();
            // `enter_3d` preserves the eye *direction* from eye/target, so both
            // are placed along the requested direction before it re-fits the
            // distance to the bounds.
            let mut camera = cad_app::Camera::top_view_2d();
            camera.target = target_point;
            camera.eye = Point3 {
                x: target_point.x + direction.x * 1000.0,
                y: target_point.y + direction.y * 1000.0,
                z: target_point.z + direction.z * 1000.0,
            };
            camera.up = view.up_hint();
            let camera = camera.enter_3d(bounds3, fov_radians)?;
            let params = camera.camera3d_params()?;
            let frame = renderer
                .render_3d(
                    Camera3d {
                        eye: params.eye,
                        target: params.target,
                        up: params.up,
                        fov_y: params.fov_y,
                        near: params.near,
                        far: params.far,
                    },
                    target,
                )
                .map_err(render_error)?;
            Ok((
                frame,
                serde_json::json!({
                    "mode": "3d",
                    "view": view.preset_key(),
                    "eye": [params.eye.x, params.eye.y, params.eye.z],
                    "target": [params.target.x, params.target.y, params.target.z],
                    "fov_deg": fov_degrees,
                }),
            ))
        }
    }
}
