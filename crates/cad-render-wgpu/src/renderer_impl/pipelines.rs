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

    /// Create the WIPEOUT mask pass pipelines: the fill and, when
    /// `WIPEOUTFRAME` is enabled, the boundary frame.
    ///
    /// Both are the frame's **final** pass and share
    /// `depth_compare: Always`, `depth_write_enabled: false`:
    ///
    /// * the fill ignores the depth buffer and paints the clear colour;
    /// * the frame reuses the shared anti-aliased line shader and pre-expanded
    ///   quad layout (ADR-0009), so it rasterizes identical line ink and paints
    ///   **after** the fill without ever being covered by it. It is deliberately
    ///   not `PolygonMode::Line`, which the WebGL2 backend does not support.
    pub(super) fn create_mask_pipelines(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        line_shader: &wgpu::ShaderModule,
        target_format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
        // Wipeout mask pipeline (ADR 0022 C1). This is the frame's **final**
        // pass, and its state is the whole point of the decision:
        //
        // * `depth_compare = Always` — the shared pass is depth-tested with
        //   depth writes (meshes `Less`, lines `LessEqual`), so a filled quad
        //   drawn later at the same world z is rejected. A mask must ignore what
        //   is already in the depth buffer and cover the pixels beneath it.
        // * `depth_write_enabled = false` — a mask hides what was painted
        //   before it; it must not add its own depth or it would occlude later
        //   masks at the same depth.
        // * `cull_mode = None` — the even-odd fill emits consistently wound
        //   triangles, but a mask must never be dropped by winding.
        // * fragment output = the clear colour (the renderer owns the
        //   background; `cad-representation` stays colourless).
        let mask_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-mask"),
            source: wgpu::ShaderSource::Wgsl(MASK_SHADER.into()),
        });
        let mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-mask-pipeline"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &mask_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(position_only_layout())],
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &mask_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    // Blended over: with the clear colour's alpha (1.0) this is
                    // a replace, so the region equals the clear colour exactly.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // WIPEOUT boundary-frame pipeline (`WIPEOUTFRAME` 1/2). It reuses the
        // shared anti-aliased line shader and pre-expanded quad vertex layout
        // (`docs/adr/0009-antialiased-line-strokes.md`) — never
        // `PolygonMode::Line`, which the WebGL2 backend does not support — but
        // with the mask pass's depth state:
        //
        // * `depth_compare = Always` / `depth_write_enabled = false`: the frame
        //   must paint **after** the fill, on top of whatever is already there,
        //   and must not occlude a later mask.
        // * the shared pipeline layout is used so group 0 (the per-draw uniform)
        //   and group 1 (the selection-flags table) are identical to the line
        //   pipeline; the frame carries pick key 0, so flags never tint it.
        let mask_frame_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-mask-frame-pipeline"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: line_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(line_quad_layout())],
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: line_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        (mask_pipeline, mask_frame_pipeline)
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
        plan::caps_from(
            device.adapter_info().backend,
            cfg!(target_arch = "wasm32"),
            &device.limits(),
        )
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
        if self.target.is_some()
            && self.overlay_target.is_some()
            && self.target_size == (target.width.max(1), target.height.max(1))
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
        // Budget gate **before** any target exists: a refused frame size costs no
        // allocation and the previous targets keep rendering (see `gpu_memory`).
        // All four attachments are admitted in one call, so a refusal can never
        // publish a partially allocated base/overlay pair.
        let colour_request = gpu_memory::texture_alloc_bytes(self.target_format, size, 1, 1);
        let depth_request = gpu_memory::texture_alloc_bytes(DEPTH_FORMAT, size, 1, 1);
        self.try_admit_gpu_memory(&[
            (GpuMemoryCategory::ColorTarget, colour_request),
            (GpuMemoryCategory::DepthTarget, depth_request),
            (GpuMemoryCategory::ColorTarget, colour_request),
            (GpuMemoryCategory::DepthTarget, depth_request),
        ])?;
        let texture = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("cad-base-target"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.target_format,
                // COPY_SRC lets the base be copied into the presented overlay
                // every frame. The headless evidence path reads the *overlay*
                // (presented) texture back to a buffer; production composition
                // never does a per-frame GPU→CPU copy.
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
                label: Some("cad-base-depth"),
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
        // The presented target: `copy(base colour)` then the transient overlay,
        // composited with its own depth (cleared every frame). COPY_DST receives
        // the base copy; COPY_SRC serves the headless readback and the host's
        // presentation path.
        let overlay_texture = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("cad-overlay-target"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            GpuMemoryCategory::ColorTarget,
            colour_request,
        )?;
        let overlay_depth = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("cad-overlay-depth"),
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
        // Replace the charges with the new targets': dropping the previous set
        // releases exactly the bytes the old offscreen targets held.
        self.target_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.depth_view = Some(depth.create_view(&wgpu::TextureViewDescriptor::default()));
        self.overlay_view =
            Some(overlay_texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.overlay_depth_view =
            Some(overlay_depth.create_view(&wgpu::TextureViewDescriptor::default()));
        self.target_charge = Some(self.gpu_charge(
            GpuMemoryCategory::ColorTarget,
            gpu_memory::texture_bytes(&texture),
        ));
        self.depth_charge = Some(self.gpu_charge(
            GpuMemoryCategory::DepthTarget,
            gpu_memory::texture_bytes(&depth),
        ));
        self.overlay_target_charge = Some(self.gpu_charge(
            GpuMemoryCategory::ColorTarget,
            gpu_memory::texture_bytes(&overlay_texture),
        ));
        self.overlay_depth_charge = Some(self.gpu_charge(
            GpuMemoryCategory::DepthTarget,
            gpu_memory::texture_bytes(&overlay_depth),
        ));
        self.target = Some(texture);
        self.overlay_target = Some(overlay_texture);
        self.overlay_depth = Some(overlay_depth);
        self.texture_revision += 1;
        self.target_size = (size.width, size.height);
        Ok(())
    }
}
