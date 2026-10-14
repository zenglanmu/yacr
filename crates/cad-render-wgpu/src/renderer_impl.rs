//! Implementation of [`Renderer`]: GPU upload, batching and frame submission.

use super::*;

// `std::time::Instant::now()` panics on wasm32-unknown-unknown ("time not
// implemented on this platform"); the browser gets `Performance.now()` through
// `web-time`. Native keeps the std type.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

impl Renderer {
    pub fn new(preference: BackendPreference) -> Self {
        Renderer {
            preference,
            recovery_limit: 2,
            frame_budget: FrameBudget {
                max_vertices: 8_000_000,
                max_triangles: 2_000_000,
                max_bytes: usize::MAX,
            },
            // No device yet, so no backend has actually been activated.
            active_backend: None,
            device: None,
            queue: None,
            layout: None,
            line_pipeline: None,
            line_instanced_pipeline: None,
            unit_segment_buffer: None,
            mesh_pipeline: None,
            mesh_pipeline_mirrored: None,
            mesh_pipeline_transparent: None,
            mesh_pipeline_mirrored_transparent: None,
            mesh_pipeline_merged: None,
            mesh_pipeline_merged_mirrored: None,
            image_pipeline: None,
            image_texture_layout: None,
            image_sampler: None,
            target: None,
            target_view: None,
            depth_view: None,
            target_size: (1, 1),
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            batches: Vec::new(),
            instance_streams: Vec::new(),
            mesh_chunks: Vec::new(),
            line_instancing: true,
            mesh_merging: true,
            max_instances_per_stream: MAX_INSTANCES_PER_STREAM,
            max_vertices_per_mesh_stream: MAX_VERTICES_PER_MESH_STREAM,
            instance_stream_uploads: 0,
            mesh_chunk_uploads: 0,
            images: Vec::new(),
            image_textures: HashMap::new(),
            uniforms: UniformArena::new(&wgpu::Limits::default()),
            frame_slots: Vec::new(),
            image_slots: Vec::new(),
            scratch_opaque: Vec::new(),
            scratch_transparent: Vec::new(),
            lineweight_not_drawn: Vec::new(),
            device_generation: 0,
            texture_revision: 0,
            uploaded_bytes: 0,
            last_upload_ms: None,
            draw_calls: 0,
            progressive: false,
            progressive_frame: None,
            draw_plan: None,
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
        let caps = Self::caps_for(&device);
        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-lines"),
            source: wgpu::ShaderSource::Wgsl(LINE_SHADER.into()),
        });
        let line_instanced_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-lines-instanced"),
            source: wgpu::ShaderSource::Wgsl(LINE_INSTANCED_SHADER.into()),
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
                    // One shared uniform buffer is bound with a per-draw dynamic
                    // offset (the batch's slot); see `UniformArena`.
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(u64::from(UNIFORM_STRUCT_BYTES)),
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
        // Instanced line pipeline: the same primitive/depth/blend state as the
        // line pipeline, but slot 0 is the shared unit segment and slot 1 is the
        // per-instance buffer (`VertexStepMode::Instance`).
        let line_instanced_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cad-lines-instanced-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &line_instanced_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(position_only_layout()), Some(line_instance_layout())],
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
                    module: &line_instanced_shader,
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
        // One shared unit segment for every instanced line draw.
        let unit_segment_buffer = Self::vertex_buffer(
            &device,
            Some(&queue),
            &pack_positions(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]),
        );
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
        // Merged-mesh pipelines: the flat-merge layout (one interleaved buffer
        // carrying position/normal/baked colour/entity) and the same shading
        // body. Only opaque chunks are merged, so there is no transparent
        // variant; a merged chunk never spans both mirror states.
        let mesh_merged_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-mesh-merged"),
            source: wgpu::ShaderSource::Wgsl(MESH_MERGED_SHADER.into()),
        });
        let mesh_pipeline_merged = Self::create_mesh_merged_pipeline(
            &device,
            &pipeline_layout,
            &mesh_merged_shader,
            self.target_format,
            wgpu::FrontFace::Ccw,
            "cad-mesh-merged-pipeline",
        );
        let mesh_pipeline_merged_mirrored = Self::create_mesh_merged_pipeline(
            &device,
            &pipeline_layout,
            &mesh_merged_shader,
            self.target_format,
            wgpu::FrontFace::Cw,
            "cad-mesh-merged-pipeline-mirrored",
        );
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-image"),
            source: wgpu::ShaderSource::Wgsl(IMAGE_SHADER.into()),
        });
        let image_texture_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cad-image-texture-layout"),
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
        let image_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cad-image-pipeline-layout"),
                bind_group_layouts: &[Some(&layout), Some(&image_texture_layout)],
                immediate_size: 0,
            });
        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-image-pipeline"),
            layout: Some(&image_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &image_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(image_vertex_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // A transformed unit square may be mirrored; culling would drop
                // legitimate images, so image quads are never culled.
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                // Same depth format as the mesh pipeline, `LessEqual` so a
                // coplanar overlay is not rejected by the depth test.
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &image_shader,
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
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cad-image-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // A successful attachment establishes a new device generation. No
        // batch/attachment from the previous generation may survive, even if
        // the new target later has exactly the same dimensions.
        self.batches.clear();
        self.images.clear();
        self.image_textures.clear();
        self.instance_streams.clear();
        self.mesh_chunks.clear();
        self.uniforms = UniformArena::new(&device.limits());
        self.frame_slots.clear();
        self.image_slots.clear();
        self.target = None;
        self.target_view = None;
        self.depth_view = None;
        self.target_size = (0, 0);
        self.draw_plan = None;
        self.device = Some(device);
        self.queue = Some(queue);
        self.layout = Some(layout);
        self.line_pipeline = Some(line_pipeline);
        self.line_instanced_pipeline = Some(line_instanced_pipeline);
        self.unit_segment_buffer = Some(unit_segment_buffer);
        self.mesh_pipeline = Some(mesh_pipeline);
        self.mesh_pipeline_mirrored = Some(mesh_pipeline_mirrored);
        self.mesh_pipeline_transparent = Some(mesh_pipeline_transparent);
        self.mesh_pipeline_mirrored_transparent = Some(mesh_pipeline_mirrored_transparent);
        self.mesh_pipeline_merged = Some(mesh_pipeline_merged);
        self.mesh_pipeline_merged_mirrored = Some(mesh_pipeline_merged_mirrored);
        self.image_pipeline = Some(image_pipeline);
        self.image_texture_layout = Some(image_texture_layout);
        self.image_sampler = Some(image_sampler);
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
                buffers: &[
                    Some(mesh_vertex_layout()),
                    Some(mesh_normal_layout()),
                    Some(mesh_color_layout()),
                ],
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

    /// Create one merged-mesh pipeline: triangle list, depth tested, back-face
    /// culled, with the flat-merge interleaved vertex layout and depth writes on
    /// (merged chunks are always opaque).
    fn create_mesh_merged_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
        front_face: wgpu::FrontFace,
        label: &str,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(mesh_merged_layout())],
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
                depth_write_enabled: Some(true),
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

    /// Capability snapshot derived from the actual shared device, not the request.
    fn caps_for(device: &wgpu::Device) -> BackendCapabilities {
        let limits = device.limits();
        let (actual, api) = match device.adapter_info().backend {
            wgpu::Backend::BrowserWebGpu => (ActiveBackend::WebGpu, "webgpu"),
            wgpu::Backend::Gl if cfg!(target_arch = "wasm32") => (ActiveBackend::WebGl2, "webgl2"),
            wgpu::Backend::Vulkan => (ActiveBackend::Native, "vulkan"),
            wgpu::Backend::Metal => (ActiveBackend::Native, "metal"),
            wgpu::Backend::Dx12 => (ActiveBackend::Native, "dx12"),
            wgpu::Backend::Gl => (ActiveBackend::Native, "opengl"),
            _ => (ActiveBackend::Native, "unknown"),
        };
        match actual {
            ActiveBackend::WebGl2 => BackendCapabilities {
                actual,
                api,
                compute: false,
                storage_buffers: false,
                indirect_draw: false,
                max_texture_dimension: limits
                    .max_texture_dimension_2d
                    .max(MIN_GUARANTEED_TEXTURE_DIMENSION),
            },
            _ => BackendCapabilities {
                actual,
                api,
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
        let caps = Self::caps_for(device);
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
        // A frame larger than the adapter's texture limit must fail with a
        // structured frame error, not a `create_texture` validation panic.
        let limit = device.limits().max_texture_dimension_2d;
        if size.width > limit || size.height > limit {
            return Err(RenderError::Frame(format!(
                "render target {}x{} exceeds the adapter's maximum texture dimension {limit}",
                size.width, size.height
            )));
        }
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
        self.texture_revision += 1;
        self.target_size = (size.width, size.height);
        Ok(())
    }

    // Upload CPU batches as static GPU buffers (once per change).
    //
    // Mesh batches get an index buffer and a repaired normal buffer; line
    // batches keep the position-only layout.
    pub fn upload(&mut self, delta: &SceneDelta) -> CadResult<()> {
        let prepared = self.prepare_upload(delta)?;
        self.commit_upload(prepared, self.batches.len())
    }

    /// Build resources without changing the active scene. Synchronous input and
    /// device-limit validation precedes allocation; async driver failures remain
    /// covered by the render error scope/device lifecycle, not this return value.
    pub fn prepare_upload(&self, delta: &SceneDelta) -> CadResult<PreparedUpload> {
        self.prepare_upload_batches(&delta.added)
    }

    /// Stage a bounded borrowed batch slice without cloning a full scene delta.
    /// Active GPU batches are unchanged until explicit atomic commit.
    pub fn prepare_upload_batches(
        &self,
        batches: &[cad_scene::RenderBatch],
    ) -> CadResult<PreparedUpload> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?;
        let started = Instant::now();
        let mut staged = Vec::with_capacity(batches.len());
        let mut uploaded_bytes = 0;
        // Which batches convert to the instanced line path and which to the
        // flat-merge mesh path this upload. Computed before validation so the
        // device-limit check uses the true upload stride per path.
        let line_converted = convert_line_batches(batches, self.line_instancing, self.mesh_merging);
        let mesh_converted: Vec<usize> = if self.mesh_merging {
            batches
                .iter()
                .enumerate()
                .filter(|(_, batch)| mesh_batch_is_mergeable(batch))
                .map(|(index, _)| index)
                .collect()
        } else {
            Vec::new()
        };
        let mut converted_mask = vec![false; batches.len()];
        for source in &line_converted {
            converted_mask[source.batch] = true;
        }
        for &index in &mesh_converted {
            converted_mask[index] = true;
        }
        let cap = self.max_instances_per_stream.max(1);
        let mesh_cap = self.max_vertices_per_mesh_stream.max(3);
        let max_buffer_size = device.limits().max_buffer_size;
        for (index, batch) in batches.iter().enumerate() {
            // Conservative *estimates* of the largest single buffer this batch can
            // produce, used only as an early gate. A converted batch uploads
            // 56-byte instances (one per segment) or 44-byte merged vertices (one
            // per stored triangle vertex) and 4-byte merged indices, never the
            // legacy 12 B/vertex; a batch over a cap is split, so the largest
            // single vertex/instance buffer stays within the cap. These bounds are
            // deliberately loose (an overlay's segment count uses the
            // `3·triangles/2` edge bound, not `vertices/2`); the *authoritative*
            // size checks are the per-stream/per-chunk guards at upload below
            // (`bytes.len() > max_buffer_size`), which see the real buffer
            // payloads after the cap split.
            let vertex_bytes: u64 = if converted_mask[index] {
                let instance_bytes = if self.mesh_merging && mesh_batch_is_mergeable(batch) {
                    batch
                        .vertices
                        .len()
                        .min(mesh_cap)
                        .saturating_mul(MESH_VERTEX_STRIDE as usize) as u64
                } else {
                    0
                };
                // A `Lines` batch has `vertices/2` segments; a mesh edge overlay
                // has one segment per edge and `edges <= 3·triangles` before
                // dedup, so `3·triangles/2` upper-bounds its segments. Take the
                // larger bound.
                let segments =
                    (batch.vertices.len() / 2).max(batch.indices.len().saturating_mul(3) / 2);
                let line_bytes = segments
                    .min(cap)
                    .saturating_mul(LINE_INSTANCE_STRIDE as usize)
                    as u64;
                instance_bytes + line_bytes
            } else {
                (batch.vertices.len() as u64).saturating_mul(12)
            };
            if batch.vertices.len() > u32::MAX as usize
                || batch.indices.len() > u32::MAX as usize / 3
                || vertex_bytes > max_buffer_size
                || batch.indices.len().saturating_mul(24) as u64 > max_buffer_size
                || batch
                    .indices
                    .iter()
                    .flatten()
                    .any(|i| *i as usize >= batch.vertices.len())
                || batch.vertices.iter().flatten().any(|v| !v.is_finite())
            {
                return Err(CadError::InvalidInput(
                    "invalid or over-limit GPU batch".into(),
                ));
            }
        }

        // --- instanced line streams -----------------------------------------
        //
        // Convert eligible opaque `LineList` batches (and the edge overlays of
        // merged meshes) into static instance streams here, at upload. Each
        // stream is a spatially bounded chunk with its own `f64` origin (see
        // `build_instance_chunks`); segments are packed in original batch order,
        // so no re-sorting happens here. The per-instance pick key is
        // first-class (see `LineInstance`); this slice does not yet consume the
        // flags.
        let mut instance_streams: Vec<InstanceStream> = Vec::new();
        let mut batch_runs: Vec<Option<Vec<InstanceRun>>> = vec![None; batches.len()];
        if !line_converted.is_empty() {
            let (chunks, runs) = build_instance_chunks(
                batches,
                &line_converted,
                self.max_instances_per_stream,
                MAX_STREAM_RADIUS,
            );
            batch_runs = runs;
            for chunk in chunks {
                // Each chunk is non-empty and within the cap, so its buffer is
                // non-zero and legal; guard the device limit defensively.
                let bytes = line_instance_bytes(&chunk.instances);
                if bytes.len() as u64 > max_buffer_size {
                    return Err(CadError::InvalidInput(
                        "invalid or over-limit GPU batch".into(),
                    ));
                }
                let buffer = Self::vertex_buffer(device, self.queue.as_ref(), &bytes);
                instance_streams.push(InstanceStream {
                    buffer,
                    instance_count: chunk.instances.len() as u32,
                    origin: chunk.origin,
                });
            }
        }

        // --- merged mesh chunks ---------------------------------------------
        //
        // Concatenate adjacent same-pipeline opaque `Mesh` batches into one
        // interleaved vertex buffer per chunk and draw each contiguous run as one
        // indexed draw (see `build_mesh_chunks`). The per-batch tint is baked
        // into the per-vertex colour (bit-identical to the legacy per-pixel
        // product only for a constant-colour mesh or a white tint) and a
        // per-vertex `entity_flags` carries the first source's `EntityId` low
        // bits for a later picking phase (the legacy per-batch draw carried no
        // explicit pick key).
        let mut mesh_chunk_buffers: Vec<GpuMeshChunk> = Vec::new();
        let mut batch_mesh_runs: Vec<Option<Vec<MeshRun>>> = vec![None; batches.len()];
        if !mesh_converted.is_empty() {
            let (chunks, runs) = build_mesh_chunks(
                batches,
                &mesh_converted,
                self.max_vertices_per_mesh_stream,
                MAX_STREAM_RADIUS,
            );
            batch_mesh_runs = runs;
            for chunk in chunks {
                let vertex_bytes = mesh_vertex_bytes(&chunk.vertices);
                let index_bytes: Vec<u8> =
                    chunk.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                if vertex_bytes.len() as u64 > max_buffer_size
                    || index_bytes.len() as u64 > max_buffer_size
                {
                    return Err(CadError::InvalidInput(
                        "invalid or over-limit GPU batch".into(),
                    ));
                }
                let vertex_buffer = Self::vertex_buffer(device, self.queue.as_ref(), &vertex_bytes);
                let index_buffer = Self::index_buffer(device, self.queue.as_ref(), &chunk.indices);
                mesh_chunk_buffers.push(GpuMeshChunk {
                    vertex_buffer,
                    index_buffer,
                    vertex_count: chunk.vertices.len() as u32,
                    index_count: chunk.indices.len() as u32,
                    origin: chunk.origin,
                    mirrored: chunk.mirrored,
                });
            }
        }

        for (index, batch) in batches.iter().enumerate() {
            let run = batch_runs[index].take();
            let mesh_run = batch_mesh_runs[index].take();
            let instanced = run.is_some();
            let merged = mesh_run.is_some();
            // A converted batch owns no per-batch buffers: skip packing its
            // geometry entirely (its bytes live in the shared stream/chunk).
            let (vertices, indices, edge_indices, normals, colors) = if instanced || merged {
                Default::default()
            } else {
                match batch.topology {
                    RenderTopology::Mesh => {
                        let verts = pack_positions(&batch.vertices);
                        let idx: Vec<u32> = batch
                            .indices
                            .iter()
                            .flat_map(|t| [t[0], t[1], t[2]])
                            .collect();
                        let edges = geometry::mesh_edge_indices(batch);
                        let normals = geometry::repaired_normals(batch);
                        // Always bound (white when the batch has no per-vertex
                        // colours), so the mesh pipeline's `@location(2)` is valid.
                        let colors = geometry::repaired_colors(batch);
                        (
                            verts,
                            idx,
                            edges,
                            pack_positions(&normals),
                            pack_positions(&colors),
                        )
                    }
                    RenderTopology::MeshEdges => {
                        let verts = pack_positions(&batch.vertices);
                        let idx: Vec<u32> = (0..batch.vertices.len() as u32).collect();
                        (verts, Vec::new(), idx, Vec::new(), Vec::new())
                    }
                    RenderTopology::Lines => (
                        pack_positions(&batch.vertices),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    ),
                }
            };

            // A converted batch owns no per-batch vertex buffer; its exact upload
            // charge is the instance/merged bytes it contributed (never the
            // legacy vertex bytes).
            let vertex_buffer = if instanced || merged {
                None
            } else {
                Some(Self::vertex_buffer(device, self.queue.as_ref(), &vertices))
            };
            let is_mesh = batch.topology == RenderTopology::Mesh;
            // A merged mesh batch owns no per-batch index buffer, but its
            // triangle-index count must still drive the frame triangle budget,
            // so it is summed from the batch's merged runs.
            let triangle_index_count = if merged {
                mesh_run
                    .as_ref()
                    .map_or(0, |run| run.iter().map(|piece| piece.index_count).sum())
            } else {
                indices.len() as u32
            };
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
            let normal_buffer = if is_mesh && !merged {
                Some(Self::vertex_buffer(device, self.queue.as_ref(), &normals))
            } else {
                None
            };
            let color_buffer = if is_mesh && !merged {
                Some(Self::vertex_buffer(device, self.queue.as_ref(), &colors))
            } else {
                None
            };

            // The exact bytes this batch uploads: its instance-stream share
            // (56 B per segment) and/or its merged-chunk share (44 B per stored
            // vertex + 4 B per triangle index), otherwise positions/normals/
            // colours 12 B per vertex and triangle/edge indices 4 B each. Kept in
            // lockstep with `GpuBatch::upload_size_bytes`, which the frame budget
            // charges.
            let line_bytes = run.as_ref().map_or(0, |run| {
                run.iter()
                    .map(|piece| piece.count as usize * LINE_INSTANCE_STRIDE as usize)
                    .sum()
            });
            let mesh_bytes = mesh_run.as_ref().map_or(0, |run| {
                run.iter()
                    .map(|piece| {
                        piece.vertex_count as usize * MESH_VERTEX_STRIDE as usize
                            + piece.index_count as usize * 4
                    })
                    .sum()
            });
            let bytes = if instanced || merged {
                line_bytes + mesh_bytes
            } else {
                normals.len()
                    + vertices.len()
                    + colors.len()
                    + indices.len() * 4
                    + edge_indices.len() * 4
            };
            uploaded_bytes += bytes as u64;
            staged.push(GpuBatch {
                vertices: vertex_buffer,
                vertex_count: batch.vertices.len() as u32,
                normals: normal_buffer,
                colors: color_buffer,
                topology: batch.topology,
                indices: index_buffer,
                index_count: triangle_index_count,
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
                instanced: run.unwrap_or_default(),
                mesh_runs: mesh_run.unwrap_or_default(),
                upload_bytes: bytes,
            });
        }
        Ok(PreparedUpload {
            batches: staged,
            instances: instance_streams,
            mesh_chunks: mesh_chunk_buffers,
            device_generation: self.device_generation,
            device: device.clone(),
            bytes: uploaded_bytes,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Atomically replace the suffix after `keep_prefix` (0 replaces the scene).
    /// This supports independently updating an annotation overlay while keeping
    /// base drawing buffers. A stale-device upload cannot be published.
    pub fn commit_upload(
        &mut self,
        mut prepared: PreparedUpload,
        keep_prefix: usize,
    ) -> CadResult<()> {
        if self.device.as_ref() != Some(&prepared.device)
            || prepared.device_generation != self.device_generation
            || self.device_lost
            || keep_prefix > self.batches.len()
        {
            return Err(CadError::StaleResult);
        }
        // Re-base the prepared streams' run indices onto the existing streams so
        // a kept prefix's converted batches keep pointing at valid buffers.
        let base = self.instance_streams.len() as u32;
        for batch in &mut prepared.batches {
            for run in &mut batch.instanced {
                run.stream += base;
            }
        }
        let mesh_base = self.mesh_chunks.len() as u32;
        for batch in &mut prepared.batches {
            for run in &mut batch.mesh_runs {
                run.chunk += mesh_base;
            }
        }
        self.instance_stream_uploads += prepared.instances.len() as u64;
        self.mesh_chunk_uploads += prepared.mesh_chunks.len() as u64;
        self.instance_streams.append(&mut prepared.instances);
        self.mesh_chunks.append(&mut prepared.mesh_chunks);
        self.batches.truncate(keep_prefix);
        self.batches.extend(prepared.batches);
        self.gc_instance_streams();
        self.gc_mesh_chunks();
        self.progressive_frame = None;
        self.draw_plan = None;
        self.rebuild_lineweight_cache();
        self.uploaded_bytes += prepared.bytes;
        self.last_upload_ms = Some(prepared.elapsed_ms);
        Ok(())
    }

    /// Drop instance streams no surviving batch references and re-number the
    /// rest, so a truncated scene does not leak GPU buffers across commits.
    fn gc_instance_streams(&mut self) {
        if self.instance_streams.is_empty() {
            return;
        }
        let mut map: Vec<Option<usize>> = vec![None; self.instance_streams.len()];
        let mut next = 0usize;
        for batch in &mut self.batches {
            for run in &mut batch.instanced {
                let slot = &mut map[run.stream as usize];
                let new_index = match *slot {
                    Some(index) => index,
                    None => {
                        *slot = Some(next);
                        next += 1;
                        next - 1
                    }
                };
                run.stream = new_index as u32;
            }
        }
        if next == self.instance_streams.len()
            && map
                .iter()
                .enumerate()
                .all(|(index, slot)| *slot == Some(index))
        {
            // Identity mapping: nothing dropped and no renumbering happened.
            return;
        }
        let mut result: Vec<Option<InstanceStream>> = (0..next).map(|_| None).collect();
        for (old_index, stream) in std::mem::take(&mut self.instance_streams)
            .into_iter()
            .enumerate()
        {
            if let Some(new_index) = map[old_index] {
                result[new_index] = Some(stream);
            }
        }
        self.instance_streams = result
            .into_iter()
            .map(|stream| stream.expect("referenced by a surviving batch"))
            .collect();
    }

    /// Drop merged mesh chunks no surviving batch references and re-number the
    /// rest, so a truncated scene does not leak GPU buffers across commits.
    fn gc_mesh_chunks(&mut self) {
        if self.mesh_chunks.is_empty() {
            return;
        }
        let mut map: Vec<Option<usize>> = vec![None; self.mesh_chunks.len()];
        let mut next = 0usize;
        for batch in &mut self.batches {
            for run in &mut batch.mesh_runs {
                let slot = &mut map[run.chunk as usize];
                let new_index = match *slot {
                    Some(index) => index,
                    None => {
                        *slot = Some(next);
                        next += 1;
                        next - 1
                    }
                };
                run.chunk = new_index as u32;
            }
        }
        if next == self.mesh_chunks.len()
            && map
                .iter()
                .enumerate()
                .all(|(index, slot)| *slot == Some(index))
        {
            // Identity mapping: nothing dropped and no renumbering happened.
            return;
        }
        let mut result: Vec<Option<GpuMeshChunk>> = (0..next).map(|_| None).collect();
        for (old_index, chunk) in std::mem::take(&mut self.mesh_chunks)
            .into_iter()
            .enumerate()
        {
            if let Some(new_index) = map[old_index] {
                result[new_index] = Some(chunk);
            }
        }
        self.mesh_chunks = result
            .into_iter()
            .map(|chunk| chunk.expect("referenced by a surviving batch"))
            .collect();
    }

    /// Upload raster image quads from `images`, resolving each `ResourceKey`
    /// through `cache`.
    ///
    /// Every image that resolves to a decodable [`cad_resources::DecodedImage`]
    /// becomes a textured quad: the RGBA8 bytes go to an `Rgba8UnormSrgb`
    /// texture, a linear clamp-to-edge sampler view/bind group is created, and
    /// `transform` maps the unit-square corners `(0,0),(1,0),(1,1),(0,1)` (UVs
    /// `(0,0),(1,0),(1,1),(0,1)`, two triangles) into world space.
    ///
    /// An image whose resource is absent from the cache (or whose decoded buffer
    /// is malformed) is **skipped**, never drawn as a placeholder, and listed in
    /// [`ImageUploadReport::unresolved`] so the caller can surface
    /// `image.unresolved`. This replaces the resident image set.
    ///
    /// One GPU texture (and bind group) is allocated per unique `ResourceKey`
    /// and shared by every quad that references it, so `N` entities referencing
    /// the same resource upload the texture once. The texture cache is pruned to
    /// the keys of the resident set on each call.
    ///
    /// Reuse is identity-aware rather than key-only: a cached texture is only
    /// reused while `cache.get(key)` yields the *same* [`DecodedImage`] it was
    /// uploaded from. Because the `Renderer` outlives the per-document
    /// [`DecodedImageCache`], a key that reappears with different bytes (for
    /// example two drawings that both reference `logo.png`) rebuilds the texture
    /// instead of sampling the previous drawing's image. Texture bytes are
    /// charged only when a texture is created or rebuilt.
    ///
    /// When a batch carries a `clip` polygon, that surviving convex polygon
    /// (world-space positions with per-vertex texture coordinates) is drawn and
    /// fan-triangulated instead of the unit square; the whole unit square is
    /// drawn only when `clip` is `None`.
    ///
    /// Not implemented, and therefore not claimed: WIPEOUT-style masking and
    /// full draw-order interleaving with geometry batches are not performed —
    /// images draw after opaque/transparent geometry, ordered among themselves
    /// by ascending `draw_order`.
    pub fn upload_images(
        &mut self,
        images: &[ImageBatch],
        cache: &DecodedImageCache,
    ) -> CadResult<ImageUploadReport> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let texture_layout = self
            .image_texture_layout
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();
        let sampler = self
            .image_sampler
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?
            .clone();

        let mut report = ImageUploadReport::default();
        let mut staged = Vec::with_capacity(images.len());
        // Resource keys still referenced after this call; textures for any other
        // key are dropped so the cache tracks the resident set, not history.
        let mut live: std::collections::HashSet<ResourceKey> = std::collections::HashSet::new();
        for batch in images {
            let Some(decoded) = cache.get(&batch.resource) else {
                report.unresolved.push(batch.resource.clone());
                continue;
            };
            let expected = decoded.width as usize * decoded.height as usize * 4;
            if decoded.width == 0 || decoded.height == 0 || decoded.rgba.len() != expected {
                // A malformed decoded buffer is neither drawn nor hidden.
                report.unresolved.push(batch.resource.clone());
                continue;
            }
            // One texture per key: reuse the cached texture when an earlier batch
            // (or a previous call) already uploaded this resource, so N entities
            // referencing the same key allocate a single texture/bind group.
            //
            // Reuse is identity-aware, not key-only: a `ResourceKey` can outlive
            // the bytes it names (each document open builds a fresh
            // `DecodedImageCache` while the `Renderer` persists). Rebuild whenever
            // the cached texture was uploaded from a different decoded image, so
            // two drawings that both reference e.g. `logo.png` with different
            // bytes cannot silently share the first drawing's texture. Within one
            // cache `get` clones the same `Arc`, so sibling batches still reuse.
            let reusable = self
                .image_textures
                .get(&batch.resource)
                .is_some_and(|existing| Arc::ptr_eq(&existing.decoded, &decoded));
            if !reusable {
                let created =
                    Self::build_texture(&device, &queue, &texture_layout, &sampler, &decoded);
                report.bytes += created.upload_bytes as u64;
                self.image_textures.insert(batch.resource.clone(), created);
            }
            let texture = &self.image_textures[&batch.resource];
            let (vertices, indices) = image_quad_vertices(batch);
            let vertex_buffer = Self::vertex_buffer(&device, Some(&queue), &vertices);
            let index_buffer = Self::index_buffer(&device, Some(&queue), &indices);
            let quad_bytes = vertices.len() + indices.len() * 4;
            report.bytes += quad_bytes as u64;
            report.uploaded += 1;
            live.insert(batch.resource.clone());
            staged.push(GpuImage {
                resource: batch.resource.clone(),
                vertex_buffer,
                index_buffer,
                index_count: indices.len() as u32,
                vertex_count: (vertices.len() / 20) as u32,
                texture_bind_group: texture.bind_group.clone(),
                _texture: texture.texture.clone(),
                texture_bytes: texture.upload_bytes,
                alpha: clamp_alpha(batch.alpha),
                draw_order: batch.draw_order,
                upload_bytes: quad_bytes,
            });
        }
        self.image_textures.retain(|key, _| live.contains(key));
        self.images = staged;
        Ok(report)
    }

    /// Upload one decoded RGBA image into a device texture and bind it to the
    /// shared sampler. The returned texture is cached per resource key and
    /// shared by every quad that references it.
    fn build_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        decoded: &Arc<DecodedImage>,
    ) -> GpuTexture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cad-image-texture"),
            size: wgpu::Extent3d {
                width: decoded.width,
                height: decoded.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // `write_texture` needs the row stride padded to the 256-byte copy
        // alignment; the source rows are tightly packed, so copy through a
        // padded scratch buffer only when the stride is not already aligned.
        let row_bytes = decoded.width as usize * 4;
        let aligned = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let padded;
        let data: &[u8] = if aligned == row_bytes {
            decoded.rgba.as_ref()
        } else {
            let mut buffer = vec![0u8; aligned * decoded.height as usize];
            for row in 0..decoded.height as usize {
                buffer[row * aligned..row * aligned + row_bytes]
                    .copy_from_slice(&decoded.rgba[row * row_bytes..(row + 1) * row_bytes]);
            }
            padded = buffer;
            &padded
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aligned as u32),
                rows_per_image: Some(decoded.height),
            },
            wgpu::Extent3d {
                width: decoded.width,
                height: decoded.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-image-texture-bind"),
            layout: texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        GpuTexture {
            texture,
            bind_group,
            upload_bytes: aligned * decoded.height as usize,
            decoded: Arc::clone(decoded),
        }
    }

    /// Opt-in bounded multipass accumulation. Opaque/transparent global order
    /// is preserved across pages, with color/depth retained between submissions.
    pub fn set_progressive_rendering(&mut self, enabled: bool) {
        self.progressive = enabled;
        self.progressive_frame = None;
    }

    pub fn frame_pending(&self) -> bool {
        self.progressive_frame
            .as_ref()
            .is_some_and(|frame| frame.cursor < frame.ordered.len())
    }

    /// Submitted visible batches versus all ordered visible batches.
    pub fn frame_progress(&self) -> Option<(usize, usize)> {
        self.progressive_frame
            .as_ref()
            .map(|frame| (frame.cursor, frame.ordered.len()))
    }

    /// Identity of the actual offscreen attachment, not just its dimensions.
    pub fn texture_revision(&self) -> u64 {
        self.texture_revision
    }

    /// Wall-clock milliseconds of the most recent [`Renderer::upload`] call.
    ///
    /// `None` until an upload has run. This is the real upload phase measured by
    /// the renderer, not a derived estimate.
    pub fn last_upload_ms(&self) -> Option<f64> {
        self.last_upload_ms
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

    /// Rebuild the cached "lineweight carried but not drawn" report from the
    /// resident batch set.
    ///
    /// Called only when `batches` changes (upload/clear/device loss), so the
    /// frame path clones a small Vec instead of scanning every batch.
    fn rebuild_lineweight_cache(&mut self) {
        self.lineweight_not_drawn.clear();
        self.lineweight_not_drawn.extend(
            self.batches
                .iter()
                .filter(|batch| batch.lineweight > 0.0)
                .map(|batch| LineweightNotDrawn {
                    millimeters: batch.lineweight,
                }),
        );
    }

    pub fn clear_batches(&mut self) {
        self.progressive_frame = None;
        self.draw_plan = None;
        self.batches.clear();
        self.instance_streams.clear();
        self.mesh_chunks.clear();
        self.rebuild_lineweight_cache();
    }

    /// Reset derived GPU resources after a device loss.
    ///
    /// The old device cannot be reused, so this only tears state down and marks
    /// the renderer as lost. The host must call [`Renderer::initialize_with_device`]
    /// and re-upload the scene; batches are *not* silently recreated here.
    pub fn rebuild_device(&mut self, _target: &RenderTarget) -> CadResult<()> {
        self.line_pipeline = None;
        self.line_instanced_pipeline = None;
        self.unit_segment_buffer = None;
        self.mesh_pipeline = None;
        self.mesh_pipeline_mirrored = None;
        self.mesh_pipeline_transparent = None;
        self.mesh_pipeline_mirrored_transparent = None;
        self.mesh_pipeline_merged = None;
        self.mesh_pipeline_merged_mirrored = None;
        self.image_pipeline = None;
        self.image_texture_layout = None;
        self.image_sampler = None;
        self.layout = None;
        self.target = None;
        self.target_view = None;
        self.depth_view = None;
        self.batches.clear();
        self.images.clear();
        self.image_textures.clear();
        self.uniforms.reset();
        self.frame_slots.clear();
        self.image_slots.clear();
        self.lineweight_not_drawn.clear();
        self.device = None;
        self.queue = None;
        self.active_backend = None;
        self.device_lost = true;
        self.draw_plan = None;
        self.instance_streams.clear();
        self.mesh_chunks.clear();
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
        self.line_instanced_pipeline = None;
        self.unit_segment_buffer = None;
        self.mesh_pipeline = None;
        self.mesh_pipeline_mirrored = None;
        self.mesh_pipeline_transparent = None;
        self.mesh_pipeline_mirrored_transparent = None;
        self.mesh_pipeline_merged = None;
        self.mesh_pipeline_merged_mirrored = None;
        self.image_pipeline = None;
        self.image_texture_layout = None;
        self.image_sampler = None;
        self.layout = None;
        self.target = None;
        self.target_view = None;
        self.depth_view = None;
        self.batches.clear();
        self.images.clear();
        self.image_textures.clear();
        self.uniforms.reset();
        self.frame_slots.clear();
        self.image_slots.clear();
        self.lineweight_not_drawn.clear();
        self.active_backend = None;
        self.device_lost = true;
        self.draw_plan = None;
        self.instance_streams.clear();
        self.mesh_chunks.clear();
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
        // CAD and clip space are both Y-up. wgpu's viewport maps positive
        // clip Y to the top of the image; negating here mirrors the drawing
        // and disagrees with the host's screen-to-world picking transform.
        let sy = 2.0 / (h * camera.world_per_px);
        // Camera-only world→clip matrix, column-major (WGSL `mat4x4<f32>`). It
        // is the per-batch transform with the batch origin removed, so image
        // vertices (already in world space) can share the same camera uniform.
        let camera_matrix = [
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
            (sx * -camera.center.x) as f32,
            (sy * -camera.center.y) as f32,
            (camera.z_plane as f64 - camera.center.z) as f32,
            1.0,
        ];
        self.render_with_projection(
            FrameProjection::Ortho {
                sx,
                sy,
                center: [camera.center.x, camera.center.y, camera.center.z],
                z_plane: camera.z_plane,
            },
            camera_matrix,
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
        // Camera-only VP, column-major flat, for image vertices that are already
        // in world space.
        let mut camera_matrix = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                camera_matrix[col * 4 + row] = vp[col][row];
            }
        }
        let eye = [
            camera.eye.x as f32,
            camera.eye.y as f32,
            camera.eye.z as f32,
        ];
        self.render_with_projection(
            FrameProjection::Perspective { vp },
            camera_matrix,
            eye,
            target,
        )
    }

    fn render_with_projection(
        &mut self,
        projection: FrameProjection,
        camera_matrix: [f32; 16],
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
        // CPU-path timer: plan + uniforms + encode + submit. Stopped right after
        // `queue.submit` so the blocking device poll below is never counted as
        // CPU work.
        let cpu_started = Instant::now();
        self.ensure_target(target)?;
        // Phase timers below break `cpu_ms` down into plan / uniform / encode /
        // submit; `wait_ms` separately measures the blocking poll (excluded from
        // `cpu_ms`).
        let plan_started = Instant::now();
        // Times the frame-key mismatch reset the progressive cursor this frame.
        let mut progressive_restarts = 0u64;
        // The draw-order partition is camera-independent, so it survives camera
        // moves. Rebuild it only when the batch set changed (the mutation paths
        // that clear or replace `batches` set `draw_plan = None`).
        if self.draw_plan.is_none() {
            let entries = self
                .batches
                .iter()
                .map(|batch| BatchOrderEntry {
                    draw_order: batch.draw_order,
                    alpha: batch.alpha,
                    centroid: batch.centroid,
                })
                .collect();
            self.draw_plan = Some(DrawOrderCache::new(entries));
        }
        let frame_key = FrameKey {
            camera_matrix,
            camera_position,
            target_size: self.target_size,
            texture_revision: self.texture_revision,
        };
        if self.progressive
            && self
                .progressive_frame
                .as_ref()
                .is_none_or(|frame| frame.key != frame_key)
        {
            progressive_restarts += 1;
            // Reuse the previous frame's `ordered` allocation: the restart
            // replaces it, so the old Vec is cleared and refilled instead of a
            // fresh collect per camera move.
            let mut ordered = self
                .progressive_frame
                .take()
                .map(|frame| frame.ordered)
                .unwrap_or_default();
            let cache = self.draw_plan.as_mut().expect("draw plan cache");
            cache.sort_transparent(Some(camera_position));
            let opaque_count = cache.opaque().len();
            let invisible = cache.invisible().len();
            ordered.clear();
            ordered.extend(cache.opaque().iter().chain(cache.transparent()).copied());
            self.progressive_frame = Some(ProgressiveFrame {
                key: frame_key,
                ordered,
                opaque_count,
                invisible,
                cursor: 0,
            });
        }
        // Cloned so the shared uniform arena can be mutated (grown/filled) while
        // this frame is encoded without fighting the borrow checker over `self`.
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?
            .clone();
        let device = self.device.as_ref().unwrap();
        let Some(line_pipeline) = self.line_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(line_instanced_pipeline) = self.line_instanced_pipeline.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(unit_segment_buffer) = self.unit_segment_buffer.as_ref() else {
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
        let Some(mesh_pipeline_merged) = self.mesh_pipeline_merged.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(mesh_pipeline_merged_mirrored) = self.mesh_pipeline_merged_mirrored.as_ref()
        else {
            return Err(RenderError::NotInitialized(
                "renderer resources missing".into(),
            ));
        };
        let Some(image_pipeline) = self.image_pipeline.as_ref() else {
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
        let Some(layout) = self.layout.as_ref() else {
            return Err(RenderError::NotInitialized(
                "renderer uniform layout missing".into(),
            ));
        };

        // Scratch index buffers are taken out of `self` and returned before this
        // function ends, so the opaque/transparent split does not allocate every
        // frame.
        let mut opaque = std::mem::take(&mut self.scratch_opaque);
        let mut transparent = std::mem::take(&mut self.scratch_transparent);
        opaque.clear();
        transparent.clear();

        let (mut budget_plan, invisible_batches, clear) =
            if let Some(frame) = &self.progressive_frame {
                let plan = plan::plan_ordered_page(
                    &self.batches,
                    &frame.ordered,
                    frame.cursor,
                    &self.frame_budget,
                );
                if plan.accepted.is_empty() && frame.cursor < frame.ordered.len() {
                    return Err(RenderError::Frame(
                        "one GPU batch exceeds the progressive frame budget".into(),
                    ));
                }
                let split = frame
                    .opaque_count
                    .saturating_sub(frame.cursor)
                    .min(plan.accepted.len());
                opaque.extend_from_slice(&plan.accepted[..split]);
                transparent.extend_from_slice(&plan.accepted[split..]);
                (plan, frame.invisible, frame.cursor == 0)
            } else {
                let plan = plan_from_gpu(&self.batches, &self.frame_budget);
                let cache = self.draw_plan.as_mut().expect("draw plan cache");
                cache.sort_transparent(Some(camera_position));
                if plan.accepted.len() == self.batches.len() {
                    // Full acceptance keeps upload order, so the cached
                    // camera-sorted partition over all batches is exact.
                    opaque.extend_from_slice(cache.opaque());
                    transparent.extend_from_slice(cache.transparent());
                    (plan, cache.invisible().len(), true)
                } else {
                    // Over budget: `accepted` is an upload-order prefix. Fall
                    // back to the fresh plan over that prefix; this path is rare
                    // (the frame reported over-budget) and must stay identical to
                    // the pre-cache behaviour.
                    let accepted: Vec<usize> = plan.accepted.clone();
                    let entries: Vec<_> = accepted
                        .iter()
                        .map(|&i| BatchOrderEntry {
                            draw_order: self.batches[i].draw_order,
                            alpha: self.batches[i].alpha,
                            centroid: self.batches[i].centroid,
                        })
                        .collect();
                    let passes = draw_passes(&accepted, &entries, Some(camera_position));
                    opaque.extend_from_slice(&passes.opaque);
                    transparent.extend_from_slice(&passes.transparent);
                    (plan, passes.invisible, true)
                }
            };

        // Charge the resident image quads against the same frame budget and
        // assign their uniform slots. Images are drawn last (after opaque and
        // transparent geometry); full draw-order interleaving and WIPEOUT
        // masking are not implemented. The accepted images are ordered among
        // themselves by ascending `draw_order`.
        //
        // Images are drawn only on the page that clears the target (the first
        // page of a progressive frame, or every frame when not progressive).
        // Continuation pages load the existing colour, so re-drawing an
        // `alpha < 1` image every page would accumulate its opacity and charge
        // it repeatedly; a continuation page must not composite images again.
        //
        // Charged here, before the uniform upload, so the image slots are known
        // when the arena capacity is sized.
        let mut draw_images: Vec<usize> = Vec::new();
        if clear {
            let (accepted_images, image_over_budget) =
                charge_images(&mut budget_plan.usage, &self.frame_budget, &self.images);
            if budget_plan.over_budget.is_none() {
                budget_plan.over_budget = image_over_budget;
            }
            draw_images = accepted_images;
            draw_images.sort_by_key(|&i| self.images[i].draw_order);
        }

        // Lineweight is carried, never drawn: the report is cached at batch
        // mutation (`rebuild_lineweight_cache`), so the frame path only clones
        // the small cached Vec instead of scanning every resident batch.
        let lineweight_not_drawn: Vec<LineweightNotDrawn> = self.lineweight_not_drawn.clone();
        let plan_ms = plan_started.elapsed().as_secs_f64() * 1000.0;

        // Slots are assigned per frame in accepted order; the slot maps are
        // reused across frames to avoid a per-frame allocation.
        let uniform_started = Instant::now();
        let accepted_len = budget_plan.accepted.len();
        if self.frame_slots.len() != self.batches.len() {
            self.frame_slots.resize(self.batches.len(), u32::MAX);
        }
        if self.image_slots.len() != self.images.len() {
            self.image_slots.resize(self.images.len(), u32::MAX);
        }
        // Upper bound for arena growth; the frame flushes only the slots it
        // actually filled (converted batches are compacted out; see below).
        let max_slots =
            accepted_len + draw_images.len() + self.instance_streams.len() + self.mesh_chunks.len();
        self.uniforms.ensure_slots(max_slots, device, layout);

        // Write only this page's uniforms. Rewriting every batch for every
        // continuation would turn bounded accumulation into repeated full work.
        // Each uniform is the transform matrix followed by the constant
        // per-batch colour (normalized sRGB) and alpha (both clamped by the same
        // policy used for drawing); they are packed into the shared arena and
        // uploaded with one `write_buffer` per page below.
        //
        // Batches converted to an instanced stream or a merged mesh chunk own no
        // per-batch uniform (the whole stream/chunk shares one), so they are
        // skipped and the legacy slots are compacted: a fully converted scene
        // needs one stream slot and one chunk slot, not one slot per batch.
        let mut legacy_slots = 0usize;
        for &index in &budget_plan.accepted {
            let batch = &self.batches[index];
            if !batch.instanced.is_empty() || !batch.mesh_runs.is_empty() {
                self.frame_slots[index] = u32::MAX;
                continue;
            }
            let (m, [r, g, b], alpha) = (
                projection.batch_matrix(batch.origin),
                renderer_color(batch.color),
                clamp_alpha(batch.alpha),
            );
            let mut uniform = [0.0f32; 20];
            uniform[..16].copy_from_slice(&m);
            uniform[16] = r;
            uniform[17] = g;
            uniform[18] = b;
            uniform[19] = alpha;
            self.uniforms
                .write_slot(legacy_slots, bytemuck::bytes_of(&uniform));
            self.frame_slots[index] = legacy_slots as u32;
            legacy_slots += 1;
        }

        // Image slots follow the geometry slots; the same page upload covers
        // them. Their transform is the camera-only matrix (image vertices are
        // already world space) and the tint is white with the image's alpha.
        for (j, &i) in draw_images.iter().enumerate() {
            let alpha = self.images[i].alpha;
            let mut uniform = [0.0f32; 20];
            uniform[..16].copy_from_slice(&camera_matrix);
            uniform[16] = 1.0;
            uniform[17] = 1.0;
            uniform[18] = 1.0;
            uniform[19] = alpha;
            let slot = legacy_slots + j;
            self.uniforms.write_slot(slot, bytemuck::bytes_of(&uniform));
            self.image_slots[i] = slot as u32;
        }

        // Instanced line streams: one uniform slot per stream, carrying the
        // stream's fixed-point origin through the same `f64` camera subtraction
        // as a legacy batch (see `FrameProjection::batch_matrix_f64`). Every run
        // of a stream shares it, so streams — not batches — bound this cost.
        let stream_base = legacy_slots + draw_images.len();
        let stream_slots: Vec<u32> = (0..self.instance_streams.len())
            .map(|s| (stream_base + s) as u32)
            .collect();
        for (s, &slot) in stream_slots.iter().enumerate() {
            let matrix = projection.batch_matrix_f64(self.instance_streams[s].origin);
            let mut uniform = [0.0f32; 20];
            uniform[..16].copy_from_slice(&matrix);
            // The instanced shader ignores the tint (colour is per-instance) but
            // the uniform layout keeps it; write opaque white.
            uniform[16] = 1.0;
            uniform[17] = 1.0;
            uniform[18] = 1.0;
            uniform[19] = 1.0;
            self.uniforms
                .write_slot(slot as usize, bytemuck::bytes_of(&uniform));
        }

        // Merged mesh chunks: one uniform slot per chunk, carrying the chunk's
        // fixed-point origin through the same `f64` camera subtraction. The tint
        // is opaque white because the per-batch colour is baked into each vertex.
        let chunk_base = stream_base + self.instance_streams.len();
        let chunk_slots: Vec<u32> = (0..self.mesh_chunks.len())
            .map(|c| (chunk_base + c) as u32)
            .collect();
        for (c, &slot) in chunk_slots.iter().enumerate() {
            let matrix = projection.batch_matrix_f64(self.mesh_chunks[c].origin);
            let mut uniform = [0.0f32; 20];
            uniform[..16].copy_from_slice(&matrix);
            uniform[16] = 1.0;
            uniform[17] = 1.0;
            uniform[18] = 1.0;
            uniform[19] = 1.0;
            self.uniforms
                .write_slot(slot as usize, bytemuck::bytes_of(&uniform));
        }

        // One `Queue::write_buffer` per page (vs. one per batch), reported as
        // `FrameStats::uniform_writes`. Only the compacted slot count is flushed.
        let total_slots =
            legacy_slots + draw_images.len() + self.instance_streams.len() + self.mesh_chunks.len();
        let uniform_writes = self.uniforms.flush(&queue, total_slots);
        let uniform_ms = uniform_started.elapsed().as_secs_f64() * 1000.0;

        let mut encode_state = EncodeState::new();
        // Coalesced-instanced bookkeeping (kept outside the render-pass block).
        let mut instanced_draws = 0u64;
        let mut instanced_batches = 0usize;
        let mut mesh_draws = 0u64;
        let mut merged_mesh_batches = 0usize;
        // Opaque batches that issued no legacy per-batch draw because they were
        // converted to the line path and/or the merged mesh path. A mesh batch
        // with an instanced overlay counts once here.
        let mut converted_batches = 0usize;
        let encode_started = Instant::now();
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
                        load: if clear {
                            wgpu::LoadOp::Clear(crate::CAD_CLEAR_COLOR)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: if clear {
                            wgpu::LoadOp::Clear(1.0)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Pass 1 — opaque, in ascending draw order. Depth writes are on.
            //
            // Two conversion paths collapse draws here:
            //
            // * instanced line runs — a maximal sequence of adjacent (in draw
            //   order) batches whose instance slices are contiguous in the same
            //   stream; instances are stored in original batch order, so merging
            //   adjacent slices reproduces the exact draw sequence;
            // * merged mesh runs — a maximal sequence of adjacent batches whose
            //   triangle ranges are contiguous in the same chunk; the chunk's
            //   vertices are concatenated in upload order and only *contiguous*
            //   ranges coalesce, so equal-depth ties are broken by in-order
            //   rasterization exactly as the sequential draws were.
            //
            // Frame-discipline invariant: opaque meshes and opaque lines both
            // write depth (`Less` for meshes, `LessEqual` for lines) and blend by
            // replace, so their *cross-class* draw order is not observable — a
            // line wins an equal-depth tie and the nearer fragment wins otherwise.
            // This is what lets a mesh merge across an intervening line batch.
            // Fenced by: `merged_meshes_match_legacy_pixels` (non-overlapping
            // grid), `merged_overlapping_ties_match_legacy_pixels` (mesh-mesh,
            // line-mesh and line-line fragments sharing pixels at equal depth,
            // plus a reversed-order non-vacuous control),
            // `merged_meshes_match_legacy_in_perspective` (Camera3d), and the
            // real-fixture `realscene_render_frame_perf` leg in `cad-cli-tools`
            // (a merged/instanced frame of an opened DWG on the real adapter —
            // non-blank and adapter-labelled, *not* a byte-identity comparison).
            //
            // A mesh run's start flushes the pending line run, so a mesh keeps its
            // interleaved position relative to the lines around it (matching the
            // legacy per-batch draw count in a mixed scene). A line batch does
            // **not** flush a *pending mesh* run while its pending line run can
            // still be extended; when a line run ends (a non-contiguous run
            // follows) the pending mesh run is flushed first, so
            // `mesh A + overlay A`, line, `mesh B` keeps the legacy
            // `A.tri, A.overlay, line, B.tri` order instead of drawing `A.tri`
            // after the line (fenced by
            // `merged_mesh_overlay_line_mesh_keeps_legacy_order`).
            let mut pending: Option<(usize, u32, u32)> = None;
            let mut pending_mesh: Option<(usize, u32, u32)> = None;
            for &i in &opaque {
                let batch = &self.batches[i];
                let has_mesh = !batch.mesh_runs.is_empty();
                let has_line = !batch.instanced.is_empty();
                if !has_mesh && !has_line {
                    // Legacy per-batch draw: flush both pending runs.
                    if flush_pending_mesh(
                        &mut pass,
                        &mut encode_state,
                        &mut pending_mesh,
                        &self.uniforms,
                        &chunk_slots,
                        &self.mesh_chunks,
                        mesh_pipeline_merged,
                        mesh_pipeline_merged_mirrored,
                    ) {
                        mesh_draws += 1;
                    }
                    if flush_pending_run(
                        &mut pass,
                        &mut encode_state,
                        &mut pending,
                        &self.uniforms,
                        &stream_slots,
                        &self.instance_streams,
                        unit_segment_buffer,
                        line_instanced_pipeline,
                    ) {
                        instanced_draws += 1;
                    }
                    let (bind_group, offset) = self.uniforms.binding(self.frame_slots[i] as usize);
                    draw_batch(
                        &mut pass,
                        &mut encode_state,
                        &self.batches[i],
                        bind_group,
                        offset,
                        line_pipeline,
                        if self.batches[i].mirrored {
                            mesh_pipeline_mirrored
                        } else {
                            mesh_pipeline
                        },
                    );
                    continue;
                }
                converted_batches += 1;
                if has_mesh {
                    merged_mesh_batches += 1;
                    // A batch over the vertex cap contributes several runs (one
                    // per piece); a normal batch one.
                    for run in &batch.mesh_runs {
                        let contiguous = matches!(
                            pending_mesh,
                            Some((chunk, _, end))
                                if chunk == run.chunk as usize && end == run.index_offset
                        );
                        if contiguous {
                            let (chunk, start, _) = pending_mesh.expect("contiguous mesh run");
                            pending_mesh = Some((chunk, start, run.index_offset + run.index_count));
                            continue;
                        }
                        if flush_pending_mesh(
                            &mut pass,
                            &mut encode_state,
                            &mut pending_mesh,
                            &self.uniforms,
                            &chunk_slots,
                            &self.mesh_chunks,
                            mesh_pipeline_merged,
                            mesh_pipeline_merged_mirrored,
                        ) {
                            mesh_draws += 1;
                        }
                        // A new mesh run starts: flush the pending line run so the
                        // mesh keeps its interleaved position among the lines.
                        if flush_pending_run(
                            &mut pass,
                            &mut encode_state,
                            &mut pending,
                            &self.uniforms,
                            &stream_slots,
                            &self.instance_streams,
                            unit_segment_buffer,
                            line_instanced_pipeline,
                        ) {
                            instanced_draws += 1;
                        }
                        pending_mesh = Some((
                            run.chunk as usize,
                            run.index_offset,
                            run.index_offset + run.index_count,
                        ));
                    }
                }
                if has_line {
                    // Instanced batch: draw each of its runs in order. A batch
                    // over the stream cap contributes several runs; a normal
                    // batch one.
                    instanced_batches += 1;
                    for run in &batch.instanced {
                        if let Some((stream, _, end)) = pending {
                            if stream == run.stream as usize && end == run.offset {
                                let start = pending.map_or(0, |pending| pending.1);
                                pending = Some((stream, start, run.offset + run.count));
                                continue;
                            }
                        }
                        // This line run cannot extend the pending run, so the
                        // line-coalescing window ends here. If a line run is
                        // actually pending (a draw is about to be issued), flush
                        // the pending merged-mesh run *first* so a mesh that
                        // would otherwise merge across this line batch keeps its
                        // legacy position relative to it: legacy order for
                        // `mesh A + overlay A`, line, `mesh B` is
                        // `A.tri, A.overlay, line, B.tri`. Without this flush the
                        // merged `A+B.tri` run would draw after the line. Exact
                        // depth ties still resolve line-wins, but non-exact
                        // coplanar fragments could flip on other vendors.
                        if pending.is_some()
                            && flush_pending_mesh(
                                &mut pass,
                                &mut encode_state,
                                &mut pending_mesh,
                                &self.uniforms,
                                &chunk_slots,
                                &self.mesh_chunks,
                                mesh_pipeline_merged,
                                mesh_pipeline_merged_mirrored,
                            )
                        {
                            mesh_draws += 1;
                        }
                        if flush_pending_run(
                            &mut pass,
                            &mut encode_state,
                            &mut pending,
                            &self.uniforms,
                            &stream_slots,
                            &self.instance_streams,
                            unit_segment_buffer,
                            line_instanced_pipeline,
                        ) {
                            instanced_draws += 1;
                        }
                        pending = Some((run.stream as usize, run.offset, run.offset + run.count));
                    }
                }
            }
            if flush_pending_mesh(
                &mut pass,
                &mut encode_state,
                &mut pending_mesh,
                &self.uniforms,
                &chunk_slots,
                &self.mesh_chunks,
                mesh_pipeline_merged,
                mesh_pipeline_merged_mirrored,
            ) {
                mesh_draws += 1;
            }
            if flush_pending_run(
                &mut pass,
                &mut encode_state,
                &mut pending,
                &self.uniforms,
                &stream_slots,
                &self.instance_streams,
                unit_segment_buffer,
                line_instanced_pipeline,
            ) {
                instanced_draws += 1;
            }
            // Pass 2 — transparent, back-to-front. Depth writes are off so each
            // blended layer composites over the already-resolved opaque pass.
            for &i in &transparent {
                let (bind_group, offset) = self.uniforms.binding(self.frame_slots[i] as usize);
                draw_batch(
                    &mut pass,
                    &mut encode_state,
                    &self.batches[i],
                    bind_group,
                    offset,
                    line_pipeline,
                    if self.batches[i].mirrored {
                        mesh_pipeline_mirrored_transparent
                    } else {
                        mesh_pipeline_transparent
                    },
                );
            }
            // Pass 3 — raster images, after opaque/transparent geometry. Full
            // draw-order interleaving with geometry batches is NOT implemented;
            // images are ordered among themselves by ascending draw_order.
            for &i in &draw_images {
                let (bind_group, offset) = self.uniforms.binding(self.image_slots[i] as usize);
                draw_image(
                    &mut pass,
                    &mut encode_state,
                    &self.images[i],
                    bind_group,
                    offset,
                    image_pipeline,
                );
            }
        }
        let encode_ms = encode_started.elapsed().as_secs_f64() * 1000.0;
        // Capture the split sizes before the scratch Vecs are returned to `self`
        // below.
        let opaque_batches = opaque.len();
        let transparent_batches = transparent.len();
        // Converted batches own no per-batch draw; they contribute coalesced
        // instanced draws and/or merged mesh draws instead (a batch over a cap
        // contributes one draw per run). Everything else stays one draw per batch.
        let legacy_opaque = opaque_batches.saturating_sub(converted_batches);
        let draw_calls = (legacy_opaque
            + instanced_draws as usize
            + mesh_draws as usize
            + transparent_batches
            + draw_images.len()) as u64;

        let submit_started = Instant::now();
        let submission = queue.submit(Some(encoder.finish()));
        let submit_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
        // Stop the CPU timer here: everything above is the render thread's CPU
        // work (plan/cache, uniforms, encode, submit). The poll below blocks on
        // the GPU and is not CPU time.
        let cpu_ms = cpu_started.elapsed().as_secs_f64() * 1000.0;
        // Distinguish device loss from a bad frame. A bounded wait classifies
        // the submission; the host's device-lost callback is the authoritative
        // loss signal (see `note_device_lost`).
        let wait_started = Instant::now();
        let outcome = self.device_scope_outcome(device, submission);
        let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
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
        if let Some(frame) = &mut self.progressive_frame {
            frame.cursor += budget_plan.accepted.len();
        }
        // Return the scratch index buffers so the next frame reuses them.
        self.scratch_opaque = opaque;
        self.scratch_transparent = transparent;
        Ok(FrameStats {
            cpu_ms,
            gpu_ms: None,
            draw_calls,
            uniform_writes,
            uploaded_bytes: self.uploaded_bytes,
            vertices: budget_plan.usage.vertices,
            triangles: budget_plan.usage.triangles,
            over_budget: budget_plan.over_budget,
            opaque_batches,
            transparent_batches,
            invisible_batches,
            image_batches: draw_images.len(),
            lineweight_not_drawn: lineweight_not_drawn.clone(),
            lineweight_reason: lineweight_reason(&lineweight_not_drawn),
            plan_ms,
            uniform_ms,
            encode_ms,
            submit_ms,
            wait_ms,
            set_pipeline_calls: encode_state.tally.set_pipeline_calls,
            set_bind_group_calls: encode_state.tally.set_bind_group_calls,
            set_vertex_buffer_calls: encode_state.tally.set_vertex_buffer_calls,
            set_index_buffer_calls: encode_state.tally.set_index_buffer_calls,
            uniform_slots: total_slots,
            progressive_restarts,
            instanced_draws,
            instanced_batches,
            instance_streams: self.instance_streams.len(),
            mesh_draws,
            merged_mesh_batches,
            mesh_chunks: self.mesh_chunks.len(),
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

    /// Enable or disable converting eligible opaque line batches into instanced
    /// streams. Enabled by default; disabling forces the legacy per-batch path
    /// (used to compare the two paths). The two are byte-identical only for an
    /// `f32`-representable shared origin; otherwise the instanced path is bound
    /// to `MAX_STREAM_RADIUS` rather than exact (see the deep-zoom fence).
    ///
    /// The flag takes effect on the **next** upload: the resident scene keeps
    /// rendering with the streams it already holds, so a batch set is never left
    /// with dangling stream references. Re-upload (or `clear_batches`) after
    /// toggling to switch the resident scene onto the other path.
    pub fn set_line_instancing(&mut self, enabled: bool) {
        self.line_instancing = enabled;
    }

    /// Enable or disable flat-merging eligible opaque mesh batches into chunks
    /// (and instancing their edge overlays). Enabled by default; disabling forces
    /// the legacy per-batch mesh path so the two can be compared.
    ///
    /// The flag takes effect on the **next** upload: the resident scene keeps
    /// rendering with the chunks it already holds, so a batch set is never left
    /// with dangling chunk references. Re-upload (or `clear_batches`) after
    /// toggling to switch the resident scene onto the other path.
    pub fn set_mesh_merging(&mut self, enabled: bool) {
        self.mesh_merging = enabled;
    }

    /// Override the maximum vertices per merged mesh chunk (default
    /// [`plan::MAX_VERTICES_PER_MESH_STREAM`]). A batch with more vertices is
    /// split across several chunks. Test hook. Takes effect on the next upload.
    pub fn set_max_vertices_per_mesh_stream(&mut self, max_vertices: usize) {
        self.max_vertices_per_mesh_stream = max_vertices;
    }

    /// Override the maximum instances per instanced stream buffer (default
    /// [`MAX_INSTANCES_PER_STREAM`]). A batch with more segments is split across
    /// several streams. Test hook: it lets the split path be exercised without
    /// allocating a million-segment batch. Takes effect on the next upload.
    pub fn set_max_instances_per_stream(&mut self, max_instances: usize) {
        self.max_instances_per_stream = max_instances;
    }

    /// Number of resident instanced line streams.
    pub fn instance_stream_count(&self) -> usize {
        self.instance_streams.len()
    }

    /// Total number of instanced segments (one per 2-vertex pair) across all
    /// resident streams.
    pub fn instance_segment_count(&self) -> u64 {
        self.instance_streams
            .iter()
            .map(|stream| u64::from(stream.instance_count))
            .sum()
    }

    /// Monotonic count of instanced stream buffers uploaded since the last
    /// device initialization. A camera move (pan/zoom) must never increase it.
    pub fn instance_stream_uploads(&self) -> u64 {
        self.instance_stream_uploads
    }

    /// Number of resident merged mesh chunks.
    pub fn mesh_chunk_count(&self) -> usize {
        self.mesh_chunks.len()
    }

    /// Total number of merged mesh vertices across all resident chunks.
    pub fn mesh_vertex_count(&self) -> u64 {
        self.mesh_chunks
            .iter()
            .map(|chunk| u64::from(chunk.vertex_count))
            .sum()
    }

    /// Monotonic count of merged mesh chunk buffers uploaded since the last
    /// device initialization. A camera move (pan/zoom) must never increase it.
    pub fn mesh_chunk_uploads(&self) -> u64 {
        self.mesh_chunk_uploads
    }

    /// Number of distinct GPU textures resident for the current image set.
    ///
    /// Image quads that share a [`ResourceKey`] share one texture, so this is
    /// the count of unique resources, not of quads.
    pub fn image_texture_count(&self) -> usize {
        self.image_textures.len()
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

/// Per-batch world→clip projection for one frame.
///
/// The 2D variant reproduces, bit for bit, the f64 camera-centre subtraction the
/// large-coordinate precision test fences before narrowing to `f32`; the 3D
/// variant multiplies the view-projection by the batch origin (matching the
/// previous `translate_left(&vp, origin)`). Building the matrix inline per
/// accepted batch avoids materialising and cloning a full `Vec<[f32; 16]>` every
/// frame.
#[derive(Clone, Copy)]
enum FrameProjection {
    Ortho {
        sx: f64,
        sy: f64,
        /// Camera centre, kept in f64 so the subtraction stays precise.
        center: [f64; 3],
        z_plane: f32,
    },
    Perspective {
        vp: [[f32; 4]; 4],
    },
}

impl FrameProjection {
    fn batch_matrix(self, origin: [f32; 3]) -> [f32; 16] {
        self.batch_matrix_f64([
            f64::from(origin[0]),
            f64::from(origin[1]),
            f64::from(origin[2]),
        ])
    }

    /// Per-batch matrix for an `f64` origin.
    ///
    /// The **Ortho** variant keeps the origin and the camera centre in `f64`
    /// through the subtraction before narrowing — the precision contract fenced
    /// by `large_coordinate_batch_keeps_precision`. The instanced line path
    /// passes its stream's exact `f64` fixed-point origin here, so stream
    /// positions stay small even at large drawing coordinates. The **Perspective**
    /// variant narrows the origin to `f32` before `translate_left`, so this `f64`
    /// camera-relative subtraction does not apply on the 3D path.
    fn batch_matrix_f64(self, origin: [f64; 3]) -> [f32; 16] {
        match self {
            FrameProjection::Ortho {
                sx,
                sy,
                center,
                z_plane,
            } => [
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
                (sx * (origin[0] - center[0])) as f32,
                (sy * (origin[1] - center[1])) as f32,
                (origin[2] - center[2]) as f32 + z_plane,
                1.0,
            ],
            // Perspective narrows the origin to `f32` *before* applying the
            // (already `f32`) view-projection, so the `f64` camera-relative
            // subtraction claimed for the Ortho arm above does not apply here.
            FrameProjection::Perspective { vp } => {
                translate_left(&vp, [origin[0] as f32, origin[1] as f32, origin[2] as f32])
            }
        }
    }
}
