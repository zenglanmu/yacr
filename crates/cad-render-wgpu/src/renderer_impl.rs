//! Implementation of [`Renderer`]: GPU upload, batching and frame submission.

use super::*;

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
            let bytes = vertices.len()
                + indices.len() * 4
                + edge_indices.len() * 4
                + normals.len()
                + colors.len();
            self.uploaded_bytes += bytes as u64;
            self.batches.push(GpuBatch {
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
