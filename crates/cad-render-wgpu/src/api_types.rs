use super::*;

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
    /// Every accepted batch (and every drawn image) writes its uniform
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
    /// Full draw-order interleaving with geometry batches is **not**
    /// implemented; images are drawn last, in ascending `draw_order` among
    /// themselves (see [`Renderer::upload_images`]).
    pub image_batches: usize,
    /// Wipeout masks submitted this frame by the final mask pass.
    ///
    /// The mask pass runs after images with `depth_compare: Always`, no depth
    /// write and the clear colour (ADR 0022 C1). Masks do not interleave with
    /// geometry draw order in this slice.
    pub mask_draws: u64,
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
