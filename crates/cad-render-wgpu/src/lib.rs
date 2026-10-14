//! wgpu pipelines over CPU scene batches.
//!
//! Spec v2.0 §5.2, §8: the renderer does not own the window or the device. The
//! host (Slint) provides a shared `Device`/`Queue`; CAD draws into a texture the
//! host composites. Camera navigation updates only small uniform buffers, never
//! the static vertex buffers.
//!
//! Two pipelines are provided: a `LineList` pipeline for polylines and a
//! `TriangleList` pipeline for mesh geometry with a depth buffer, back-face
//! culling and per-fragment shading (audit F14). The pure geometry decisions
//! live in [`geometry`] and are unit tested without a GPU.

pub use wgpu;

pub mod geometry;

/// Native headless path: create a software device, render offscreen, read the
/// target back and encode PNG. Native only; the browser path gets its device
/// from the host canvas instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod headless;

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

pub struct FrameStats {
    /// Wall-clock milliseconds of this frame's CPU path on the render thread:
    /// scene plan/cache lookup, per-batch uniform writes, command encoding and
    /// `queue.submit`. It deliberately **excludes** the blocking device poll, so
    /// it does not fold GPU execution time into a CPU number.
    pub cpu_ms: f64,
    pub gpu_ms: Option<f64>,
    /// Draw submissions this frame, derived from the plan's batch/run accounting
    /// (non-converted opaque batches + coalesced instanced runs + coalesced
    /// merged-mesh runs + transparent batches + drawn images) rather than by
    /// intercepting every `draw`/`draw_indexed` call. It is a batch-collapse
    /// metric: a converted batch is represented by the coalesced runs it joins,
    /// and each run contributes one entry however many member batches it covers.
    pub draw_calls: u64,
    /// Number of `Queue::write_buffer` calls issued for this frame's uniforms.
    ///
    /// Every accepted batch (and every drawn image) writes its 80-byte uniform
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
    pub over_budget: Option<OverBudgetReason>,
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
    /// Line index buffer over the shared mesh vertices (wireframe only).
    edge_indices: Option<wgpu::Buffer>,
    edge_index_count: u32,
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
    /// Exact bytes this batch uploads, cached at upload time so the per-frame
    /// byte budget does not recompute the edge index list every frame.
    upload_bytes: usize,
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
    mirrored: bool,
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
    /// Group 1 bind group (texture view + shared sampler), shared by every quad
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
    /// Group 1: the sampled texture and its sampler (shared per resource key).
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
/// Every accepted geometry batch (and drawn image) writes its 80-byte uniform
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
pub const LINE_INSTANCE_BYTES: u32 = plan::LINE_INSTANCE_STRIDE;

// Compile-time floor: mirrors `texture_dimension_floor_is_guaranteed`.
const _: () = assert!(MIN_GUARANTEED_TEXTURE_DIMENSION >= 2048);

/// The CAD renderer. Owns only derived GPU resources.
pub struct Renderer {
    pub preference: BackendPreference,
    pub recovery_limit: u32,
    /// Per-frame vertex/triangle budget, enforced before submission.
    pub frame_budget: FrameBudget,
    /// Real activated backend. `None` until a device is initialized.
    active_backend: Option<ActiveBackend>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    layout: Option<wgpu::BindGroupLayout>,
    line_pipeline: Option<wgpu::RenderPipeline>,
    /// Instanced line pipeline: one instance per 2-vertex `LineList` segment,
    /// expanded against `unit_segment_buffer`.
    line_instanced_pipeline: Option<wgpu::RenderPipeline>,
    /// Shared `(0,0,0)-(1,0,0)` unit segment bound at slot 0 of the instanced
    /// line pipeline.
    unit_segment_buffer: Option<wgpu::Buffer>,
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
    /// Textured-quad pipeline for raster images (group 1 = texture + sampler).
    image_pipeline: Option<wgpu::RenderPipeline>,
    /// Bind-group layout for group 1 of the image pipeline.
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
    /// Monotonic count of merged mesh chunk buffers uploaded since init. A pan or
    /// zoom must never increase it (asserted by the zero-reupload test).
    mesh_chunk_uploads: u64,
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
    draw_calls: u64,
    progressive: bool,
    progressive_frame: Option<ProgressiveFrame>,
    /// Camera-independent draw-order partition over `batches`. Rebuilt lazily
    /// when the batch set changes (upload/clear/device-loss) and, per frame,
    /// only its transparent slice is re-sorted for the camera. `None` means
    /// "stale, rebuild from `batches`".
    draw_plan: Option<DrawOrderCache>,
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
}

/// Staged GPU resources, owned by the renderer that prepared them. Publishing
/// is separate so failed preparation leaves the active batches untouched.
pub struct PreparedUpload {
    batches: Vec<GpuBatch>,
    /// Instanced line streams created for the staged batches; their run indices
    /// are re-based when the upload is published.
    instances: Vec<InstanceStream>,
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
        self.batches.append(&mut piece.batches);
        self.instances.append(&mut piece.instances);
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

mod plan;
mod renderer_impl;

pub(crate) use plan::*;

#[cfg(test)]
mod tests;
