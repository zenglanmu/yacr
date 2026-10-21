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
use cad_scene::{FrameBudget, ImageBatch, MaskBatch, RenderTopology, SceneDelta};
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
    /// World-space z range `[min, max]` of this batch's geometry, captured at
    /// upload. The orthographic frame derives its depth band from the union over
    /// resident batches (see [`plan::OrthoDepth`]), so a solid's world-z extent
    /// maps into the depth range instead of being clipped by it.
    z_range: [f32; 2],
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

/// One resident WIPEOUT mask: a tessellated fill plus the `f64` reference origin
/// its stored positions are measured from.
///
/// A mask is **not** a [`GpuBatch`]: it is painted by the frame's final pass with
/// `depth_compare: Always`, no depth write, and the clear colour (ADR 0022 D1),
/// so it must never enter the ordered geometry plan. The fill is tessellated with
/// the same even-odd fill solid HATCH uses (`cad_domain::hatch::fill_rings`), so a
/// concave boundary does not leak.
///
/// A non-`inverted` mask is filled at upload from its boundary alone. An
/// `inverted` mask fills the **complement**, which needs a ring covering the
/// frame's visible world rect: its geometry is therefore built by the first frame
/// that draws it (see `ensure_inverted_mask_fill`) and rebuilt only when the view
/// leaves the resident ring.
pub(crate) struct GpuMask {
    /// Triangle list positions, relative to [`GpuMask::origin`]. `None` until an
    /// inverted mask's first frame has built its complement fill.
    vertices: Option<wgpu::Buffer>,
    vertex_count: u32,
    origin: [f64; 3],
    /// The untransformed world boundary, retained for the inverted fill (the ring
    /// depends on the view, so it cannot be baked once).
    polygon: Arc<[Point3]>,
    /// `true` for clip mode Inside: mask everything outside `polygon`.
    inverted: bool,
    /// World-XY rectangle the resident inverted fill guarantees to cover. `None`
    /// for a non-inverted mask or before the first inverted fill.
    covered: Option<MaskCoverage>,
}

/// An axis-aligned world-XY rectangle.
///
/// Used to describe both the visible world rect of a frame and the region an
/// inverted mask's complement fill covers. `min`/`max` are per-axis bounds in
/// world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MaskCoverage {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl MaskCoverage {
    pub(crate) fn new(min: [f64; 2], max: [f64; 2]) -> Self {
        Self { min, max }
    }

    /// A square of half-extent `radius` centred on `center` (world XY).
    pub(crate) fn around(center: [f64; 2], radius: f64) -> Self {
        Self {
            min: [center[0] - radius, center[1] - radius],
            max: [center[0] + radius, center[1] + radius],
        }
    }

    /// Axis-aligned bounds of `boundary`; a square of half-extent 1 at the origin
    /// when the boundary has no finite extent (never used to clip, only to grow a
    /// ring, so the fallback is harmless and non-finite values cannot escape).
    pub(crate) fn boundary(boundary: &[Point3]) -> Self {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in boundary {
            if !(point.x.is_finite() && point.y.is_finite()) {
                return Self::around([0.0, 0.0], 1.0);
            }
            min[0] = min[0].min(point.x);
            min[1] = min[1].min(point.y);
            max[0] = max[0].max(point.x);
            max[1] = max[1].max(point.y);
        }
        if min[0] > max[0] || min[1] > max[1] {
            return Self::around([0.0, 0.0], 1.0);
        }
        Self { min, max }
    }

    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    /// Whether `other` lies entirely inside `self` (inclusive bounds).
    pub(crate) fn contains(self, other: Self) -> bool {
        self.min[0] <= other.min[0]
            && self.min[1] <= other.min[1]
            && self.max[0] >= other.max[0]
            && self.max[1] >= other.max[1]
    }

    pub(crate) fn expanded(self, margin: f64) -> Self {
        Self {
            min: [self.min[0] - margin, self.min[1] - margin],
            max: [self.max[0] + margin, self.max[1] + margin],
        }
    }

    pub(crate) fn is_finite(self) -> bool {
        [self.min[0], self.min[1], self.max[0], self.max[1]]
            .iter()
            .all(|value| value.is_finite())
    }
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
    /// Adaptive page-budget controller state (see [`pacing`]). Reset on a
    /// **base-changing** scene mutation (`keep_prefix == 0`), `clear_batches` and
    /// a device-generation change; **never** on camera motion or a suffix-only
    /// overlay commit (cost knowledge is scene-specific, not view-specific, and a
    /// suffix commit does not change the base draw-cost profile).
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
    /// Wipeout mask pipeline: triangle list, `depth_compare = Always`, no depth
    /// write, fragment output = the clear colour (ADR 0022 C1). Drawn in the
    /// frame's final pass.
    mask_pipeline: Option<wgpu::RenderPipeline>,
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
    /// Resident WIPEOUT masks uploaded by [`Renderer::upload_masks`]. Painted by
    /// the frame's final pass, in this order (insertion = scene build order).
    masks: Vec<GpuMask>,
    /// Mask boundaries of the last [`Renderer::upload_masks`] call that could not
    /// be tessellated, with the named reason. Never silent: a non-empty list means
    /// a mask was not drawn.
    mask_skips: Vec<String>,
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
    /// Diagnostic: cumulative **full** pick-table rebuilds (never a behaviour).
    /// An append/suffix commit that keeps the resident prefix must not bump this
    /// (see [`Renderer::pick_table_full_rebuilds`]).
    pick_table_full_rebuilds: u64,
    /// Diagnostic: cumulative incremental pick-table suffix replacements.
    pick_table_incremental_extends: u64,
    /// The projection of the most recent color frame. `pick_at` reuses it so the
    /// id pass draws the same view the user is looking at.
    last_projection: Option<FrameProjection>,
    /// Camera-only world→clip matrix of the most recent color frame (the
    /// transform image vertices use; they are already in world space).
    last_camera_matrix: [f32; 16],
    /// Camera position of the most recent color frame (transparent ordering).
    last_camera_position: Option<[f32; 3]>,
    /// Light vector (`rgb` = unit toward the light, world space) of the most
    /// recent color frame, so the id pass writes the same uniform layout.
    last_light: [f32; 4],
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

mod api_types;
mod gpu_image;
mod renderer_state;
mod uniform_arena;

pub use api_types::*;
pub(crate) use gpu_image::*;
pub use renderer_state::*;
pub(crate) use uniform_arena::*;
