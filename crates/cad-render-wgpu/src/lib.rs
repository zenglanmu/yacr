//! wgpu pipelines over CPU scene batches.
//!
//! Spec v2.0 §5.2, §8: the renderer does not own the window or the device. The
//! host provides a shared `Device`/`Queue`; CAD draws into a texture the host
//! composites. Camera navigation updates only small uniform buffers, never
//! the static vertex buffers.
//!
//! Two pipelines are provided: a `LineList` pipeline for polylines and a
//! `TriangleList` pipeline for mesh geometry with a depth buffer, back-face
//! culling and per-fragment shading (audit F14). The pure geometry decisions
//! live in [`geometry`] and are unit tested without a GPU.

pub use wgpu;

pub mod geometry;

/// Time-driven adaptive page-budget controller (pure; no wgpu types). See
/// [`pacing`] for the adaptation policy and the disabled-signal boundary.
pub mod pacing;

/// Native headless path: create a software device, render offscreen, read the
/// target back and encode PNG. Native only; the browser path gets its device
/// from the host canvas instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod headless;

/// Measurement-only GPU frame timing (ADR 0013 class 1): the explicit opt-in
/// config, the pure feature-negotiation policy and the tick→ms conversion.
/// Never a runtime dependency; never a product default.
pub mod gpu_time;

/// Native-only surface probe presenter (forced Vulkan, real GPU) for host
/// bring-up. Deliberately **not** the production CAD presenter: it draws only a
/// grid, a moving colour block and corner markers, performs no readback, and
/// owns no event loop. See [`native_presenter`].
#[cfg(not(target_arch = "wasm32"))]
pub mod native_presenter;

#[cfg(not(target_arch = "wasm32"))]
pub use native_presenter::{
    NativeTextureKey, ProbeAdapterInfo, ProbeError, ProbeFrameStats, ProbePresentMode,
    ProbeStageTimings, SurfaceProbePresenter,
};

/// Native id-pass shader sources (see [`pick_at`](Renderer::pick_at)).
#[cfg(not(target_arch = "wasm32"))]
pub use pick::{pick_id_shader_sources, pick_shader_source};

/// Alias for hosts that think of this as a native surface presenter. The
/// canonical name stays [`SurfaceProbePresenter`] so existing callers compile.
#[cfg(not(target_arch = "wasm32"))]
pub use native_presenter::SurfaceProbePresenter as NativeSurfacePresenter;

use cad_diagnostics::codes;
use cad_diagnostics::{DiagnosticParameter, DiagnosticReason};
use cad_domain::*;
use cad_resources::{DecodedImage, DecodedImageCache, ResourceKey};
use cad_scene::{FrameBudget, ImageBatch, RenderTopology, SceneDelta};
use geometry::OverBudget;
pub use geometry::{
    clamp_alpha, classify_alpha, plan_draw_order, AlphaClass, BatchOrderEntry, DrawOrderCache,
    DrawOrderPlan,
};
pub use geometry::{
    front_face_ccw, has_vertex_colors, normals_need_repair, repaired_colors, repaired_normals,
    winding_is_flipped,
};
pub use geometry::{Camera2d, Camera3d};
use std::collections::HashMap;
use std::sync::Arc;

/// The CAD render-target clear color: **pure black, fully opaque**.
///
/// This is the actual clear of the CAD render target (not a DOM/CSS overlay) and
/// applies to every space (model and paper). It is the global default; a host
/// that needs a different background must change it explicitly.
pub(crate) const CAD_CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPreference {
    Auto,
    WebGpu,
    WebGl2,
}

/// Adapter preference within the active backend, for native/headless use.
///
/// This is an explicit *preference*, never a guarantee: a machine without a
/// discrete (or integrated) adapter falls back to the default adapter order.
/// On hybrid machines (iGPU + discrete) the default `Auto` prefers the discrete
/// GPU, so `nvidia-smi` shows the process without any flag. Strings are stable
/// for CLI/UI/diagnostics: `auto`, `high`, `low`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuSelection {
    /// Prefer a discrete GPU (`device_type == discrete_gpu`); fall back to the
    /// first adapter of the highest-priority backend when none exists. This is
    /// the dual-GPU default: discrete-first.
    #[default]
    Auto,
    /// Same discrete-first preference as `Auto`, kept as an explicit spelling.
    HighPerformance,
    /// Prefer an integrated GPU (`device_type == integrated_gpu`); fall back to
    /// the first adapter of the highest-priority backend when none exists.
    LowPower,
}

impl GpuSelection {
    /// Stable lowercase name for CLI/UI/diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            GpuSelection::Auto => "auto",
            GpuSelection::HighPerformance => "high",
            GpuSelection::LowPower => "low",
        }
    }
    /// Parse the stable CLI/UI name; unknown values are `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(GpuSelection::Auto),
            "high" => Some(GpuSelection::HighPerformance),
            "low" => Some(GpuSelection::LowPower),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveBackend {
    WebGpu,
    WebGl2,
    Native,
}

impl ActiveBackend {
    /// Stable lowercase name for UI/CLI/diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            ActiveBackend::WebGpu => "webgpu",
            ActiveBackend::WebGl2 => "webgl2",
            ActiveBackend::Native => "native",
        }
    }
}

/// Which draw organization the renderer is using for the converted opaque
/// classes.
///
/// Chosen **once per device** in [`Renderer::initialize_with_device`] from the
/// device's own backend/limits, never by requesting a feature from a
/// host-provided device. The live tier is reported by [`Renderer::render_tier`]
/// and on every [`FrameStats::tier`], so a downgrade is labelled rather than
/// silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTier {
    /// Per-run draws: the WebGL2-safe path. Opaque lines/meshes coalesce into
    /// instanced/merged runs bounded by the per-chunk stream count, each drawn
    /// with its own per-stream uniform.
    Universal,
    /// One draw per converted class per pass: primitives are pulled from
    /// read-only vertex-stage storage buffers (see `shaders/*_pulled.wgsl`).
    /// Requires storage buffers in the vertex stage (absent on WebGL2).
    Accelerated,
}

impl RenderTier {
    /// Stable lowercase name for UI/CLI/diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            RenderTier::Universal => "universal",
            RenderTier::Accelerated => "accelerated",
        }
    }
}

/// Observable state of the accelerated whole-scene **pulled** organisation (the
/// one-draw-per-class vertex-pulling path).
///
/// It is a *decision* record, not a performance metric: it exists so a skipped
/// accelerated organisation is never reported as success (AGENTS.md iron rule
/// #2). Every variant other than [`PulledState::Active`] means the covered
/// classes are drawn by the universal per-run path instead — which produces the
/// same pixels, so an over-conservative skip costs speed, never correctness.
///
/// The state is decided in `Renderer::rebuild_pulled` (per commit) and is
/// surfaced on every [`FrameStats`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PulledState {
    /// Every resident converted class is built as pulled vertex-stage storage
    /// (whole-scene line segments and/or merged-mesh segments).
    Active,
    /// The rebuild was **skipped before allocating**: the exact bytes the pulled
    /// organisation needs exceed the renderer's memory budget for it, derived
    /// from the device's own limits (see `Renderer::pulled_budget_bytes`). The
    /// universal per-run path is used instead; pixels are unchanged.
    SkippedInsufficientVram {
        /// Exact bytes one rebuilt pulled organisation would allocate (line +
        /// mesh storage payloads and both chunk tables; see
        /// `pulled_payload_bytes`).
        required_bytes: u64,
        /// The budget those bytes were compared against.
        budget_bytes: u64,
    },
    /// The rebuild was skipped because an out-of-memory error was **captured**
    /// from the device while allocating the pulled buffers (fragmentation the
    /// pre-flight estimate cannot see). Any partially built pulled state was
    /// discarded and the universal per-run path was rebuilt.
    SkippedOutOfMemory {
        /// The pre-flight requirement that was under budget (i.e. the estimate
        /// the capture disproved).
        required_bytes: u64,
        /// The budget the estimate was checked against.
        budget_bytes: u64,
        /// The captured wgpu error text (whitespace-normalized).
        message: String,
    },
    /// The rebuild was skipped because the device reported an error while a
    /// pulled resource was being allocated that is **not** provably an
    /// out-of-memory condition.
    ///
    /// This is not a catch-all for convenience: on wgpu 30.0.1 a `create_buffer`
    /// the driver cannot satisfy hands back an *invalid* handle, and the failure
    /// is reported as a **validation** error on the next use of that handle
    /// (`Buffer with '…' label is invalid`). Reporting that as
    /// [`PulledState::SkippedOutOfMemory`] would claim evidence of VRAM
    /// exhaustion the device never gave, so it is its own verdict. Any partially
    /// built pulled state was discarded and the universal per-run path was
    /// rebuilt; pixels are unchanged.
    SkippedAllocationFailed {
        /// The pre-flight requirement the failed attempt was trying to allocate
        /// (the pulled payload estimate), so a caller can compare it with the
        /// budget exactly as it would for the other skip verdicts.
        required_bytes: u64,
        /// The budget the estimate was checked against.
        budget_bytes: u64,
        /// The captured wgpu error text (whitespace-normalized), plus a note when
        /// the universal fallback itself could not be reallocated.
        message: String,
    },
    /// A class could not be built because it cannot be segmented below the
    /// device's storage-binding limit: a single resident stream/chunk (or the
    /// chunk table) is larger than one binding, and a run is never split across
    /// two buffers.
    SkippedUnsegmentable {
        /// Bytes of the largest single allocation the class would need.
        largest_class_bytes: u64,
        /// The storage-binding limit it must fit in.
        binding_limit_bytes: u64,
    },
    /// Nothing to pull: no converted line/mesh class is resident.
    SkippedNoConvertedGeometry,
    /// The pulled organisation does not apply to this renderer: the live tier is
    /// [`RenderTier::Universal`] (WebGL2 / no vertex-stage storage buffers), or
    /// the device and its derived pulled resources are absent.
    Unavailable,
}

impl PulledState {
    /// Stable machine code for CLI/UI/diagnostics.
    pub fn as_str(&self) -> &'static str {
        match self {
            PulledState::Active => "active",
            PulledState::SkippedInsufficientVram { .. } => "skipped-insufficient-vram",
            PulledState::SkippedOutOfMemory { .. } => "skipped-out-of-memory",
            PulledState::SkippedAllocationFailed { .. } => "skipped-allocation-failed",
            PulledState::SkippedUnsegmentable { .. } => "skipped-unsegmentable",
            PulledState::SkippedNoConvertedGeometry => "skipped-no-converted-geometry",
            PulledState::Unavailable => "unavailable",
        }
    }

    /// Whether this state permits the accelerated per-class draw organisation.
    ///
    /// A frame may legitimately report `accelerated == false` while the state is
    /// [`PulledState::Active`] (a paginated/over-budget page that did not accept
    /// every converted batch), but the converse is impossible: a skipped state
    /// must never be drawn through the pulled path.
    pub fn allows_accelerated(&self) -> bool {
        matches!(self, PulledState::Active)
    }
}

impl std::fmt::Display for PulledState {
    /// Single-line token for probe logs: `name` plus its numbers, no whitespace.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PulledState::Active => write!(f, "active"),
            PulledState::SkippedInsufficientVram {
                required_bytes,
                budget_bytes,
            } => write!(
                f,
                "skipped-insufficient-vram(required_bytes={required_bytes},budget_bytes={budget_bytes})"
            ),
            PulledState::SkippedOutOfMemory {
                required_bytes,
                budget_bytes,
                message,
            } => write!(
                f,
                "skipped-out-of-memory(required_bytes={required_bytes},budget_bytes={budget_bytes},message={})",
                message.split_whitespace().collect::<Vec<_>>().join("_")
            ),
            PulledState::SkippedAllocationFailed {
                required_bytes,
                budget_bytes,
                message,
            } => write!(
                f,
                "skipped-allocation-failed(required_bytes={required_bytes},budget_bytes={budget_bytes},message={})",
                message.split_whitespace().collect::<Vec<_>>().join("_")
            ),
            PulledState::SkippedUnsegmentable {
                largest_class_bytes,
                binding_limit_bytes,
            } => write!(
                f,
                "skipped-unsegmentable(largest_class_bytes={largest_class_bytes},binding_limit_bytes={binding_limit_bytes})"
            ),
            PulledState::SkippedNoConvertedGeometry => write!(f, "skipped-no-converted-geometry"),
            PulledState::Unavailable => write!(f, "unavailable"),
        }
    }
}

pub struct BackendCapabilities {
    pub actual: ActiveBackend,
    /// Stable API name obtained from the shared device's adapter information.
    pub api: &'static str,
    pub compute: bool,
    pub storage_buffers: bool,
    pub indirect_draw: bool,
    pub max_texture_dimension: u32,
}

pub struct RenderTarget {
    pub host_texture: u64,
    pub width: u32,
    pub height: u32,
    pub device_generation: u64,
    pub premultiplied_alpha: bool,
    pub srgb: bool,
    pub samples: u32,
}

impl RenderTarget {
    pub fn new(width: u32, height: u32) -> Self {
        RenderTarget {
            host_texture: 0,
            width,
            height,
            device_generation: 0,
            premultiplied_alpha: true,
            srgb: true,
            samples: 1,
        }
    }
}

/// Microsecond timings of the last [`Renderer::commit_upload`] call (diagnostics
/// only). Purely observational: it changes no commit semantics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommitTiming {
    /// Re-basing prepared runs onto the existing streams (append/reindex).
    pub rebase_us: u64,
    /// Appending the prepared streams/batches into the resident sets.
    pub append_us: u64,
    /// Garbage-collecting the resident instance/arc/mesh streams.
    pub gc_us: u64,
    /// Rebuilding the pulled vertex-stage storage buffers.
    pub pulled_us: u64,
    /// Rebuilding the pick table.
    pub pick_us: u64,
    /// Rebuilding the lineweight cache.
    pub lineweight_us: u64,
    /// Total `commit_upload` wall time.
    pub total_us: u64,
}

pub struct FrameStats {
    /// Wall-clock milliseconds of this frame's CPU path on the render thread:
    /// scene plan/cache lookup, per-batch uniform writes, command encoding and
    /// `queue.submit`. It deliberately **excludes** the blocking device poll, so
    /// it does not fold GPU execution time into a CPU number.
    pub cpu_ms: f64,
    /// Real GPU milliseconds, present **only** when the measurement-only opt-in
    /// (ADR 0013 class 1) was explicitly enabled *and* the device granted the
    /// timestamp feature set. `None` is honest: no estimate is ever substituted,
    /// and the product configuration (measurement off) always reports `None`.
    /// The value comes from a timestamp-query bracket around this frame's GPU
    /// work; it is a **measurement** (the readback perturbs the frame) and never
    /// a product-configuration number.
    pub gpu_ms: Option<f64>,
    /// The explicit measurement state token for this frame
    /// ([`gpu_time::GpuTimeState`]): `enabled`, `disabled` or
    /// `unsupported(<FEATURE>)`. Report-only; the renderer never depends on it.
    pub gpu_time: gpu_time::GpuTimeState,
    /// Draw submissions this frame, derived from the plan's batch/run accounting
    /// (non-converted opaque batches + coalesced instanced runs + coalesced
    /// merged-mesh runs + transparent batches + drawn images) rather than by
    /// intercepting every `draw`/`draw_indexed` call. It is a batch-collapse
    /// metric: a converted batch is represented by the coalesced runs it joins,
    /// and each run contributes one entry however many member batches it covers.
    pub draw_calls: u64,
    /// Opaque batches drawn as a legacy per-batch draw this frame, i.e.
    /// `opaque_batches - converted_batches`. This is the `legacy_opaque` term of
    /// [`FrameStats::draw_calls`]: every other term of that sum is an explicit
    /// field (`arc_draws`, `instanced_draws`, `mesh_draws`) or `transparent`.
    /// The identity
    /// `legacy_opaque_draws + arc_draws + instanced_draws + mesh_draws +
    /// transparent_batches + image_batches == draw_calls` holds by construction
    /// (see `renderer_impl.rs`, the `draw_calls` expression). Diagnostics only;
    /// no behaviour depends on it.
    pub legacy_opaque_draws: u64,
    /// Number of `Queue::write_buffer` calls issued for this frame's uniforms.
    ///
    /// Every accepted batch (and every drawn image) writes its 96-byte uniform
    /// into a shared, dynamically-offset slot buffer, so a frame issues **one
    /// write per uniform page** touched (a page holds
    /// `max_uniform_buffer_binding_size / slot_size` slots) instead of one per
    /// batch. Diagnostics only: it exists so a frame's per-call submission cost
    /// is measurable and not guessed.
    pub uniform_writes: u64,
    pub uploaded_bytes: u64,
    /// Vertices submitted this frame (post-budget).
    pub vertices: usize,
    /// Triangles submitted this frame (post-budget).
    pub triangles: usize,
    /// Present when part of the scene was held back because the frame budget
    /// was crossed. The caller must surface this; batches are never silently
    /// dropped.
    ///
    /// The literal meaning is unchanged by adaptive pacing: "this frame's plan
    /// crossed the *active* page budget" (the scaled budget while
    /// [`FrameStats::pacing`] is `active`, the static ceiling otherwise). A
    /// frame that only crossed the scaled budget to make progress carries the
    /// category `pacing-floor`; the diagnostic code stays
    /// `render.frame_over_budget`.
    pub over_budget: Option<OverBudgetReason>,
    /// Adaptive page-budget scale this frame's plan **used** (1.0 when pacing is
    /// disabled or the renderer is not paging). See [`pacing`].
    pub budget_scale: f64,
    /// Adaptive page-budget target for `cpu_ms + wait_ms`, in milliseconds.
    pub budget_target_ms: f64,
    /// Cost EWMA (milliseconds) the scale was based on, i.e. the value **before**
    /// this frame's own sample was folded in. `0.0` before the first observation.
    pub budget_cost_ema_ms: f64,
    /// Active page-budget vertex limit this frame's plan used.
    pub budget_vertices: usize,
    /// Active page-budget triangle limit this frame's plan used.
    pub budget_triangles: usize,
    /// Pacing regime for this frame: `"active"`,
    /// `"disabled-no-completion-signal"` (wasm non-blocking poll: no measured
    /// completion, static budget) or `"at-ceiling"` (non-progressive frame
    /// planned against the un-paced ceiling).
    pub pacing: &'static str,
    /// Opaque batches submitted this frame.
    pub opaque_batches: usize,
    /// Transparent batches (`0 < alpha < 1`) submitted after the opaque pass.
    pub transparent_batches: usize,
    /// Batches skipped because `alpha <= 0`. Reported, never silently drawn.
    pub invisible_batches: usize,
    /// Raster image quads submitted this frame, after opaque geometry.
    ///
    /// Full draw-order interleaving with geometry batches and WIPEOUT-style
    /// masking are **not** implemented; images are drawn last, in ascending
    /// `draw_order` among themselves (see [`Renderer::upload_images`]).
    pub image_batches: usize,
    /// Chunks (instance streams + merged mesh chunks + SDF conic streams) tested
    /// by this frame's view cull. `0` when the scene has no chunked geometry.
    /// Reporting only; `kept + culled == total`.
    pub cull_chunks_total: u64,
    /// Chunks kept (visible) by this frame's view cull. A culled chunk's runs are
    /// neither drawn nor charged to the page budget (see `cull.rs`).
    pub cull_chunks_kept: u64,
    /// Lineweights that were carried but **not drawn** this frame.
    ///
    /// Wide lines are not portable across wgpu backends (and the browser
    /// WebGl2/WebGPU line width is always 1 px), so the renderer never claims a
    /// lineweight it did not rasterize. Every submitted batch with a non-zero
    /// lineweight is reported here, with the code
    /// `render.lineweight_not_drawn`, so the gap is explicit rather than silent
    /// (see `docs/entity-style.md`).
    pub lineweight_not_drawn: Vec<LineweightNotDrawn>,
    /// Set when at least one lineweight was not drawn; a single structured
    /// reason aggregating the count, for callers that surface one diagnostic.
    pub lineweight_reason: Option<DiagnosticReason>,
    /// CPU milliseconds of the frame-plan block: draw-order cache build/scan,
    /// transparent re-sort, the accepted/opaque/transparent split and the
    /// lineweight scan. Part of [`FrameStats::cpu_ms`].
    pub plan_ms: f64,
    /// CPU milliseconds of the uniform path: matrix build, slot write into the
    /// staging arena and `flush` (`Queue::write_buffer`, one per page). Part of
    /// [`FrameStats::cpu_ms`].
    pub uniform_ms: f64,
    /// CPU milliseconds of command encoding: `begin_render_pass` and every
    /// batch/image `set_*` + draw call. Part of [`FrameStats::cpu_ms`].
    pub encode_ms: f64,
    /// CPU milliseconds of `Queue::submit` (excluding the blocking poll). Part
    /// of [`FrameStats::cpu_ms`].
    pub submit_ms: f64,
    /// Wall-clock milliseconds of the blocking device poll that classifies the
    /// submission. Deliberately **excluded** from [`FrameStats::cpu_ms`]; it
    /// measures the wait on the GPU, not the render thread's CPU work.
    pub wait_ms: f64,
    /// `RenderPass::set_pipeline` calls actually issued this frame (post-dedup).
    pub set_pipeline_calls: u64,
    /// `RenderPass::set_bind_group` calls actually issued this frame (post-dedup).
    pub set_bind_group_calls: u64,
    /// `RenderPass::set_vertex_buffer` calls actually issued this frame
    /// (post-dedup).
    pub set_vertex_buffer_calls: u64,
    /// `RenderPass::set_index_buffer` calls actually issued this frame
    /// (post-dedup).
    pub set_index_buffer_calls: u64,
    /// Uniform arena slots used this frame (accepted geometry batches + drawn
    /// images).
    pub uniform_slots: usize,
    /// Times the progressive frame cursor was reset this frame because the
    /// camera/attachment key changed (0 for a non-progressive renderer).
    pub progressive_restarts: u64,
    /// Instanced line draw calls issued this frame (coalesced runs). Each one
    /// expands many segments of one [`InstanceRun`]-carrying stream.
    pub instanced_draws: u64,
    /// Batches this frame drew through the instanced line path (the sum of all
    /// coalesced runs' member batches). `0` when nothing converted.
    pub instanced_batches: usize,
    /// Resident instanced line streams. Uploaded once per scene change; a pan or
    /// zoom never changes it.
    pub instance_streams: usize,
    /// Merged mesh draw calls issued this frame (coalesced runs). Each one draws
    /// the triangle range of many flat-merged mesh batches as a single draw.
    pub mesh_draws: u64,
    /// Batches this frame drew through the merged mesh path (the sum of all
    /// coalesced runs' member batches). `0` when nothing converted.
    pub merged_mesh_batches: usize,
    /// Resident merged mesh chunks. Uploaded once per scene change; a pan or zoom
    /// never changes it.
    pub mesh_chunks: usize,
    /// SDF conic draw calls issued this frame (coalesced runs).
    pub arc_draws: u64,
    /// Batches this frame drew through the SDF conic path.
    pub arc_batches: usize,
    /// Resident SDF conic streams. Uploaded once per scene change; a pan or zoom
    /// never changes it.
    pub arc_streams: usize,
    /// SDF conic instances resident this frame (summed across streams).
    pub arc_instances: u64,
    /// The draw tier this frame used (see [`RenderTier`]). Chosen once per
    /// device; an accelerated device reports [`RenderTier::Accelerated`] even on
    /// a frame that falls back to the universal per-run organization (for
    /// example a paginated/over-budget frame — see [`FrameStats::accelerated`]).
    pub tier: RenderTier,
    /// Whether this frame's opaque pass actually used the accelerated per-class
    /// draw organization. `false` on a [`RenderTier::Accelerated`] device when
    /// the frame could not (no converted geometry, or a page that did not accept
    /// every converted batch).
    pub accelerated: bool,
    /// Draw calls issued by the accelerated per-class path this frame (opaque
    /// line class + merged-mesh classes). `0` when the universal path was used.
    pub accelerated_draws: u64,
    /// Trailing unpaired vertices dropped from `Lines`/`MeshEdges` batches whose
    /// vertex count is odd. A `LineList` cannot pair them and both the legacy and
    /// the converted path drop the same trailing vertex; this counts the drop so
    /// it is reported rather than silent (see `docs/render-order.md`).
    pub line_odd_vertex_skips: u64,
    /// Observable state of the accelerated pulled organisation at this frame
    /// (see [`PulledState`]). `Active` never appears while the pulled path was
    /// skipped: a skip is reported with its reason and byte numbers.
    pub pulled_state: PulledState,
}

/// One batch whose lineweight was carried but not rasterized.
#[derive(Debug, Clone, PartialEq)]
pub struct LineweightNotDrawn {
    /// The requested lineweight in millimetres.
    pub millimeters: f32,
}

/// A structured over-budget reason, mirroring [`geometry::OverBudget`] with a
/// stable diagnostic code attached.
#[derive(Debug, Clone)]
pub struct OverBudgetReason {
    pub report: OverBudget,
    pub reason: DiagnosticReason,
}

impl OverBudgetReason {
    pub fn diagnostic(&self) -> &DiagnosticReason {
        &self.reason
    }
}

/// One fragment-level result of [`Renderer::pick_at`].
///
/// The id pass writes one 32-bit **pick key** per fragment (see [`PickTable`]).
/// `entity_key` is that raw value; the remaining fields are its CPU-side
/// resolution, which preserves the full `u128` `EntityId` and reports — never
/// guesses — anything it cannot resolve exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct PickHit {
    /// The raw key the id pass wrote for the picked fragment. `0` is the cleared
    /// background and never reaches a `PickHit`.
    pub entity_key: u32,
    /// The exact identity when the key names exactly one source. `None` when the
    /// hit is ambiguous or the batch carries no source (see `reason`).
    pub source: Option<SelectionRef>,
    /// Every entity that can own a fragment of this key, in the owning batch's
    /// order. One element for an exact hit; several when the batch concatenated
    /// sources whose geometry is indistinguishable after representation.
    pub candidates: Vec<SelectionRef>,
    /// A mesh-face sub-element when the id pass could resolve one.
    ///
    /// Always `None` in this slice: wgpu exposes no primitive index to the
    /// fragment stage, so a face cannot be derived from the id pass. Sub-element
    /// picking therefore stays on CPU refinement (`cad_spatial`), and this field
    /// is not faked to look precise.
    pub sub_element: Option<SubElementId>,
    /// Machine key describing how the key resolved (never translated prose).
    ///
    /// It is set for a non-exact resolution (`source == None`, e.g.
    /// `pick-multi-source-batch`) **and** for an exact hit the id pass cannot
    /// refine to a sub-element (`pick-sub-element-requires-cpu-refinement`), so
    /// a `Some(reason)` does **not** by itself mean the hit is ambiguous.
    /// [`PickHit::is_ambiguous`] is the real predicate for "more than one
    /// possible entity".
    pub reason: Option<&'static str>,
    /// `true` when one of this key's sources shares its low-32 `EntityId` with a
    /// different full identity elsewhere in the resident scene: the `u128`
    /// truncation collision that a 32-bit-only consumer could not see.
    pub key_collision: bool,
}

impl PickHit {
    /// Whether the key resolved to more than one possible entity.
    pub fn is_ambiguous(&self) -> bool {
        self.source.is_none()
    }
}

/// One detected `EntityId` truncation collision: two or more *distinct* full
/// identities whose low 32 bits agree.
#[derive(Debug, Clone, PartialEq)]
pub struct PickCollision {
    /// The shared low-32 value (`EntityId.0 as u32`).
    pub truncated_key: u32,
    /// The distinct identities that share it, in first-seen order.
    pub identities: Vec<SelectionRef>,
}

/// Selection/hover/visibility bits of the flags table (see
/// [`Renderer::set_entity_flags`]).
///
/// The canonical values live in `cad-scene` (`cad_scene::FLAG_*`) so the app
/// layer, which does not depend on this crate, can name them without a second
/// definition drifting out of sync; this crate re-exports them and its WGSL
/// shaders declare the matching constants.
pub use cad_scene::{FLAG_HIDDEN, FLAG_HOVERED, FLAG_SELECTED};

/// Result of [`Renderer::set_entity_flags`].
///
/// `matched` is the number of distinct pick keys the update wrote bits to.
/// `unmatched` names the requested refs that resolved to **no** resident
/// whole-entity pick key — for example an entity the base build excluded (a
/// hidden/frozen layer). The caller must surface these as a diagnostic rather
/// than treat the update as a silent success (iron rule #2: an unsupported
/// capability is modelled explicitly, never turned into an empty success).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityFlagReport {
    /// Distinct pick keys that carry a flag after the update.
    pub matched: usize,
    /// Requested refs that matched no resident whole-entity key.
    pub unmatched: Vec<SelectionRef>,
}

/// The color-attachment format of the id target.
///
/// `R32Uint` is the primary choice: the WebGPU spec guarantees it as a render
/// attachment (`R32Uint.guaranteed_format_features`), and wgpu's GL backend
/// likewise advertises `COLOR_ATTACHMENT` for it, so one 32-bit integer channel
/// carries a whole key with no packing. `PackedRgba8` is the fallback for a
/// device whose adapter does not report `R32Uint` renderable: the key is split
/// into four `unorm8` channels, which every tier can render to. The active
/// choice is decided at init from the device and reported by
/// [`Renderer::id_target_format`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdTargetFormat {
    /// One `u32` channel (`TextureFormat::R32Uint`).
    R32Uint,
    /// Four `unorm8` channels (`TextureFormat::Rgba8Unorm`), low byte in `r`.
    PackedRgba8,
}

impl IdTargetFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            IdTargetFormat::R32Uint => "r32uint",
            IdTargetFormat::PackedRgba8 => "packed-rgba8",
        }
    }

    /// The wgpu texture format this choice renders into.
    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            IdTargetFormat::R32Uint => wgpu::TextureFormat::R32Uint,
            IdTargetFormat::PackedRgba8 => wgpu::TextureFormat::Rgba8Unorm,
        }
    }

    /// Bytes per texel in the readback buffer.
    pub fn bytes_per_texel(self) -> u32 {
        4
    }
}

/// A renderer failure that the caller can distinguish from a bad frame.
///
/// Audit F12: a device loss must not be confused with a per-frame validation
/// error, and the renderer must not silently recreate state while batches are
/// lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The GPU device was lost (driver reset, `destroy`, backend failure). All
    /// derived GPU resources are gone; the host must supply a new device and the
    /// scene must be re-uploaded via [`Renderer::upload`].
    DeviceLost(String),
    /// A per-frame validation/back-end error that is *not* a device loss. The
    /// device and its batches are intact; the frame was rejected.
    Frame(String),
    /// The renderer was used before a device was initialized, or a required
    /// derived resource is missing. Not a device loss.
    NotInitialized(String),
}

impl RenderError {
    /// Whether the caller should tear down and rebuild GPU state.
    pub fn is_device_loss(&self) -> bool {
        matches!(self, RenderError::DeviceLost(_))
    }

    pub fn message(&self) -> &str {
        match self {
            RenderError::DeviceLost(m) | RenderError::Frame(m) | RenderError::NotInitialized(m) => {
                m
            }
        }
    }
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for RenderError {}

/// Where a render call's error scope reported a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeOutcome {
    Clean,
    FrameError,
    /// Only native backends classify a poll timeout as loss; on wasm the
    /// non-blocking poll never produces it, but the outcome type is shared.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    DeviceLost,
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

struct GpuBatch {
    /// Per-batch position vertex buffer (`None` when the batch was converted to
    /// an instanced line stream, which owns the geometry instead).
    vertices: Option<wgpu::Buffer>,
    vertex_count: u32,
    /// Per-vertex normals (mesh topology only); kept alive for binding.
    normals: Option<wgpu::Buffer>,
    /// Per-vertex colours (mesh topology only); kept alive for binding. Always
    /// bound for a mesh (white when the batch has no gradient), so the shader's
    /// `@location(2)` attribute is always provided.
    colors: Option<wgpu::Buffer>,
    topology: RenderTopology,
    /// Triangle index buffer (mesh topology only).
    indices: Option<wgpu::Buffer>,
    index_count: u32,
    /// Pre-expanded anti-aliased line quads for this batch's line geometry: the
    /// `Lines`/`MeshEdges` line list, or a non-merged mesh's wireframe overlay.
    /// `None` when the batch owns no per-batch line geometry (a converted
    /// instance stream, or a mesh with no overlay).
    line_quads: Option<wgpu::Buffer>,
    /// Vertex count of [`GpuBatch::line_quads`] (six per segment).
    line_quad_count: u32,
    /// `true` when the batch's transform mirrors winding.
    mirrored: bool,
    /// Constant per-object alpha.
    alpha: f32,
    /// Constant per-object colour (normalized sRGB), sanitized at upload.
    color: [f32; 3],
    /// Requested lineweight in millimetres; carried but not drawn.
    lineweight: f32,
    /// Paint order relative to sibling batches (larger is later/on top). The
    /// frame's draw plan performs a stable sort on this key.
    draw_order: i64,
    /// World-space centroid, the transparent pass's back-to-front sort key.
    centroid: [f32; 3],
    origin: [f32; 3],
    /// Empty unless this batch's segments live in instanced line streams. The
    /// frame path draws each `[offset, offset + count)` instance slice in order
    /// (coalescing against adjacent slices in the same stream) instead of a
    /// per-batch draw. Usually one run; a batch larger than a stream's instance
    /// cap contributes one run per chunk it was split across.
    instanced: Vec<InstanceRun>,
    /// Empty unless this batch's triangles live in merged mesh chunks (see
    /// [`MeshChunkData`]). The frame path draws each `[index_offset,
    /// index_offset + index_count)` triangle range in order (coalescing against
    /// adjacent ranges in the same chunk) instead of a per-batch draw. Usually
    /// one run; a batch larger than the vertex cap contributes one run per piece.
    mesh_runs: Vec<MeshRun>,
    /// Empty unless this batch's conics live in SDF instance streams (see
    /// [`ArcChunk`]). The frame path draws each `[offset, offset + count)`
    /// instance slice in order (coalescing against adjacent slices in the same
    /// stream). Usually one run; a batch larger than the instance cap contributes
    /// one run per chunk.
    arc_runs: Vec<ArcRun>,
    /// Exact bytes this batch uploads, cached at upload time so the per-frame
    /// byte budget does not recompute the edge index list every frame.
    upload_bytes: usize,
    /// This batch's renderer pick key (see [`PickTable`]): `1 + <ordinal>`, never
    /// `0`. Packed into every instance/merged vertex the batch contributes and
    /// into `Camera::key_flags.x` of its per-batch uniform, so the id pass and
    /// the selection-flags table can resolve any fragment of it.
    pick_key: u32,
    /// The full `SelectionRef` list this batch's fragments can belong to,
    /// preserved so the CPU-side pick table can return the whole `u128` identity
    /// (the scene builder may concatenate several entities into one batch).
    sources: Vec<SelectionRef>,
}

/// One device-resident merged mesh chunk: an interleaved vertex buffer, a
/// triangle index buffer rebased onto it, and the `f64` reference origin its
/// stored positions are measured from.
///
/// Uploaded once (upload/commit) and never rewritten on pan/zoom: a camera move
/// is a small uniform write only. Like [`InstanceStream`], each chunk is
/// spatially bounded (see [`build_mesh_chunks`]), so a stored coordinate is
/// within [`MAX_STREAM_RADIUS`] of `origin` and the f32 ulp stays under ~1 µm.
pub(crate) struct GpuMeshChunk {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertex_count: u32,
    index_count: u32,
    origin: [f64; 3],
    /// World-space AABB of the chunk's vertices, retained from the builder for
    /// per-frame view culling (see [`cull`]). Never rewritten on a camera move.
    aabb: WorldAabb,
    mirrored: bool,
    /// CPU copy of the chunk's packed vertices and rebased indices, kept so the
    /// accelerated tier's whole-scene pulled buffers can be rebuilt on every
    /// commit (including an overlay append that keeps a prefix). Only populated
    /// when the accelerated tier is live; the universal tier leaves both empty.
    pulled_vertices: Vec<MeshVertex>,
    pulled_indices: Vec<u32>,
}

/// Where a converted batch's segments live in the instanced line streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstanceRun {
    /// Index into [`Renderer::instance_streams`].
    stream: u32,
    /// First instance of this batch within its stream.
    offset: u32,
    /// Number of instances (segments) this batch contributes.
    count: u32,
}

/// Where a conic batch's SDF instances live in the conic streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArcRun {
    /// Index into [`Renderer::arc_streams`].
    stream: u32,
    /// First instance of this batch within its stream.
    offset: u32,
    /// Number of conic instances this batch contributes.
    count: u32,
}

/// One device-resident instanced line stream, plus the `f64` reference origin
/// its positions are stored relative to.
///
/// Uploaded once (upload/commit) and never rewritten on pan/zoom: a camera move
/// is a small uniform write only. On the **orthographic** path the per-frame
/// matrix subtracts the camera centre from `origin` in `f64` on the CPU before
/// narrowing, so stream positions stay small; the **perspective** path narrows
/// the origin to `f32` before `translate_left` (see
/// `FrameProjection::batch_matrix_f64`), so that `f64` subtraction guarantee is
/// Ortho-only. Each stream covers a spatially bounded chunk of the converted
/// geometry (see [`build_instance_chunks`]), so a stored coordinate is within
/// [`MAX_STREAM_RADIUS`] of `origin` — the f32 ulp stays under ~1 µm and the
/// deep-zoom-on-a-large-drawing precision floor holds. It is not byte-exact
/// against the legacy per-batch path for arbitrary origins; see the deep-zoom
/// parity fence.
pub(crate) struct InstanceStream {
    buffer: wgpu::Buffer,
    instance_count: u32,
    origin: [f64; 3],
    /// World-space AABB of the chunk's segments, retained from the builder for
    /// per-frame view culling (see [`cull`]). Never rewritten on a camera move.
    aabb: WorldAabb,
    /// First instance of this stream within its buffer. `0` on the universal
    /// tier (each stream owns its buffer); on the accelerated tier every stream
    /// in a storage segment shares one `VERTEX | STORAGE` buffer, so a run's
    /// global instance index is `base + run.offset`.
    base: u32,
    /// CPU copy of the stream's packed instances, kept so the accelerated tier's
    /// whole-scene pulled line buffer can be rebuilt on every commit. Only
    /// populated when the accelerated tier is live; the universal tier leaves it
    /// empty.
    pulled_instances: Vec<LineInstance>,
}

/// One device-resident SDF conic stream, plus the `f64` reference origin its
/// centers are stored relative to.
///
/// Uploaded once (upload/commit) and never rewritten on pan/zoom: a camera move
/// is a small uniform write only. Each stream covers a spatially bounded chunk of
/// the converted conics (see `build_arc_chunks`), so a stored center is within
/// `MAX_STREAM_RADIUS` of `origin` and the f32 ulp stays under ~1 µm. The conic
/// path is **universal-tier only** (no pulled variant this round); `base` is kept
/// for symmetry with [`InstanceStream`] and is always `0`.
pub(crate) struct ArcStream {
    buffer: wgpu::Buffer,
    instance_count: u32,
    origin: [f64; 3],
    /// World-space AABB of the chunk's conic full extents, retained from the
    /// builder for per-frame view culling (see [`cull`]). Never rewritten on a
    /// camera move.
    aabb: WorldAabb,
    /// First instance of this stream within its buffer (always `0`: each conic
    /// stream owns its buffer).
    #[allow(dead_code)]
    base: u32,
}

/// One accelerated line storage segment: a bind group over a
/// `VERTEX | STORAGE` buffer holding the [`PulledLine`] records of a run of
/// whole instance streams. A real scene can exceed
/// `max_storage_buffer_binding_size`, so the class is split into segments and
/// drawn once per segment (still `O(1)` in the chunk count, not one per chunk).
/// The same buffer is bound as the per-run instance vertex buffer (64-byte
/// layout), so the accelerated tier keeps **one** copy of the line geometry —
/// not a per-run copy plus a pulled copy.
pub(crate) struct PulledLineSegment {
    /// Group 2 of the pulled line pipeline: this segment's instances + the shared
    /// chunk table.
    bind_group: wgpu::BindGroup,
    instance_count: u32,
    /// The segment's `VERTEX | STORAGE` geometry buffer (also the per-run instance
    /// vertex buffer). Retained across admitted suffix commits so the base
    /// geometry is not re-encoded/re-uploaded.
    geometry_buffer: wgpu::Buffer,
}

/// How a commit's pulled rebuild should proceed (see [`PulledPrefixCheckpoint`]).
pub(crate) enum PulledFastPath {
    /// Full rebuild of the pulled classes. `establish` is the surviving prefix
    /// length (`keep_prefix`) when a checkpoint should be recorded afterwards,
    /// else `None` (a base replacement or no prefix).
    Full { establish: Option<usize> },
    /// Reuse the retained base segments and build only the suffix.
    Reuse(Box<PulledPrefixCheckpoint>),
}

/// A retained pulled-buffer prefix checkpoint (accelerated tier only).
///
/// Established after a **full** pulled rebuild that retained a base batch prefix
/// (`keep_prefix > 0`). An admitted later suffix commit then reuses the retained
/// base geometry buffers and rebuilds only the suffix, instead of re-encoding and
/// re-uploading the whole base. Every field is verified before the fast path is
/// taken; anything unexpected falls back to a full rebuild (never a skip).
#[derive(Clone)]
pub(crate) struct PulledPrefixCheckpoint {
    /// Device generation the retained buffers were built on.
    device_generation: u64,
    /// Renderer pulled-cache epoch (bumped on every invalidation).
    cache_epoch: u64,
    /// Retained batch prefix length (the commit `keep_prefix`).
    retained_batches: usize,
    /// Retained resource prefix counts.
    instance_streams: usize,
    mesh_chunks: usize,
    arc_streams: usize,
    /// Retained pulled segment counts.
    line_segments: usize,
    mesh_segments: usize,
    /// Retained base segment resource ranges (whole-stream / whole-chunk ranges).
    line_base_ranges: Vec<(usize, usize)>,
    mesh_base_ranges: Vec<(usize, usize)>,
}

/// The accelerated tier's whole-scene line storage: every resident instance in
/// binding-sized segments, plus the per-chunk translation table rewritten each
/// frame.
pub(crate) struct PulledLines {
    segments: Vec<PulledLineSegment>,
    /// Per-chunk translation columns (one `vec4<f32>` per resident stream),
    /// rewritten each frame exactly like the universal path's per-stream uniform.
    chunk_table: wgpu::Buffer,
}

/// One accelerated merged-mesh storage segment (see [`PulledLineSegment`]).
pub(crate) struct PulledMeshSegment {
    /// Group 2 of the pulled mesh pipeline: this segment's vertices + indices +
    /// the shared chunk table.
    bind_group: wgpu::BindGroup,
    vertex_count: u32,
    index_count: u32,
    /// Number of non-mirrored indices (the first `mirror_split` entries).
    mirror_split: u32,
    #[allow(dead_code)]
    vertex_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    index_buffer: wgpu::Buffer,
}

/// The accelerated tier's whole-scene merged-mesh storage.
pub(crate) struct PulledMesh {
    segments: Vec<PulledMeshSegment>,
    /// Per-chunk translation columns, rewritten each frame.
    chunk_table: wgpu::Buffer,
}

impl GpuBatch {
    /// Exact bytes this batch uploads to the device.
    ///
    /// Positions/normals are 12 bytes per vertex, edge indices and the triangle
    /// index buffer are `u32` (4 bytes) each. This matches the byte count
    /// [`Renderer::upload`] accumulates, so the frame byte budget and the
    /// reported `FrameStats::uploaded_bytes` cannot disagree.
    fn upload_size_bytes(&self) -> usize {
        self.upload_bytes
    }

    /// The [`INSTANCE_FLAG_*`] bitfield of this resident batch, for
    /// `Camera::key_flags.y` of its per-batch uniform. Mirrors
    /// [`batch_instance_flags`] on the packed form (converted batches are always
    /// opaque, a legacy batch keeps its own alpha class).
    fn instance_flags(&self) -> u32 {
        (if classify_alpha(self.alpha) == AlphaClass::Opaque {
            INSTANCE_FLAG_OPAQUE
        } else {
            0
        }) | if self.sources.is_empty() {
            0
        } else {
            INSTANCE_FLAG_HAS_SOURCE
        } | if self.lineweight > 0.0 {
            INSTANCE_FLAG_LINEWEIGHT
        } else {
            0
        }
    }
}

/// A device-resident texture for one [`ResourceKey`], shared by every resident
/// image quad that references it.
///
/// [`Renderer`] caches one per key (see [`Renderer::upload_images`]) so `N`
/// entities referencing the same resource allocate a single texture and bind
/// group rather than `N` copies.
struct GpuTexture {
    /// The uploaded texture. Kept alive both for the bind group and so a new
    /// quad can clone a handle into its [`GpuImage`].
    texture: wgpu::Texture,
    /// Group 2 bind group (texture view + shared sampler), shared by every quad
    /// that samples this key.
    bind_group: wgpu::BindGroup,
    /// Padded RGBA bytes actually uploaded: `height` rows of `width * 4` bytes
    /// each rounded up to wgpu's 256-byte copy alignment. Charged once per key
    /// against the frame byte budget.
    upload_bytes: usize,
    /// The decoded image this texture was uploaded from, held by identity so the
    /// cache can tell whether a reused [`ResourceKey`] still refers to the same
    /// bytes. A new [`DecodedImageCache`] produces a new `Arc`, so a different
    /// document's `logo.png` is re-uploaded instead of sampling the previous
    /// drawing's texture. Holding the `Arc` (rather than a raw pointer) keeps the
    /// allocation alive, so `Arc::ptr_eq` cannot report a false match after the
    /// old image is freed.
    decoded: Arc<DecodedImage>,
}

/// One uploaded raster image: a textured quad plus the bind groups needed to
/// draw it. Owned by [`Renderer`] and rebuilt by [`Renderer::upload_images`].
struct GpuImage {
    /// The resource key this quad samples; quads sharing a key share a texture.
    resource: ResourceKey,
    /// World-space quad vertices (position + uv), indexed by `index_buffer`.
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    vertex_count: u32,
    /// Group 2: the sampled texture and its sampler (shared per resource key).
    texture_bind_group: wgpu::BindGroup,
    /// A handle to the shared texture, kept alive for as long as the texture view
    /// bound in `texture_bind_group` is used.
    _texture: wgpu::Texture,
    /// Padded texture bytes for this quad's resource key, charged to the frame
    /// byte budget once per key (not once per quad).
    texture_bytes: usize,
    /// Constant per-image alpha, already sanitized at upload.
    alpha: f32,
    /// Paint order relative to sibling images (larger is later/on top). Full
    /// interleaving with geometry batches is *not* implemented (see
    /// [`Renderer::upload_images`]).
    draw_order: i64,
    /// Exact bytes this quad uploads (vertex + index buffers).
    upload_bytes: usize,
    /// This image's renderer pick key (see [`PickTable`]).
    pick_key: u32,
    /// The full identities this quad can belong to (an `ImageBatch` carries its
    /// own `sources`), preserved for the CPU-side pick table.
    sources: Vec<SelectionRef>,
}

/// One shared uniform buffer plus its bind group (group 0).
///
/// The bind group binds exactly one slot (`slot_size` bytes); the per-draw
/// dynamic offset selects which slot, so a single bind group serves every batch
/// resident in the page.
struct UniformPage {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Shared per-frame uniform storage.
///
/// Every accepted geometry batch (and drawn image) writes its 96-byte uniform
/// into a slot of a shared uniform buffer and draws it with the slot's byte
/// offset as a *dynamic* offset. This replaces one `queue.write_buffer` per
/// batch with one per page: a page holds `slots_per_page` slots and its total
/// bytes are kept within `max_uniform_buffer_binding_size`, so each page stays a
/// legal uniform binding even on the WebGPU default 64 KiB limit.
///
/// Slots are assigned per frame in accepted order; the arena is reused across
/// frames (it only grows).
struct UniformArena {
    /// Stride between slots: the uniform struct size rounded up to the device's
    /// `min_uniform_buffer_offset_alignment` (dynamic offsets must be aligned).
    slot_size: u32,
    /// Slots per page: `max_uniform_buffer_binding_size / slot_size`, capped so
    /// an adapter reporting a very large binding size cannot allocate an
    /// unbounded buffer.
    slots_per_page: usize,
    pages: Vec<UniformPage>,
    /// CPU staging laid out exactly like the GPU slots (`slot * slot_size`), so
    /// each page uploads with one contiguous `write_buffer`.
    staging: Vec<u8>,
}

impl UniformArena {
    fn new(limits: &wgpu::Limits) -> Self {
        let (slot_size, slots_per_page) = uniform_layout_params(limits);
        UniformArena {
            slot_size,
            slots_per_page,
            pages: Vec::new(),
            staging: Vec::new(),
        }
    }

    /// Drop every GPU page (device loss / re-init).
    fn reset(&mut self) {
        self.pages.clear();
        self.staging.clear();
    }

    /// Grow to hold `needed` slots. Pages are created lazily and never shrink.
    fn ensure_slots(
        &mut self,
        needed: usize,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) {
        let pages_needed = needed.div_ceil(self.slots_per_page);
        while self.pages.len() < pages_needed {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cad-uniform-page"),
                size: (self.slots_per_page * self.slot_size as usize) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cad-uniform-page-bind"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        // One slot; the dynamic offset slides this window over
                        // the page, which is why the binding itself stays tiny.
                        size: wgpu::BufferSize::new(u64::from(self.slot_size)),
                    }),
                }],
            });
            self.pages.push(UniformPage { buffer, bind_group });
        }
        let needed_bytes = needed * self.slot_size as usize;
        if self.staging.len() < needed_bytes {
            self.staging.resize(needed_bytes, 0);
        }
    }

    /// Copy one batch/image uniform into its staging slot.
    fn write_slot(&mut self, slot: usize, uniform: &[u8]) {
        let start = slot * self.slot_size as usize;
        self.staging[start..start + uniform.len()].copy_from_slice(uniform);
    }

    /// The bind group and dynamic offset a draw must bind for `slot`.
    fn binding(&self, slot: usize) -> (&wgpu::BindGroup, u32) {
        let (page, offset) = slot_location(slot, self.slots_per_page, self.slot_size);
        (&self.pages[page].bind_group, offset)
    }

    /// Upload the first `used_slots` slots, one `write_buffer` per page, and
    /// return the number of calls issued (this frame's `uniform_writes`).
    fn flush(&self, queue: &wgpu::Queue, used_slots: usize) -> u64 {
        let mut writes = 0u64;
        let mut start = 0usize;
        while start < used_slots {
            let in_page = (used_slots - start).min(self.slots_per_page);
            let byte_offset = start * self.slot_size as usize;
            let byte_len = in_page * self.slot_size as usize;
            let page = start / self.slots_per_page;
            queue.write_buffer(
                &self.pages[page].buffer,
                0,
                &self.staging[byte_offset..byte_offset + byte_len],
            );
            writes += 1;
            start += in_page;
        }
        writes
    }
}

/// Outcome of [`Renderer::upload_images`]: how many image quads became resident
/// and which resources could not be resolved.
///
/// A resource that is absent from the [`DecodedImageCache`] (or whose decoded
/// buffer is malformed) is **never** replaced by a placeholder: it is skipped
/// and listed in [`ImageUploadReport::unresolved`] so the caller can surface
/// `image.unresolved`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageUploadReport {
    /// Image quads successfully uploaded and made resident this call.
    pub uploaded: usize,
    /// Resource keys that could not be resolved to a decodable image. Non-empty
    /// means part of the scene was not drawn; the caller must surface it.
    pub unresolved: Vec<ResourceKey>,
    /// Bytes uploaded for the resident quads: every quad's vertex + index
    /// buffers plus, once per unique resource, the padded RGBA texture bytes
    /// (`height` rows of `width * 4`, rounded up to the 256-byte copy
    /// alignment).
    pub bytes: u64,
}

impl ImageUploadReport {
    /// A structured diagnostic for the unresolved images, or `None` when every
    /// requested image resolved.
    ///
    /// The code is [`codes::RESOURCE_MISSING`] (the image's resource is not
    /// available); the caller maps it to its `image.unresolved` surface. This is
    /// a report, not a fabricated draw.
    pub fn diagnostic(&self) -> Option<DiagnosticReason> {
        if self.unresolved.is_empty() {
            return None;
        }
        let keys = self
            .unresolved
            .iter()
            .map(|key| key.as_str())
            .collect::<Vec<_>>()
            .join(";");
        Some(DiagnosticReason::missing(
            codes::RESOURCE_MISSING,
            vec![
                DiagnosticParameter::Count(self.unresolved.len() as u64),
                DiagnosticParameter::Key(keys),
            ],
        ))
    }
}

/// The widest 2D texture dimension a host can rely on.
pub const MIN_GUARANTEED_TEXTURE_DIMENSION: u32 = 2048;

/// Byte size of one packed instanced line segment on the GPU (2-vertex
/// `LineList` pair). Public so callers can reason about the exact instance-stream
/// byte charge without duplicating the layout.
///
/// This is the **universal** tier's stride. The accelerated tier packs the same
/// instance into a 64-byte `VERTEX | STORAGE` record (the extra 8 bytes are the
/// per-chunk origin index the pulled shader reads); see
/// `docs/adr/0007-entity-as-data-renderer.md`.
pub const LINE_INSTANCE_BYTES: u32 = plan::LINE_INSTANCE_STRIDE;

/// Bytes of one **legacy** anti-aliased line-quad vertex (the pre-expanded
/// per-batch path: two `vec3<f32>` endpoints plus a `vec2<f32>` quad corner).
/// Six such vertices make one segment's quad.
pub const LINE_QUAD_BYTES: u32 = plan::LINE_QUAD_STRIDE;

// Compile-time floor: mirrors `texture_dimension_floor_is_guaranteed`.
const _: () = assert!(MIN_GUARANTEED_TEXTURE_DIMENSION >= 2048);

/// The GPU objects a measurement-only timestamp bracket needs, created once and
/// reused across frames (no per-frame allocation of the measurement resources).
///
/// Created only by [`Renderer::set_gpu_time_measurement`] when the opt-in is
/// enabled and the device granted the feature set; otherwise no such resources
/// exist. The readback buffer is `MAP_READ`; it is unmapped before every frame so
/// the next frame's `copy_buffer_to_buffer` is valid.
pub(crate) struct GpuTimeResources {
    /// Two timestamp queries: index 0 before the frame's GPU work, index 1 after.
    pub(crate) query_set: wgpu::QuerySet,
    /// `QUERY_RESOLVE` target for `resolve_query_set` (256-byte aligned, ≥ 16 B).
    pub(crate) resolve: wgpu::Buffer,
    /// `MAP_READ` staging buffer the two resolved `u64` ticks are copied into.
    pub(crate) readback: wgpu::Buffer,
}

/// The CAD renderer. Owns only derived GPU resources.
pub struct Renderer {
    pub preference: BackendPreference,
    pub recovery_limit: u32,
    /// Per-frame vertex/triangle budget, enforced before submission.
    ///
    /// This is the **un-paced ceiling**. Adaptive pacing scales a copy of it per
    /// frame (see [`pacing`]); every consumer that deliberately ignores pacing
    /// (picking) reads this field directly.
    pub frame_budget: FrameBudget,
    /// Adaptive page-budget controller state (see [`pacing`]). Reset on scene
    /// mutation / `clear_batches` / a device-generation change; **never** on
    /// camera motion (cost knowledge is scene-specific, not view-specific).
    pacing: pacing::PacingState,
    /// Set when a scene mutation reset the controller; the next observed frame
    /// seeds the EWMA instead of adapting ([`pacing::Sample::restart`]).
    pacing_restart_pending: bool,
    /// Test hook: override the "a completion signal exists" decision used by
    /// [`Renderer::pacing_mode`]. `None` derives it from the target: the wasm
    /// non-blocking poll path has no completion signal, native does.
    pacing_signal_override: Option<bool>,
    /// Real activated backend. `None` until a device is initialized.
    active_backend: Option<ActiveBackend>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    /// Measurement-only timestamp resources (ADR 0013 class 1). `Some` only when
    /// the explicit opt-in was enabled **and** the device granted the feature
    /// set; `None` on the product configuration. Reused across frames (created
    /// once in [`Renderer::set_gpu_time_measurement`]); never a runtime
    /// dependency of the render path.
    gpu_time: Option<GpuTimeResources>,
    /// The explicit measurement state token ([`gpu_time::GpuTimeState`]) carried
    /// onto every [`FrameStats`]. `Disabled` until the opt-in is applied.
    gpu_time_state: gpu_time::GpuTimeState,
    /// Test hook: override the feature set `set_gpu_time_measurement` reads, so
    /// the `Unsupported(<feature>)` path is exercisable without faking a device
    /// that lacks `TIMESTAMP_QUERY`. `None` uses the live device's granted set.
    gpu_time_feature_override: Option<wgpu::Features>,
    /// Monotonic count of measurement-resource creations. It must stay `1` across
    /// frames once measurement is enabled (the query set / buffers are reused,
    /// never re-allocated per frame); diagnostics only.
    gpu_time_resource_creations: u64,
    layout: Option<wgpu::BindGroupLayout>,
    line_pipeline: Option<wgpu::RenderPipeline>,
    /// Instanced line pipeline: one instance per 2-vertex segment, expanded into
    /// an anti-aliased quad against [`Renderer::unit_quad_buffer`].
    line_instanced_pipeline: Option<wgpu::RenderPipeline>,
    /// SDF conic pipeline: one instance per analytic circle/arc, expanded against
    /// [`Renderer::unit_quad_buffer`].
    arc_pipeline: Option<wgpu::RenderPipeline>,
    /// Shared unit quad `[0,1]^2` (6 vertices, `TriangleList`) bound at slot 0 of
    /// the SDF conic **and** the anti-aliased instanced line pipelines.
    unit_quad_buffer: Option<wgpu::Buffer>,
    mesh_pipeline: Option<wgpu::RenderPipeline>,
    /// Mirrored mesh pipeline: same shader but `front_face = Cw`, so mirrored
    /// batches cull the correct side instead of the visible one.
    mesh_pipeline_mirrored: Option<wgpu::RenderPipeline>,
    /// Transparent (`0 < alpha < 1`) mesh pipelines: same shaders, but depth
    /// writes are disabled so a back-to-front transparent pass does not occlude
    /// the geometry behind it with its own depth.
    mesh_pipeline_transparent: Option<wgpu::RenderPipeline>,
    mesh_pipeline_mirrored_transparent: Option<wgpu::RenderPipeline>,
    /// Merged-mesh pipelines: the flat-merge vertex layout (interleaved
    /// position/normal/baked-colour/entity) and the same shading body as the
    /// mesh pipelines. `_mirrored` uses `front_face = Cw`, like the legacy pair.
    mesh_pipeline_merged: Option<wgpu::RenderPipeline>,
    mesh_pipeline_merged_mirrored: Option<wgpu::RenderPipeline>,
    /// Textured-quad pipeline for raster images (group 2 = texture + sampler).
    image_pipeline: Option<wgpu::RenderPipeline>,
    /// Bind-group layout for group 2 of the image pipeline.
    image_texture_layout: Option<wgpu::BindGroupLayout>,
    /// Shared linear, clamp-to-edge sampler for every uploaded image.
    image_sampler: Option<wgpu::Sampler>,
    target: Option<wgpu::Texture>,
    target_view: Option<wgpu::TextureView>,
    depth_view: Option<wgpu::TextureView>,
    target_size: (u32, u32),
    target_format: wgpu::TextureFormat,
    batches: Vec<GpuBatch>,
    /// Device-resident instanced line streams. Uploaded once per scene change;
    /// never rewritten on pan/zoom. Garbage-collected (and re-based) on commit.
    instance_streams: Vec<InstanceStream>,
    /// Device-resident SDF conic streams. Uploaded once per scene change; never
    /// rewritten on pan/zoom. Garbage-collected on commit. Universal-tier only.
    arc_streams: Vec<ArcStream>,
    /// Device-resident merged mesh chunks. Uploaded once per scene change; never
    /// rewritten on pan/zoom. Garbage-collected (and re-based) on commit.
    mesh_chunks: Vec<GpuMeshChunk>,
    /// Whether eligible opaque line batches are converted to instanced streams.
    /// Defaults to `true`; disabling forces the legacy per-batch path so the two
    /// can be compared.
    line_instancing: bool,
    /// Whether eligible opaque mesh batches are flat-merged into chunks and their
    /// edge overlays instanced. Defaults to `true`; disabling forces the legacy
    /// per-batch mesh path so the two can be compared.
    mesh_merging: bool,
    /// Upper bound on instances per instanced stream buffer (default
    /// [`plan::MAX_INSTANCES_PER_STREAM`]). A batch with more segments is split
    /// across several streams. Injectable so the split path is testable without
    /// allocating a million-segment batch.
    max_instances_per_stream: usize,
    /// Upper bound on vertices per merged mesh chunk (default
    /// [`plan::MAX_VERTICES_PER_MESH_STREAM`]). A batch with more vertices is
    /// split across several chunks. Injectable for the same reason.
    max_vertices_per_mesh_stream: usize,
    /// Monotonic count of instanced stream buffers uploaded since init. A pan or
    /// zoom must never increase it (asserted by the zero-reupload test).
    instance_stream_uploads: u64,
    /// Monotonic count of SDF conic stream buffers uploaded since init. A pan or
    /// zoom must never increase it (asserted by the zero-reupload test).
    arc_stream_uploads: u64,
    /// Monotonic count of degenerate conics skipped at pack (a defensive guard
    /// the provider already excludes). Counted, never silent (iron rule #2).
    arc_degenerate_skips: u64,
    /// Monotonic count of trailing unpaired vertices dropped from odd-length
    /// `Lines`/`MeshEdges` batches. Counted, never silent (iron rule #2).
    line_odd_vertex_skips: u64,
    /// Monotonic count of merged mesh chunk buffers uploaded since init. A pan or
    /// zoom must never increase it (asserted by the zero-reupload test).
    mesh_chunk_uploads: u64,
    /// Live draw tier (see [`RenderTier`]), chosen once per device in
    /// `initialize_with_device`.
    render_tier: RenderTier,
    /// Test hook: force storage-buffer support on/off for the tier decision
    /// before a device is initialized, so the automatic downgrade is testable
    /// without a WebGL2 device.
    storage_support_override: Option<bool>,
    /// Accelerated whole-scene line storage; `None` on the universal tier or
    /// when no instanced lines are resident.
    pulled_lines: Option<PulledLines>,
    /// Accelerated whole-scene merged-mesh storage.
    pulled_mesh: Option<PulledMesh>,
    /// Test hook: override the storage-buffer binding limit `rebuild_pulled`
    /// sizes its segments against, so the reduced-limit regime (oversize stream,
    /// clean downgrade) is exercisable without a low-limit device. `None` uses
    /// the device's real limit.
    pulled_binding_limit_override: Option<u64>,
    /// Test hook: override the maximum instances per accelerated line storage
    /// segment, so the multi-segment path is exercisable on a small scene.
    /// `None` uses the binding-derived capacity.
    pulled_line_segment_elems_override: Option<usize>,
    /// Test hook: override the memory budget the pre-flight pulled capacity gate
    /// compares its requirement against. `None` derives the budget from the
    /// device limits (see [`Renderer::pulled_budget_bytes`]).
    pulled_memory_budget_override: Option<u64>,
    /// Test hook (test builds only): when set, the pulled rebuild treats the
    /// allocation error scope as if it had captured this out-of-memory message,
    /// so the capture → discard → universal-fallback path is exercisable without
    /// provoking a real driver OOM (which no portable test can do
    /// deterministically).
    #[cfg(test)]
    pulled_oom_injection: Option<&'static str>,
    /// Observable state of the accelerated pulled organisation (see
    /// [`PulledState`]), updated by every `rebuild_pulled`.
    pulled_state: PulledState,
    /// Test hook: override the anti-aliased line SDF ramp width (world units per
    /// pixel) for the line class only. `None` uses the projection's real value.
    /// Used by the visual-acceptance test to render the **pre-unification**
    /// hard-aliased line look (a sub-pixel ramp) as a genuine GPU "before".
    line_ramp_override: Option<f32>,
    /// Test hook: disable chunk-level view culling for the frame so a test can
    /// render the *unculled* reference and compare it to the culled frame. The
    /// product always culls (`false`).
    cull_disabled: bool,
    /// Group-0 layout/uniform/bind group of the pulled pipelines: the three
    /// origin-independent transform columns shared by every chunk.
    pulled_camera_layout: Option<wgpu::BindGroupLayout>,
    pulled_camera_buffer: Option<wgpu::Buffer>,
    pulled_camera_bind_group: Option<wgpu::BindGroup>,
    /// Group-2 storage layouts of the pulled pipelines.
    pulled_line_storage_layout: Option<wgpu::BindGroupLayout>,
    pulled_mesh_storage_layout: Option<wgpu::BindGroupLayout>,
    /// The accelerated pipelines (only created on the accelerated tier).
    line_pulled_pipeline: Option<wgpu::RenderPipeline>,
    mesh_pulled_pipeline: Option<wgpu::RenderPipeline>,
    mesh_pulled_mirrored_pipeline: Option<wgpu::RenderPipeline>,
    /// Resident image quads uploaded by [`Renderer::upload_images`].
    images: Vec<GpuImage>,
    /// One GPU texture per resolved [`ResourceKey`], shared by every resident
    /// image quad that references it. Bounded to the keys of the current image
    /// set (unreferenced entries are pruned on upload).
    image_textures: HashMap<ResourceKey, GpuTexture>,
    /// Shared per-frame uniform arena (one slot per accepted batch/image).
    uniforms: UniformArena,
    /// Per-batch uniform slot assigned this frame, in accepted order.
    /// `u32::MAX` marks a batch not drawn this frame; only drawn indices are
    /// read. Reused across frames to avoid a per-frame allocation.
    frame_slots: Vec<u32>,
    /// Per-image uniform slot assigned this frame (images share the geometry
    /// arena and draw only on the cleared page).
    image_slots: Vec<u32>,
    /// Scratch buffers for this frame's opaque/transparent accepted indices.
    /// Taken by `render_with_projection` and returned before it finishes, so the
    /// per-frame `to_vec`/`collect` allocations are avoided across frames.
    scratch_opaque: Vec<usize>,
    scratch_transparent: Vec<usize>,
    /// Cached "lineweight carried but not drawn" report over `batches`.
    ///
    /// Rebuilt only when the batch set changes (`commit_upload`/`clear_batches`/
    /// device loss), never per frame: the frame path clones it instead of
    /// scanning every batch.
    lineweight_not_drawn: Vec<LineweightNotDrawn>,
    device_generation: u64,
    texture_revision: u64,
    uploaded_bytes: u64,
    /// Wall-clock ms of the last `upload`, or `None` before the first upload.
    /// Real measured upload time for `LoadTimings::upload_ms`.
    last_upload_ms: Option<f64>,
    /// Microsecond substage timings of the last `commit_upload` (diagnostics only).
    last_commit_timing: Option<CommitTiming>,
    /// Retained pulled-buffer prefix checkpoint (accelerated tier; see
    /// [`PulledPrefixCheckpoint`]).
    pulled_prefix_checkpoint: Option<PulledPrefixCheckpoint>,
    /// Pulled-cache epoch: bumped on every invalidation so an admission can
    /// reject a checkpoint built before the invalidation.
    pulled_cache_epoch: u64,
    /// Diagnostics: suffix commits that reused the retained pulled base.
    pulled_prefix_reuses: u64,
    /// Diagnostics: full pulled rebuilds that (re)established a checkpoint.
    pulled_prefix_rebuilds: u64,
    draw_calls: u64,
    progressive: bool,
    progressive_frame: Option<ProgressiveFrame>,
    /// Camera-independent draw-order partition over `batches`. Rebuilt lazily
    /// when the batch set changes (upload/clear/device-loss) and, per frame,
    /// only its transparent slice is re-sorted for the camera. `None` means
    /// "stale, rebuild from `batches`".
    draw_plan: Option<DrawOrderCache>,
    /// Batch-set facts for the segmented page planner (per-class chunk→batch
    /// lists, the always-active set) plus the once-per-batch-set class flags.
    /// Rebuilt in the **same** block that rebuilds `draw_plan`, so it shares that
    /// single invalidation (`draw_plan = None`) and is never a second truth.
    batch_facts: plan::BatchFacts,
    /// This frame's set of batches that participate in the encode pass (own no
    /// runs, or own a run that survives the cull). Rebuilt per frame from
    /// `batch_facts` and the frame cull; used to skip provably-non-emitting
    /// accepted entries in the uniform and encode loops.
    encode_emit_mask: plan::EncodeEmitMask,
    /// Set once a device-loss is observed, until the host rebuilds.
    device_lost: bool,
    /// Detail string carried by the last device-loss observation.
    last_device_lost: Option<String>,
    /// Bounded wait applied to a submitted frame before classification.
    ///
    /// Defaults to 1 s, which suits interactive native hosts. A software
    /// adapter (Mesa lavapipe) can legitimately spend longer than that on a
    /// large frame, so a headless caller raises it explicitly instead of the
    /// renderer misreporting a slow CPU frame as a device loss.
    poll_timeout: std::time::Duration,
    /// Monotonic allocator for renderer pick keys within the current device
    /// generation (see [`PickTable`]). Reset when the resident batch list is
    /// emptied, so a full re-upload starts again from key 1.
    pick_key_counter: std::sync::atomic::AtomicU32,
    /// CPU-side key → identity table for the resident batches. Rebuilt on every
    /// commit/clear; read by `pick_at`.
    pick_table: PickTable,
    /// The projection of the most recent color frame. `pick_at` reuses it so the
    /// id pass draws the same view the user is looking at.
    last_projection: Option<FrameProjection>,
    /// Camera-only world→clip matrix of the most recent color frame (the
    /// transform image vertices use; they are already in world space).
    last_camera_matrix: [f32; 16],
    /// Camera position of the most recent color frame (transparent ordering).
    last_camera_position: Option<[f32; 3]>,
    /// Active id-target format (see [`IdTargetFormat`]).
    id_format: IdTargetFormat,
    /// Test hook: forces a format regardless of the adapter probe.
    id_format_override: Option<IdTargetFormat>,
    /// Offscreen id target: same size as the color target, own depth buffer, so
    /// a pick never disturbs the color pass's attachments.
    pick_target: Option<wgpu::Texture>,
    pick_view: Option<wgpu::TextureView>,
    pick_depth_view: Option<wgpu::TextureView>,
    pick_size: (u32, u32),
    /// Id-pass pipelines. One per vertex layout/primitive state the color pass
    /// uses (the id pass must rasterize the same fragments).
    pick_line_pipeline: Option<wgpu::RenderPipeline>,
    pick_line_instanced_pipeline: Option<wgpu::RenderPipeline>,
    pick_arc_pipeline: Option<wgpu::RenderPipeline>,
    pick_mesh_pipeline: Option<wgpu::RenderPipeline>,
    pick_mesh_pipeline_mirrored: Option<wgpu::RenderPipeline>,
    pick_mesh_merged_pipeline: Option<wgpu::RenderPipeline>,
    pick_mesh_merged_pipeline_mirrored: Option<wgpu::RenderPipeline>,
    pick_image_pipeline: Option<wgpu::RenderPipeline>,
    /// Group-1 layout of the selection-flags table (`texture_2d<u32>` + control
    /// uniform), shared by every color pipeline.
    flags_layout: Option<wgpu::BindGroupLayout>,
    flags_texture: Option<wgpu::Texture>,
    flags_view: Option<wgpu::TextureView>,
    flags_bind_group: Option<wgpu::BindGroup>,
    /// 16-byte control uniform: `enabled` + the texture row width.
    flags_ctl_buffer: Option<wgpu::Buffer>,
    /// Whether the shaders apply the flags tint. Off by default so the existing
    /// highlight-overlay batches keep working unchanged.
    flags_enabled: bool,
    /// CPU mirror of the flags table, indexed by pick key.
    flags_bits: Vec<u32>,
    /// Texels per row of the flags texture (see `FlagsCtl::width`).
    flags_width: u32,
    /// Set when the GPU table must be re-uploaded from the CPU mirror (table
    /// recreation or a scene reset) so a retired key's bits cannot linger.
    flags_table_dirty: bool,
    /// Bytes uploaded by flags updates since init. A flags update must never
    /// grow this and the static stream counters together.
    flags_upload_bytes: u64,
    /// Number of flags updates that reached the GPU.
    flags_upload_count: u64,
    /// Keys whose bits were last written by [`Renderer::set_entity_flags`].
    ///
    /// Used to diff an entity-flag update so a selection change only uploads the
    /// rows whose bits actually changed (and clears the rows that fell out of the
    /// selection). `set_pick_key_flags` does not touch this set, so a manual
    /// per-key write is never clobbered by a later entity-flag diff unless the
    /// same key is in the entity set.
    entity_flag_keys: std::collections::BTreeSet<u32>,
    /// The last entity-flag request, retained so a pick-table rebuild (a base
    /// change or an image re-upload renumbers keys) re-resolves the flags against
    /// the new keys instead of leaving them on retired ones. Cleared by a scene
    /// reset (`reset_pick_keys`).
    entity_flags_request: Vec<(SelectionRef, u32)>,
}

/// Staged GPU resources, owned by the renderer that prepared them. Publishing
/// is separate so failed preparation leaves the active batches untouched.
pub struct PreparedUpload {
    batches: Vec<GpuBatch>,
    /// Instanced line streams created for the staged batches; their run indices
    /// are re-based when the upload is published.
    instances: Vec<InstanceStream>,
    /// SDF conic streams created for the staged batches; their run indices are
    /// re-based when the upload is published.
    arcs: Vec<ArcStream>,
    /// Degenerate conics skipped at pack in this upload (counted, never silent).
    arc_degenerate_skips: u64,
    /// Trailing unpaired vertices dropped from odd-length `Lines`/`MeshEdges`
    /// batches in this upload (counted, never silent).
    line_odd_vertex_skips: u64,
    /// Merged mesh chunks created for the staged batches; their run indices are
    /// re-based when the upload is published.
    mesh_chunks: Vec<GpuMeshChunk>,
    device_generation: u64,
    device: wgpu::Device,
    bytes: u64,
    elapsed_ms: f64,
}

struct ProgressiveFrame {
    /// O(1) camera/attachment key the cached `ordered` plan is valid for.
    key: FrameKey,
    ordered: Vec<usize>,
    opaque_count: usize,
    invisible: usize,
    cursor: usize,
    /// Per-`ordered` index (batch→position inverse, byte prefix sums) for the
    /// segmented page planner. Built in the same restart sweep that rebuilds
    /// `ordered`, so it is derived from that one truth, never a second one.
    page_index: plan::OrderedPageIndex,
}

/// O(1)-comparable identity of the camera + attachments a cached frame plan was
/// built for.
///
/// The per-batch transform is a pure function of the camera and the batch
/// origins, and batch origins only change on upload/clear/device-loss — all of
/// which reset `progressive_frame`. So comparing this key is equivalent to the
/// previous full-matrix `transforms` comparison, without materialising or
/// comparing a per-batch matrix.
#[derive(Clone, Copy, PartialEq)]
struct FrameKey {
    camera_matrix: [f32; 16],
    camera_position: [f32; 3],
    target_size: (u32, u32),
    texture_revision: u64,
}

impl PreparedUpload {
    /// Append another staged slice from the same device generation. No active
    /// scene is mutated, and stale-device pieces cannot be mixed.
    pub fn append(&mut self, mut piece: Self) -> CadResult<()> {
        if self.device != piece.device || self.device_generation != piece.device_generation {
            return Err(CadError::StaleResult);
        }
        // Re-base the appended piece's stream indices onto this piece's streams.
        let base = self.instances.len() as u32;
        for batch in &mut piece.batches {
            for run in &mut batch.instanced {
                run.stream += base;
            }
        }
        // Re-base the appended piece's merged chunk indices the same way.
        let mesh_base = self.mesh_chunks.len() as u32;
        for batch in &mut piece.batches {
            for run in &mut batch.mesh_runs {
                run.chunk += mesh_base;
            }
        }
        // Re-base the appended piece's conic stream indices.
        let arc_base = self.arcs.len() as u32;
        for batch in &mut piece.batches {
            for run in &mut batch.arc_runs {
                run.stream += arc_base;
            }
        }
        self.batches.append(&mut piece.batches);
        self.instances.append(&mut piece.instances);
        self.arcs.append(&mut piece.arcs);
        self.arc_degenerate_skips += piece.arc_degenerate_skips;
        self.mesh_chunks.append(&mut piece.mesh_chunks);
        self.bytes += piece.bytes;
        self.elapsed_ms += piece.elapsed_ms;
        Ok(())
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Renderer::new(BackendPreference::Auto)
    }
}

/// Chunk-level view culling (backlog §P1): pure predicate over the retained
/// per-chunk world AABBs. Kept separate from the planners so it is unit-testable
/// without a GPU.
mod cull;
/// Native id-pass + readback (`Renderer::pick_at`). The browser tiers keep CPU
/// picking (`cad_spatial`); a blocking GPU→CPU readback is not available there.
#[cfg(not(target_arch = "wasm32"))]
mod pick;
mod plan;
mod renderer_impl;

pub(crate) use cull::{ChunkCull, WorldAabb};
pub(crate) use plan::*;

// Private accelerated-tier GPU contracts for the pulled-prefix fast path. Test
// only (it reads private renderer state and needs a real device); native only,
// like `headless`.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod pulled_budget_gpu_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod pulled_prefix_gpu_tests;
#[cfg(test)]
mod tests;
