//! On-demand GPU id pass and `Renderer::pick_at` (native only).
//!
//! A pick is a **separate, on-demand pass**, never part of a color frame: the
//! same visible geometry is drawn once more into a small offscreen *id target*
//! whose fragment value is the owning batch's renderer pick key (see
//! [`PickTable`]). Exactly one texel is then copied back and resolved on the CPU.
//!
//! Why a second pass instead of a per-frame id buffer: a per-frame id target
//! would cost bandwidth on every navigation frame for a gesture that happens on
//! click. The color pass is left completely untouched — a pick uses its own color
//! attachment, its own depth buffer and its own uniform slots — so a frame
//! rendered after a pick is byte-identical to a frame rendered without one (the
//! pick-pass non-interference fence).
//!
//! The id pass deliberately does **not** coalesce instanced/merged runs: each
//! batch (and each of its runs) is drawn with its own uniform carrying its own
//! pick key, so a fragment can never inherit a neighbouring batch's key.
//!
//! Sub-element (mesh-face) resolution is **not** attempted here: wgpu exposes no
//! primitive index to the fragment stage, so a face cannot be derived from the id
//! pass. Sub-element picking stays on CPU refinement (`cad_spatial`) and is never
//! faked.
//!
//! This module is native only (a blocking GPU→CPU readback is not available on
//! the browser tiers, which keep CPU picking).

use super::*;

/// Fragment tail for the `R32Uint` id target: one `u32` channel carries the key.
const PICK_FS_U32: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) u32 {
    return fs_key(in);
}
"#;

/// Fragment tail for the packed RGBA8 id target: the key is split into four
/// `unorm8` channels, low byte in `r`. The division by 255 is exact for every
/// 8-bit value (each is representable in `f32` and the unorm8 conversion is
/// round-to-nearest), so the readback recovers the key bit-for-bit.
const PICK_FS_PACKED: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let key = fs_key(in);
    return vec4<f32>(
        f32(key & 0xFFu),
        f32((key >> 8u) & 0xFFu),
        f32((key >> 16u) & 0xFFu),
        f32((key >> 24u) & 0xFFu),
    ) / 255.0;
}
"#;

// Shader bodies: uniform + vertex stage + `fs_key`, assembled with a fragment
// tail above (see `pick.rs`'s module docs). Both tails are validated offline.
const PICK_LINE_SHADER: &str = include_str!("../shaders/pick_line.wgsl");
const PICK_LINE_INSTANCED_SHADER: &str = include_str!("../shaders/pick_line_instanced.wgsl");
const PICK_MESH_SHADER: &str = include_str!("../shaders/pick_mesh.wgsl");
const PICK_MESH_MERGED_SHADER: &str = include_str!("../shaders/pick_mesh_merged.wgsl");
const PICK_IMAGE_SHADER: &str = include_str!("../shaders/pick_image.wgsl");

/// Assemble one id-pass shader module source for `format`.
pub fn pick_shader_source(body: &str, format: IdTargetFormat) -> String {
    let tail = match format {
        IdTargetFormat::R32Uint => PICK_FS_U32,
        IdTargetFormat::PackedRgba8 => PICK_FS_PACKED,
    };
    format!("{body}\n{tail}")
}

/// The id-pass shader bodies, for the offline WGSL validation test.
const PICK_SHADER_BODIES: [(&str, &str); 5] = [
    ("pick_line.wgsl", PICK_LINE_SHADER),
    ("pick_line_instanced.wgsl", PICK_LINE_INSTANCED_SHADER),
    ("pick_mesh.wgsl", PICK_MESH_SHADER),
    ("pick_mesh_merged.wgsl", PICK_MESH_MERGED_SHADER),
    ("pick_image.wgsl", PICK_IMAGE_SHADER),
];

/// Every id-pass shader module source assembled for `format`, as `(name,
/// source)` pairs.
///
/// The bodies alone are not complete WGSL modules: the fragment entry point is
/// generated per id-target format. Exposed so offline validation can check the
/// exact text the pipelines compile, including that tail.
pub fn pick_id_shader_sources(format: IdTargetFormat) -> Vec<(String, String)> {
    PICK_SHADER_BODIES
        .iter()
        .map(|(name, body)| ((*name).to_string(), pick_shader_source(body, format)))
        .collect()
}

/// One draw the id pass issues: the geometry plus the uniform slot that carries
/// its batch's pick key.
#[derive(Debug, Clone, Copy)]
enum PickDraw {
    /// A legacy per-batch draw (lines, meshes, `MeshEdges`, transparent batches).
    Legacy(usize),
    /// One instanced line run: `(stream, first instance, instance count)`.
    LineRun(usize, u32, u32),
    /// One merged mesh run: `(chunk, first index, one-past-last index)`.
    MeshRun(usize, u32, u32),
    /// One textured image quad.
    Image(usize),
}

impl Renderer {
    /// Create the id-pass pipelines. Mirrors the color pass's per-topology
    /// primitive state (same winding, culling and depth compare) so the id pass
    /// rasterizes the same fragments the user sees; depth writes are on for every
    /// id draw so the front-most fragment wins.
    pub(crate) fn create_pick_pipelines(&mut self, device: &wgpu::Device) {
        let (Some(camera_layout), Some(flags_layout), Some(image_layout)) = (
            self.layout.clone(),
            self.flags_layout.clone(),
            self.image_texture_layout.clone(),
        ) else {
            return;
        };
        let id_format = self.id_format;
        let pick_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-pick-pipeline-layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        // The image pipeline uses the *color* image pipeline's group order
        // (camera, flags table, sampled texture) so `draw_image` can bind the
        // texture at group 2 exactly as it does in the color pass. The flags
        // group is unused by the id shader but must be present (and bound) to
        // satisfy the pipeline layout.
        let pick_image_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-pick-image-pipeline-layout"),
            bind_group_layouts: &[
                Some(&camera_layout),
                Some(&flags_layout),
                Some(&image_layout),
            ],
            immediate_size: 0,
        });
        let module = |label: &str, body: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(pick_shader_source(body, id_format).into()),
            })
        };

        let line_shader = module("cad-pick-lines", PICK_LINE_SHADER);
        let line_instanced_shader = module("cad-pick-lines-instanced", PICK_LINE_INSTANCED_SHADER);
        let mesh_shader = module("cad-pick-mesh", PICK_MESH_SHADER);
        let mesh_merged_shader = module("cad-pick-mesh-merged", PICK_MESH_MERGED_SHADER);
        let image_shader = module("cad-pick-image", PICK_IMAGE_SHADER);

        // Line primitives: no culling, `LessEqual` and depth writes on, exactly
        // like `cad-lines-pipeline`.
        let line_primitive = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        };
        let line_depth = wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        self.pick_line_pipeline = Some(device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("cad-pick-lines-pipeline"),
                layout: Some(&pick_layout),
                vertex: wgpu::VertexState {
                    module: &line_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(position_only_layout())],
                },
                primitive: line_primitive,
                depth_stencil: Some(line_depth.clone()),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(IdFragment::new(&line_shader, id_format).state()),
                multiview_mask: None,
                cache: None,
            },
        ));
        self.pick_line_instanced_pipeline = Some(device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("cad-pick-lines-instanced-pipeline"),
                layout: Some(&pick_layout),
                vertex: wgpu::VertexState {
                    module: &line_instanced_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(position_only_layout()),
                        // The accelerated tier's instance buffer is also its
                        // pulled storage buffer (64-byte stride); the id pass must
                        // read the same layout or it would rasterize different
                        // fragments.
                        Some(if self.render_tier == RenderTier::Accelerated {
                            line_instance_layout_64()
                        } else {
                            line_instance_layout()
                        }),
                    ],
                },
                primitive: line_primitive,
                depth_stencil: Some(line_depth.clone()),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(IdFragment::new(&line_instanced_shader, id_format).state()),
                multiview_mask: None,
                cache: None,
            },
        ));

        // Mesh primitives: back-face culled with `Less`, matching
        // `cad-mesh-pipeline`. Two winding variants, as in the color pass.
        let mesh_pipeline = |front_face: wgpu::FrontFace, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pick_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(mesh_vertex_layout()),
                        Some(mesh_normal_layout()),
                        Some(mesh_color_layout()),
                    ],
                },
                primitive: mesh_primitive(front_face),
                depth_stencil: Some(mesh_depth()),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(IdFragment::new(&mesh_shader, id_format).state()),
                multiview_mask: None,
                cache: None,
            })
        };
        self.pick_mesh_pipeline = Some(mesh_pipeline(
            wgpu::FrontFace::Ccw,
            "cad-pick-mesh-pipeline",
        ));
        self.pick_mesh_pipeline_mirrored = Some(mesh_pipeline(
            wgpu::FrontFace::Cw,
            "cad-pick-mesh-pipeline-mirrored",
        ));

        let merged_pipeline = |front_face: wgpu::FrontFace, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pick_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_merged_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(mesh_merged_layout())],
                },
                primitive: mesh_primitive(front_face),
                depth_stencil: Some(mesh_depth()),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(IdFragment::new(&mesh_merged_shader, id_format).state()),
                multiview_mask: None,
                cache: None,
            })
        };
        self.pick_mesh_merged_pipeline = Some(merged_pipeline(
            wgpu::FrontFace::Ccw,
            "cad-pick-mesh-merged-pipeline",
        ));
        self.pick_mesh_merged_pipeline_mirrored = Some(merged_pipeline(
            wgpu::FrontFace::Cw,
            "cad-pick-mesh-merged-pipeline-mirrored",
        ));

        self.pick_image_pipeline = Some(device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("cad-pick-image-pipeline"),
                layout: Some(&pick_image_layout),
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
                    // Image quads are never culled in the color pass either.
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
                fragment: Some(IdFragment::new(&image_shader, id_format).state()),
                multiview_mask: None,
                cache: None,
            },
        ));
    }

    /// Ensure the id target (and its private depth buffer) matches the color
    /// target's size.
    fn ensure_pick_target(&mut self, device: &wgpu::Device) -> Result<(), RenderError> {
        let (width, height) = self.target_size;
        if self.pick_target.is_some() && self.pick_size == (width, height) {
            return Ok(());
        }
        let limit = device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit {
            return Err(RenderError::Frame(format!(
                "id target {width}x{height} is not renderable on this adapter"
            )));
        }
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cad-pick-target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.id_format.texture_format(),
            // COPY_SRC is the single texel readback; the target is never sampled.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cad-pick-depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.pick_view = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.pick_depth_view = Some(depth.create_view(&wgpu::TextureViewDescriptor::default()));
        self.pick_target = Some(texture);
        self.pick_size = (width, height);
        Ok(())
    }

    /// Encode and submit one id-pass frame for the current scene and projection.
    ///
    /// The geometry is exactly the color pass's: the same batches, the same
    /// camera-sorted opaque/transparent order and the same per-batch transforms.
    /// The one deliberate difference is that no runs are coalesced, so each draw's
    /// uniform names its own batch's pick key.
    fn render_id_pass(
        &mut self,
        projection: FrameProjection,
        camera_position: [f32; 3],
        camera_matrix: [f32; 16],
    ) -> Result<(), RenderError> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?
            .clone();
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?
            .clone();
        let Some(layout) = self.layout.clone() else {
            return Err(RenderError::NotInitialized(
                "renderer uniform layout missing".into(),
            ));
        };
        let Some(pick_view) = self.pick_view.clone() else {
            return Err(RenderError::NotInitialized("id target missing".into()));
        };
        let Some(pick_depth) = self.pick_depth_view.clone() else {
            return Err(RenderError::NotInitialized(
                "id depth target missing".into(),
            ));
        };
        let Some(line_pipeline) = self.pick_line_pipeline.clone() else {
            return Err(RenderError::NotInitialized(
                "id-pass pipelines missing".into(),
            ));
        };
        let Some(line_instanced_pipeline) = self.pick_line_instanced_pipeline.clone() else {
            return Err(RenderError::NotInitialized(
                "id-pass pipelines missing".into(),
            ));
        };
        let Some(mesh_pipeline) = self.pick_mesh_pipeline.clone() else {
            return Err(RenderError::NotInitialized(
                "id-pass pipelines missing".into(),
            ));
        };
        let Some(mesh_pipeline_mirrored) = self.pick_mesh_pipeline_mirrored.clone() else {
            return Err(RenderError::NotInitialized(
                "id-pass pipelines missing".into(),
            ));
        };
        let Some(mesh_merged_pipeline) = self.pick_mesh_merged_pipeline.clone() else {
            return Err(RenderError::NotInitialized(
                "id-pass pipelines missing".into(),
            ));
        };
        let Some(mesh_merged_pipeline_mirrored) = self.pick_mesh_merged_pipeline_mirrored.clone()
        else {
            return Err(RenderError::NotInitialized(
                "id-pass pipelines missing".into(),
            ));
        };
        let Some(image_pipeline) = self.pick_image_pipeline.clone() else {
            return Err(RenderError::NotInitialized(
                "id-pass pipelines missing".into(),
            ));
        };

        // The same budgeted plan a single color page uses; the id pass is not
        // progressive (a pick always covers the whole resident page).
        let plan = plan_from_gpu(&self.batches, &self.frame_budget);
        let entries: Vec<BatchOrderEntry> = plan
            .accepted
            .iter()
            .map(|&index| {
                let batch = &self.batches[index];
                BatchOrderEntry {
                    draw_order: batch.draw_order,
                    alpha: batch.alpha,
                    centroid: batch.centroid,
                }
            })
            .collect();
        let passes = draw_passes(&plan.accepted, &entries, Some(camera_position));
        let ordered: Vec<usize> = passes
            .opaque
            .iter()
            .chain(passes.transparent.iter())
            .copied()
            .collect();

        // Collect the draws (and their transforms/keys) before touching the
        // uniform arena, so the immutable borrows of the batch/stream/chunk fields
        // end first.
        struct Pending {
            slot: u32,
            matrix: [f32; 16],
            key: u32,
            draw: PickDraw,
        }
        let mut pending: Vec<Pending> = Vec::new();
        let mut slot = 0u32;
        for &index in &ordered {
            let batch = &self.batches[index];
            // Mirror the colour pass's order for a converted mesh batch: the
            // triangles first, then the wireframe overlay. Both are drawn (no
            // coalescing), each with this batch's own key.
            if !batch.mesh_runs.is_empty() {
                for run in &batch.mesh_runs {
                    let origin = self.mesh_chunks[run.chunk as usize].origin;
                    pending.push(Pending {
                        slot,
                        matrix: projection.batch_matrix_f64(origin),
                        key: batch.pick_key,
                        draw: PickDraw::MeshRun(
                            run.chunk as usize,
                            run.index_offset,
                            run.index_offset + run.index_count,
                        ),
                    });
                    slot += 1;
                }
            }
            if !batch.instanced.is_empty() {
                for run in &batch.instanced {
                    let stream = &self.instance_streams[run.stream as usize];
                    let origin = stream.origin;
                    // `run.offset` is stream-local (see `build_instance_chunks`);
                    // on the accelerated tier every stream in a storage segment
                    // shares one buffer, so the global instance index is
                    // `stream.base + run.offset` — exactly as the colour path
                    // re-bases it. `base` is 0 on the universal tier, where each
                    // stream owns its buffer, so the two paths agree there too.
                    let start = stream.base + run.offset;
                    pending.push(Pending {
                        slot,
                        matrix: projection.batch_matrix_f64(origin),
                        key: batch.pick_key,
                        draw: PickDraw::LineRun(run.stream as usize, start, run.count),
                    });
                    slot += 1;
                }
            }
            if batch.mesh_runs.is_empty() && batch.instanced.is_empty() {
                pending.push(Pending {
                    slot,
                    matrix: projection.batch_matrix(batch.origin),
                    key: batch.pick_key,
                    draw: PickDraw::Legacy(index),
                });
                slot += 1;
            }
        }
        // Images are drawn last, exactly as in the color pass.
        for index in 0..self.images.len() {
            let image = &self.images[index];
            pending.push(Pending {
                slot,
                matrix: camera_matrix,
                key: image.pick_key,
                draw: PickDraw::Image(index),
            });
            slot += 1;
        }

        let total_slots = slot as usize;
        self.uniforms.ensure_slots(total_slots, &device, &layout);
        for entry in &pending {
            let uniform = uniform_bytes(&entry.matrix, [1.0, 1.0, 1.0, 1.0], [entry.key, 0, 0, 0]);
            self.uniforms.write_slot(entry.slot as usize, &uniform);
        }
        self.uniforms.flush(&queue, total_slots);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cad-pick-encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-pick-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pick_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Key 0 is "no geometry"; the target is cleared to it.
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &pick_depth,
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
            let mut state = EncodeState::new();
            // The pick-image pipeline shares the color image pipeline's group
            // order (camera, flags table, sampled texture), so group 1 must be
            // bound for an image draw even though the id shaders never read it.
            // Frame-constant, exactly as in the color pass.
            if let Some(flags_bind_group) = self.flags_bind_group.as_ref() {
                pass.set_bind_group(1, flags_bind_group, &[]);
            }
            for entry in &pending {
                let (bind_group, offset) = self.uniforms.binding(entry.slot as usize);
                match entry.draw {
                    PickDraw::Image(index) => {
                        draw_image(
                            &mut pass,
                            &mut state,
                            &self.images[index],
                            bind_group,
                            offset,
                            &image_pipeline,
                        );
                    }
                    PickDraw::Legacy(index) => {
                        draw_batch(
                            &mut pass,
                            &mut state,
                            &self.batches[index],
                            bind_group,
                            offset,
                            &line_pipeline,
                            if self.batches[index].mirrored {
                                &mesh_pipeline_mirrored
                            } else {
                                &mesh_pipeline
                            },
                        );
                    }
                    PickDraw::LineRun(stream, start, count) => {
                        draw_instance_run(
                            &mut pass,
                            &mut state,
                            self.unit_segment_buffer
                                .as_ref()
                                .expect("unit segment buffer"),
                            &self.instance_streams[stream].buffer,
                            bind_group,
                            offset,
                            start,
                            count,
                            &line_instanced_pipeline,
                        );
                    }
                    PickDraw::MeshRun(chunk, start, end) => {
                        let mesh = &self.mesh_chunks[chunk];
                        state.set_pipeline(
                            &mut pass,
                            if mesh.mirrored {
                                &mesh_merged_pipeline_mirrored
                            } else {
                                &mesh_merged_pipeline
                            },
                        );
                        state.set_bind_group0(&mut pass, bind_group, offset);
                        state.set_vertex_buffer(&mut pass, 0, &mesh.vertex_buffer);
                        state.set_index_buffer(&mut pass, &mesh.index_buffer);
                        pass.draw_indexed(start..end, 0, 0..1);
                    }
                }
            }
        }
        let submission = queue.submit(Some(encoder.finish()));
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(self.poll_timeout),
            })
            .map_err(|e| RenderError::Frame(format!("id pass poll failed: {e}")))?;
        Ok(())
    }

    /// Pick the entity at target pixel `(x, y)`.
    ///
    /// `(0, 0)` is the top-left of the render target, matching the readback and
    /// the image convention. The pick runs the id pass with the **most recent
    /// color frame's** projection, so it always agrees with what is on screen;
    /// before any frame has been rendered it returns `None` (there is no view to
    /// pick against).
    ///
    /// `None` means the pixel is the cleared background or an unknown key. A hit
    /// whose key resolves to several sources comes back with `source: None` and
    /// the candidate list filled in — ambiguous, never silently narrowed.
    pub fn pick_at(&mut self, x: u32, y: u32) -> Result<Option<PickHit>, RenderError> {
        if self.device_lost {
            return Err(RenderError::DeviceLost(
                self.last_device_lost
                    .clone()
                    .unwrap_or_else(|| "device lost; rebuild required".into()),
            ));
        }
        let (Some(projection), Some(camera_position)) =
            (self.last_projection, self.last_camera_position)
        else {
            return Ok(None);
        };
        let camera_matrix = self.last_camera_matrix;
        let (width, height) = self.target_size;
        if x >= width || y >= height {
            return Ok(None);
        }
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?
            .clone();
        self.ensure_pick_target(&device)?;
        self.render_id_pass(projection, camera_position, camera_matrix)?;

        let key = self.read_pick_key(x, y)?;
        if key == PICK_KEY_NONE {
            return Ok(None);
        }
        Ok(self.pick_table.resolve(key).map(|resolved| PickHit {
            entity_key: key,
            source: resolved.source,
            candidates: resolved.sources,
            // The id pass cannot see a primitive index, so a face is never
            // claimed here; CPU refinement owns sub-elements.
            sub_element: None,
            reason: resolved.reason.or(Some(PICK_REASON_SUB_ELEMENT)),
            key_collision: resolved.collision,
        }))
    }

    /// Copy the single id-target texel at `(x, y)` back and decode its key.
    fn read_pick_key(&self, x: u32, y: u32) -> Result<u32, RenderError> {
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("no GPU device initialized".into()))?;
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("no GPU queue initialized".into()))?;
        let texture = self
            .pick_target
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("no id target".into()))?;
        // One texel, one row: `bytes_per_row` still has to be copy-aligned, so the
        // buffer is one padded row and only the first four bytes are read.
        let bytes_per_row = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad-pick-readback"),
            size: u64::from(bytes_per_row),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cad-pick-readback-encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(self.poll_timeout),
            })
            .map_err(|e| RenderError::Frame(format!("id readback poll failed: {e}")))?;
        let key = {
            let view = slice
                .get_mapped_range()
                .map_err(|e| RenderError::Frame(format!("id readback map failed: {e}")))?;
            match self.id_format {
                IdTargetFormat::R32Uint => u32::from_le_bytes([view[0], view[1], view[2], view[3]]),
                // Four unorm8 channels, low byte in `r`. `round` recovers the
                // exact byte for every value the shader wrote.
                IdTargetFormat::PackedRgba8 => {
                    let channel = |byte: u8| -> u32 {
                        (f32::from(byte) / 255.0 * 255.0).round() as u32 & 0xFF
                    };
                    channel(view[0])
                        | (channel(view[1]) << 8)
                        | (channel(view[2]) << 16)
                        | (channel(view[3]) << 24)
                }
            }
        };
        buffer.unmap();
        Ok(key)
    }
}

/// Owns the id-target color state so a pipeline descriptor can borrow it.
///
/// `FragmentState` borrows its target list, so the list cannot be a temporary of
/// the helper call; this keeps the array alive next to the descriptor.
struct IdFragment<'a> {
    module: &'a wgpu::ShaderModule,
    targets: [Option<wgpu::ColorTargetState>; 1],
}

impl<'a> IdFragment<'a> {
    fn new(module: &'a wgpu::ShaderModule, format: IdTargetFormat) -> Self {
        IdFragment {
            module,
            targets: [Some(wgpu::ColorTargetState {
                format: format.texture_format(),
                // No blending: the id target holds raw keys, and every id draw
                // writes depth so the front-most fragment replaces the others.
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }
    }

    fn state(&self) -> wgpu::FragmentState<'_> {
        wgpu::FragmentState {
            module: self.module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &self.targets,
        }
    }
}

fn mesh_primitive(front_face: wgpu::FrontFace) -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        strip_index_format: None,
        front_face,
        cull_mode: Some(wgpu::Face::Back),
        unclipped_depth: false,
        polygon_mode: wgpu::PolygonMode::Fill,
        conservative: false,
    }
}

fn mesh_depth() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

#[cfg(test)]
mod shader_tests {
    use super::*;
    use naga::valid::{Capabilities, ValidationFlags, Validator};

    /// Assemble every id-pass shader body with both fragment tails and validate
    /// it with the same `naga` wgpu embeds. The bodies alone are not complete
    /// modules (the fragment entry point comes from the tail), so this is the
    /// only place the real module text is checked.
    fn validate(name: &str, source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{name}: WGSL parse error: {}", e.emit_to_string(source)));
        let mut validator = Validator::new(ValidationFlags::all(), Capabilities::empty());
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: WGSL validation error: {e:?}"));
        let has = |entry: &str, stage: naga::ShaderStage| {
            module
                .entry_points
                .iter()
                .any(|e| e.name == entry && e.stage == stage)
        };
        assert!(
            has("vs_main", naga::ShaderStage::Vertex),
            "{name}: no vs_main"
        );
        assert!(
            has("fs_main", naga::ShaderStage::Fragment),
            "{name}: no fs_main"
        );
    }

    #[test]
    fn every_id_shader_assembles_and_validates_for_both_targets() {
        for (name, body) in PICK_SHADER_BODIES {
            for format in [IdTargetFormat::R32Uint, IdTargetFormat::PackedRgba8] {
                let source = pick_shader_source(body, format);
                validate(&format!("{name} ({})", format.as_str()), &source);
            }
        }
    }

    #[test]
    fn the_two_tails_declare_the_matching_fragment_output() {
        assert!(PICK_FS_U32.contains("-> @location(0) u32"));
        assert!(PICK_FS_PACKED.contains("-> @location(0) vec4<f32>"));
        // Both read the shared `fs_key`, which is what the bodies declare.
        assert!(PICK_FS_U32.contains("fs_key(in)"));
        assert!(PICK_FS_PACKED.contains("fs_key(in)"));
    }

    #[test]
    fn packed_encoding_round_trips_every_byte() {
        // The shader writes `byte / 255.0` into a unorm8 channel; the readback
        // multiplies back and rounds. Every 8-bit value must survive.
        for byte in 0u32..=255 {
            let encoded = byte as f32 / 255.0;
            let decoded = (encoded * 255.0).round() as u32 & 0xFF;
            assert_eq!(decoded, byte, "unorm8 round trip for {byte}");
        }
    }
}
