use super::*;

impl Renderer {
    /// Build the geometry pipeline family at one MSAA sample count.
    ///
    /// The base pass builds this set at the effective sample count; when MSAA is
    /// on, the overlay pass builds a second set at `1` so UI/mask edges stay
    /// exact. Shaders are re-created per set deliberately: the overlay set is
    /// only built when the sample count changes, never per frame.
    pub(super) fn create_geometry_pipelines(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        image_pipeline_layout: &wgpu::PipelineLayout,
        target_format: wgpu::TextureFormat,
        accelerated: bool,
        samples: u32,
    ) -> GeometryPipelines {
        let multisample = wgpu::MultisampleState {
            count: samples.max(1),
            ..Default::default()
        };
        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-lines"),
            source: wgpu::ShaderSource::Wgsl(LINE_SHADER.into()),
        });
        let line = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-lines-pipeline"),
            layout: Some(pipeline_layout),
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
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &line_shader,
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
        let line_instanced_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-lines-instanced"),
            source: wgpu::ShaderSource::Wgsl(LINE_INSTANCED_SHADER.into()),
        });
        let line_instanced = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-lines-instanced-pipeline"),
            layout: Some(pipeline_layout),
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
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &line_instanced_shader,
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
        let arc_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-arcs"),
            source: wgpu::ShaderSource::Wgsl(ARC_SHADER.into()),
        });
        let arc = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-arcs-pipeline"),
            layout: Some(pipeline_layout),
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
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &arc_shader,
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
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-mesh"),
            source: wgpu::ShaderSource::Wgsl(MESH_SHADER.into()),
        });
        let mesh = Self::create_mesh_pipeline(
            device,
            pipeline_layout,
            &mesh_shader,
            target_format,
            wgpu::FrontFace::Ccw,
            true,
            samples,
            "cad-mesh-pipeline",
        );
        let mesh_mirrored = Self::create_mesh_pipeline(
            device,
            pipeline_layout,
            &mesh_shader,
            target_format,
            wgpu::FrontFace::Cw,
            true,
            samples,
            "cad-mesh-pipeline-mirrored",
        );
        // Transparent variants: depth-tested but not depth-written, so the
        // back-to-front pass composites instead of hiding geometry behind it.
        let mesh_transparent = Self::create_mesh_pipeline(
            device,
            pipeline_layout,
            &mesh_shader,
            target_format,
            wgpu::FrontFace::Ccw,
            false,
            samples,
            "cad-mesh-pipeline-transparent",
        );
        let mesh_mirrored_transparent = Self::create_mesh_pipeline(
            device,
            pipeline_layout,
            &mesh_shader,
            target_format,
            wgpu::FrontFace::Cw,
            false,
            samples,
            "cad-mesh-pipeline-mirrored-transparent",
        );
        let mesh_merged_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-mesh-merged"),
            source: wgpu::ShaderSource::Wgsl(MESH_MERGED_SHADER.into()),
        });
        let mesh_merged = Self::create_mesh_merged_pipeline(
            device,
            pipeline_layout,
            &mesh_merged_shader,
            target_format,
            wgpu::FrontFace::Ccw,
            samples,
            "cad-mesh-merged-pipeline",
        );
        let mesh_merged_mirrored = Self::create_mesh_merged_pipeline(
            device,
            pipeline_layout,
            &mesh_merged_shader,
            target_format,
            wgpu::FrontFace::Cw,
            samples,
            "cad-mesh-merged-pipeline-mirrored",
        );
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-image"),
            source: wgpu::ShaderSource::Wgsl(IMAGE_SHADER.into()),
        });
        let image = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-image-pipeline"),
            layout: Some(image_pipeline_layout),
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
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &image_shader,
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
        GeometryPipelines {
            line,
            line_instanced,
            arc,
            mesh,
            mesh_mirrored,
            mesh_transparent,
            mesh_mirrored_transparent,
            mesh_merged,
            mesh_merged_mirrored,
            image,
        }
    }
}
