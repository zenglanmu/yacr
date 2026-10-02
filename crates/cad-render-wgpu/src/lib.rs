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

use cad_diagnostics::codes;
use cad_diagnostics::{DiagnosticParameter, DiagnosticReason};
use cad_domain::*;
use cad_scene::{FrameBudget, RenderTopology, SceneDelta};
use geometry::OverBudget;
pub use geometry::{
    clamp_alpha, classify_alpha, plan_draw_order, AlphaClass, BatchOrderEntry, DrawOrderPlan,
};
pub use geometry::{
    front_face_ccw, has_vertex_colors, normals_need_repair, repaired_colors, repaired_normals,
    winding_is_flipped,
};
pub use geometry::{Camera2d, Camera3d};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPreference {
    Auto,
    WebGpu,
    WebGl2,
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
    pub cpu_ms: f64,
    pub gpu_ms: Option<f64>,
    pub draw_calls: u64,
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
    vertices: wgpu::Buffer,
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
    camera: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// The widest 2D texture dimension a host can rely on.
pub const MIN_GUARANTEED_TEXTURE_DIMENSION: u32 = 2048;

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
    mesh_pipeline: Option<wgpu::RenderPipeline>,
    /// Mirrored mesh pipeline: same shader but `front_face = Cw`, so mirrored
    /// batches cull the correct side instead of the visible one.
    mesh_pipeline_mirrored: Option<wgpu::RenderPipeline>,
    /// Transparent (`0 < alpha < 1`) mesh pipelines: same shaders, but depth
    /// writes are disabled so a back-to-front transparent pass does not occlude
    /// the geometry behind it with its own depth.
    mesh_pipeline_transparent: Option<wgpu::RenderPipeline>,
    mesh_pipeline_mirrored_transparent: Option<wgpu::RenderPipeline>,
    target: Option<wgpu::Texture>,
    target_view: Option<wgpu::TextureView>,
    depth_view: Option<wgpu::TextureView>,
    target_size: (u32, u32),
    target_format: wgpu::TextureFormat,
    batches: Vec<GpuBatch>,
    device_generation: u64,
    uploaded_bytes: u64,
    draw_calls: u64,
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
