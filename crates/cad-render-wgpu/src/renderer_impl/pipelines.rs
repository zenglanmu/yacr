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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_mesh_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
        front_face: wgpu::FrontFace,
        depth_write: bool,
        samples: u32,
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
            multisample: wgpu::MultisampleState {
                count: samples.max(1),
                ..Default::default()
            },
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
        samples: u32,
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
            multisample: wgpu::MultisampleState {
                count: samples.max(1),
                ..Default::default()
            },
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

    /// Create one textured-mesh pipeline (`mesh_textured`, ADR-0038 D9).
    ///
    /// It is the mesh pipeline's state plus a UV0 vertex buffer:
    ///
    /// * vertex slots 0/1/2 = position/normal/colour, exactly as the untextured
    ///   mesh pipeline; slot 3 = `Float32x2` UV0 (stride 8), bound only for a
    ///   batch that carries a texture;
    /// * group 2 = the image pipeline's `texture_2d<f32>` + filtering sampler
    ///   layout, so a mesh texture and a raster image share one texture layout;
    /// * `depth_compare: Less` with `depth_write` matching the variant (true for
    ///   opaque, false for the transparent pair), back-face culling with the same
    ///   `front_face` rule as the corresponding mesh pipeline.
    ///
    /// Nothing here is shared with the untextured pipelines, so their behaviour is
    /// untouched by this feature.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_mesh_textured_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
        front_face: wgpu::FrontFace,
        depth_write: bool,
        samples: u32,
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
                    Some(mesh_uv_layout()),
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
            multisample: wgpu::MultisampleState {
                count: samples.max(1),
                ..Default::default()
            },
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

    /// Create the textured-mesh pipeline family (ADR-0038 D9).
    ///
    /// Four pipelines shadow the untextured mesh set — opaque/transparent ×
    /// normal/mirrored — so a textured batch keeps exactly the depth and winding
    /// behaviour it would have had without a texture. They are a separate family
    /// because they add a UV0 vertex buffer (slot 3) and group 2 (sampled texture
    /// and filtering sampler, the image pipeline's layout); nothing about the
    /// existing mesh pipelines changes.
    pub(super) fn create_mesh_textured_pipelines(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        flags_layout: &wgpu::BindGroupLayout,
        texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> (
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
        wgpu::RenderPipeline,
    ) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-mesh-textured"),
            source: wgpu::ShaderSource::Wgsl(MESH_TEXTURED_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-mesh-textured-pipeline-layout"),
            // Flags stay at group 1 so every colour pipeline in this crate agrees
            // on the selection table's index; the texture is group 2.
            bind_group_layouts: &[
                Some(camera_layout),
                Some(flags_layout),
                Some(texture_layout),
            ],
            immediate_size: 0,
        });
        let make = |front_face: wgpu::FrontFace, depth_write: bool, label: &str| {
            Self::create_mesh_textured_pipeline(
                device,
                &pipeline_layout,
                &shader,
                format,
                front_face,
                depth_write,
                samples,
                label,
            )
        };
        (
            make(wgpu::FrontFace::Ccw, true, "cad-mesh-textured-pipeline"),
            make(
                wgpu::FrontFace::Cw,
                true,
                "cad-mesh-textured-pipeline-mirrored",
            ),
            make(
                wgpu::FrontFace::Ccw,
                false,
                "cad-mesh-textured-pipeline-transparent",
            ),
            make(
                wgpu::FrontFace::Cw,
                false,
                "cad-mesh-textured-pipeline-mirrored-transparent",
            ),
        )
    }

    /// Group-1 layout of the selection-flags table: an integer lookup texture
    /// (`texture_2d<u32>`, addressed with `textureLoad`) plus a 16-byte control
    /// uniform. Deliberately a texture, not a vertex-stage storage buffer, so the
    /// layout stays WebGL2-safe.
    pub(super) fn create_flags_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad-flags-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(u64::from(FLAGS_CTL_BYTES)),
                    },
                    count: None,
                },
            ],
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
        let size = (target.width.max(1), target.height.max(1));
        // The frame input is the live MSAA request. Reconciling may rebuild the
        // base pipeline family and drop the old attachments; it never touches
        // the committed batches or the resident scene.
        self.reconcile_msaa(target.samples)?;
        let samples = self.msaa_samples.max(1);
        if self.target.is_some()
            && self.overlay_target.is_some()
            && self.target_size == size
            && self.target_samples == samples
        {
            return Ok(());
        }
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?;
        let extent = wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        };
        // A frame larger than the adapter's texture limit must fail with a
        // structured frame error, not a `create_texture` validation panic.
        let limit = device.limits().max_texture_dimension_2d;
        if extent.width > limit || extent.height > limit {
            return Err(RenderError::Frame(format!(
                "render target {}x{} exceeds the adapter's maximum texture dimension {limit}",
                extent.width, extent.height
            )));
        }
        // Budget gate **before** any target exists: a refused frame size costs no
        // allocation and the previous targets keep rendering (see `gpu_memory`).
        // All four attachments are admitted in one call, so a refusal can never
        // publish a partially allocated base/overlay pair. The base pair carries
        // the MSAA sample multiplier; the presented pair stays 1×.
        let base_colour_request =
            gpu_memory::texture_alloc_bytes(self.target_format, extent, samples, 1);
        let base_depth_request = gpu_memory::texture_alloc_bytes(DEPTH_FORMAT, extent, samples, 1);
        let overlay_colour_request =
            gpu_memory::texture_alloc_bytes(self.target_format, extent, 1, 1);
        let overlay_depth_request = gpu_memory::texture_alloc_bytes(DEPTH_FORMAT, extent, 1, 1);
        self.try_admit_gpu_memory(&[
            (GpuMemoryCategory::ColorTarget, base_colour_request),
            (GpuMemoryCategory::DepthTarget, base_depth_request),
            (GpuMemoryCategory::ColorTarget, overlay_colour_request),
            (GpuMemoryCategory::DepthTarget, overlay_depth_request),
        ])?;
        let texture = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("cad-base-target"),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: self.target_format,
                // COPY_SRC is only legal on a single-sample texture; the MSAA
                // path resolves into the presented overlay instead of copying.
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | if samples == 1 {
                        wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::empty()
                    },
                view_formats: &[],
            },
            GpuMemoryCategory::ColorTarget,
            base_colour_request,
        )?;
        let depth = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("cad-base-depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
            GpuMemoryCategory::DepthTarget,
            base_depth_request,
        )?;
        // The presented target: at 1× it receives `copy(base colour)`; at n× it
        // is the base pass's resolve target. Then the 1× overlay/mask pass draws
        // UI on top. COPY_SRC serves the headless readback and the host's
        // presentation path.
        let overlay_texture = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("cad-overlay-target"),
                size: extent,
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
            overlay_colour_request,
        )?;
        let overlay_depth = self.create_texture_checked(
            device,
            &wgpu::TextureDescriptor {
                label: Some("cad-overlay-depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
            GpuMemoryCategory::DepthTarget,
            overlay_depth_request,
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
        self.target_size = size;
        self.target_samples = samples;
        Ok(())
    }
}
