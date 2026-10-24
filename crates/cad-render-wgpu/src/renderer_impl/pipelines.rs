use super::*;

impl Renderer {
    /// Apply the measurement-only GPU frame-time opt-in (ADR 0013 class 1).
    ///
    /// Pure-config in, explicit verdict out. `config.enabled` is the **only**
    /// way measurement turns on; the renderer never reads the environment. The
    /// feature set is requested at **device creation** (see
    /// `headless::request_device_with_negotiated_limits`), so this method reads
    /// the live device's granted features and:
    ///
    /// * disabled ⇒ no resources, [`gpu_time::GpuTimeState::Disabled`];
    /// * enabled + granted ⇒ create the query set/buffers once and report
    ///   [`gpu_time::GpuTimeState::Enabled`];
    /// * enabled + missing ⇒ no resources, and report
    ///   [`gpu_time::GpuTimeState::Unsupported`] naming the missing feature —
    ///   never a silent fallback that pretends measurement worked.
    ///
    /// On `wasm32` the platform has no blocking poll to read the result back, so
    /// the state is always [`gpu_time::GpuTimeState::Disabled`] (the wasm path
    /// never pretends to measure).
    pub fn set_gpu_time_measurement(
        &mut self,
        config: gpu_time::GpuTimeConfig,
    ) -> gpu_time::GpuTimeState {
        self.gpu_time = None;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = config;
            self.gpu_time_state = gpu_time::GpuTimeState::Disabled;
            return self.gpu_time_state;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(device) = self.device.as_ref() else {
                self.gpu_time_state = gpu_time::GpuTimeState::Disabled;
                return self.gpu_time_state;
            };
            let features = self
                .gpu_time_feature_override
                .unwrap_or_else(|| device.features());
            let report = gpu_time::negotiate_gpu_time(features, config);
            self.gpu_time_state = report.state;
            if report.state == gpu_time::GpuTimeState::Enabled {
                let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("cad-gpu-time-queries"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                });
                // `resolve_query_set` needs a 256-byte-aligned destination; two
                // timestamps need only 16 bytes, so 256 satisfies both.
                let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cad-gpu-time-resolve"),
                    size: wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cad-gpu-time-readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let charges = [
                    self.gpu_charge(GpuMemoryCategory::Other, resolve.size()),
                    self.gpu_charge(GpuMemoryCategory::Other, readback.size()),
                ];
                self.gpu_time = Some(GpuTimeResources {
                    query_set,
                    resolve,
                    readback,
                    charges,
                });
                self.gpu_time_resource_creations += 1;
            }
            self.gpu_time_state
        }
    }

    /// The current measurement state token (see [`Renderer::set_gpu_time_measurement`]).
    pub fn gpu_time_state(&self) -> gpu_time::GpuTimeState {
        self.gpu_time_state
    }

    /// How many times the measurement-only timestamp resources were created.
    ///
    /// Diagnostics/contract only: it must stay `1` across frames once measurement
    /// is enabled (the resources are reused, not re-allocated per frame).
    pub fn gpu_time_resource_creations(&self) -> u64 {
        self.gpu_time_resource_creations
    }

    /// Test hook: override the feature set `set_gpu_time_measurement` reads.
    ///
    /// `Some(features)` simulates an adapter whose granted set differs from the
    /// live device's (for example missing `TIMESTAMP_QUERY`), so the
    /// `Unsupported(<feature>)` state is exercisable **without** faking a device.
    /// `None` uses the live device's granted set.
    pub fn set_gpu_time_feature_override(&mut self, features: Option<wgpu::Features>) {
        self.gpu_time_feature_override = features;
    }

    pub(super) fn create_mesh_pipeline(
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
    pub(super) fn create_mesh_merged_pipeline(
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
    pub(super) fn caps_for(device: &wgpu::Device) -> BackendCapabilities {
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

    pub(super) fn ensure_target(&mut self, target: &RenderTarget) -> Result<(), RenderError> {
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
        // Budget gate **before** either target exists: a refused frame size costs
        // no allocation and the previous target keeps rendering (see `gpu_memory`).
        let colour_request = gpu_memory::texture_alloc_bytes(self.target_format, size, 1, 1);
        let depth_request = gpu_memory::texture_alloc_bytes(DEPTH_FORMAT, size, 1, 1);
        self.try_admit_gpu_memory(&[
            (GpuMemoryCategory::ColorTarget, colour_request),
            (GpuMemoryCategory::DepthTarget, depth_request),
        ])?;
        let texture = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
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
            },
            GpuMemoryCategory::ColorTarget,
            colour_request,
        )?;
        let depth = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("cad-depth"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
            GpuMemoryCategory::DepthTarget,
            depth_request,
        )?;
        // Replace the charges with the new targets': dropping the previous pair
        // releases exactly the bytes the old offscreen targets held.
        self.target_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.depth_view = Some(depth.create_view(&wgpu::TextureViewDescriptor::default()));
        self.target_charge = Some(self.gpu_charge(
            GpuMemoryCategory::ColorTarget,
            gpu_memory::texture_bytes(&texture),
        ));
        self.depth_charge = Some(self.gpu_charge(
            GpuMemoryCategory::DepthTarget,
            gpu_memory::texture_bytes(&depth),
        ));
        self.target = Some(texture);
        self.texture_revision += 1;
        self.target_size = (size.width, size.height);
        Ok(())
    }
}
