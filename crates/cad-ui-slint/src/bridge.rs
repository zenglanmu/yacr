//! Slint ⇄ wgpu composition bridge (spec v2.0 §5.2).
//!
//! Slint is the single presentation coordinator. It hands out its shared
//! `Device`/`Queue` through `set_rendering_notifier`; the CAD renderer draws
//! into an offscreen texture that Slint composites as an `Image`. There is no
//! per-frame GPU→CPU readback, and the CAD module never owns the window.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_db::DrawingDatabase;
use cad_domain::{CadError, CadResult, DocumentId, Point3, TaskStamp, TolerancePolicy};
use cad_render_wgpu::{BackendPreference, Camera2d, RenderTarget, Renderer};
use cad_representation::{ProviderRegistry, RepresentationContext};
use cad_scene::{SceneBudget, SceneCache, SceneDelta};

use crate::{UiHandle, YacrWindow};

/// A shared slot the application fills when a drawing becomes available.
pub type IncomingDocument = Rc<RefCell<Option<Arc<DrawingDatabase>>>>;

/// Camera the bridge keeps in sync with the UI.
#[derive(Clone, Copy)]
pub struct BridgeCamera {
    pub center: Point3,
    pub world_per_px: f64,
}

impl Default for BridgeCamera {
    fn default() -> Self {
        BridgeCamera { center: Point3 { x: 0.0, y: 0.0, z: 0.0 }, world_per_px: 1.0 }
    }
}

/// Build scene batches from a drawing database through the provider chain.
pub fn build_scene(database: &DrawingDatabase, stamp: TaskStamp) -> CadResult<SceneDelta> {
    let registry = ProviderRegistry::with_default_provider();
    let context = RepresentationContext::new(DocumentId(0), TolerancePolicy::default(), stamp.clone());
    let mut cache = SceneCache::new(SceneBudget::default());
    let mut combined = SceneDelta { stamp: stamp.clone(), added: Vec::new(), removed_chunks: Vec::new() };
    for entity in database.model_space() {
        let representation = registry.build(entity, &context)?;
        let delta = cache.build(&representation, stamp.clone())?;
        combined.added.extend(delta.added);
    }
    Ok(combined)
}

/// Compute a fit-to-drawing camera from the database bounds.
pub fn fit_camera(database: &DrawingDatabase, logical_size: [f64; 2]) -> BridgeCamera {
    match database.bounds() {
        Some((min, max)) => {
            let ex = (max.x - min.x).max(1e-6);
            let ey = (max.y - min.y).max(1e-6);
            BridgeCamera {
                center: Point3 { x: (min.x + max.x) * 0.5, y: (min.y + max.y) * 0.5, z: 0.0 },
                world_per_px: (ex / logical_size[0].max(1.0)).max(ey / logical_size[1].max(1.0)) * 1.05,
            }
        }
        None => BridgeCamera::default(),
    }
}

struct BridgeState {
    renderer: Option<Renderer>,
    camera: BridgeCamera,
    document: Option<DatabaseIdKey>,
    image_size: Option<(u32, u32)>,
    error: Option<String>,
}

type DatabaseIdKey = cad_domain::DatabaseId;

impl Default for BridgeState {
    fn default() -> Self {
        BridgeState { renderer: None, camera: BridgeCamera::default(), document: None, image_size: None, error: None }
    }
}

/// Install the CAD rendering notifier on the Slint window.
pub fn install(handle: UiHandle, window: &slint::Window, incoming: IncomingDocument) -> CadResult<()> {
    let state = Rc::new(RefCell::new(BridgeState::default()));
    let state_for_notifier = state.clone();
    let frame_handle = handle.clone();

    window
        .set_rendering_notifier(move |render_state, graphics_api| match (render_state, graphics_api) {
            (slint::RenderingState::RenderingSetup, slint::GraphicsAPI::WGPU30 { device, queue, .. }) => {
                let mut s = state_for_notifier.borrow_mut();
                let mut renderer = Renderer::new(BackendPreference::WebGpu);
                match renderer.initialize_with_device(device.clone(), queue.clone()) {
                    Ok(_) => s.renderer = Some(renderer),
                    Err(e) => s.error = Some(format!("CAD renderer init failed: {e}")),
                }
            }
            (slint::RenderingState::BeforeRendering, _) => {
                let mut s = state_for_notifier.borrow_mut();
                // Adopt a newly opened drawing exactly once.
                if let Some(doc) = incoming.borrow().clone() {
                    if s.document != Some(doc.id()) {
                        let stamp = TaskStamp::new(DocumentId(0), 0);
                        if let Ok(delta) = build_scene(&doc, stamp.clone()) {
                            if let Some(renderer) = s.renderer.as_mut() {
                                renderer.clear_batches();
                                let _ = renderer.upload(&delta);
                            }
                            let size = frame_handle.physical_size().unwrap_or(slint::PhysicalSize::new(1, 1));
                            s.camera = fit_camera(&doc, [size.width as f64, size.height as f64]);
                            s.document = Some(doc.id());
                            s.image_size = None;
                        }
                    }
                }
                let size = frame_handle.physical_size().unwrap_or(slint::PhysicalSize::new(1, 1));
                let target = RenderTarget::new(size.width.max(1), size.height.max(1));
                let camera = Camera2d { center: s.camera.center, world_per_px: s.camera.world_per_px, z_plane: 0.0 };
                let rendered = match s.renderer.as_mut() {
                    Some(renderer) => renderer.render(camera, &target).is_ok(),
                    None => return,
                };
                if rendered && s.image_size != Some((target.width, target.height)) {
                    if let Some(renderer) = s.renderer.as_ref() {
                        if let Some(texture) = renderer.frame_texture() {
                            if let Ok(image) = slint::Image::try_from(texture.clone()) {
                                let _ = frame_handle.set_cad_frame(image);
                                s.image_size = Some((target.width, target.height));
                            }
                        }
                    }
                }
            }
            (slint::RenderingState::RenderingTeardown, _) => {
                let mut s = state_for_notifier.borrow_mut();
                s.renderer = None;
                s.image_size = None;
            }
            _ => {}
        })
        .map_err(|e| CadError::GpuFailure(format!("set_rendering_notifier failed: {e}")))?;
    Ok(())
}

/// The Slint component type, re-exported for hosts.
pub type UiWindow = YacrWindow;

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::DrawingDatabaseBuilder;

    #[test]
    fn empty_database_produces_an_empty_scene() {
        let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1)).finish().unwrap();
        let delta = build_scene(&db, TaskStamp::new(DocumentId(0), 0)).unwrap();
        assert!(delta.added.is_empty());
    }

    #[test]
    fn fit_camera_on_empty_database_is_default() {
        let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1)).finish().unwrap();
        let c = fit_camera(&db, [100.0, 100.0]);
        assert_eq!(c.world_per_px, 1.0);
    }
}
