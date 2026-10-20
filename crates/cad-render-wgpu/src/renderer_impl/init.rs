use super::*;

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
            pacing: pacing::PacingState::default(),
            pacing_restart_pending: true,
            pacing_signal_override: None,
            // No device yet, so no backend has actually been activated.
            active_backend: None,
            device: None,
            queue: None,
            // Measurement-only; created on demand by `set_gpu_time_measurement`.
            gpu_time: None,
            gpu_time_state: gpu_time::GpuTimeState::Disabled,
            gpu_time_feature_override: None,
            gpu_time_resource_creations: 0,
            layout: None,
            line_pipeline: None,
            line_instanced_pipeline: None,
            arc_pipeline: None,
            unit_quad_buffer: None,
            mesh_pipeline: None,
            mesh_pipeline_mirrored: None,
            mesh_pipeline_transparent: None,
            mesh_pipeline_mirrored_transparent: None,
            mesh_pipeline_merged: None,
            mesh_pipeline_merged_mirrored: None,
            image_pipeline: None,
            mask_pipeline: None,
            image_texture_layout: None,
            image_sampler: None,
            target: None,
            target_view: None,
            depth_view: None,
            target_size: (1, 1),
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            batches: Vec::new(),
            instance_streams: Vec::new(),
            arc_streams: Vec::new(),
            mesh_chunks: Vec::new(),
            line_instancing: true,
            mesh_merging: true,
            max_instances_per_stream: MAX_INSTANCES_PER_STREAM,
            max_vertices_per_mesh_stream: MAX_VERTICES_PER_MESH_STREAM,
            instance_stream_uploads: 0,
            arc_stream_uploads: 0,
            arc_degenerate_skips: 0,
            line_odd_vertex_skips: 0,
            mesh_chunk_uploads: 0,
            // No device yet: the universal path is the safe default until
            // `initialize_with_device` probes the real device.
            render_tier: RenderTier::Universal,
            storage_support_override: None,
            pulled_lines: None,
            pulled_mesh: None,
            pulled_binding_limit_override: None,
            pulled_line_segment_elems_override: None,
            pulled_memory_budget_override: None,
            #[cfg(test)]
            pulled_oom_injection: None,
            pulled_state: PulledState::Unavailable,
            line_ramp_override: None,
            cull_disabled: false,
            pulled_camera_layout: None,
            pulled_camera_buffer: None,
            pulled_camera_bind_group: None,
            pulled_line_storage_layout: None,
            pulled_mesh_storage_layout: None,
            line_pulled_pipeline: None,
            mesh_pulled_pipeline: None,
            mesh_pulled_mirrored_pipeline: None,
            images: Vec::new(),
            masks: Vec::new(),
            mask_skips: Vec::new(),
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
            last_commit_timing: None,
            pulled_prefix_checkpoint: None,
            pulled_cache_epoch: 0,
            pulled_prefix_reuses: 0,
            pulled_prefix_rebuilds: 0,
            draw_calls: 0,
            progressive: false,
            progressive_frame: None,
            draw_plan: None,
            batch_facts: plan::BatchFacts::default(),
            encode_emit_mask: plan::EncodeEmitMask::default(),
            device_lost: false,
            last_device_lost: None,
            poll_timeout: std::time::Duration::from_secs(1),
            pick_key_counter: std::sync::atomic::AtomicU32::new(1),
            pick_table: PickTable::default(),
            pick_table_full_rebuilds: 0,
            pick_table_incremental_extends: 0,
            last_projection: None,
            last_camera_matrix: [0.0; 16],
            last_camera_position: None,
            id_format: IdTargetFormat::R32Uint,
            id_format_override: None,
            pick_target: None,
            pick_view: None,
            pick_depth_view: None,
            pick_size: (0, 0),
            pick_line_pipeline: None,
            pick_line_instanced_pipeline: None,
            pick_arc_pipeline: None,
            pick_mesh_pipeline: None,
            pick_mesh_pipeline_mirrored: None,
            pick_mesh_merged_pipeline: None,
            pick_mesh_merged_pipeline_mirrored: None,
            pick_image_pipeline: None,
            flags_layout: None,
            flags_texture: None,
            flags_view: None,
            flags_bind_group: None,
            flags_ctl_buffer: None,
            flags_enabled: false,
            flags_bits: Vec::new(),
            flags_width: 1,
            flags_table_dirty: true,
            flags_upload_bytes: 0,
            flags_upload_count: 0,
            entity_flag_keys: std::collections::BTreeSet::new(),
            entity_flags_request: Vec::new(),
        }
    }

    /// Initialize from a host-provided shared device/queue.
    ///
    /// The host owns the `Device`/`Queue` (from its windowing/UI runtime or a
    /// headless device) and hands them in; the renderer never creates or owns
    /// them.
    pub fn initialize_with_device(
        &mut self,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> CadResult<BackendCapabilities> {
        let caps = Self::caps_for(&device);
        // Choose the draw tier once per device from the device's own
        // backend/limits. Storage buffers in the vertex stage are core WebGPU but
        // absent on WebGL2, so this is capability-gated rather than assumed; a
        // host-provided device is never asked for a feature. A device that fails
        // the gate downgrades to the universal per-run path, labelled by
        // `render_tier()` / `FrameStats::tier`.
        self.render_tier = tier_for(
            caps.storage_buffers,
            &device.limits(),
            self.storage_support_override,
        );
        // The flags layout exists on every tier and is a precondition of the
        // pulled pipelines, so create it first and let the tier be finalised
        // before any pipeline picks a vertex layout from it.
        let flags_layout = Self::create_flags_layout(&device);
        self.flags_layout = Some(flags_layout.clone());
        // The tier is accelerated only if its pulled resources can actually be
        // created. If they cannot, downgrade **here**, before the
        // line-instanced pipeline chooses its 56- vs 64-byte stride, so the
        // universal path is used consistently instead of drawing a placeholder
        // buffer through an accelerated pipeline (the tier must never stay
        // labelled Accelerated with a silently missing pulled path).
        let pulled_ready = self.create_pulled_resources(&device);
        self.render_tier = finalize_tier(self.render_tier, pulled_ready);
        let accelerated = self.render_tier == RenderTier::Accelerated;
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
            bind_group_layouts: &[Some(&layout), Some(&flags_layout)],
            immediate_size: 0,
        });
        // Anti-aliased line pipeline: a pre-expanded triangle-list quad per
        // segment (see `shaders/line.wgsl` and
        // `docs/adr/0009-antialiased-line-strokes.md`). Each vertex carries the
        // segment endpoints plus a quad corner.
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-lines-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &line_shader,
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
                    buffers: &[
                        Some(position_only_layout()),
                        // On the accelerated tier the instance buffer is also the
                        // pulled storage buffer, so it uses the 64-byte stride.
                        Some(if accelerated {
                            line_instance_layout_64()
                        } else {
                            line_instance_layout()
                        }),
                    ],
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
        // SDF conic pipeline (slice B): one instance per analytic circle/arc, the
        // shared unit quad expanded to the conic's extent. Same layout/depth/blend
        // state as the line pipeline (the flags table is group 1); the fragment
        // shader computes the signed distance and discards the quad outside the
        // anti-aliased band.
        let arc_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-arcs"),
            source: wgpu::ShaderSource::Wgsl(ARC_SHADER.into()),
        });
        let arc_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-arcs-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &arc_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(position_only_layout()), Some(arc_instance_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // The quad may be mirrored by the conic frame; never cull it.
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
                module: &arc_shader,
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
        // One shared unit quad `[0,1]^2` (two triangles) for every SDF conic draw
        // and every anti-aliased instanced line draw (the six corners match
        // `line_pulled.wgsl`'s constant table).
        let unit_quad_buffer = Self::vertex_buffer(
            &device,
            Some(&queue),
            &pack_positions(&[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ]),
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
                // Flags precede the sampled texture so every geometry pipeline in
                // this crate binds the flags table at the same group index.
                bind_group_layouts: &[
                    Some(&layout),
                    Some(&flags_layout),
                    Some(&image_texture_layout),
                ],
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
            layout: Some(&pipeline_layout),
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
                    format: self.target_format,
                    // Blended over: with the clear colour's alpha (1.0) this is
                    // a replace, so the region equals the clear colour exactly.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // A successful attachment establishes a new device generation. No
        // batch/attachment from the previous generation may survive, even if
        // the new target later has exactly the same dimensions.
        self.batches.clear();
        self.images.clear();
        self.masks.clear();
        self.mask_skips.clear();
        self.image_textures.clear();
        self.instance_streams.clear();
        self.arc_streams.clear();
        self.mesh_chunks.clear();
        self.pulled_lines = None;
        self.pulled_mesh = None;
        self.invalidate_pulled_prefix();
        // A fresh device has no converted geometry yet; the pulled organisation
        // is decidable, not skipped for capacity.
        self.pulled_state = if accelerated {
            PulledState::SkippedNoConvertedGeometry
        } else {
            PulledState::Unavailable
        };
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
        self.arc_pipeline = Some(arc_pipeline);
        self.unit_quad_buffer = Some(unit_quad_buffer);
        self.mesh_pipeline = Some(mesh_pipeline);
        self.mesh_pipeline_mirrored = Some(mesh_pipeline_mirrored);
        self.mesh_pipeline_transparent = Some(mesh_pipeline_transparent);
        self.mesh_pipeline_mirrored_transparent = Some(mesh_pipeline_mirrored_transparent);
        self.mesh_pipeline_merged = Some(mesh_pipeline_merged);
        self.mesh_pipeline_merged_mirrored = Some(mesh_pipeline_merged_mirrored);
        self.image_pipeline = Some(image_pipeline);
        self.mask_pipeline = Some(mask_pipeline);
        self.image_texture_layout = Some(image_texture_layout);
        self.image_sampler = Some(image_sampler);
        self.active_backend = Some(caps.actual);
        self.device_generation += 1;
        // A new device generation invalidates any page-cost knowledge.
        self.reset_pacing();
        self.device_lost = false;
        // Selection flags: the group layout and a 1x1 table (grown on demand).
        self.flags_layout = Some(flags_layout);
        self.flags_enabled = false;
        // A new device generation restarts the key space **and** retires the
        // retained entity-flag request: the old device's keys are gone and a
        // later request would otherwise re-tint on the rebuilt scene.
        self.reset_pick_keys_and_drop_request();
        let device_ref = self.device.as_ref().expect("device just set").clone();
        let queue_ref = self.queue.as_ref().expect("queue just set").clone();
        self.ensure_flags(&device_ref, &queue_ref, 1);
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.id_format = self
                .id_format_override
                .unwrap_or_else(|| Self::probe_id_format(&device_ref));
            self.create_pick_pipelines(&device_ref);
        }
        Ok(caps)
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

    /// Create the accelerated tier's pipelines, layouts and per-frame camera
    /// uniform. Returns `false` (and leaves the pulled resources cleared) unless
    /// the tier is [`RenderTier::Accelerated`] and the shared flags layout is
    /// present; the caller downgrades the tier on `false` rather than leaving it
    /// labelled `Accelerated` with a missing pulled path.
    ///
    /// The pulled pipelines have **no vertex buffers**: every input is pulled
    /// from group 2's read-only storage buffers, so they are not WebGL2-safe and
    /// exist only on the accelerated tier.
    pub(super) fn create_pulled_resources(&mut self, device: &wgpu::Device) -> bool {
        // Drop any previous device generation's pulled resources first, so a
        // downgrade cannot leave stale handles from the old device.
        self.pulled_camera_bind_group = None;
        self.pulled_camera_layout = None;
        self.pulled_camera_buffer = None;
        self.pulled_line_storage_layout = None;
        self.pulled_mesh_storage_layout = None;
        self.line_pulled_pipeline = None;
        self.mesh_pulled_pipeline = None;
        self.mesh_pulled_mirrored_pipeline = None;
        if self.render_tier != RenderTier::Accelerated {
            return false;
        }
        let Some(flags_layout) = self.flags_layout.clone() else {
            return false;
        };
        let pulled_camera_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cad-pulled-camera-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // The pulled line fragment stage reads `c3.x` (the AA line SDF
                    // ramp width), so the camera uniform must be visible to both
                    // stages; the mesh fragment stage ignores it.
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(u64::from(PULLED_CAMERA_BYTES)),
                    },
                    count: None,
                }],
            });
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad-pulled-camera"),
            size: u64::from(PULLED_CAMERA_BYTES),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-pulled-camera-bind"),
            layout: &pulled_camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &camera_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(u64::from(PULLED_CAMERA_BYTES)),
                }),
            }],
        });
        let storage_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let line_storage_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cad-pulled-line-storage-layout"),
                entries: &[storage_entry(0), storage_entry(1)],
            });
        let mesh_storage_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cad-pulled-mesh-storage-layout"),
                entries: &[storage_entry(0), storage_entry(1), storage_entry(2)],
            });
        let line_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-pulled-line-pipeline-layout"),
            bind_group_layouts: &[
                Some(&pulled_camera_layout),
                Some(&flags_layout),
                Some(&line_storage_layout),
            ],
            immediate_size: 0,
        });
        let mesh_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-pulled-mesh-pipeline-layout"),
            bind_group_layouts: &[
                Some(&pulled_camera_layout),
                Some(&flags_layout),
                Some(&mesh_storage_layout),
            ],
            immediate_size: 0,
        });
        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-pulled-lines"),
            source: wgpu::ShaderSource::Wgsl(LINE_PULLED_SHADER.into()),
        });
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-pulled-mesh"),
            source: wgpu::ShaderSource::Wgsl(MESH_PULLED_SHADER.into()),
        });
        self.line_pulled_pipeline = Some(device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("cad-pulled-lines-pipeline"),
                layout: Some(&line_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &line_shader,
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
            },
        ));
        let mesh_pipeline = |front_face: wgpu::FrontFace, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&mesh_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
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
                    module: &mesh_shader,
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
            })
        };
        self.mesh_pulled_pipeline = Some(mesh_pipeline(
            wgpu::FrontFace::Ccw,
            "cad-pulled-mesh-pipeline",
        ));
        self.mesh_pulled_mirrored_pipeline = Some(mesh_pipeline(
            wgpu::FrontFace::Cw,
            "cad-pulled-mesh-pipeline-mirrored",
        ));
        self.pulled_camera_layout = Some(pulled_camera_layout);
        self.pulled_camera_buffer = Some(camera_buffer);
        self.pulled_camera_bind_group = Some(camera_bind_group);
        self.pulled_line_storage_layout = Some(line_storage_layout);
        self.pulled_mesh_storage_layout = Some(mesh_storage_layout);
        true
    }

    /// Decide the id-target format from the device's own format features.
    ///
    /// `R32Uint` is preferred: the WebGPU spec's guaranteed features list it as a
    /// render attachment, and wgpu's GL backend advertises `COLOR_ATTACHMENT` for
    /// it too, so one 32-bit integer channel carries the whole key. A device that
    /// does not report it renderable falls back to the packed RGBA8 encoding,
    /// which every tier can render to. Pure decision, so it is unit-testable
    /// without a GPU.
    pub fn id_format_for_features(features: wgpu::Features) -> IdTargetFormat {
        let renderable = wgpu::TextureFormat::R32Uint
            .guaranteed_format_features(features)
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
        if renderable {
            IdTargetFormat::R32Uint
        } else {
            IdTargetFormat::PackedRgba8
        }
    }

    pub(super) fn probe_id_format(device: &wgpu::Device) -> IdTargetFormat {
        Self::id_format_for_features(device.features())
    }

    /// The active device's enabled features, or `Features::empty()` before a
    /// device exists. Exposed so a caller can reproduce the id-target probe.
    pub fn device_features(&self) -> wgpu::Features {
        self.device
            .as_ref()
            .map_or(wgpu::Features::empty(), |device| device.features())
    }

    /// The active id-target format, or `None` before a device exists.
    pub fn id_target_format(&self) -> Option<IdTargetFormat> {
        self.device.as_ref().map(|_| self.id_format)
    }

    /// Test hook: force the id-target format regardless of the adapter probe.
    ///
    /// Exists so the packed-RGBA8 fallback (the path a device without a
    /// renderable `R32Uint` takes) is exercised on hardware that has `R32Uint`.
    /// Takes effect on the next device initialization.
    pub fn set_id_format_override(&mut self, format: Option<IdTargetFormat>) {
        self.id_format_override = format;
    }
}
