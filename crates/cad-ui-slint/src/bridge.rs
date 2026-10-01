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
use cad_domain::{
    CadError, CadResult, DocumentId, Point3, SceneIdentity, TaskStamp, TolerancePolicy,
};
use cad_render_wgpu::{
    ActiveBackend, BackendCapabilities, BackendPreference, Camera2d, RenderTarget, Renderer,
};
use cad_representation::{FontEngine, ProviderRegistry, RepresentationContext};
use cad_scene::{SceneBudget, SceneCache, SceneDelta};

use crate::{UiHandle, YacrWindow};

use cad_app::layers::LayerOverrideSet;

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
        BridgeCamera {
            center: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            world_per_px: 1.0,
        }
    }
}

/// Build scene batches from a drawing database through the provider chain.
pub fn build_scene(database: &DrawingDatabase, stamp: TaskStamp) -> CadResult<SceneDelta> {
    build_scene_with_fonts(database, stamp, None)
}

/// Build scene batches, shaping text with `fonts` when supplied.
pub fn build_scene_with_fonts(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
) -> CadResult<SceneDelta> {
    build_scene_with_overrides(database, stamp, fonts, &LayerOverrideSet::new())
}

/// Build scene batches honouring temporary layer visibility (spec F03).
///
/// An entity whose layer is hidden — either by the drawing's own `visible` flag
/// or by a session override — is skipped, so hiding a layer changes the batches
/// without re-importing or re-parsing the drawing database. The override set is
/// a plain value owned by `cad-app`; `cad-representation` and `cad-scene` are
/// untouched, so this filter is the whole wire-up (see `docs/panels.md`).
pub fn build_scene_with_overrides(
    database: &DrawingDatabase,
    stamp: TaskStamp,
    fonts: Option<Arc<FontEngine>>,
    overrides: &LayerOverrideSet,
) -> CadResult<SceneDelta> {
    let registry = ProviderRegistry::with_default_provider();
    let mut context =
        RepresentationContext::new(DocumentId(0), TolerancePolicy::default(), stamp.clone());
    if let Some(fonts) = fonts {
        context = context.with_fonts(fonts);
    }
    let mut cache = SceneCache::new(SceneBudget::default());
    let mut combined = SceneDelta {
        stamp: stamp.clone(),
        added: Vec::new(),
        removed_chunks: Vec::new(),
    };
    for entity in cad_app::layers::visible_model_entities(database, overrides) {
        let representation = registry.build_expanded(database, entity, &context)?;
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

struct BridgeState {
    renderer: Option<Renderer>,
    camera: BridgeCamera,
    document: Option<SceneIdentity>,
    image_size: Option<(u32, u32)>,
    error: Option<String>,
    caps: Option<BackendCapabilities>,
    fonts_present: bool,
    /// Fingerprint of the applied layer overrides; a change forces a rebuild.
    overrides_fingerprint: u64,
}

impl Default for BridgeState {
    fn default() -> Self {
        BridgeState {
            renderer: None,
            camera: BridgeCamera::default(),
            document: None,
            image_size: None,
            error: None,
            caps: None,
            fonts_present: false,
            overrides_fingerprint: 0,
        }
    }
}

/// Cloneable view control the host uses to navigate and inspect the CAD frame.
///
/// The authoritative camera lives in the application `Viewport`; the host syncs
/// it here after every command, so there is exactly one camera truth (spec §4.2).
#[derive(Clone)]
pub struct CadView {
    state: Rc<RefCell<BridgeState>>,
    handle: UiHandle,
    incoming: IncomingDocument,
    fonts: Rc<RefCell<Option<Arc<FontEngine>>>>,
    overrides: Rc<RefCell<LayerOverrideSet>>,
    preference: BackendPreference,
}

impl CadView {
    /// Mirror the authoritative viewport camera into the render state.
    pub fn set_camera(&self, center: Point3, world_per_px: f64) {
        if !world_per_px.is_finite() || world_per_px <= 0.0 {
            return;
        }
        let mut s = self.state.borrow_mut();
        s.camera.center = Point3 {
            x: center.x,
            y: center.y,
            z: 0.0,
        };
        s.camera.world_per_px = world_per_px.clamp(1e-12, 1e18);
        drop(s);
        let _ = self.handle.request_redraw();
    }

    /// Snapshot of the render camera for diagnostics/tests.
    pub fn camera(&self) -> BridgeCamera {
        self.state.borrow().camera
    }

    pub fn active_backend(&self) -> Option<ActiveBackend> {
        self.state.borrow().caps.as_ref().map(|c| c.actual)
    }

    pub fn capabilities(&self) -> Option<(ActiveBackend, bool, u32)> {
        self.state
            .borrow()
            .caps
            .as_ref()
            .map(|c| (c.actual, c.compute, c.max_texture_dimension))
    }

    pub fn last_error(&self) -> Option<String> {
        self.state.borrow().error.clone()
    }

    pub fn preference(&self) -> BackendPreference {
        self.preference
    }

    /// Request a frame; used after the host mutates the document slot.
    pub fn request_redraw(&self) {
        let _ = self.handle.request_redraw();
    }

    /// Install the shaping fonts the host loaded from the font catalog.
    ///
    /// The next frame rebuilds the scene so text is shaped; passing a new set
    /// (for example after the CDN fetch resolves) triggers a rebuild too.
    pub fn set_fonts(&self, fonts: Arc<FontEngine>) {
        *self.fonts.borrow_mut() = Some(fonts);
        let _ = self.handle.request_redraw();
    }

    /// Drop the shaping fonts; text falls back to unshaped primitives.
    pub fn clear_fonts(&self) {
        *self.fonts.borrow_mut() = None;
        let _ = self.handle.request_redraw();
    }

    /// Apply the session's temporary layer overrides (spec F03).
    ///
    /// The next frame rebuilds the scene batches honouring the new visibility;
    /// the drawing database is not re-parsed and its revision does not change.
    pub fn set_layer_overrides(&self, overrides: LayerOverrideSet) {
        *self.overrides.borrow_mut() = overrides;
        let _ = self.handle.request_redraw();
    }

    /// Drop derived GPU resources (device loss or backend rebuild).
    pub fn teardown(&self) {
        let mut s = self.state.borrow_mut();
        s.renderer = None;
        s.image_size = None;
        s.caps = None;
        self.incoming.borrow_mut().take();
    }
}

/// Install the CAD rendering notifier on the Slint window.
pub fn install(
    handle: UiHandle,
    window: &slint::Window,
    incoming: IncomingDocument,
) -> CadResult<CadView> {
    install_with_preference(handle, window, incoming, BackendPreference::WebGpu)
}

/// Install with an explicit renderer preference (hosts choose the device).
pub fn install_with_preference(
    handle: UiHandle,
    window: &slint::Window,
    incoming: IncomingDocument,
    preference: BackendPreference,
) -> CadResult<CadView> {
    let state = Rc::new(RefCell::new(BridgeState::default()));
    let state_for_notifier = state.clone();
    let frame_handle = handle.clone();
    let scene_incoming = incoming.clone();
    let fonts_slot: Rc<RefCell<Option<Arc<FontEngine>>>> = Rc::new(RefCell::new(None));
    let scene_fonts = fonts_slot.clone();
    let overrides_slot: Rc<RefCell<LayerOverrideSet>> =
        Rc::new(RefCell::new(LayerOverrideSet::new()));
    let scene_overrides = overrides_slot.clone();

    window
        .set_rendering_notifier(move |render_state, graphics_api| {
            match (render_state, graphics_api) {
                (
                    slint::RenderingState::RenderingSetup,
                    slint::GraphicsAPI::WGPU30 { device, queue, .. },
                ) => {
                    let mut s = state_for_notifier.borrow_mut();
                    let mut renderer = Renderer::new(preference);
                    match renderer.initialize_with_device(device.clone(), queue.clone()) {
                        Ok(caps) => {
                            s.caps = Some(caps);
                            s.renderer = Some(renderer);
                        }
                        Err(e) => s.error = Some(format!("CAD renderer init failed: {e}")),
                    }
                }
                (slint::RenderingState::BeforeRendering, _) => {
                    let mut s = state_for_notifier.borrow_mut();
                    // Adopt a newly opened drawing exactly once. The camera is owned
                    // by the application viewport and mirrored through `CadView`.
                    if let Some(doc) = scene_incoming.borrow().clone() {
                        let stamp = TaskStamp::new(DocumentId(0), 0);
                        // Compare full content identity, not the bare database
                        // id: a different drawing reusing the same id must still
                        // rebuild the GPU batches (audit B04).
                        let identity = doc.scene_identity();
                        let fonts = scene_fonts.borrow().clone();
                        let has_fonts = fonts.is_some();
                        let overrides = scene_overrides.borrow().clone();
                        let overrides_fingerprint = overrides.fingerprint();
                        if s.document != Some(identity)
                            || s.fonts_present != has_fonts
                            || s.overrides_fingerprint != overrides_fingerprint
                        {
                            if let Ok(delta) =
                                build_scene_with_overrides(&doc, stamp.clone(), fonts, &overrides)
                            {
                                if let Some(renderer) = s.renderer.as_mut() {
                                    renderer.clear_batches();
                                    let _ = renderer.upload(&delta);
                                }
                                s.document = Some(identity);
                                s.fonts_present = has_fonts;
                                s.overrides_fingerprint = overrides_fingerprint;
                                s.image_size = None;
                            }
                        }
                    }
                    let size = frame_handle
                        .physical_size()
                        .unwrap_or(slint::PhysicalSize::new(1, 1));
                    let target = RenderTarget::new(size.width.max(1), size.height.max(1));
                    let camera = Camera2d {
                        center: s.camera.center,
                        world_per_px: s.camera.world_per_px,
                        z_plane: 0.0,
                    };
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
                    s.caps = None;
                }
                _ => {}
            }
        })
        .map_err(|e| CadError::GpuFailure(format!("set_rendering_notifier failed: {e}")))?;
    Ok(CadView {
        state,
        handle,
        incoming,
        fonts: fonts_slot,
        overrides: overrides_slot,
        preference,
    })
}

/// The Slint component type, re-exported for hosts.
pub type UiWindow = YacrWindow;

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::DrawingDatabaseBuilder;

    #[test]
    fn empty_database_produces_an_empty_scene() {
        let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
            .finish()
            .unwrap();
        let delta = build_scene(&db, TaskStamp::new(DocumentId(0), 0)).unwrap();
        assert!(delta.added.is_empty());
    }

    #[test]
    fn fit_camera_on_empty_database_is_default() {
        let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
            .finish()
            .unwrap();
        let c = fit_camera(&db, [100.0, 100.0]);
        assert_eq!(c.world_per_px, 1.0);
    }

    #[test]
    fn camera_stays_finite_under_extreme_zoom() {
        let db = DrawingDatabaseBuilder::new(cad_domain::DatabaseId(1))
            .finish()
            .unwrap();
        let c = fit_camera(&db, [100.0, 100.0]);
        assert!(c.world_per_px.is_finite());
    }
}
