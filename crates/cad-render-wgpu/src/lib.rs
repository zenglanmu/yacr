//! wgpu 2D pipelines over CPU scene batches.
//!
//! Spec v2.0 §5.2, §8: the renderer does not own the window or the device. The
//! host (Slint) provides a shared `Device`/`Queue`; CAD draws into a texture the
//! host composites. Camera navigation updates only small uniform buffers, never
//! the static vertex buffers.

use cad_domain::*;
use cad_scene::SceneDelta;

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
}

/// Camera used by the CAD viewport, in world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2d {
    pub center: Point3,
    pub world_per_px: f64,
    pub z_plane: f32,
}

impl Default for Camera2d {
    fn default() -> Self {
        Camera2d { center: Point3 { x: 0.0, y: 0.0, z: 0.0 }, world_per_px: 1.0, z_plane: 0.0 }
    }
}

struct GpuBatch {
    vertices: wgpu::Buffer,
    vertex_count: u32,
    origin: [f32; 3],
    camera: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// The CAD renderer. Owns only derived GPU resources.
pub struct Renderer {
    pub preference: BackendPreference,
    pub recovery_limit: u32,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    layout: Option<wgpu::BindGroupLayout>,
    pipeline: Option<wgpu::RenderPipeline>,
    target: Option<wgpu::Texture>,
    target_view: Option<wgpu::TextureView>,
    target_size: (u32, u32),
    target_format: wgpu::TextureFormat,
    batches: Vec<GpuBatch>,
    device_generation: u64,
    uploaded_bytes: u64,
    draw_calls: u64,
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
            device: None,
            queue: None,
            layout: None,
            pipeline: None,
            target: None,
            target_view: None,
            target_size: (1, 1),
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            batches: Vec::new(),
            device_generation: 0,
            uploaded_bytes: 0,
            draw_calls: 0,
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
        let caps = Self::caps_for(&device);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-lines"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad-camera-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-lines-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: (std::mem::size_of::<f32>() * 3) as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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

        self.device = Some(device);
        self.queue = Some(queue);
        self.layout = Some(layout);
        self.pipeline = Some(pipeline);
        self.device_generation += 1;
        Ok(caps)
    }

    fn caps_for(device: &wgpu::Device) -> BackendCapabilities {
        let limits = device.limits();
        BackendCapabilities {
            actual: ActiveBackend::WebGpu,
            compute: true,
            storage_buffers: true,
            indirect_draw: true,
            max_texture_dimension: limits.max_texture_dimension_2d,
        }
    }

    fn ensure_target(&mut self, target: &RenderTarget) -> CadResult<()> {
        if self.target.is_some() && self.target_size == (target.width.max(1), target.height.max(1)) {
            return Ok(());
        }
        let device = self.device.as_ref().ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?;
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.target_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.target = Some(texture);
        self.target_size = (size.width, size.height);
        Ok(())
    }

    // Upload CPU batches as static GPU buffers (once per change).
    pub fn upload(&mut self, delta: &SceneDelta) -> CadResult<()> {
        let device = self.device.as_ref().ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?;
        let layout = self.layout.as_ref().ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?;
        for batch in &delta.added {
            let bytes: Vec<u8> = batch
                .vertices
                .iter()
                .flat_map(|v| {
                    [
                        v[0].to_le_bytes(),
                        v[1].to_le_bytes(),
                        v[2].to_le_bytes(),
                    ]
                    .concat()
                })
                .collect();
            let vertices = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cad-batch"),
                size: bytes.len().max(12) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            if let Some(queue) = self.queue.as_ref() {
                queue.write_buffer(&vertices, 0, &bytes);
            }
            let camera = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cad-batch-camera"),
                size: 64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cad-batch-bind"),
                layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera.as_entire_binding() }],
            });
            self.uploaded_bytes += bytes.len() as u64;
            self.batches.push(GpuBatch {
                vertices,
                vertex_count: batch.vertices.len() as u32,
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

    pub fn clear_batches(&mut self) {
        self.batches.clear();
    }

    /// Reset derived GPU resources after a device loss.
    pub fn rebuild_device(&mut self, _target: &RenderTarget) -> CadResult<()> {
        self.pipeline = None;
        self.layout = None;
        self.target = None;
        self.target_view = None;
        self.batches.clear();
        Err(CadError::GpuFailure("device rebuild requires a new shared device from the host".into()))
    }

    pub fn switch_backend(&mut self, preference: BackendPreference) -> CadResult<()> {
        self.preference = preference;
        self.clear_batches();
        Ok(())
    }

    /// Render one frame into the offscreen target.
    pub fn render(&mut self, camera: Camera2d, target: &RenderTarget) -> CadResult<FrameStats> {
        self.ensure_target(target)?;
        let Some(queue) = self.queue.as_ref() else {
            return Err(CadError::GpuFailure("renderer not initialized".into()));
        };
        let device = self.device.as_ref().unwrap();
        let Some(pipeline) = self.pipeline.as_ref() else {
            return Err(CadError::GpuFailure("renderer resources missing".into()));
        };
        let Some(view) = self.target_view.as_ref() else {
            return Err(CadError::GpuFailure("renderer target missing".into()));
        };

        let w = self.target_size.0 as f64;
        let h = self.target_size.1 as f64;
        let sx = 2.0 / (w * camera.world_per_px);
        let sy = -2.0 / (h * camera.world_per_px);

        // Per-batch uniform = M * T(batch origin), so relative vertices land at
        // their true world position while staying precise in f32.
        for batch in &self.batches {
            let ox = batch.origin[0] as f64;
            let oy = batch.origin[1] as f64;
            let oz = batch.origin[2] as f64;
            let m: [f32; 16] = [
                sx as f32, 0.0, 0.0, 0.0,
                0.0, sy as f32, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                (sx * (ox - camera.center.x)) as f32,
                (sy * (oy - camera.center.y)) as f32,
                (oz - camera.center.z) as f32 + camera.z_plane,
                1.0,
            ];
            queue.write_buffer(&batch.camera, 0, bytemuck::cast_slice(&m));
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cad-encoder") });
        let mut draw_calls = 0u64;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.06, g: 0.07, b: 0.10, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            for batch in &self.batches {
                pass.set_bind_group(0, &batch.bind_group, &[]);
                pass.set_vertex_buffer(0, batch.vertices.slice(..));
                pass.draw(0..batch.vertex_count, 0..1);
                draw_calls += 1;
            }
        }
        queue.submit(Some(encoder.finish()));
        self.draw_calls = draw_calls;
        Ok(FrameStats { cpu_ms: 0.0, gpu_ms: None, draw_calls, uploaded_bytes: self.uploaded_bytes })
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
}

const SHADER: &str = r#"
struct Camera { transform: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.transform * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.85, 0.88, 0.92, 1.0);
}
"#;
