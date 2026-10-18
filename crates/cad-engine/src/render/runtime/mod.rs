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
    /// invented GPU timestamp). Present only under the measurement-only opt-in
    /// (ADR 0013 class 1); the product configuration always reports `None`.
    pub gpu_ms: Option<f64>,
    /// The measurement-state token for this frame
    /// (`enabled`/`disabled`/`unsupported(<FEATURE>)`), passed through from the
    /// renderer so the host can label the numbers. Report-only.
    pub gpu_time: cad_render_wgpu::gpu_time::GpuTimeState,
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
    /// Chunks (instance streams + merged mesh chunks + SDF conic streams) tested
    /// by this frame's chunk-level view cull. `0` when the scene has no chunked
    /// geometry. Reporting only; `kept + culled == total`.
    pub cull_chunks_total: u64,
    /// Chunks kept (visible) by this frame's view cull.
    pub cull_chunks_kept: u64,
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
    /// The entity-flag revision the renderer last applied. A flags-only publish
    /// advances this without touching `applied_revision`/`applied_base`: the
    /// committed geometry is unchanged and only the in-stream flags delta is
    /// pushed.
    applied_flags_revision: Option<u64>,
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
    /// The entity-flag set the runtime last pushed to the renderer, retained so
    /// the 2D navigation fallback can **re-assert** it after its truncating
    /// commit (see [`Self::suppress_overlays`]). This is a re-assertion mirror of
    /// the last applied `PreparedScene::entity_flags`, never a second truth: the
    /// controller remains the sole producer of the flags.
    applied_entity_flags: Vec<(SelectionRef, u32)>,
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
            applied_flags_revision: None,
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
            applied_entity_flags: Vec::new(),
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

/// Whether a ready scene is a **flags-only** delta over what the GPU already has.
///
/// True only when the committed scene revision is still current (or the 2D
/// fallback holds the base with the suffix suppressed) and no upload is in
/// flight. A `ready.revision` that is not yet applied takes the upload path: its
/// geometry has to be committed first, and it always re-pushes the flags.
fn flags_delta_applies(
    applied_revision: Option<u64>,
    applied_flags_revision: Option<u64>,
    pending_upload: bool,
    overlays_suppressed: bool,
    ready: &PreparedScene,
) -> bool {
    applied_flags_revision != Some(ready.flags_revision)
        && !pending_upload
        && (applied_revision == Some(ready.revision) || overlays_suppressed)
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

mod session;
#[cfg(test)]
mod tests;
