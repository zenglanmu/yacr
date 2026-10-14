//! Derived GPU state. Never builds representations or owns a window.
use super::controller::PreparedScene;
use super::*;
use cad_render_wgpu::{wgpu, BackendCapabilities, RenderError, RenderTarget, Renderer};

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

/// How complete a presented frame is relative to the authoritative scene.
///
/// [`FrameCompleteness::CurrentScene`] means the *scene resources* are fully
/// committed and resident on the GPU — the current published scene revision and
/// its base revision have been uploaded. It is **not** a claim that every
/// progressive page of those pixels has been drawn yet: a progressive frame can
/// still be accumulating (and `current_drawing_presented` stays false until it
/// completes, via `presented_revision`).
///
/// [`FrameCompleteness::BaseOnlyPendingOverlays`] is the explicit, non-silent
/// marker for the narrow 2D navigation fallback: the base drawing is the latest
/// authoritative base and is still fully uploaded, but its overlay suffix
/// (grid/axes/highlight/snap/crosshair/measurement/draw preview) is deliberately
/// withheld until preparation catches up. It is never a claim that the current
/// scene is complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameCompleteness {
    CurrentScene,
    BaseOnlyPendingOverlays,
}

pub struct PresentedFrame {
    pub binding: FrameBinding,
    /// Monotonic revision of the view (camera/space) that was actually rendered
    /// into this frame. Independent of the texture binding: a camera-only frame
    /// reuses the same offscreen texture identity while advancing this, so a
    /// host can tell a new camera was presented without a new texture.
    pub view_revision: u64,
    /// Whether this frame is the full current scene or the base-only navigation
    /// fallback. Never reported as `CurrentScene` while overlays are withheld.
    pub completeness: FrameCompleteness,
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

pub struct CadRenderRuntime {
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
    /// True while the 2D navigation fallback has dropped the transient overlay
    /// suffix from the GPU and is drawing the base-only scene over the latest
    /// authoritative camera. Cleared by a successful full-scene publication.
    overlays_suppressed: bool,
    /// Cumulative overlay-suffix commits (suffix-only uploads and fallback
    /// truncations). A camera-only navigation must leave this flat.
    overlay_suffix_commits: u64,
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
            overlays_suppressed: false,
            overlay_suffix_commits: 0,
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
        self.overlays_suppressed = false;
        self.overlay_suffix_commits = 0;
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
        // Base drawing batches are the prefix; the transient highlight/preview
        // overlay is the replaceable suffix. A highlight-only change still
        // re-uploads the suffix, but never the base.
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
        while pending.group < 2 && slices < 4 {
            #[cfg(not(target_arch = "wasm32"))]
            if started.elapsed() >= std::time::Duration::from_millis(8) {
                break;
            }
            let batches = match pending.group {
                0 => &pending.scene.base.added,
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
        if pending.group < 2 {
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
        // A suffix-only commit re-uploads (or truncates) the transient overlay
        // batch range; a base change replaces the whole scene instead.
        if !pending.base_changed {
            self.overlay_suffix_commits += 1;
        }
        // Raster images are a resident set separate from the geometry batches.
        // A base change re-resolves every image quad from the cache the scene
        // was built with; a fresh device resets `applied_base`, so this also
        // re-uploads the textures after a backend rebuild/loss. An absent cache
        // or an empty batch list leaves no resident images and reports nothing:
        // a true no-op. Unresolved keys are surfaced, never silently dropped.
        let mut image_diagnostic = None;
        if pending.base_changed {
            let empty = super::controller::DecodedImageCache::new();
            let cache = ready.images.as_deref().unwrap_or(&empty);
            let report = renderer.upload_images(&ready.base.images, cache)?;
            if let Some(reason) = report.diagnostic() {
                let keys = report
                    .unresolved
                    .iter()
                    .map(|key| key.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                image_diagnostic = Some(format!("{}: {keys}", reason.code));
            }
        }
        // Publication follows successful GPU preparation/commit, never precedes it.
        self.base_batches = ready.base.added.len();
        self.applied_base = Some(ready.base_revision);
        self.applied_revision = Some(ready.revision);
        // A committed full scene restores the transient overlay suffix, so the
        // base-only fallback is no longer active.
        self.overlays_suppressed = false;
        self.dirty.invalidate();
        self.diagnostic = image_diagnostic;
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

    /// Whether the 2D navigation fallback is currently withholding overlays.
    pub fn overlays_suppressed(&self) -> bool {
        self.overlays_suppressed
    }

    /// Cumulative overlay-suffix commits (suffix-only uploads and fallback
    /// truncations). Camera-only navigation must leave this flat.
    pub fn overlay_suffix_commits(&self) -> u64 {
        self.overlay_suffix_commits
    }

    /// The GPU base revision the runtime considers resident. Test/diagnostic only.
    #[cfg(test)]
    pub(super) fn applied_base_for_test(&self) -> Option<u64> {
        self.applied_base
    }

    /// The base batch prefix preserved across the overlay suffix. Test only.
    #[cfg(test)]
    pub(super) fn base_batches_for_test(&self) -> usize {
        self.base_batches
    }

    /// Model a resident base without a GPU so the pure-state qualifier can be
    /// exercised honestly (the GPU path is covered by the ignored hardware test).
    #[cfg(test)]
    pub(super) fn force_resident_base_for_test(&mut self, base_revision: u64, base_batches: usize) {
        self.lifecycle = RenderLifecycle::Ready { device_epoch: 1 };
        self.applied_base = Some(base_revision);
        self.applied_revision = Some(base_revision);
        self.base_batches = base_batches;
        self.pending_upload = None;
    }

    /// Whether the GPU has committed exactly `scene_revision` and its base
    /// revision. A frame may only be labeled [`FrameCompleteness::CurrentScene`]
    /// when this holds: a CPU-ready scene whose chunked upload is still pending
    /// must never be presented under that label (it would show the resident old
    /// scene while claiming it is current).
    pub fn scene_committed(&self, scene_revision: u64, base_revision: u64) -> bool {
        self.applied_revision == Some(scene_revision) && self.applied_base == Some(base_revision)
    }

    /// The latest published base is fully resident on the GPU: the device is
    /// ready, the applied base revision matches `base_publication_revision`, and
    /// no pending upload is replacing the base. This is the narrow qualifier that
    /// lets a base-only frame stand in for the full scene during 2D navigation
    /// without ever re-uploading base geometry.
    pub fn base_fully_resident(&self, base_publication_revision: u64) -> bool {
        matches!(self.lifecycle, RenderLifecycle::Ready { .. })
            && self.applied_base == Some(base_publication_revision)
            && self
                .pending_upload
                .as_ref()
                .is_none_or(|pending| !pending.base_changed)
    }

    /// Drop the stale transient overlay suffix while keeping the base drawing
    /// buffers. Runs at most once per fallback entry; a later full-scene commit
    /// restores the suffix from the same base prefix (`base_batches`).
    ///
    /// `prepare_upload_batches(&[])` stages zero bytes, so this re-upload of the
    /// base is impossible by construction: only the replaceable suffix after
    /// `base_batches` is truncated.
    fn suppress_overlays(&mut self) -> Result<(), RenderError> {
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?;
        let empty = renderer
            .prepare_upload_batches(&[])
            .map_err(|error| RenderError::Frame(error.to_string()))?;
        renderer
            .commit_upload(empty, self.base_batches)
            .map_err(|error| RenderError::Frame(error.to_string()))?;
        self.overlays_suppressed = true;
        self.overlay_suffix_commits += 1;
        // The discarded suffix upload (if any) is overlay-only by the
        // `base_fully_resident` qualifier; the full scene re-commits it.
        self.pending_upload = None;
        // The presented frame is no longer the full current scene.
        self.applied_revision = None;
        self.presented_revision = None;
        self.dirty.invalidate();
        Ok(())
    }

    pub fn render_if_dirty(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
        view_revision: u64,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        let completeness = if self.overlays_suppressed {
            FrameCompleteness::BaseOnlyPendingOverlays
        } else {
            FrameCompleteness::CurrentScene
        };
        self.render_frame(view, logical, scale, view_revision, completeness)
    }

    /// Render the base-only navigation frame over the latest authoritative
    /// camera. The caller has already verified that the published base is fully
    /// resident and that only view/overlay inputs are pending.
    pub fn render_base_only(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
        view_revision: u64,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        if !matches!(self.lifecycle, RenderLifecycle::Ready { .. }) {
            return Ok(None);
        }
        if !self.overlays_suppressed {
            self.suppress_overlays()?;
        }
        self.render_frame(
            view,
            logical,
            scale,
            view_revision,
            FrameCompleteness::BaseOnlyPendingOverlays,
        )
    }

    fn render_frame(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
        view_revision: u64,
        completeness: FrameCompleteness,
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
                    // A changed camera makes `render` rebuild the progressive
                    // frame (transforms/camera differ), so no earlier page
                    // continues under the new camera.
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
            view_revision,
            completeness,
            texture: texture.clone(),
        }))
    }
}
