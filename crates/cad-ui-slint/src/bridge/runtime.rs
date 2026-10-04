//! Derived GPU state. Never builds representations or owns a window.
use super::controller::PreparedScene;
use super::*;
use cad_render_wgpu::{wgpu, RenderError};

struct PendingUpload {
    scene: PreparedScene,
    group: usize,
    offset: usize,
    prepared: Option<cad_render_wgpu::PreparedUpload>,
    base_changed: bool,
}

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
    pending_upload: Option<PendingUpload>,
    pub presented_revision: Option<u64>,
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
            pending_upload: None,
            presented_revision: None,
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
        renderer.set_progressive_rendering(true);
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
        self.pending_upload = None;
        self.presented_revision = None;
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
            self.pending_upload = None;
            return Ok(());
        };
        if self.applied_revision == Some(ready.revision) {
            return Ok(());
        }
        let renderer = self.renderer.as_mut().expect("ready renderer");
        let base_changed = self.applied_base != Some(ready.base_revision);
        // Base drawing batches are the prefix; the annotation overlay and the
        // transient highlight/preview overlay are the replaceable suffix. A
        // highlight-only change still re-uploads the suffix (the annotation
        // batches are recomputed from their cached `Arc`), but never the base.
        if self
            .pending_upload
            .as_ref()
            .is_none_or(|pending| pending.scene.revision != ready.revision)
        {
            self.pending_upload = Some(PendingUpload {
                scene: ready.clone(),
                group: usize::from(!base_changed),
                offset: 0,
                prepared: None,
                base_changed,
            });
        }
        let pending = self.pending_upload.as_mut().unwrap();
        #[cfg(not(target_arch = "wasm32"))]
        let started = std::time::Instant::now();
        let mut slices = 0;
        while pending.group < 3 && slices < 4 {
            #[cfg(not(target_arch = "wasm32"))]
            if started.elapsed() >= std::time::Duration::from_millis(8) {
                break;
            }
            let batches = match pending.group {
                0 => &pending.scene.base.added,
                1 => &pending.scene.overlay.added,
                _ => &pending.scene.highlight.added,
            };
            if pending.offset == batches.len() {
                pending.group += 1;
                pending.offset = 0;
                continue;
            }
            let mut end = pending.offset;
            let mut vertices = 0usize;
            while end < batches.len() && end - pending.offset < 256 {
                let next = vertices.saturating_add(batches[end].vertices.len());
                if end > pending.offset && next > 262_144 {
                    break;
                }
                vertices = next;
                end += 1;
            }
            let piece = renderer.prepare_upload_batches(&batches[pending.offset..end])?;
            if let Some(prepared) = &mut pending.prepared {
                prepared.append(piece)?;
            } else {
                pending.prepared = Some(piece);
            }
            pending.offset = end;
            slices += 1;
        }
        if pending.group < 3 {
            return Ok(());
        }
        let pending = self.pending_upload.take().unwrap();
        let prepared = match pending.prepared {
            Some(prepared) => prepared,
            None => renderer.prepare_upload_batches(&[])?,
        };
        renderer.commit_upload(
            prepared,
            if pending.base_changed {
                0
            } else {
                self.base_batches
            },
        )?;
        // Publication follows successful GPU preparation/commit, never precedes it.
        self.base_batches = ready.base.added.len();
        self.applied_base = Some(ready.base_revision);
        self.applied_revision = Some(ready.revision);
        self.dirty.invalidate();
        self.diagnostic = None;
        Ok(())
    }

    pub fn uploading(&self) -> bool {
        self.pending_upload.is_some()
    }
    pub fn drawing_pages(&self) -> bool {
        self.renderer.as_ref().is_some_and(Renderer::frame_pending)
    }
    pub fn drawing_progress(&self) -> Option<(usize, usize)> {
        self.renderer.as_ref().and_then(Renderer::frame_progress)
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
            if !renderer.frame_pending() {
                self.dirty.complete();
            }
            self.frames_rendered += 1;
            self.presented_revision = if renderer.frame_pending() {
                None
            } else {
                self.applied_revision
            };
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
