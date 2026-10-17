//! Derived GPU state. Never builds representations or owns a window.
use super::controller::PreparedScene;
use super::*;
use cad_app::render_scene::SceneDelta;
use cad_domain::SelectionRef;
use cad_render_wgpu::{wgpu, BackendCapabilities, RenderError, RenderTarget, Renderer};
use std::sync::Arc;

/// Observation of one **actual** renderer invocation (`Renderer::render`/
/// `render_3d`). Produced only when the engine really invoked the renderer this
/// frame; a retained/reused frame or a gate `Ok(None)` never produces one, so a
/// host can never replay stale stats. Delivered at most once per render.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderStatsObservation {
    /// Monotonic engine render sequence (one per actual renderer invocation).
    pub render_seq: u64,
    /// Device epoch this frame rendered on.
    pub device_epoch: u64,
    /// View revision tagged on this frame.
    pub view_revision: u64,
    /// Scene revision committed at render time.
    pub applied_revision: Option<u64>,
    /// Base publication revision committed at render time.
    pub applied_base: Option<u64>,
    /// Upload bookkeeping counters at render time.
    pub uploads_started: u64,
    pub upload_revision_resets: u64,
    pub ready_none_clears: u64,
    /// CPU milliseconds of the renderer's CPU path (excludes the blocking poll).
    pub cpu_ms: f64,
    /// GPU milliseconds if the backend reported them; `None` is honest (no
    /// invented GPU timestamp).
    pub gpu_ms: Option<f64>,
    pub plan_ms: f64,
    pub uniform_ms: f64,
    pub encode_ms: f64,
    pub submit_ms: f64,
    /// Blocking device-poll wait (excluded from `cpu_ms`).
    pub wait_ms: f64,
    pub draw_calls: u64,
    pub vertices: usize,
    pub triangles: usize,
    pub instanced_draws: u64,
    pub mesh_draws: u64,
    /// Opaque batches drawn as a legacy per-batch draw (the `legacy_opaque` term
    /// of `draw_calls`; diagnostics only).
    pub legacy_opaque_draws: u64,
    /// Coalesced SDF conic draws (the `arc_draws` term of `draw_calls`).
    pub arc_draws: u64,
    /// Transparent batches drawn (the `transparent_batches` term of `draw_calls`).
    pub transparent_batches: usize,
    /// Raster image quads drawn (the `image_batches` term of `draw_calls`).
    pub image_batches: usize,
    /// Whether this frame's opaque pass used the accelerated per-class draw
    /// organization (see `cad_render_wgpu::FrameStats::accelerated`). Required to
    /// interpret `instanced_draws`/`mesh_draws`: on the accelerated tier they are
    /// per-pulled-segment draws, on the universal tier per-coalesced-run draws.
    pub accelerated: bool,
    /// Observable state of the accelerated pulled organisation at this frame
    /// (`cad_render_wgpu::PulledState`): `Active`, or the explicit reason the
    /// organisation was skipped (insufficient VRAM with its byte numbers,
    /// captured OOM, an unsegmentable class, nothing converted, unavailable).
    /// A skip is never reported as a success.
    pub pulled_state: cad_render_wgpu::PulledState,
    /// Resident instance streams (one per chunk) backing `instanced_draws`.
    /// Comparing this with `instanced_draws` yields draws-per-stream, which is what
    /// decides whether a chunk-level skip can remove whole draws.
    pub instance_streams: usize,
    /// Batches that contributed coalesced instanced runs this frame.
    pub instanced_batches: usize,
    /// Stable lowercase render tier (`"universal"`/`"accelerated"`).
    pub tier: &'static str,
    /// Whether part of the scene was held back by the frame budget.
    pub over_budget: bool,
    /// Adaptive page-budget scale this frame's plan used (1.0 when pacing is
    /// disabled or the renderer is not paging).
    pub budget_scale: f64,
    /// Adaptive page-budget target (`cpu_ms + wait_ms`) in milliseconds.
    pub budget_target_ms: f64,
    /// Cost EWMA (ms) the scale was based on, before this frame's own sample.
    pub budget_cost_ema_ms: f64,
    /// Active page-budget vertex limit this frame's plan used.
    pub budget_vertices: usize,
    /// Active page-budget triangle limit this frame's plan used.
    pub budget_triangles: usize,
    /// Pacing regime: `"active"`, `"disabled-no-completion-signal"` (no measured
    /// completion signal; static budget) or `"at-ceiling"` (non-progressive).
    pub pacing: &'static str,
    /// Progressive cursor / total (existing `frame_progress`), if progressive.
    pub progressive_cursor: Option<usize>,
    pub progressive_total: Option<usize>,
    /// Whether the progressive frame completed this render.
    pub progressive_complete: bool,
}

/// Stage timings of one bounded upload work unit (diagnostics only).
///
/// Produced only when `apply_ready_updates` actually staged or committed an
/// upload; a no-op/flags-only call produces none. Delivered at most once.
#[derive(Clone, Debug, PartialEq)]
pub struct UploadTimingObservation {
    /// Monotonic upload-timing sequence.
    pub seq: u64,
    /// Scene revision this upload belongs to.
    pub scene_revision: u64,
    /// Whether this upload replaces the base (vs an overlay-suffix re-upload).
    pub base_changed: bool,
    /// Whether the upload committed (false = chunked staging only this call).
    pub committed: bool,
    /// 0 = base group, 1 = overlay group, 2 = committed.
    pub group: usize,
    /// Batches staged (staging) or staged+committed (commit).
    pub batches: usize,
    /// `prepare_upload_batches` wall time in this call.
    pub prepare_us: u64,
    /// `PreparedUpload::append` wall time in this call.
    pub append_us: u64,
    /// `commit_upload` wall time in this call (0 when not committed).
    pub commit_us: u64,
    /// Whole `apply_ready_updates` work-unit wall time.
    pub total_us: u64,
    /// Renderer-side `commit_upload` substage timings, when it committed.
    pub commit: Option<cad_render_wgpu::CommitTiming>,
}

struct PendingUpload {
    scene: PreparedScene,
    group: usize,
    offset: usize,
    prepared: Option<cad_render_wgpu::PreparedUpload>,
    base_changed: bool,
}

/// Read-only snapshot of an in-flight chunked upload (diagnostics only).
///
/// `offset` is the number of batches already staged in the current group;
/// `group` 0 is the base prefix and group 1 the overlay suffix. A host samples
/// this over time to tell a monotonic (progressing) upload from a reset/stalled
/// one, without touching the renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadDiagnostics {
    /// Scene revision being uploaded.
    pub scene_revision: u64,
    /// 0 = base prefix, 1 = overlay suffix.
    pub group: usize,
    /// Batches staged so far in the current group.
    pub offset: usize,
    /// Whether this upload replaces the base (vs an overlay-suffix re-upload).
    pub base_changed: bool,
}

/// Read-only runtime bookkeeping for the waiting/upload diagnosis.
///
/// Pure projected state: reading it never advances an upload, a revision or the
/// view, and never forces engine work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeDiagnostics {
    /// `"detached"` / `"ready"` / `"lost"` / `"failed"`.
    pub lifecycle: &'static str,
    /// Scene revision the GPU last committed (`applied_revision`).
    pub applied_revision: Option<u64>,
    /// Base publication revision the GPU last committed.
    pub applied_base: Option<u64>,
    /// Resident base batch prefix length.
    pub resident_base_batches: usize,
    /// Whether a chunked upload is currently in flight.
    pub uploading: bool,
    /// The in-flight upload, if any.
    pub upload: Option<UploadDiagnostics>,
    /// Whether the 2D navigation fallback is withholding overlays.
    pub overlays_suppressed: bool,
    /// Cumulative overlay-suffix commits.
    pub overlay_suffix_commits: u64,
    /// Cumulative pulled-prefix **reuses** (the accelerator retained the base
    /// pulled segments for a suffix commit), `0` when no renderer is attached.
    pub pulled_prefix_reuses: u64,
    /// Cumulative pulled-prefix **rebuilds** (a full pulled rebuild that
    /// (re)established a checkpoint), `0` when no renderer is attached.
    pub pulled_prefix_rebuilds: u64,
    /// Cumulative frames the renderer actually produced (`frames_rendered`).
    pub frames_rendered: u64,
    /// Uploads started from scratch (no prior pending upload).
    pub uploads_started: u64,
    /// Pending uploads **reset** because the ready scene revision changed.
    pub upload_revision_resets: u64,
    /// Pending uploads **cleared** because `apply_ready_updates` saw no ready scene.
    pub ready_none_clears: u64,
    /// Most recent reset reason: `"revision_replace"` / `"ready_none"` / `None`.
    pub last_upload_reset: Option<&'static str>,
    /// Last real failure/diagnostic string (never a converted success).
    pub diagnostic: Option<String>,
}

/// What `apply_ready_updates` should do with the pending upload for a ready scene.
///
/// Pure so the reset/clear counters are contract-tested without a GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UploadDecision {
    /// No ready scene: clear any pending upload (`ready_none`).
    ClearPending,
    /// Start a fresh upload.
    Start,
    /// The pending upload is for a different scene revision: reset it.
    ResetRevision,
    /// The pending upload continues for the same revision.
    Continue,
}

fn upload_decision(
    ready_present: bool,
    pending_revision: Option<u64>,
    ready_revision: u64,
) -> UploadDecision {
    if !ready_present {
        return UploadDecision::ClearPending;
    }
    match pending_revision {
        None => UploadDecision::Start,
        Some(revision) if revision == ready_revision => UploadDecision::Continue,
        Some(_) => UploadDecision::ResetRevision,
    }
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
/// a frame clean. UI-only frames reuse the previously drawn texture.
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
    /// The last committed overlay delta, retained by strong reference.
    ///
    /// An entity-only selection change republishes the scene with the **same**
    /// overlay `Arc` (the controller reuses it), so `Arc::ptr_eq` with the
    /// candidate means there is no geometry to re-upload: only the entity flags
    /// changed. Retaining the `Arc` (rather than its raw address as `usize`) is
    /// deliberate: a freed-and-reallocated `SceneDelta` could otherwise land on
    /// the same address between two applies (rapid preview/snap updates with
    /// mailbox coalescing), and the fast path would silently skip the new overlay
    /// upload and mark the stale scene current. The strong reference also keeps
    /// the address alive, so `Arc::ptr_eq` cannot observe an ABA reuse.
    applied_highlight: Option<Arc<SceneDelta>>,
    /// Diagnostic from the last base-change image upload (`None` when every image
    /// resolved). Kept separate from the flags diagnostic so a flags-only update
    /// does not wipe a still-relevant image diagnostic.
    image_diagnostic: Option<String>,
    /// Diagnostic from the last entity-flag apply: the requested refs that matched
    /// no resident whole-entity pick key. `None` when every requested ref resolved.
    flags_diagnostic: Option<String>,
    /// Upload bookkeeping for the waiting/upload diagnosis (counters only; they
    /// never influence behaviour).
    uploads_started: u64,
    upload_revision_resets: u64,
    ready_none_clears: u64,
    last_upload_reset: Option<&'static str>,
    /// Monotonic actual-renderer-invocation sequence.
    render_seq: u64,
    /// The most recent actual renderer observation, if any.
    pending_render_observation: Option<RenderStatsObservation>,
    /// Render sequence already handed out by `take_render_observation`.
    last_taken_render_seq: Option<u64>,
    /// Monotonic upload-timing sequence.
    upload_timing_seq: u64,
    /// The most recent upload stage-timing observation, if any.
    pending_upload_timing: Option<UploadTimingObservation>,
    /// Upload-timing sequence already handed out.
    last_taken_upload_timing_seq: Option<u64>,
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
            applied_highlight: None,
            image_diagnostic: None,
            flags_diagnostic: None,
            uploads_started: 0,
            upload_revision_resets: 0,
            ready_none_clears: 0,
            last_upload_reset: None,
            render_seq: 0,
            pending_render_observation: None,
            last_taken_render_seq: None,
            upload_timing_seq: 0,
            pending_upload_timing: None,
            last_taken_upload_timing_seq: None,
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
                // The production session uses the in-stream flags path for
                // entity-level selection/hover/hidden: the app resolves those
                // refs into `(SelectionRef, bits)` (see `PreparedScene::entity_flags`)
                // and `apply_ready_updates` pushes them here. Enabling the toggle
                // is a 16-byte control-uniform write; with no entity flags set it
                // is pixel-identical to the pre-flags renderer.
                renderer.set_selection_flags_enabled(true);
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
        self.applied_highlight = None;
        self.image_diagnostic = None;
        self.flags_diagnostic = None;
        self.uploads_started = 0;
        self.upload_revision_resets = 0;
        self.ready_none_clears = 0;
        self.last_upload_reset = None;
        // A device change invalidates any un-taken observation: it must never be
        // replayed against the new device epoch.
        self.pending_render_observation = None;
        self.last_taken_render_seq = None;
        self.pending_upload_timing = None;
        self.last_taken_upload_timing_seq = None;
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
            if self.pending_upload.take().is_some() {
                self.ready_none_clears += 1;
                self.last_upload_reset = Some("ready_none");
            }
            return Ok(());
        };
        if self.applied_revision == Some(ready.revision) {
            return Ok(());
        }
        let base_changed = self.applied_base != Some(ready.base_revision);
        // An entity-only selection/hover/hidden change republishes the scene with
        // the **same** overlay `Arc` (the controller reuses it) and no base
        // change, so there is no geometry to upload — only flags to push. This is
        // the "selection change uploads only flags data" path: it never re-uploads
        // a batch, stream or chunk.
        let overlay_reused =
            overlay_identity_matches(self.applied_highlight.as_ref(), &ready.highlight);
        if !base_changed
            && overlay_reused
            && self.pending_upload.is_none()
            && !self.overlays_suppressed
        {
            let report = self
                .renderer
                .as_mut()
                .expect("ready renderer")
                .set_entity_flags(&ready.entity_flags)?;
            self.flags_diagnostic = flags_diagnostic(&report.unmatched);
            self.applied_revision = Some(ready.revision);
            self.applied_highlight = Some(ready.highlight.clone());
            // A flags-only update must not wipe a still-relevant image diagnostic:
            // merge rather than clear.
            self.diagnostic = merge_diagnostics(
                self.image_diagnostic.as_deref(),
                self.flags_diagnostic.as_deref(),
            );
            self.dirty.invalidate();
            return Ok(());
        }
        let renderer = self.renderer.as_mut().expect("ready renderer");
        // Base drawing batches are the prefix; the transient highlight/preview
        // overlay is the replaceable suffix. A highlight-only change still
        // re-uploads the suffix, but never the base.
        if self
            .pending_upload
            .as_ref()
            .is_none_or(|pending| pending.scene.revision != ready.revision)
        {
            match upload_decision(
                true,
                self.pending_upload
                    .as_ref()
                    .map(|pending| pending.scene.revision),
                ready.revision,
            ) {
                UploadDecision::ResetRevision => {
                    self.upload_revision_resets += 1;
                    self.last_upload_reset = Some("revision_replace");
                }
                UploadDecision::Start => self.uploads_started += 1,
                // Unreachable by the guard above; keep the compiler honest.
                UploadDecision::Continue | UploadDecision::ClearPending => {}
            }
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
        let mut prepare_us = 0u64;
        let mut append_us = 0u64;
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
            let prepare_started = std::time::Instant::now();
            let piece = renderer.prepare_upload_batches(&batches[pending.offset..end])?;
            prepare_us += prepare_started.elapsed().as_micros() as u64;
            let append_started = std::time::Instant::now();
            if let Some(prepared) = &mut pending.prepared {
                prepared.append(piece)?;
            } else {
                pending.prepared = Some(piece);
            }
            append_us += append_started.elapsed().as_micros() as u64;
            pending.offset = end;
            slices += 1;
        }
        if self
            .pending_upload
            .as_ref()
            .is_some_and(|pending| pending.group < 2)
        {
            // Chunked staging only (no commit yet): record the staged progress.
            // Copy the facts out first so the `pending_upload` borrow ends before
            // the whole-`self` timing record.
            let (group, offset, scene_revision, base_changed) = {
                let pending = self.pending_upload.as_ref().expect("checked above");
                (
                    pending.group,
                    pending.offset,
                    pending.scene.revision,
                    pending.base_changed,
                )
            };
            // `started` only exists off wasm (no monotonic clock there); report 0.
            #[cfg(not(target_arch = "wasm32"))]
            let total_us = started.elapsed().as_micros() as u64;
            #[cfg(target_arch = "wasm32")]
            let total_us = 0u64;
            self.record_upload_timing(UploadTimingObservation {
                seq: 0,
                scene_revision,
                base_changed,
                committed: false,
                group,
                batches: offset,
                prepare_us,
                append_us,
                commit_us: 0,
                total_us,
                commit: None,
            });
            return Ok(());
        }
        let pending = self.pending_upload.take().unwrap();
        let prepared = match pending.prepared {
            Some(prepared) => prepared,
            None => renderer.prepare_upload_batches(&[])?,
        };
        let commit_started = std::time::Instant::now();
        renderer.commit_upload(
            prepared,
            if pending.base_changed {
                0
            } else {
                self.base_batches
            },
        )?;
        let commit_us = commit_started.elapsed().as_micros() as u64;
        let commit_timing = renderer.last_commit_timing();
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
        // Push the entity-level flags for the freshly committed scene, **after**
        // any base/image re-upload. A base change (re)allocates pick keys —
        // including the image keys, which `upload_images` assigns fresh per call —
        // so flags pushed before the re-upload would stay on retired keys and the
        // flagged image/geometry would flash back to unflagged after an edit. The
        // renderer also re-applies its retained request on the rebuild; this call
        // surfaces the unmatched refs so the runtime can report them.
        let flag_report = renderer.set_entity_flags(&ready.entity_flags)?;
        // Publication follows successful GPU preparation/commit, never precedes it.
        self.base_batches = ready.base.added.len();
        self.applied_base = Some(ready.base_revision);
        self.applied_revision = Some(ready.revision);
        self.applied_highlight = Some(ready.highlight.clone());
        // A committed full scene restores the transient overlay suffix, so the
        // base-only fallback is no longer active.
        self.overlays_suppressed = false;
        self.dirty.invalidate();
        self.image_diagnostic = image_diagnostic;
        self.flags_diagnostic = flags_diagnostic(&flag_report.unmatched);
        self.diagnostic = merge_diagnostics(
            self.image_diagnostic.as_deref(),
            self.flags_diagnostic.as_deref(),
        );
        // `started` only exists off wasm (no monotonic clock there); report 0.
        #[cfg(not(target_arch = "wasm32"))]
        let total_us = started.elapsed().as_micros() as u64;
        #[cfg(target_arch = "wasm32")]
        let total_us = 0u64;
        // Record the committed upload's stage timings **after** the last `renderer`
        // borrow (diagnostics only; taken at most once so a host never replays them).
        self.record_upload_timing(UploadTimingObservation {
            seq: 0,
            scene_revision: pending.scene.revision,
            base_changed: pending.base_changed,
            committed: true,
            group: 2,
            batches: pending.scene.base.added.len() + pending.scene.highlight.added.len(),
            prepare_us,
            append_us,
            commit_us,
            total_us,
            commit: commit_timing,
        });
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

    /// The retained applied-overlay `Arc`, so a hardware test can assert it is the
    /// same allocation as the published scene's overlay (the ABA guard). Test only.
    #[cfg(test)]
    pub(super) fn applied_highlight_for_test(&self) -> Option<Arc<SceneDelta>> {
        self.applied_highlight.clone()
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

    /// The in-flight upload's `base_changed` kind, if any (`diagnostics` only).
    pub(super) fn pending_upload_kind(&self) -> Option<bool> {
        self.pending_upload
            .as_ref()
            .map(|pending| pending.base_changed)
    }

    /// The current device epoch (incremented on every successful attach).
    pub(super) fn device_epoch(&self) -> u64 {
        self.epoch
    }

    /// Record the observation of one actual renderer invocation.
    fn record_render_observation(
        &mut self,
        stats: cad_render_wgpu::FrameStats,
        view_revision: u64,
        progressive: Option<(usize, usize)>,
        progressive_complete: bool,
    ) {
        self.render_seq += 1;
        let (cursor, total) = match progressive {
            Some((cursor, total)) => (Some(cursor), Some(total)),
            None => (None, None),
        };
        self.pending_render_observation = Some(RenderStatsObservation {
            render_seq: self.render_seq,
            device_epoch: self.epoch,
            view_revision,
            applied_revision: self.applied_revision,
            applied_base: self.applied_base,
            uploads_started: self.uploads_started,
            upload_revision_resets: self.upload_revision_resets,
            ready_none_clears: self.ready_none_clears,
            cpu_ms: stats.cpu_ms,
            gpu_ms: stats.gpu_ms,
            plan_ms: stats.plan_ms,
            uniform_ms: stats.uniform_ms,
            encode_ms: stats.encode_ms,
            submit_ms: stats.submit_ms,
            wait_ms: stats.wait_ms,
            draw_calls: stats.draw_calls,
            vertices: stats.vertices,
            triangles: stats.triangles,
            instanced_draws: stats.instanced_draws,
            mesh_draws: stats.mesh_draws,
            legacy_opaque_draws: stats.legacy_opaque_draws,
            arc_draws: stats.arc_draws,
            transparent_batches: stats.transparent_batches,
            image_batches: stats.image_batches,
            accelerated: stats.accelerated,
            pulled_state: stats.pulled_state,
            instance_streams: stats.instance_streams,
            instanced_batches: stats.instanced_batches,
            tier: stats.tier.as_str(),
            over_budget: stats.over_budget.is_some(),
            budget_scale: stats.budget_scale,
            budget_target_ms: stats.budget_target_ms,
            budget_cost_ema_ms: stats.budget_cost_ema_ms,
            budget_vertices: stats.budget_vertices,
            budget_triangles: stats.budget_triangles,
            pacing: stats.pacing,
            progressive_cursor: cursor,
            progressive_total: total,
            progressive_complete,
        });
    }

    /// Take the latest actual-render observation, at most once per render
    /// sequence (so a retained/waiting frame can never replay stale stats).
    pub fn take_render_observation(&mut self) -> Option<RenderStatsObservation> {
        let observation = self.pending_render_observation.as_ref()?;
        if self.last_taken_render_seq == Some(observation.render_seq) {
            return None;
        }
        let taken = self.pending_render_observation.take();
        self.last_taken_render_seq = taken.as_ref().map(|observation| observation.render_seq);
        taken
    }

    /// Record one upload work-unit's stage timings (assigning its sequence).
    fn record_upload_timing(&mut self, mut observation: UploadTimingObservation) {
        self.upload_timing_seq += 1;
        observation.seq = self.upload_timing_seq;
        self.pending_upload_timing = Some(observation);
    }

    /// Take the latest upload stage-timing observation, at most once per upload
    /// work unit (so a host never replays stale timings).
    pub fn take_upload_timing_observation(&mut self) -> Option<UploadTimingObservation> {
        let observation = self.pending_upload_timing.as_ref()?;
        if self.last_taken_upload_timing_seq == Some(observation.seq) {
            return None;
        }
        let taken = self.pending_upload_timing.take();
        self.last_taken_upload_timing_seq = taken.as_ref().map(|observation| observation.seq);
        taken
    }

    /// Read-only runtime bookkeeping for the waiting/upload diagnosis.
    ///
    /// Pure projection: it borrows `&self` and never advances an upload, a
    /// revision or the view, and never forces engine work.
    pub fn diagnostics(&self) -> RuntimeDiagnostics {
        RuntimeDiagnostics {
            lifecycle: match &self.lifecycle {
                RenderLifecycle::Detached => "detached",
                RenderLifecycle::Ready { .. } => "ready",
                RenderLifecycle::Lost { .. } => "lost",
                RenderLifecycle::Failed { .. } => "failed",
            },
            applied_revision: self.applied_revision,
            applied_base: self.applied_base,
            resident_base_batches: self.base_batches,
            uploading: self.pending_upload.is_some(),
            upload: self
                .pending_upload
                .as_ref()
                .map(|pending| UploadDiagnostics {
                    scene_revision: pending.scene.revision,
                    group: pending.group,
                    offset: pending.offset,
                    base_changed: pending.base_changed,
                }),
            overlays_suppressed: self.overlays_suppressed,
            overlay_suffix_commits: self.overlay_suffix_commits,
            pulled_prefix_reuses: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.pulled_prefix_reuses()),
            pulled_prefix_rebuilds: self
                .renderer
                .as_ref()
                .map_or(0, |renderer| renderer.pulled_prefix_rebuilds()),
            frames_rendered: self.frames_rendered,
            uploads_started: self.uploads_started,
            upload_revision_resets: self.upload_revision_resets,
            ready_none_clears: self.ready_none_clears,
            last_upload_reset: self.last_upload_reset,
            diagnostic: self.diagnostic.clone(),
        }
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
            let stats = match view.mode {
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
                    renderer.render(camera, &target)?
                }
                ProjectionKind::ThreeD => {
                    let params = view
                        .camera
                        .camera3d_params()
                        .map_err(|e| RenderError::Frame(e.to_string()))?;
                    renderer.render_3d(camera3d_from_params(params), &target)?
                }
            };
            self.target_size = Some(size);
            let progressive_complete = !renderer.frame_pending();
            let progressive = renderer.frame_progress();
            if progressive_complete {
                self.dirty.complete();
            }
            self.frames_rendered += 1;
            self.presented_revision = if progressive_complete {
                self.applied_revision
            } else {
                None
            };
            // Capture the attachment identity/texture, ending the `renderer` field
            // borrow, then record the **actual** renderer invocation (the stats
            // that would otherwise be dropped). Retained/reused frames never reach
            // here.
            let texture_revision = renderer.texture_revision();
            let texture = renderer.frame_texture().cloned();
            self.record_render_observation(stats, view_revision, progressive, progressive_complete);
            return Ok(texture.map(|texture| PresentedFrame {
                binding: FrameBinding {
                    device_epoch: self.epoch,
                    texture_revision,
                    size,
                },
                view_revision,
                completeness,
                texture,
            }));
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

/// Whether the applied overlay `Arc` is the **same allocation** as `candidate`.
///
/// Pointer equality on a retained `Arc` — never a raw address — so a freed and
/// reallocated `SceneDelta` cannot alias the previous overlay (the ABA hazard of
/// the earlier `usize` identity).
fn overlay_identity_matches(
    applied: Option<&Arc<SceneDelta>>,
    candidate: &Arc<SceneDelta>,
) -> bool {
    applied.is_some_and(|previous| Arc::ptr_eq(previous, candidate))
}

/// The `flags.unresolved` diagnostic for the refs an entity-flag apply could not
/// place on a resident key, or `None` when every ref resolved.
fn flags_diagnostic(unmatched: &[SelectionRef]) -> Option<String> {
    if unmatched.is_empty() {
        return None;
    }
    let refs = unmatched
        .iter()
        .map(|reference| reference.entity.0.to_string())
        .collect::<Vec<_>>()
        .join(",");
    Some(format!("flags.unresolved: {refs}"))
}

/// Combine the image and flags diagnostics so a flags-only update preserves a
/// still-relevant image diagnostic instead of wiping it.
fn merge_diagnostics(image: Option<&str>, flags: Option<&str>) -> Option<String> {
    match (image, flags) {
        (Some(image), Some(flags)) => Some(format!("{image}; {flags}")),
        (Some(image), None) => Some(image.to_string()),
        (None, Some(flags)) => Some(flags.to_string()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{DocumentId, EntityId, InstancePath, TaskStamp};

    fn delta(revision: u64) -> SceneDelta {
        SceneDelta {
            stamp: TaskStamp::new(DocumentId(1), revision),
            added: Vec::new(),
            removed_chunks: Vec::new(),
            images: Vec::new(),
        }
    }

    fn reference(entity: u128) -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(entity),
            instance: InstancePath::default(),
            sub_element: None,
        }
    }

    #[test]
    fn overlay_reuse_is_pointer_identity_on_a_retained_arc() {
        // The applied overlay is retained by strong reference, so its allocation
        // stays alive and a later rebuild cannot land on the same address. This
        // is what makes `Arc::ptr_eq` sound where the raw `usize` identity was
        // ABA-prone (a freed overlay reallocated at the same address between two
        // applies silently skipped the new overlay upload).
        let applied = Arc::new(delta(1));
        let runtime = CadRenderRuntime {
            applied_highlight: Some(applied.clone()),
            ..CadRenderRuntime::default()
        };
        assert!(
            overlay_identity_matches(runtime.applied_highlight.as_ref(), &applied),
            "the same allocation is reused"
        );
        assert_eq!(
            Arc::strong_count(&applied),
            2,
            "the runtime retains the overlay Arc, so its address cannot be reused"
        );
        drop(applied);
        // A rebuild is a distinct allocation: the fast path must not match it.
        let rebuilt = Arc::new(delta(2));
        assert!(
            !overlay_identity_matches(runtime.applied_highlight.as_ref(), &rebuilt),
            "a distinct overlay must not take the flags-only fast path"
        );
    }

    #[test]
    fn flags_diagnostic_names_unmatched_refs_and_is_none_when_all_resolve() {
        assert_eq!(flags_diagnostic(&[]), None);
        let diagnostic = flags_diagnostic(&[reference(7), reference(9)]).expect("unmatched");
        assert!(diagnostic.starts_with("flags.unresolved: "));
        assert!(diagnostic.contains('7') && diagnostic.contains('9'));
    }

    #[test]
    fn merge_diagnostics_preserves_the_image_part_across_a_flags_update() {
        assert_eq!(merge_diagnostics(None, None), None);
        assert_eq!(
            merge_diagnostics(Some("image: a"), None).as_deref(),
            Some("image: a")
        );
        assert_eq!(
            merge_diagnostics(None, Some("flags.unresolved: 3")).as_deref(),
            Some("flags.unresolved: 3")
        );
        assert_eq!(
            merge_diagnostics(Some("image: a"), Some("flags.unresolved: 3")).as_deref(),
            Some("image: a; flags.unresolved: 3")
        );
    }

    #[test]
    fn applied_highlight_strong_count_is_observable_for_the_hardware_test() {
        // The hardware session test asserts the runtime retains the applied
        // overlay (the ABA guard); this keeps the helper exercised without a GPU.
        let runtime = CadRenderRuntime::default();
        assert!(runtime.applied_highlight_for_test().is_none());
        let applied = Arc::new(delta(1));
        let runtime = CadRenderRuntime {
            applied_highlight: Some(applied.clone()),
            ..CadRenderRuntime::default()
        };
        let retained = runtime.applied_highlight_for_test().expect("retained");
        assert!(Arc::ptr_eq(&retained, &applied));
        drop(retained);
        assert_eq!(
            Arc::strong_count(&applied),
            2,
            "the runtime retains the overlay Arc"
        );
    }

    #[test]
    fn upload_decision_distinguishes_clear_start_continue_and_reset() {
        // No ready scene clears any pending upload.
        assert_eq!(
            upload_decision(false, Some(1), 1),
            UploadDecision::ClearPending
        );
        assert_eq!(
            upload_decision(false, None, 1),
            UploadDecision::ClearPending
        );
        // A ready scene with nothing pending starts a fresh upload.
        assert_eq!(upload_decision(true, None, 7), UploadDecision::Start);
        // Same revision continues (no reset, offset keeps advancing).
        assert_eq!(upload_decision(true, Some(7), 7), UploadDecision::Continue);
        // A different revision resets (the reset/starvation signal).
        assert_eq!(
            upload_decision(true, Some(7), 8),
            UploadDecision::ResetRevision
        );
    }

    #[test]
    fn diagnostics_snapshot_reports_the_default_detached_state_without_side_effects() {
        let runtime = CadRenderRuntime::default();
        let diagnostics = runtime.diagnostics();
        assert_eq!(diagnostics.lifecycle, "detached");
        assert_eq!(diagnostics.applied_revision, None);
        assert_eq!(diagnostics.applied_base, None);
        assert_eq!(diagnostics.resident_base_batches, 0);
        assert!(!diagnostics.uploading);
        assert_eq!(diagnostics.upload, None);
        assert_eq!(diagnostics.frames_rendered, 0);
        assert_eq!(diagnostics.uploads_started, 0);
        assert_eq!(diagnostics.upload_revision_resets, 0);
        assert_eq!(diagnostics.ready_none_clears, 0);
        assert_eq!(diagnostics.last_upload_reset, None);
        assert_eq!(diagnostics.diagnostic, None);
        // Reading twice is stable (pure projection).
        assert_eq!(runtime.diagnostics(), diagnostics);
    }

    fn sample_observation(render_seq: u64) -> RenderStatsObservation {
        RenderStatsObservation {
            render_seq,
            device_epoch: 1,
            view_revision: 2,
            applied_revision: Some(1),
            applied_base: Some(1),
            uploads_started: 0,
            upload_revision_resets: 0,
            ready_none_clears: 0,
            cpu_ms: 0.0,
            gpu_ms: None,
            plan_ms: 0.0,
            uniform_ms: 0.0,
            encode_ms: 0.0,
            submit_ms: 0.0,
            wait_ms: 0.0,
            draw_calls: 0,
            vertices: 0,
            triangles: 0,
            instanced_draws: 0,
            mesh_draws: 0,
            legacy_opaque_draws: 0,
            arc_draws: 0,
            transparent_batches: 0,
            image_batches: 0,
            accelerated: false,
            pulled_state: cad_render_wgpu::PulledState::Unavailable,
            instance_streams: 0,
            instanced_batches: 0,
            tier: "universal",
            over_budget: false,
            budget_scale: 1.0,
            budget_target_ms: 13.0,
            budget_cost_ema_ms: 0.0,
            budget_vertices: 8_000_000,
            budget_triangles: 2_000_000,
            pacing: "active",
            progressive_cursor: None,
            progressive_total: None,
            progressive_complete: true,
        }
    }

    #[test]
    fn render_observation_is_delivered_once_and_never_replayed() {
        // A retained/waiting frame (no actual render) has nothing to take.
        let mut runtime = CadRenderRuntime::default();
        assert_eq!(runtime.take_render_observation(), None);
        let mut runtime = CadRenderRuntime {
            pending_render_observation: Some(sample_observation(1)),
            ..CadRenderRuntime::default()
        };
        let first = runtime
            .take_render_observation()
            .expect("first actual render");
        assert_eq!(first.render_seq, 1);
        assert_eq!(first.gpu_ms, None, "gpu_ms stays honestly absent");
        // The SAME observation is never delivered twice (no replay on a retained
        // or waiting frame).
        assert_eq!(runtime.take_render_observation(), None);
        runtime.pending_render_observation = Some(sample_observation(2));
        assert_eq!(
            runtime.take_render_observation().map(|o| o.render_seq),
            Some(2)
        );
        assert_eq!(runtime.take_render_observation(), None);
    }

    fn upload_timing(scene_revision: u64, committed: bool) -> UploadTimingObservation {
        UploadTimingObservation {
            seq: 0,
            scene_revision,
            base_changed: true,
            committed,
            group: if committed { 2 } else { 0 },
            batches: 512,
            prepare_us: 1200,
            append_us: 300,
            commit_us: if committed { 4200 } else { 0 },
            total_us: if committed { 5700 } else { 1600 },
            commit: None,
        }
    }

    #[test]
    fn upload_timing_observation_is_delivered_once_and_never_replayed() {
        let mut runtime = CadRenderRuntime::default();
        // No upload work -> nothing to take.
        assert_eq!(runtime.take_upload_timing_observation(), None);
        let mut runtime = CadRenderRuntime {
            pending_upload_timing: Some(upload_timing(3, false)),
            ..CadRenderRuntime::default()
        };
        // `record_upload_timing` assigns the sequence.
        let staged = runtime.pending_upload_timing.take().unwrap();
        runtime.record_upload_timing(staged);
        let observation = runtime.take_upload_timing_observation().expect("first");
        assert_eq!(observation.seq, 1);
        assert_eq!(observation.scene_revision, 3);
        assert!(!observation.committed);
        assert_eq!((observation.prepare_us, observation.append_us), (1200, 300));
        // The same observation is never delivered twice (no replay).
        assert_eq!(runtime.take_upload_timing_observation(), None);
    }

    #[test]
    fn detach_invalidates_an_untaken_upload_timing_observation() {
        let mut runtime = CadRenderRuntime {
            pending_upload_timing: Some(upload_timing(1, true)),
            ..CadRenderRuntime::default()
        };
        let staged = runtime.pending_upload_timing.take().unwrap();
        runtime.record_upload_timing(staged);
        assert!(runtime.take_upload_timing_observation().is_some());
        // Inject a fresh, un-taken observation, then drop the device: it must not
        // be replayed afterwards.
        let mut observation = upload_timing(2, true);
        runtime.record_upload_timing(observation.clone());
        observation = runtime.pending_upload_timing.clone().unwrap();
        assert_eq!(observation.scene_revision, 2);
        runtime.detach();
        assert_eq!(runtime.take_upload_timing_observation(), None);
    }

    #[test]
    fn detach_invalidates_an_untaken_render_observation() {
        let mut runtime = CadRenderRuntime {
            pending_render_observation: Some(sample_observation(5)),
            ..CadRenderRuntime::default()
        };
        // A device change must not let an old-epoch observation be replayed.
        runtime.detach();
        assert_eq!(runtime.take_render_observation(), None);
        assert_eq!(runtime.device_epoch(), 0);
        // The render sequence stays monotonic for the session.
        assert_eq!(runtime.render_seq, 0);
    }
}
