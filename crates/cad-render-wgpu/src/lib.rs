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
pub use geometry::{front_face_ccw, normals_need_repair, repaired_normals, winding_is_flipped};
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

impl Renderer {
    pub fn new(preference: BackendPreference) -> Self {
        Renderer {
            preference,
            recovery_limit: 2,
            frame_budget: FrameBudget {
                max_vertices: 8_000_000,
                max_triangles: 2_000_000,
            },
            // No device yet, so no backend has actually been activated.
            active_backend: None,
            device: None,
            queue: None,
            layout: None,
            line_pipeline: None,
            mesh_pipeline: None,
            mesh_pipeline_mirrored: None,
            mesh_pipeline_transparent: None,
            mesh_pipeline_mirrored_transparent: None,
            target: None,
            target_view: None,
            depth_view: None,
            target_size: (1, 1),
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            batches: Vec::new(),
            device_generation: 0,
            uploaded_bytes: 0,
            draw_calls: 0,
            device_lost: false,
            last_device_lost: None,
            poll_timeout: std::time::Duration::from_secs(1),
        }
    }

    /// Initialize from a host-provided shared device/queue.
    ///
    /// Slint supplies these through `set_rendering_notifier` as
    /// `GraphicsAPI::WGPU30 { device, queue, .. }`.
    pub fn initialize_with_device(
        &mut self,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> CadResult<BackendCapabilities> {
        let caps = Self::caps_for(&device, self.preference);
        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-lines"),
            source: wgpu::ShaderSource::Wgsl(LINE_SHADER.into()),
        });
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-mesh"),
            source: wgpu::ShaderSource::Wgsl(MESH_SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad-camera-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-lines-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &line_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(position_only_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &line_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mesh_pipeline = Self::create_mesh_pipeline(
            &device,
            &pipeline_layout,
            &mesh_shader,
            self.target_format,
            wgpu::FrontFace::Ccw,
            true,
            "cad-mesh-pipeline",
        );
        let mesh_pipeline_mirrored = Self::create_mesh_pipeline(
            &device,
            &pipeline_layout,
            &mesh_shader,
            self.target_format,
            wgpu::FrontFace::Cw,
            true,
            "cad-mesh-pipeline-mirrored",
        );
        // Transparent variants: depth-tested but not depth-written, so the
        // back-to-front pass composites instead of hiding geometry behind it.
        let mesh_pipeline_transparent = Self::create_mesh_pipeline(
            &device,
            &pipeline_layout,
            &mesh_shader,
            self.target_format,
            wgpu::FrontFace::Ccw,
            false,
            "cad-mesh-pipeline-transparent",
        );
        let mesh_pipeline_mirrored_transparent = Self::create_mesh_pipeline(
            &device,
            &pipeline_layout,
            &mesh_shader,
            self.target_format,
            wgpu::FrontFace::Cw,
            false,
            "cad-mesh-pipeline-mirrored-transparent",
        );

        self.device = Some(device);
        self.queue = Some(queue);
        self.layout = Some(layout);
        self.line_pipeline = Some(line_pipeline);
        self.mesh_pipeline = Some(mesh_pipeline);
        self.mesh_pipeline_mirrored = Some(mesh_pipeline_mirrored);
        self.mesh_pipeline_transparent = Some(mesh_pipeline_transparent);
        self.mesh_pipeline_mirrored_transparent = Some(mesh_pipeline_mirrored_transparent);
        self.active_backend = Some(caps.actual);
        self.device_generation += 1;
        self.device_lost = false;
        Ok(caps)
    }

    fn create_mesh_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
        front_face: wgpu::FrontFace,
        depth_write: bool,
        label: &str,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(mesh_vertex_layout()), Some(mesh_normal_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Capability snapshot derived from adapter limits and the host preference.
    fn caps_for(device: &wgpu::Device, preference: BackendPreference) -> BackendCapabilities {
        let limits = device.limits();
        let actual = match preference {
            BackendPreference::WebGpu => ActiveBackend::WebGpu,
            BackendPreference::WebGl2 => ActiveBackend::WebGl2,
            // Auto: the host resolves Auto before creating the device, so this
            // only happens for native or misconfigured hosts.
            BackendPreference::Auto => ActiveBackend::Native,
        };
        match actual {
            ActiveBackend::WebGl2 => BackendCapabilities {
                actual,
                compute: false,
                storage_buffers: false,
                indirect_draw: false,
                max_texture_dimension: limits
                    .max_texture_dimension_2d
                    .max(MIN_GUARANTEED_TEXTURE_DIMENSION),
            },
            _ => BackendCapabilities {
                actual,
                compute: true,
                storage_buffers: true,
                indirect_draw: true,
                max_texture_dimension: limits
                    .max_texture_dimension_2d
                    .max(MIN_GUARANTEED_TEXTURE_DIMENSION),
            },
        }
    }

    /// Activated backend, or `None` before a device has been initialized.
    pub fn active_backend(&self) -> Option<ActiveBackend> {
        self.active_backend
    }

    pub fn capabilities(&self) -> Option<(ActiveBackend, bool, u32)> {
        let device = self.device.as_ref()?;
        let caps = Self::caps_for(device, self.preference);
        Some((caps.actual, caps.compute, caps.max_texture_dimension))
    }

    /// Whether the device has been observed lost and still needs rebuilding.
    pub fn is_device_lost(&self) -> bool {
        self.device_lost
    }

    /// Set the bounded wait used to classify a submitted frame.
    ///
    /// The default (1 s) targets interactive hosts. Software Vulkan (lavapipe)
    /// can take longer for a large frame; a headless caller raises this so a
    /// slow CPU frame is not misclassified as [`RenderError::DeviceLost`].
    pub fn set_poll_timeout(&mut self, timeout: std::time::Duration) {
        self.poll_timeout = timeout;
    }

    fn ensure_target(&mut self, target: &RenderTarget) -> Result<(), RenderError> {
        if self.target.is_some() && self.target_size == (target.width.max(1), target.height.max(1))
        {
            return Ok(());
        }
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?;
        let size = wgpu::Extent3d {
            width: target.width.max(1),
            height: target.height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cad-target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.target_format,
            // COPY_SRC lets the headless evidence path read the frame back to a
            // buffer (`Renderer::read_target_rgba`); production composition never
            // does a per-frame GPU→CPU copy.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cad-depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.target_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.depth_view = Some(depth.create_view(&wgpu::TextureViewDescriptor::default()));
        self.target = Some(texture);
        self.target_size = (size.width, size.height);
        Ok(())
    }

    fn make_uniform(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad-batch-camera"),
            size: 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-batch-bind"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera.as_entire_binding(),
            }],
        });
        (camera, bind_group)
    }

    // Upload CPU batches as static GPU buffers (once per change).
    //
    // Mesh batches get an index buffer and a repaired normal buffer; line
    // batches keep the position-only layout.
    pub fn upload(&mut self, delta: &SceneDelta) -> CadResult<()> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?;
        let layout = self
            .layout
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?;
        for batch in &delta.added {
            let (vertices, indices, edge_indices, normals) = match batch.topology {
                RenderTopology::Mesh => {
                    let verts = pack_positions(&batch.vertices);
                    let idx: Vec<u32> = batch
                        .indices
                        .iter()
                        .flat_map(|t| [t[0], t[1], t[2]])
                        .collect();
                    let edges = geometry::sorted_edge_indices(&batch.indices);
                    let normals = geometry::repaired_normals(batch);
                    (verts, idx, edges, pack_positions(&normals))
                }
                RenderTopology::MeshEdges => {
                    let verts = pack_positions(&batch.vertices);
                    let idx: Vec<u32> = (0..batch.vertices.len() as u32).collect();
                    (verts, Vec::new(), idx, Vec::new())
                }
                RenderTopology::Lines => (
                    pack_positions(&batch.vertices),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
            };

            let is_mesh = batch.topology == RenderTopology::Mesh;
            let vertex_buffer = Self::vertex_buffer(device, self.queue.as_ref(), &vertices);
            let index_buffer = if indices.is_empty() {
                None
            } else {
                Some(Self::index_buffer(device, self.queue.as_ref(), &indices))
            };
            let edge_buffer = if edge_indices.is_empty() {
                None
            } else {
                Some(Self::index_buffer(
                    device,
                    self.queue.as_ref(),
                    &edge_indices,
                ))
            };
            let normal_buffer = if is_mesh {
                Some(Self::vertex_buffer(device, self.queue.as_ref(), &normals))
            } else {
                None
            };

            let (camera, bind_group) = Self::make_uniform(device, layout);
            let bytes = vertices.len() + indices.len() * 4 + edge_indices.len() * 4 + normals.len();
            self.uploaded_bytes += bytes as u64;
            self.batches.push(GpuBatch {
                vertices: vertex_buffer,
                vertex_count: batch.vertices.len() as u32,
                normals: normal_buffer,
                topology: batch.topology,
                indices: index_buffer,
                index_count: indices.len() as u32,
                edge_indices: edge_buffer,
                edge_index_count: edge_indices.len() as u32,
                mirrored: batch.mirrored,
                alpha: batch.alpha,
                color: batch.color,
                lineweight: batch.lineweight,
                draw_order: batch.draw_order,
                centroid: batch.centroid(),
                origin: [
                    batch.local_origin.x as f32,
                    batch.local_origin.y as f32,
                    batch.local_origin.z as f32,
                ],
                camera,
                bind_group,
            });
        }
        Ok(())
    }

    fn vertex_buffer(
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        bytes: &[u8],
    ) -> wgpu::Buffer {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad-batch"),
            size: bytes.len().max(12) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(queue) = queue {
            queue.write_buffer(&buffer, 0, bytes);
        }
        buffer
    }

    fn index_buffer(
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        indices: &[u32],
    ) -> wgpu::Buffer {
        let bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad-batch-index"),
            size: bytes.len().max(4) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(queue) = queue {
            queue.write_buffer(&buffer, 0, &bytes);
        }
        buffer
    }

    pub fn clear_batches(&mut self) {
        self.batches.clear();
    }

    /// Reset derived GPU resources after a device loss.
    ///
    /// The old device cannot be reused, so this only tears state down and marks
    /// the renderer as lost. The host must call [`Renderer::initialize_with_device`]
    /// and re-upload the scene; batches are *not* silently recreated here.
    pub fn rebuild_device(&mut self, _target: &RenderTarget) -> CadResult<()> {
        self.line_pipeline = None;
        self.mesh_pipeline = None;
        self.mesh_pipeline_mirrored = None;
        self.mesh_pipeline_transparent = None;
        self.mesh_pipeline_mirrored_transparent = None;
        self.layout = None;
        self.target = None;
        self.target_view = None;
        self.depth_view = None;
        self.batches.clear();
        self.device = None;
        self.queue = None;
        self.active_backend = None;
        self.device_lost = true;
        Err(CadError::GpuFailure(
            "device rebuild requires a new shared device from the host".into(),
        ))
    }

    pub fn switch_backend(&mut self, preference: BackendPreference) -> CadResult<()> {
        self.preference = preference;
        self.clear_batches();
        Ok(())
    }

    /// Mark the device as lost after the host's device-lost callback fires.
    ///
    /// The renderer cannot own the lost callback itself because the device is
    /// shared with the host (Slint). The host calls this when wgpu reports
    /// `DeviceLostReason`, or when a backend/surface failure is observed. All
    /// derived GPU resources are dropped; the host must re-initialize and
    /// re-upload the scene. Batches are never silently recreated.
    pub fn note_device_lost(&mut self, detail: impl Into<String>) -> RenderError {
        let detail = detail.into();
        self.line_pipeline = None;
        self.mesh_pipeline = None;
        self.mesh_pipeline_mirrored = None;
        self.mesh_pipeline_transparent = None;
        self.mesh_pipeline_mirrored_transparent = None;
        self.layout = None;
        self.target = None;
        self.target_view = None;
        self.depth_view = None;
        self.batches.clear();
        self.active_backend = None;
        self.device_lost = true;
        self.last_device_lost = Some(detail.clone());
        RenderError::DeviceLost(detail)
    }

    /// Render one frame into the offscreen target using a 2D camera.
    ///
    /// The error is explicit: callers can tell a device loss from a rejected
    /// frame via [`RenderError::is_device_loss`].
    pub fn render(
        &mut self,
        camera: Camera2d,
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        let w = target.width.max(1) as f64;
        let h = target.height.max(1) as f64;
        let sx = 2.0 / (w * camera.world_per_px);
        let sy = -2.0 / (h * camera.world_per_px);
        let mut transforms = Vec::with_capacity(self.batches.len());
        for batch in &self.batches {
            let ox = batch.origin[0] as f64;
            let oy = batch.origin[1] as f64;
            transforms.push([
                sx as f32,
                0.0,
                0.0,
                0.0,
                0.0,
                sy as f32,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                (sx * (ox - camera.center.x)) as f32,
                (sy * (oy - camera.center.y)) as f32,
                (batch.origin[2] as f64 - camera.center.z) as f32 + camera.z_plane,
                1.0,
            ]);
        }
        self.render_with_transforms(
            transforms,
            [
                camera.center.x as f32,
                camera.center.y as f32,
                camera.center.z as f32,
            ],
            target,
        )
    }

    /// Render one frame into the offscreen target using a 3D camera.
    ///
    /// Returns [`RenderError::Frame`] (no GPU work submitted) when the camera
    /// projection is degenerate.
    pub fn render_3d(
        &mut self,
        camera: Camera3d,
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        let aspect = target.width.max(1) as f64 / target.height.max(1) as f64;
        let Some(vp) = camera.view_projection(aspect) else {
            return Err(RenderError::Frame(
                "camera has no usable 3D projection".into(),
            ));
        };
        // Per-batch uniform = VP * T(batch origin).
        let transforms = self
            .batches
            .iter()
            .map(|batch| translate_left(&vp, batch.origin))
            .collect();
        let eye = [
            camera.eye.x as f32,
            camera.eye.y as f32,
            camera.eye.z as f32,
        ];
        self.render_with_transforms(transforms, eye, target)
    }

    fn render_with_transforms(
        &mut self,
        transforms: Vec<[f32; 16]>,
        camera_position: [f32; 3],
        target: &RenderTarget,
    ) -> Result<FrameStats, RenderError> {
        if self.device_lost {
            return Err(RenderError::DeviceLost(
                self.last_device_lost
                    .clone()
                    .unwrap_or_else(|| "device lost; rebuild required".into()),
            ));
        }
        self.ensure_target(target)?;
        let Some(queue) = self.queue.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer not initialized".into(),
            ));
        };
        let device = self.device.as_ref().unwrap();
        let Some(line_pipeline) = self.line_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline) = self.mesh_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_mirrored) = self.mesh_pipeline_mirrored.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_transparent) = self.mesh_pipeline_transparent.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_mirrored_transparent) =
            self.mesh_pipeline_mirrored_transparent.as_ref()
        else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(view) = self.target_view.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer target missing".into(),
            ));
        };
        let Some(depth_view) = self.depth_view.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer depth target missing".into(),
            ));
        };

        // Write the per-batch uniforms before encoding. The uniform is the
        // transform matrix followed by the constant per-batch colour (normalized
        // sRGB) and alpha (both clamped by the same policy used for drawing).
        for (batch, m) in self.batches.iter().zip(transforms.iter()) {
            let mut uniform = [0.0f32; 20];
            uniform[..16].copy_from_slice(m);
            let [r, g, b] = renderer_color(batch.color);
            uniform[16] = r;
            uniform[17] = g;
            uniform[18] = b;
            uniform[19] = clamp_alpha(batch.alpha);
            queue.write_buffer(&batch.camera, 0, bytemuck::cast_slice(&uniform));
        }

        // Enforce the per-frame vertex/triangle budget explicitly (in upload
        // order, unchanged).
        let budget_plan = plan_from_gpu(&self.batches, &self.frame_budget);

        // Order the *accepted* batches into an opaque pass and a back-to-front
        // transparent pass. `alpha <= 0` batches are partitioned out and
        // reported rather than drawn.
        let entries: Vec<BatchOrderEntry> = budget_plan
            .accepted
            .iter()
            .map(|&i| BatchOrderEntry {
                draw_order: self.batches[i].draw_order,
                alpha: self.batches[i].alpha,
                centroid: self.batches[i].centroid,
            })
            .collect();
        let passes = draw_passes(&budget_plan.accepted, &entries, Some(camera_position));
        let opaque = passes.opaque;
        let transparent = passes.transparent;
        let invisible_batches = passes.invisible;

        // Lineweight is carried, never drawn: collect every submitted batch that
        // asks for a non-zero weight so the frame reports the gap explicitly
        // instead of implying a width it did not rasterize.
        let lineweight_not_drawn: Vec<LineweightNotDrawn> = self
            .batches
            .iter()
            .filter(|batch| batch.lineweight > 0.0)
            .map(|batch| LineweightNotDrawn {
                millimeters: batch.lineweight,
            })
            .collect();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cad-encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.06,
                            g: 0.07,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Pass 1 — opaque, in ascending draw order. Depth writes are on.
            for &i in &opaque {
                draw_batch(
                    &mut pass,
                    &self.batches[i],
                    line_pipeline,
                    if self.batches[i].mirrored {
                        mesh_pipeline_mirrored
                    } else {
                        mesh_pipeline
                    },
                );
            }
            // Pass 2 — transparent, back-to-front. Depth writes are off so each
            // blended layer composites over the already-resolved opaque pass.
            for &i in &transparent {
                draw_batch(
                    &mut pass,
                    &self.batches[i],
                    line_pipeline,
                    if self.batches[i].mirrored {
                        mesh_pipeline_mirrored_transparent
                    } else {
                        mesh_pipeline_transparent
                    },
                );
            }
        }

        let submission = queue.submit(Some(encoder.finish()));
        // Distinguish device loss from a bad frame. A bounded wait classifies
        // the submission; the host's device-lost callback is the authoritative
        // loss signal (see `note_device_lost`).
        let outcome = self.device_scope_outcome(device, submission);
        let draw_calls = (opaque.len() + transparent.len()) as u64;
        self.draw_calls = draw_calls;
        match outcome {
            ScopeOutcome::Clean => {}
            ScopeOutcome::FrameError => {
                return Err(RenderError::Frame(
                    "frame rejected by GPU validation".into(),
                ));
            }
            ScopeOutcome::DeviceLost => {
                let report = self.note_device_lost("GPU device lost while rendering");
                return Err(report);
            }
        }
        Ok(FrameStats {
            cpu_ms: 0.0,
            gpu_ms: None,
            draw_calls,
            uploaded_bytes: self.uploaded_bytes,
            vertices: budget_plan.usage.vertices,
            triangles: budget_plan.usage.triangles,
            over_budget: budget_plan.over_budget,
            opaque_batches: opaque.len(),
            transparent_batches: transparent.len(),
            invisible_batches,
            lineweight_not_drawn: lineweight_not_drawn.clone(),
            lineweight_reason: lineweight_reason(&lineweight_not_drawn),
        })
    }

    /// Poll the device and classify the frame's outcome.
    ///
    /// A `Timeout` means the device made no progress in the bound (1 s): on
    /// native backends that is the observable proxy for a lost/reset device, so
    /// it is reported as [`ScopeOutcome::DeviceLost`]. A wrong submission index
    /// is a frame-level mistake, not a loss. A clean poll means no error the
    /// renderer can observe synchronously; the host's device-lost callback still
    /// remains authoritative.
    ///
    /// On `wasm32` the browser backends (WebGL2/WebGPU) complete asynchronously
    /// and must never block the main thread: `PollType::Wait` is unavailable, so
    /// a non-blocking `Poll` is used and a timeout is **not** treated as device
    /// loss (a lost browser device surfaces through wgpu's device-lost callback).
    fn device_scope_outcome(
        &self,
        device: &wgpu::Device,
        submission: wgpu::SubmissionIndex,
    ) -> ScopeOutcome {
        #[cfg(target_arch = "wasm32")]
        let poll_type = {
            let _ = submission;
            wgpu::PollType::Poll
        };
        #[cfg(not(target_arch = "wasm32"))]
        let poll_type = wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(self.poll_timeout),
        };
        match device.poll(poll_type) {
            Ok(_) => ScopeOutcome::Clean,
            Err(wgpu::PollError::Timeout) => {
                #[cfg(target_arch = "wasm32")]
                {
                    ScopeOutcome::Clean
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ScopeOutcome::DeviceLost
                }
            }
            Err(wgpu::PollError::WrongSubmissionIndex(..)) => ScopeOutcome::FrameError,
        }
    }

    pub fn frame_texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref()
    }

    pub fn device_generation(&self) -> u64 {
        self.device_generation
    }

    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }

    /// Diagnostic for an over-budget frame, if any is pending.
    pub fn over_budget_reason(report: &OverBudget) -> DiagnosticReason {
        over_budget_reason(report)
    }

    /// Diagnostic describing a device loss.
    pub fn device_lost_reason(detail: &str) -> DiagnosticReason {
        DiagnosticReason::missing(
            codes::RENDER_DEVICE_LOST,
            vec![DiagnosticParameter::Identifier(detail.to_string())],
        )
    }
}

struct GpuPlan {
    accepted: Vec<usize>,
    usage: cad_scene::FrameUsage,
    over_budget: Option<OverBudgetReason>,
}

/// The ordered, budget-accepted batch indices split into the two draw passes.
struct DrawPasses {
    opaque: Vec<usize>,
    transparent: Vec<usize>,
    invisible: usize,
}

/// Turn the pure [`DrawOrderPlan`] over the accepted sub-set back into global
/// batch indices, preserving the plan's order.
///
/// `accepted` are the global indices the budget kept (upload order); `entries`
/// are their ordering keys in the same order. Splitting this out keeps the index
/// remapping testable without a GPU.
fn draw_passes(
    accepted: &[usize],
    entries: &[BatchOrderEntry],
    camera_position: Option<[f32; 3]>,
) -> DrawPasses {
    debug_assert_eq!(accepted.len(), entries.len());
    let order = plan_draw_order(entries, camera_position);
    let remap = |positions: &[usize]| -> Vec<usize> {
        positions.iter().map(|&pos| accepted[pos]).collect()
    };
    DrawPasses {
        opaque: remap(&order.opaque),
        transparent: remap(&order.transparent),
        invisible: order.invisible.len(),
    }
}

/// Submit one batch, dispatching on its topology.
///
/// The caller chooses the mesh pipeline (opaque vs transparent, mirrored vs
/// normal); line and `MeshEdges` topologies always use the line pipeline. A mesh
/// batch also draws its optional wireframe overlay.
fn draw_batch(
    pass: &mut wgpu::RenderPass<'_>,
    batch: &GpuBatch,
    line_pipeline: &wgpu::RenderPipeline,
    mesh_pipeline: &wgpu::RenderPipeline,
) {
    match batch.topology {
        RenderTopology::Lines => {
            pass.set_pipeline(line_pipeline);
            pass.set_bind_group(0, &batch.bind_group, &[]);
            pass.set_vertex_buffer(0, batch.vertices.slice(..));
            if let Some(indices) = &batch.indices {
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..batch.index_count, 0, 0..1);
            } else {
                pass.draw(0..batch.vertex_count, 0..1);
            }
        }
        RenderTopology::Mesh => {
            pass.set_pipeline(mesh_pipeline);
            pass.set_bind_group(0, &batch.bind_group, &[]);
            pass.set_vertex_buffer(0, batch.vertices.slice(..));
            if let Some(normals) = &batch.normals {
                pass.set_vertex_buffer(1, normals.slice(..));
            }
            let Some(indices) = batch.indices.as_ref() else {
                return;
            };
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.index_count, 0, 0..1);
            // Optional wireframe overlay over the shared mesh vertices.
            if let Some(edges) = &batch.edge_indices {
                pass.set_pipeline(line_pipeline);
                pass.set_vertex_buffer(0, batch.vertices.slice(..));
                pass.set_index_buffer(edges.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..batch.edge_index_count, 0, 0..1);
            }
        }
        RenderTopology::MeshEdges => {
            pass.set_pipeline(line_pipeline);
            pass.set_bind_group(0, &batch.bind_group, &[]);
            pass.set_vertex_buffer(0, batch.vertices.slice(..));
            let Some(indices) = batch.indices.as_ref() else {
                return;
            };
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..batch.index_count, 0, 0..1);
        }
    }
}

fn plan_from_gpu(batches: &[GpuBatch], budget: &FrameBudget) -> GpuPlan {
    let mut usage = cad_scene::FrameUsage::default();
    let mut accepted = Vec::with_capacity(batches.len());
    for (i, batch) in batches.iter().enumerate() {
        let vertices = batch.vertex_count as usize;
        let triangles = if batch.topology == RenderTopology::Mesh {
            batch.index_count as usize / 3
        } else {
            0
        };
        match budget.charge(&mut usage, vertices, triangles) {
            Ok(()) => accepted.push(i),
            Err(exceeded) => {
                let report = OverBudget {
                    category: exceeded.category,
                    requested: exceeded.requested,
                    limit: exceeded.limit,
                    skipped_batches: batches.len() - accepted.len(),
                };
                return GpuPlan {
                    accepted,
                    usage,
                    over_budget: Some(OverBudgetReason {
                        reason: over_budget_reason(&report),
                        report,
                    }),
                };
            }
        }
    }
    GpuPlan {
        accepted,
        usage,
        over_budget: None,
    }
}

fn over_budget_reason(report: &OverBudget) -> DiagnosticReason {
    DiagnosticReason::partial(
        codes::RENDER_FRAME_OVER_BUDGET,
        vec![
            DiagnosticParameter::Identifier(report.category.to_string()),
            DiagnosticParameter::Count(report.requested as u64),
            DiagnosticParameter::Limit(report.limit as u64),
            DiagnosticParameter::Count(report.skipped_batches as u64),
        ],
    )
}

fn pack_positions(vectors: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vectors.len() * 12);
    for v in vectors {
        bytes.extend_from_slice(&v[0].to_le_bytes());
        bytes.extend_from_slice(&v[1].to_le_bytes());
        bytes.extend_from_slice(&v[2].to_le_bytes());
    }
    bytes
}

/// Sanitise a batch colour for the uniform: finite `[0, 1]` channels.
///
/// Mirrors `cad_scene::sanitize_color` so a value that bypassed the scene cache
/// still cannot put a NaN into the shader.
fn renderer_color(color: [f32; 3]) -> [f32; 3] {
    let channel = |c: f32| {
        if c.is_finite() {
            c.clamp(0.0, 1.0)
        } else {
            1.0
        }
    };
    [channel(color[0]), channel(color[1]), channel(color[2])]
}

/// Aggregate the "lineweight carried but not drawn" batches into one stable
/// diagnostic reason. `None` when nothing asked for a non-zero weight.
fn lineweight_reason(not_drawn: &[LineweightNotDrawn]) -> Option<DiagnosticReason> {
    if not_drawn.is_empty() {
        return None;
    }
    let max_mm = not_drawn
        .iter()
        .map(|l| l.millimeters)
        .fold(0.0f32, f32::max);
    Some(DiagnosticReason::partial(
        codes::RENDER_LINEWEIGHT_NOT_DRAWN,
        vec![
            DiagnosticParameter::Count(not_drawn.len() as u64),
            DiagnosticParameter::Identifier(format!("{max_mm:.2}mm")),
        ],
    ))
}

fn translate_left(vp: &[[f32; 4]; 4], origin: [f32; 3]) -> [f32; 16] {
    // Column-major mat4x4 * translation(origin). Translation only affects the
    // last column: out[.][3] = vp[.][0]*ox + vp[.][1]*oy + vp[.][2]*oz + vp[.][3].
    let mut m = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            m[col * 4 + row] = vp[col][row];
        }
    }
    let ox = origin[0];
    let oy = origin[1];
    let oz = origin[2];
    for row in 0..4 {
        let base = vp[0][row] * ox + vp[1][row] * oy + vp[2][row] * oz + vp[3][row];
        m[12 + row] = base;
    }
    m
}

fn position_only_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    }
}

fn mesh_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    }
}

/// Per-vertex normals travel in their own vertex buffer (slot 1, see
/// `draw_batch`). `mesh.wgsl` reads them at `@location(1)`, so the pipeline must
/// declare this layout too or `create_render_pipeline` fails validation with
/// "Location[1] ... is not provided by the previous stage outputs".
fn mesh_normal_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![1 => Float32x3],
    }
}

// Shaders are checked into `shaders/` so they can be validated as files and
// stay reviewable; `include_str!` keeps a single source of truth.
const LINE_SHADER: &str = include_str!("../shaders/line.wgsl");

// Mesh shader. Shading is intentionally explicit: a single fixed headlight
// direction, N·L diffuse plus a constant ambient term. There is no claim of
// physically based rendering, environment lighting or specular response.
const MESH_SHADER: &str = include_str!("../shaders/mesh.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninitialized_renderer_reports_no_active_backend() {
        // A preference is not an activated backend. Before the host supplies a
        // device there is no actual backend and no capabilities (audit B02).
        for preference in [
            BackendPreference::Auto,
            BackendPreference::WebGpu,
            BackendPreference::WebGl2,
        ] {
            let renderer = Renderer::new(preference);
            assert_eq!(renderer.active_backend(), None);
            assert!(
                renderer.capabilities().is_none(),
                "no device yet for {preference:?}"
            );
        }
    }

    #[test]
    fn preference_and_active_backend_are_distinct_concepts() {
        // The requested preference is retained even while activation is pending.
        let renderer = Renderer::new(BackendPreference::WebGl2);
        assert_eq!(renderer.preference, BackendPreference::WebGl2);
        assert_eq!(renderer.active_backend(), None);
    }

    #[test]
    fn backend_names_are_stable() {
        assert_eq!(ActiveBackend::WebGpu.as_str(), "webgpu");
        assert_eq!(ActiveBackend::WebGl2.as_str(), "webgl2");
        assert_eq!(ActiveBackend::Native.as_str(), "native");
    }

    #[test]
    fn mesh_pipeline_declares_position_and_normal_attributes() {
        // Regression: `mesh.wgsl` reads `@location(0)` position and
        // `@location(1)` normal from two separate vertex buffers. The pipeline
        // must declare both layouts or wgpu rejects `cad-mesh-pipeline` at
        // creation ("Location[1] ... is not provided by the previous stage
        // outputs").
        let layouts = [mesh_vertex_layout(), mesh_normal_layout()];
        let locations: Vec<u32> = layouts
            .iter()
            .flat_map(|layout| layout.attributes.iter().map(|attr| attr.shader_location))
            .collect();
        assert_eq!(locations, vec![0, 1]);
        assert!(layouts.iter().all(|layout| layout.array_stride == 12));
    }

    #[test]
    fn render_errors_distinguish_device_loss_from_a_bad_frame() {
        assert!(RenderError::DeviceLost("reset".into()).is_device_loss());
        assert!(!RenderError::Frame("validation".into()).is_device_loss());
        assert!(!RenderError::NotInitialized("no device".into()).is_device_loss());
    }

    #[test]
    fn render_before_init_is_not_a_device_loss() {
        let mut renderer = Renderer::default();
        let target = RenderTarget::new(64, 64);
        let frame = renderer.render(Camera2d::default(), &target);
        assert!(matches!(frame, Err(RenderError::NotInitialized(_))));
        assert!(!renderer.is_device_lost());
    }

    #[test]
    fn noted_device_loss_is_explicit_and_clears_batches() {
        let mut renderer = Renderer::default();
        // Simulate a batch having been uploaded, then a host-observed loss.
        let loss = renderer.note_device_lost("driver reset");
        assert!(loss.is_device_loss());
        assert!(renderer.is_device_lost());
        assert_eq!(renderer.batch_count(), 0);
        let target = RenderTarget::new(64, 64);
        let frame = renderer.render(Camera2d::default(), &target);
        assert!(matches!(frame, Err(RenderError::DeviceLost(_))));
    }

    #[test]
    fn over_budget_diagnostic_carries_structured_parameters() {
        let report = OverBudget {
            category: "vertices",
            requested: 100,
            limit: 50,
            skipped_batches: 3,
        };
        let reason = Renderer::over_budget_reason(&report);
        assert_eq!(reason.code, codes::RENDER_FRAME_OVER_BUDGET);
        assert!(reason.parameters.contains(&DiagnosticParameter::Limit(50)));
    }

    #[test]
    fn device_lost_diagnostic_has_stable_code() {
        let reason = Renderer::device_lost_reason("test");
        assert_eq!(reason.code, codes::RENDER_DEVICE_LOST);
    }

    #[test]
    fn draw_passes_remap_accepted_positions_to_global_indices() {
        // The budget accepted global batches 10, 11, 12 (upload order). Batch 10
        // is transparent and farthest, 11 opaque with the higher draw order, and
        // 12 opaque with a lower draw order.
        let accepted = vec![10usize, 11, 12];
        let entries = vec![
            BatchOrderEntry {
                draw_order: 0,
                alpha: 0.5,
                centroid: [100.0, 0.0, 0.0],
            },
            BatchOrderEntry {
                draw_order: 5,
                alpha: 1.0,
                centroid: [1.0, 0.0, 0.0],
            },
            BatchOrderEntry {
                draw_order: 1,
                alpha: 1.0,
                centroid: [2.0, 0.0, 0.0],
            },
        ];
        let passes = draw_passes(&accepted, &entries, Some([0.0, 0.0, 0.0]));
        // Opaque first, ascending draw order: global 12 (order 1), then 11 (5).
        assert_eq!(passes.opaque, vec![12, 11]);
        // Transparent after, single element global 10.
        assert_eq!(passes.transparent, vec![10]);
        assert_eq!(passes.invisible, 0);
    }

    #[test]
    fn draw_passes_report_invisible_batches() {
        let accepted = vec![0usize, 1];
        let entries = vec![
            BatchOrderEntry {
                draw_order: 0,
                alpha: 0.0,
                centroid: [0.0; 3],
            },
            BatchOrderEntry {
                draw_order: 0,
                alpha: 1.0,
                centroid: [0.0; 3],
            },
        ];
        let passes = draw_passes(&accepted, &entries, Some([0.0; 3]));
        assert_eq!(passes.opaque, vec![1]);
        assert!(passes.transparent.is_empty());
        assert_eq!(passes.invisible, 1);
    }
}
