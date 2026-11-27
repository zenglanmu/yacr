use super::*;

impl Renderer {
    pub fn new(preference: BackendPreference) -> Self {
        // Read once: `YACR_GPU_BUDGET_MB` (explicit bytes) and
        // `YACR_GPU_BUDGET_PERCENT` (share of probed VRAM). Honesty rules and the
        // precedence order live in `gpu_memory::budget`.
        let gpu_budget_config = GpuBudgetConfig::from_env();
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
            mesh_pipeline_textured: None,
            mesh_pipeline_textured_mirrored: None,
            mesh_pipeline_textured_transparent: None,
            mesh_pipeline_textured_mirrored_transparent: None,
            image_pipeline: None,
            mesh_pipeline_pbr: None,
            mesh_pipeline_pbr_mirrored: None,
            mesh_pipeline_pbr_transparent: None,
            mesh_pipeline_pbr_mirrored_transparent: None,
            mesh_pipeline_pbr_uv: None,
            mesh_pipeline_pbr_uv_mirrored: None,
            mesh_pipeline_pbr_uv_transparent: None,
            mesh_pipeline_pbr_uv_mirrored_transparent: None,
            mesh_pipeline_pbr_flat: None,
            mesh_pipeline_pbr_flat_mirrored: None,
            mesh_pipeline_pbr_flat_transparent: None,
            mesh_pipeline_pbr_flat_mirrored_transparent: None,
            pbr_texture_layout: None,
            pbr_identities: None,
            pbr_identity_bind_group: None,
            pbr_data_textures: HashMap::new(),
            pbr_bind_groups: std::collections::BTreeMap::new(),
            pbr_tangent_unavailable: std::sync::atomic::AtomicU64::new(0),
            mask_pipeline: None,
            mask_frame_pipeline: None,
            image_texture_layout: None,
            image_sampler: None,
            mesh_sampler: None,
            target: None,
            target_view: None,
            depth_view: None,
            overlay_target: None,
            overlay_view: None,
            overlay_depth: None,
            overlay_depth_view: None,
            target_size: (1, 1),
            target_samples: 0,
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            msaa_samples: 1,
            requested_msaa_samples: 1,
            supported_msaa_samples: Vec::new(),
            msaa_diagnostic: None,
            last_msaa_diagnostic: None,
            ui_geometry: None,
            batches: Vec::new(),
            base_end: 0,
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
            gpu_storage_disabled: false,
            pulled_lines: None,
            pulled_mesh: None,
            pulled_binding_limit_override: None,
            pulled_line_segment_elems_override: None,
            pulled_memory_budget_override: None,
            #[cfg(test)]
            pulled_oom_injection: None,
            pulled_state: PulledState::Unavailable,
            line_fallback_built: false,
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
            mesh_textures: HashMap::new(),
            mesh_texture_fallbacks: Vec::new(),
            cursor_transient: None,
            cursor_transient_buffer: None,
            cursor_transient_dirty: false,
            cursor_transient_packed: Vec::new(),
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
            pulled_suffix_skips: 0,
            gc_skipped_commits: 0,
            draw_plan_rebuilds: 0,
            batch_overwrites: 0,
            draw_calls: 0,
            progressive: false,
            navigation_lod: false,
            progressive_frame: None,
            progressive_order_reuses: 0,
            progressive_order_rebuilds: 0,
            preserve_color_on_restart: false,
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
            last_light: [0.0, 0.0, 1.0, 0.0],
            sun: None,
            fog: None,
            ibl_enabled: false,
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
            // The budget policy is resolved from the environment now, but without
            // a device no VRAM probe can run: this initial resolution is either the
            // explicit override or **no cap** (unprobed). `initialize_with_device`
            // re-resolves it against the probed device-local total.
            gpu_memory: Arc::new(GpuMemoryLedger::new(gpu_budget_config.resolve(None), None)),
            gpu_budget_config,
            gpu_vram: std::sync::Mutex::new(VramCache::default()),
            unit_quad_charge: None,
            target_charge: None,
            depth_charge: None,
            overlay_target_charge: None,
            overlay_depth_charge: None,
            pick_target_charge: None,
            pick_depth_charge: None,
            flags_texture_charge: None,
            flags_ctl_charge: None,
            pulled_camera_charge: None,
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
        // Resolve the host's requested MSAA count against what this adapter
        // guarantees/reports *before* any pipeline is created. A clamp is
        // labelled, never silent; the overlay/mask passes stay at 1×.
        self.msaa_samples =
            msaa::effective_samples(self.requested_msaa_samples, &self.supported_msaa_samples);
        self.msaa_diagnostic =
            msaa::clamp_diagnostic(self.requested_msaa_samples, self.msaa_samples);
        if let Some(diagnostic) = self.msaa_diagnostic.as_deref() {
            eprintln!("[render] {diagnostic}");
            self.last_msaa_diagnostic = Some(diagnostic.to_string());
        }
        // Choose the draw tier once per device from the device's own
        // backend/limits. Storage buffers in the vertex stage are core WebGPU but
        // absent on WebGL2, so this is capability-gated rather than assumed; a
        // host-provided device is never asked for a feature. A device that fails
        // the gate downgrades to the universal per-run path, labelled by
        // `render_tier()` / `FrameStats::tier`. The user's explicit opt-out wins
        // over the capability: no pulled copy is allocated at all.
        self.render_tier = tier_for_policy(
            self.gpu_storage_disabled,
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
            multisample: self.msaa_state(),
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
                multisample: self.msaa_state(),
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
            multisample: self.msaa_state(),
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
            self.msaa_samples,
            "cad-mesh-pipeline",
        );
        let mesh_pipeline_mirrored = Self::create_mesh_pipeline(
            &device,
            &pipeline_layout,
            &mesh_shader,
            self.target_format,
            wgpu::FrontFace::Cw,
            true,
            self.msaa_samples,
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
            self.msaa_samples,
            "cad-mesh-pipeline-transparent",
        );
        let mesh_pipeline_mirrored_transparent = Self::create_mesh_pipeline(
            &device,
            &pipeline_layout,
            &mesh_shader,
            self.target_format,
            wgpu::FrontFace::Cw,
            false,
            self.msaa_samples,
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
            self.msaa_samples,
            "cad-mesh-merged-pipeline",
        );
        let mesh_pipeline_merged_mirrored = Self::create_mesh_merged_pipeline(
            &device,
            &pipeline_layout,
            &mesh_merged_shader,
            self.target_format,
            wgpu::FrontFace::Cw,
            self.msaa_samples,
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
        // The overlay pass draws UI (grid/axes/cursor/previews) at 1× even when
        // the base pass is multisampled, so mask edges and 1 px UI strokes are
        // never resolved. With MSAA off the base set is reused (`None`).
        let ui_geometry = (self.msaa_samples > 1).then(|| {
            Self::create_geometry_pipelines(
                &device,
                &pipeline_layout,
                &image_pipeline_layout,
                self.target_format,
                accelerated,
                1,
            )
        });
        // Textured mesh (ADR-0038 D9): a **separate** pipeline family, so the
        // untextured `mesh` pipelines keep exactly their previous vertex layouts,
        // bind groups and draw state. Group 2 is the image pipeline's texture
        // layout, and the primitive/depth state matches the mesh pair it shadows
        // (see `create_mesh_textured_pipelines`).
        let (
            mesh_pipeline_textured,
            mesh_pipeline_textured_mirrored,
            mesh_pipeline_textured_transparent,
            mesh_pipeline_textured_mirrored_transparent,
        ) = Self::create_mesh_textured_pipelines(
            &device,
            &layout,
            &flags_layout,
            &image_texture_layout,
            self.target_format,
            self.msaa_samples,
        );
        // PBR channel (ADR-0039 D6): another separate family, so the untextured and
        // textured pipelines keep their exact layout/state. Group 2 is the PBR
        // channel layout (five textures + sampler); the identity textures are
        // created once here so an absent channel always has its neutral sample.
        let pbr_texture_layout = Self::create_pbr_texture_layout(&device);
        let pbr_identities = Self::create_pbr_identities(&device, &queue);
        let (mesh_pbr_pipelines, mesh_pbr_uv_pipelines, mesh_pbr_flat_pipelines) =
            Self::create_mesh_pbr_pipelines(
                &device,
                &layout,
                &flags_layout,
                &pbr_texture_layout,
                self.target_format,
                self.msaa_samples,
            );
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
            multisample: self.msaa_state(),
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
            // Image quads upload `mip_level_count: 1`, so the mip filter can
            // never select anything but level 0: CAD raster images keep exactly
            // the sampling they had. (Mesh material channels use
            // `mesh_sampler` below, which does filter mips.)
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        // Mesh material sampler (ADR-0039 D6 + the CPU mip chain): trilinear
        // filtering between the uploaded levels plus 16× anisotropy, so a large
        // albedo on a small screen footprint is filtered rather than
        // point-sampled from level 0. Mips are a genuine quality fix; they were
        // **not** the cause of the striation/smear defect — that was the
        // `ClampToEdge` wrap below, which forced every out-of-range `V` to the
        // texture's last row (see ADR-0044 §7).
        //
        // Wrap is `Repeat` for U and V: glTF 2.0's default sampler wrap is
        // REPEAT (10497), and the importer does not carry `wrapS`/`wrapT`
        // through, so REPEAT is the only honest mode for mesh material
        // textures. A document that declares a different wrap is named by the
        // importer (`import.gltf.sampler_wrap_not_imported`) rather than
        // silently reinterpreted. W stays `ClampToEdge`: a 2-D material texture
        // never samples it. (Khronos `DamagedHelmet` has V ∈ [1, 2] with a
        // default `{}` sampler; clamping there showed one stretched row as
        // 1-D bands, zero-metal and black emissive.)
        //
        // A separate sampler, not a change to `image_sampler`: the latter is
        // shared with CAD raster image quads, whose `ClampToEdge` rendering must
        // not change.
        //
        // Backend limit: wgpu clamps `anisotropy_clamp` to `min(16)` on a device
        // that reports `DownlevelFlags::ANISOTROPIC_FILTERING` (16 is the
        // interface ceiling, so 16 is never reduced there) and substitutes 1 on
        // one that does not (for example WebGL2). The renderer holds a `Device`,
        // not an `Adapter`, so that substitution cannot be observed here; it is a
        // documented backend limit, not a silent fallback in this code.
        let mesh_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cad-mesh-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        // All-identity PBR bind group: the fallback for a batch whose channel set
        // has not been resolved (or is entirely absent), so a PBR batch always has
        // a legal group 2 and a disabled channel is a neutral multiply.
        let pbr_identity_bind_group = Self::create_pbr_identity_bind_group(
            &device,
            &pbr_texture_layout,
            &pbr_identities,
            &mesh_sampler,
        );
        let (mask_pipeline, mask_frame_pipeline) = Self::create_mask_pipelines(
            &device,
            &pipeline_layout,
            &line_shader,
            self.target_format,
        );

        // A successful attachment establishes a new device generation. No
        // batch/attachment from the previous generation may survive, even if
        // the new target later has exactly the same dimensions.
        self.batches.clear();
        self.images.clear();
        self.masks.clear();
        self.mask_skips.clear();
        self.image_textures.clear();
        // Mesh textures belong to the dropped device generation too; the host
        // re-uploads them on its next base change like every other GPU resource.
        self.mesh_textures.clear();
        self.mesh_texture_fallbacks.clear();
        // PBR residency is derived from the dropped batches too: keep the identity
        // textures (device-agnostic content) but retire the per-set bind groups and
        // the linear data textures, which belong to the old generation.
        self.pbr_data_textures.clear();
        self.pbr_bind_groups.clear();
        self.instance_streams.clear();
        self.arc_streams.clear();
        self.mesh_chunks.clear();
        // The transient cursor is CPU-owned input, not scene state: keep it across
        // the device generation change and only drop/recreate its GPU buffer on the
        // next render (dirty forces the re-upload of the retained packed bytes).
        self.cursor_transient_buffer = None;
        self.cursor_transient_dirty = self.cursor_transient.is_some();
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
        self.overlay_target = None;
        self.overlay_view = None;
        self.overlay_depth = None;
        self.overlay_depth_view = None;
        self.target_samples = 0;
        self.base_end = 0;
        self.target_size = (0, 0);
        // A fresh device generation has no resident base: drop derived state.
        self.drop_derived_base_state();
        self.device = Some(device);
        self.queue = Some(queue);
        // A fresh device generation re-probes VRAM exactly once, so the budget is a
        // share of *this* card rather than the previous device's (or no cap when the
        // new backend cannot be probed). See `Renderer::refresh_gpu_budget_from_probe`.
        self.gpu_vram
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset();
        self.refresh_gpu_budget_from_probe();
        self.layout = Some(layout);
        self.ui_geometry = ui_geometry;
        self.line_pipeline = Some(line_pipeline);
        self.line_instanced_pipeline = Some(line_instanced_pipeline);
        self.arc_pipeline = Some(arc_pipeline);
        self.unit_quad_charge =
            Some(self.gpu_charge(GpuMemoryCategory::Vertex, unit_quad_buffer.size()));
        self.unit_quad_buffer = Some(unit_quad_buffer);
        self.mesh_pipeline = Some(mesh_pipeline);
        self.mesh_pipeline_mirrored = Some(mesh_pipeline_mirrored);
        self.mesh_pipeline_transparent = Some(mesh_pipeline_transparent);
        self.mesh_pipeline_mirrored_transparent = Some(mesh_pipeline_mirrored_transparent);
        self.mesh_pipeline_merged = Some(mesh_pipeline_merged);
        self.mesh_pipeline_merged_mirrored = Some(mesh_pipeline_merged_mirrored);
        self.mesh_pipeline_textured = Some(mesh_pipeline_textured);
        self.mesh_pipeline_textured_mirrored = Some(mesh_pipeline_textured_mirrored);
        self.mesh_pipeline_textured_transparent = Some(mesh_pipeline_textured_transparent);
        self.mesh_pipeline_textured_mirrored_transparent =
            Some(mesh_pipeline_textured_mirrored_transparent);
        self.install_pbr_resources(
            mesh_pbr_pipelines,
            mesh_pbr_uv_pipelines,
            mesh_pbr_flat_pipelines,
            pbr_texture_layout,
            pbr_identities,
            pbr_identity_bind_group,
        );
        self.image_pipeline = Some(image_pipeline);
        self.mask_pipeline = Some(mask_pipeline);
        self.mask_frame_pipeline = Some(mask_frame_pipeline);
        self.image_texture_layout = Some(image_texture_layout);
        self.image_sampler = Some(image_sampler);
        self.mesh_sampler = Some(mesh_sampler);
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
