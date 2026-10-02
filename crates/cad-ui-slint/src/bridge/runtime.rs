//! Derived GPU state. Never builds representations or owns a window.
use super::controller::PreparedScene;
use super::*;
use cad_render_wgpu::{wgpu, RenderError};
use cad_scene::SceneDelta;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderLifecycle {
    Detached,
    Ready { device_epoch: u64 },
    Lost { reason: String },
    Failed { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameBinding {
    pub device_epoch: u64,
    pub texture_revision: u64,
    pub size: (u32, u32),
}

pub(super) struct PresentedFrame {
    pub binding: FrameBinding,
    pub texture: wgpu::Texture,
}

/// The requested and rendered frame versions are separate: errors never mark
/// a frame clean. UI-only Slint frames reuse the previously drawn texture.
#[derive(Default)]
pub(super) struct FrameInvalidation {
    requested: u64,
    rendered: Option<u64>,
}
impl FrameInvalidation {
    pub fn invalidate(&mut self) {
        self.requested += 1;
    }
    pub fn dirty(&self) -> bool {
        self.rendered != Some(self.requested)
    }
    pub fn complete(&mut self) {
        self.rendered = Some(self.requested);
    }
}

pub(super) struct CadRenderRuntime {
    pub renderer: Option<Renderer>,
    pub lifecycle: RenderLifecycle,
    pub outcome: Option<BackendOutcome>,
    pub caps: Option<BackendCapabilities>,
    epoch: u64,
    applied_revision: Option<u64>,
    applied_base: Option<u64>,
    base_batches: usize,
    target_size: Option<(u32, u32)>,
    pub dirty: FrameInvalidation,
    pub diagnostic: Option<String>,
    pub frames_rendered: u64,
}
impl Default for CadRenderRuntime {
    fn default() -> Self {
        Self {
            renderer: None,
            lifecycle: RenderLifecycle::Detached,
            outcome: None,
            caps: None,
            epoch: 0,
            applied_revision: None,
            applied_base: None,
            base_batches: 0,
            target_size: None,
            dirty: FrameInvalidation::default(),
            diagnostic: None,
            frames_rendered: 0,
        }
    }
}
impl CadRenderRuntime {
    pub fn attach(
        &mut self,
        device: wgpu::Device,
        queue: wgpu::Queue,
        preference: BackendPreference,
    ) {
        self.detach();
        let mut renderer = Renderer::new(preference);
        match renderer.initialize_with_device(device, queue) {
            Ok(caps) => {
                self.epoch += 1;
                self.outcome = Some(BackendOutcome::Initialized {
                    preference: preference_choice(preference),
                    actual: backend_kind(caps.actual),
                });
                self.caps = Some(caps);
                self.renderer = Some(renderer);
                self.lifecycle = RenderLifecycle::Ready {
                    device_epoch: self.epoch,
                };
                self.dirty.invalidate();
            }
            Err(error) => self.fail(preference, error.to_string(), false),
        }
    }

    pub fn detach(&mut self) {
        self.renderer = None;
        self.caps = None;
        self.outcome = None;
        self.applied_revision = None;
        self.applied_base = None;
        self.target_size = None;
        self.base_batches = 0;
        self.lifecycle = RenderLifecycle::Detached;
        self.diagnostic = None;
        self.dirty.invalidate();
    }

    pub fn fail(&mut self, preference: BackendPreference, reason: String, lost: bool) {
        self.detach();
        self.lifecycle = if lost {
            RenderLifecycle::Lost {
                reason: reason.clone(),
            }
        } else {
            RenderLifecycle::Failed {
                reason: reason.clone(),
            }
        };
        self.outcome = Some(BackendOutcome::Failed {
            preference: preference_choice(preference),
            failure: BackendFailure::InitFailed {
                reason: reason.clone(),
            },
        });
        self.diagnostic = Some(reason);
    }

    pub fn apply_ready_updates(&mut self, ready: Option<&PreparedScene>) -> CadResult<()> {
        if !matches!(self.lifecycle, RenderLifecycle::Ready { .. }) {
            return Ok(());
        }
        let Some(ready) = ready else {
            return Ok(());
        };
        if self.applied_revision == Some(ready.revision) {
            return Ok(());
        }
        let renderer = self.renderer.as_mut().expect("ready renderer");
        let base_changed = self.applied_base != Some(ready.base_revision);
        let combined;
        let delta = if base_changed {
            combined = SceneDelta {
                stamp: ready.overlay.stamp.clone(),
                added: ready
                    .base
                    .added
                    .iter()
                    .chain(ready.overlay.added.iter())
                    .cloned()
                    .collect(),
                removed_chunks: vec![],
            };
            &combined
        } else {
            &ready.overlay
        };
        let prepared = renderer.prepare_upload(delta)?;
        renderer.commit_upload(prepared, if base_changed { 0 } else { self.base_batches })?;
        // Publication follows successful GPU preparation/commit, never precedes it.
        self.base_batches = ready.base.added.len();
        self.applied_base = Some(ready.base_revision);
        self.applied_revision = Some(ready.revision);
        self.dirty.invalidate();
        self.diagnostic = None;
        Ok(())
    }

    pub fn render_if_dirty(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        if !matches!(self.lifecycle, RenderLifecycle::Ready { .. }) {
            return Ok(None);
        }
        if logical.iter().any(|v| !v.is_finite() || *v <= 0.0) || !scale.is_finite() || scale <= 0.0
        {
            return Err(RenderError::Frame("invalid CAD surface dimensions".into()));
        }
        let size = (
            (logical[0] * scale).round().max(1.0) as u32,
            (logical[1] * scale).round().max(1.0) as u32,
        );
        if self.target_size != Some(size) {
            self.dirty.invalidate();
        }
        let renderer = self.renderer.as_mut().expect("ready renderer");
        // An attachment can change at the same size. Its identity is inspected
        // for presentation even when CAD pixels do not need redrawing.
        if self.dirty.dirty() {
            let target = RenderTarget::new(size.0, size.1);
            match view.mode {
                ProjectionKind::TwoD => {
                    let params = view
                        .camera
                        .camera2d_params()
                        .map_err(|e| RenderError::Frame(e.to_string()))?;
                    let mut camera = camera2d_from_params(params);
                    camera.world_per_px /= scale;
                    renderer.render(camera, &target)?;
                }
                ProjectionKind::ThreeD => {
                    let params = view
                        .camera
                        .camera3d_params()
                        .map_err(|e| RenderError::Frame(e.to_string()))?;
                    renderer.render_3d(camera3d_from_params(params), &target)?;
                }
            }
            self.target_size = Some(size);
            self.dirty.complete();
            self.frames_rendered += 1;
        }
        Ok(renderer.frame_texture().map(|texture| PresentedFrame {
            binding: FrameBinding {
                device_epoch: self.epoch,
                texture_revision: renderer.texture_revision(),
                size,
            },
            texture: texture.clone(),
        }))
    }
}
