//! Shared fullscreen-textured blit used by every surface presentation path.
//!
//! Both surface paths present the renderer's offscreen CAD frame the same way:
//! one fullscreen triangle samples the frame texture straight onto the
//! swapchain/canvas surface. No pixels are read back on either path and no extra
//! CAD surface is created. The WGSL source and the pipeline builder therefore
//! live here, so the native probe presenter and the wasm canvas presenter cannot
//! drift apart.
//!
//! [`FullscreenBlitter`] is the wasm-side stateful wrapper (pipeline plus a
//! one-texture bind-group cache). The native probe deliberately keeps its own
//! binding cache (keyed by `NativeTextureKey`) and its texture/surface size
//! fence: this module shares the shader and the pipeline construction only, so no
//! native presentation semantics change.

use crate::wgpu;

/// Fullscreen textured shader: samples a CAD texture directly to the surface (no
/// CPU readback, no extra CAD surface). Opaque display; the CAD texture is
/// expected to be opaque (premultiplied alpha is not used here).
///
/// The triangle covers the whole target with no aspect correction and no colour
/// conversion, which is why both callers pass a source texture whose size **and**
/// format match the configured surface.
pub(crate) const TEXTURE_SHADER: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var cad_texture: texture_2d<f32>;
@group(0) @binding(1) var cad_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vertex_index];
    var out: VsOut;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    // Texture origin is top-left: clip y=+1 is the screen top, so v must be
    // (1 - y)/2. Getting this wrong flips the image vertically.
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(cad_texture, cad_sampler, in.uv);
}
"#;

/// Build the fullscreen textured pipeline, its bind-group layout and sampler.
///
/// `format` is the **configured target format** (surface format, or the format a
/// test renders into); never assume BGRA — canvas formats are RGBA on every
/// browser backend.
pub(crate) fn build_texture_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cad-texture-shader"),
        source: wgpu::ShaderSource::Wgsl(TEXTURE_SHADER.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cad-texture-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cad-texture-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cad-texture-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
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
                format,
                // Opaque display: the CAD texture is expected to be opaque.
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cad-texture-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    (pipeline, layout, sampler)
}

/// Stateful wasm-side blitter: the textured pipeline plus a bind-group cache for
/// the single source texture of the last frame.
///
/// Wasm only on purpose: the native probe already owns an equivalent cache keyed
/// by its `NativeTextureKey`, and compiling a second, unused copy there would be
/// dead code. Both use [`build_texture_pipeline`] and [`TEXTURE_SHADER`].
#[cfg(target_arch = "wasm32")]
pub(crate) struct FullscreenBlitter {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The last bound source texture and its bind group. The cache is keyed by
    /// **texture identity** (a `wgpu::Texture` comparison is a stable identity
    /// comparison), because a renderer session may reuse a size across sessions.
    bound: Option<(wgpu::Texture, wgpu::BindGroup)>,
    /// Bind groups created since construction (evidence only: a steady frame loop
    /// presents the same frame texture, so this must stay flat).
    binds_created: u64,
}

#[cfg(target_arch = "wasm32")]
impl FullscreenBlitter {
    /// Build the pipeline, layout and sampler for `format` (the configured
    /// surface format).
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let (pipeline, layout, sampler) = build_texture_pipeline(device, format);
        Self {
            pipeline,
            layout,
            sampler,
            bound: None,
            binds_created: 0,
        }
    }

    /// Number of bind groups created since construction.
    pub(crate) fn binds_created(&self) -> u64 {
        self.binds_created
    }

    /// Draw `texture` across the whole `target` and submit the pass.
    ///
    /// Callers validate the source (size/format/usage) and their own error
    /// channel first; this helper performs the GPU work only. No readback, no
    /// blocking poll: the caller presents the acquired surface texture after the
    /// submit.
    pub(crate) fn blit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        texture: &wgpu::Texture,
    ) {
        self.ensure_bound(device, texture);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cad-canvas-blit-encoder"),
        });
        {
            let bind = &self
                .bound
                .as_ref()
                .expect("ensure_bound just cached the source texture")
                .1;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-canvas-blit-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }

    /// Ensure the bind-group cache holds a group for `texture`'s identity.
    fn ensure_bound(&mut self, device: &wgpu::Device, texture: &wgpu::Texture) {
        let reusable = self
            .bound
            .as_ref()
            .is_some_and(|(cached, _)| cached == texture);
        if reusable {
            return;
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-canvas-texture-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.bound = Some((texture.clone(), bind));
        self.binds_created += 1;
    }
}
