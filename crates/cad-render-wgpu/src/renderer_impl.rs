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
            mesh_pipeline: None,
            mesh_pipeline_mirrored: None,
            mesh_pipeline_transparent: None,
            mesh_pipeline_mirrored_transparent: None,
            image_pipeline: None,
            image_texture_layout: None,
            image_sampler: None,
            target: None,
            target_view: None,
            depth_view: None,
            target_size: (1, 1),
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            batches: Vec::new(),
            images: Vec::new(),
            image_textures: HashMap::new(),
            device_generation: 0,
            texture_revision: 0,
            uploaded_bytes: 0,
            last_upload_ms: None,
            draw_calls: 0,
            progressive: false,
            progressive_frame: None,
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
        self.target = None;
        self.target_view = None;
        self.depth_view = None;
        self.target_size = (0, 0);
        self.device = Some(device);
        self.queue = Some(queue);
        self.layout = Some(layout);
        self.line_pipeline = Some(line_pipeline);
        self.mesh_pipeline = Some(mesh_pipeline);
        self.mesh_pipeline_mirrored = Some(mesh_pipeline_mirrored);
        self.mesh_pipeline_transparent = Some(mesh_pipeline_transparent);
        self.mesh_pipeline_mirrored_transparent = Some(mesh_pipeline_mirrored_transparent);
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
        let layout = self
            .layout
            .as_ref()
            .ok_or_else(|| CadError::GpuFailure("renderer not initialized".into()))?;
        let started = Instant::now();
        let mut staged = Vec::with_capacity(batches.len());
        let mut uploaded_bytes = 0;
        for batch in batches {
            if batch.vertices.len() > u32::MAX as usize
                || batch.indices.len() > u32::MAX as usize / 3
                || batch.vertices.len().saturating_mul(12) as u64 > device.limits().max_buffer_size
                || batch.indices.len().saturating_mul(24) as u64 > device.limits().max_buffer_size
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
        for batch in batches {
            let (vertices, indices, edge_indices, normals, colors) = match batch.topology {
                RenderTopology::Mesh => {
                    let verts = pack_positions(&batch.vertices);
                    let idx: Vec<u32> = batch
                        .indices
                        .iter()
                        .flat_map(|t| [t[0], t[1], t[2]])
                        .collect();
                    let edges = geometry::sorted_edge_indices(&batch.indices);
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
            let color_buffer = if is_mesh {
                Some(Self::vertex_buffer(device, self.queue.as_ref(), &colors))
            } else {
                None
            };

            let (camera, bind_group) = Self::make_uniform(device, layout);
            // The exact bytes this batch packed: positions/normals/colours 12 B
            // per vertex, triangle and edge indices 4 B each. Kept in lockstep
            // with `GpuBatch::upload_size_bytes`, which the frame budget charges.
            let bytes = normals.len()
                + vertices.len()
                + colors.len()
                + indices.len() * 4
                + edge_indices.len() * 4;
            uploaded_bytes += bytes as u64;
            staged.push(GpuBatch {
                vertices: vertex_buffer,
                vertex_count: batch.vertices.len() as u32,
                normals: normal_buffer,
                colors: color_buffer,
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
                upload_bytes: bytes,
            });
        }
        Ok(PreparedUpload {
            batches: staged,
            device_generation: self.device_generation,
            device: device.clone(),
            bytes: uploaded_bytes,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Atomically replace the suffix after `keep_prefix` (0 replaces the scene).
    /// This supports independently updating an annotation overlay while keeping
    /// base drawing buffers. A stale-device upload cannot be published.
    pub fn commit_upload(&mut self, prepared: PreparedUpload, keep_prefix: usize) -> CadResult<()> {
        if self.device.as_ref() != Some(&prepared.device)
            || prepared.device_generation != self.device_generation
            || self.device_lost
            || keep_prefix > self.batches.len()
        {
            return Err(CadError::StaleResult);
        }
        self.batches.truncate(keep_prefix);
        self.batches.extend(prepared.batches);
        self.progressive_frame = None;
        self.uploaded_bytes += prepared.bytes;
        self.last_upload_ms = Some(prepared.elapsed_ms);
        Ok(())
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
        let layout = self
            .layout
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
            let (camera, camera_bind_group) = Self::make_uniform(&device, &layout);
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
                camera,
                camera_bind_group,
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

    pub fn clear_batches(&mut self) {
        self.progressive_frame = None;
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
        // CAD and clip space are both Y-up. wgpu's viewport maps positive
        // clip Y to the top of the image; negating here mirrors the drawing
        // and disagrees with the host's screen-to-world picking transform.
        let sy = 2.0 / (h * camera.world_per_px);
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
        self.render_with_transforms(
            transforms,
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
        let transforms = self
            .batches
            .iter()
            .map(|batch| translate_left(&vp, batch.origin))
            .collect();
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
        self.render_with_transforms(transforms, camera_matrix, eye, target)
    }

    fn render_with_transforms(
        &mut self,
        transforms: Vec<[f32; 16]>,
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
        self.ensure_target(target)?;
        if self.progressive
            && self.progressive_frame.as_ref().is_none_or(|frame| {
                frame.transforms != transforms
                    || frame.camera_position != camera_position
                    || frame.texture_revision != self.texture_revision
            })
        {
            let indices: Vec<_> = (0..self.batches.len()).collect();
            let entries: Vec<_> = self
                .batches
                .iter()
                .map(|batch| BatchOrderEntry {
                    draw_order: batch.draw_order,
                    alpha: batch.alpha,
                    centroid: batch.centroid,
                })
                .collect();
            let passes = draw_passes(&indices, &entries, Some(camera_position));
            let opaque_count = passes.opaque.len();
            self.progressive_frame = Some(ProgressiveFrame {
                transforms: transforms.clone(),
                camera_position,
                texture_revision: self.texture_revision,
                ordered: passes
                    .opaque
                    .into_iter()
                    .chain(passes.transparent)
                    .collect(),
                opaque_count,
                invisible: passes.invisible,
                cursor: 0,
            });
        }
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

        let (mut budget_plan, opaque, transparent, invisible_batches, clear) =
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
                let opaque = plan.accepted[..split].to_vec();
                let transparent = plan.accepted[split..].to_vec();
                (
                    plan,
                    opaque,
                    transparent,
                    frame.invisible,
                    frame.cursor == 0,
                )
            } else {
                let plan = plan_from_gpu(&self.batches, &self.frame_budget);
                let entries: Vec<_> = plan
                    .accepted
                    .iter()
                    .map(|&i| BatchOrderEntry {
                        draw_order: self.batches[i].draw_order,
                        alpha: self.batches[i].alpha,
                        centroid: self.batches[i].centroid,
                    })
                    .collect();
                let passes = draw_passes(&plan.accepted, &entries, Some(camera_position));
                (
                    plan,
                    passes.opaque,
                    passes.transparent,
                    passes.invisible,
                    true,
                )
            };

        // Write only this page's uniforms. Rewriting every batch for every
        // continuation would turn bounded accumulation into repeated full work.
        // Write the per-batch uniforms before encoding. The uniform is the
        // transform matrix followed by the constant per-batch colour (normalized
        // sRGB) and alpha (both clamped by the same policy used for drawing).
        for &index in &budget_plan.accepted {
            let batch = &self.batches[index];
            let m = &transforms[index];
            let mut uniform = [0.0f32; 20];
            uniform[..16].copy_from_slice(m);
            let [r, g, b] = renderer_color(batch.color);
            uniform[16] = r;
            uniform[17] = g;
            uniform[18] = b;
            uniform[19] = clamp_alpha(batch.alpha);
            queue.write_buffer(&batch.camera, 0, bytemuck::cast_slice(&uniform));
        }

        // Charge the resident image quads against the same frame budget and
        // write their camera uniforms. Images are drawn last (after opaque and
        // transparent geometry); full draw-order interleaving and WIPEOUT
        // masking are not implemented. The accepted images are ordered among
        // themselves by ascending `draw_order`.
        //
        // Images are drawn only on the page that clears the target (the first
        // page of a progressive frame, or every frame when not progressive).
        // Continuation pages load the existing colour, so re-drawing an
        // `alpha < 1` image every page would accumulate its opacity and charge
        // it repeatedly; a continuation page must not composite images again.
        let mut draw_images: Vec<usize> = Vec::new();
        if clear {
            let (accepted_images, image_over_budget) =
                charge_images(&mut budget_plan.usage, &self.frame_budget, &self.images);
            if budget_plan.over_budget.is_none() {
                budget_plan.over_budget = image_over_budget;
            }
            draw_images = accepted_images;
            draw_images.sort_by_key(|&i| self.images[i].draw_order);
            for &i in &draw_images {
                let image = &self.images[i];
                let mut uniform = [0.0f32; 20];
                uniform[..16].copy_from_slice(&camera_matrix);
                uniform[16] = 1.0;
                uniform[17] = 1.0;
                uniform[18] = 1.0;
                uniform[19] = image.alpha;
                queue.write_buffer(&image.camera, 0, bytemuck::cast_slice(&uniform));
            }
        }

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
                        load: if clear {
                            wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.06,
                                g: 0.07,
                                b: 0.10,
                                a: 1.0,
                            })
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
            // Pass 3 — raster images, after opaque/transparent geometry. Full
            // draw-order interleaving with geometry batches is NOT implemented;
            // images are ordered among themselves by ascending draw_order.
            for &i in &draw_images {
                draw_image(&mut pass, &self.images[i], image_pipeline);
            }
        }

        let submission = queue.submit(Some(encoder.finish()));
        // Distinguish device loss from a bad frame. A bounded wait classifies
        // the submission; the host's device-lost callback is the authoritative
        // loss signal (see `note_device_lost`).
        let outcome = self.device_scope_outcome(device, submission);
        let draw_calls = (opaque.len() + transparent.len() + draw_images.len()) as u64;
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
            image_batches: draw_images.len(),
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
