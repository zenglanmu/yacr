//! PBR channel resources and draw path (ADR-0039 D6).
//!
//! One module owns everything the `mesh_pbr` channel adds: its pipeline families,
//! the five-texture bind-group layout, the exact multiplicative-identity textures
//! bound for a disabled channel, the per-channel-set bind-group cache, the
//! per-batch uniform payload and the draw dispatch. Everything else in this crate
//! keeps its previous layout and state.
//!
//! Kept out of `init.rs`/`encode.rs`/`pipelines.rs` so those stay within the
//! file-size gate; the items are inherent methods/functions of the same `Renderer`
//! and reach the rest of the crate through `super::*`.

use super::*;

/// Which vertex streams a PBR pipeline consumes (ADR-0039 D6).
///
/// Chosen from the batch's own buffers at draw time, so a pipeline never declares
/// an attribute the batch cannot supply and no UV/tangent is fabricated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PbrVertexLayout {
    /// position/normal/colour/UV0/tangent (`vs_main`): all five channels usable.
    Full,
    /// position/normal/colour/UV0 (`vs_uv`): no normal map (no tangent frame).
    Uv,
    /// position/normal/colour (`vs_flat`): scalar factors only, no UV0 at all.
    Flat,
}

impl PbrVertexLayout {
    /// The layout a batch's own streams allow: tangents imply UVs.
    pub(crate) fn of(has_uvs: bool, has_tangents: bool) -> Self {
        if has_tangents {
            PbrVertexLayout::Full
        } else if has_uvs {
            PbrVertexLayout::Uv
        } else {
            PbrVertexLayout::Flat
        }
    }
}

impl Renderer {
    /// Create one PBR mesh pipeline (`mesh_pbr`, ADR-0039 D6).
    ///
    /// It is the mesh pipeline's primitive/depth state plus one of three vertex
    /// layouts, chosen by the batch's own streams so no attribute is ever invented:
    ///
    /// * [`PbrVertexLayout::Full`]: slots 0–4 =
    ///   position/normal/colour/UV0/tangent, through `vs_main`. Every texture
    ///   channel (including the normal map) is usable.
    /// * [`PbrVertexLayout::Uv`]: slots 0–3 (no tangent), through `vs_uv`. The
    ///   UV0-sampling channels work; the normal map is disabled because no tangent
    ///   frame exists.
    /// * [`PbrVertexLayout::Flat`]: slots 0–2 (position/normal/colour), through
    ///   `vs_flat`. The scalar-only material: no UV or tangent buffer is declared.
    ///
    /// Group 2 is the PBR channel layout (five textures + one filtering sampler).
    /// All variants share one fragment entry point. Nothing here is shared with
    /// the untextured or textured pipelines, so their behaviour is untouched.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_mesh_pbr_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
        front_face: wgpu::FrontFace,
        depth_write: bool,
        samples: u32,
        label: &str,
        layout: PbrVertexLayout,
    ) -> wgpu::RenderPipeline {
        let (entry_point, buffers): (&str, &[Option<wgpu::VertexBufferLayout<'_>>]) = match layout {
            PbrVertexLayout::Full => (
                "vs_main",
                &[
                    Some(mesh_vertex_layout()),
                    Some(mesh_normal_layout()),
                    Some(mesh_color_layout()),
                    Some(mesh_uv_layout()),
                    Some(mesh_tangent_layout()),
                ],
            ),
            PbrVertexLayout::Uv => (
                "vs_uv",
                &[
                    Some(mesh_vertex_layout()),
                    Some(mesh_normal_layout()),
                    Some(mesh_color_layout()),
                    Some(mesh_uv_layout()),
                ],
            ),
            PbrVertexLayout::Flat => (
                "vs_flat",
                &[
                    Some(mesh_vertex_layout()),
                    Some(mesh_normal_layout()),
                    Some(mesh_color_layout()),
                ],
            ),
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                buffers,
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

    /// Create the PBR mesh pipeline families (ADR-0039 D6).
    ///
    /// Twelve pipelines: three vertex layouts ([`PbrVertexLayout`]) ×
    /// opaque/transparent × normal/mirrored, so a PBR batch keeps exactly the depth
    /// and winding behaviour the mesh pipeline would have had.
    #[allow(clippy::type_complexity)]
    pub(super) fn create_mesh_pbr_pipelines(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        flags_layout: &wgpu::BindGroupLayout,
        pbr_texture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> (
        [wgpu::RenderPipeline; 4],
        [wgpu::RenderPipeline; 4],
        [wgpu::RenderPipeline; 4],
    ) {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-mesh-pbr"),
            source: wgpu::ShaderSource::Wgsl(MESH_PBR_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-mesh-pbr-pipeline-layout"),
            bind_group_layouts: &[
                Some(camera_layout),
                Some(flags_layout),
                Some(pbr_texture_layout),
            ],
            immediate_size: 0,
        });
        let family = |layout: PbrVertexLayout, prefix: &str| {
            let make = |front_face: wgpu::FrontFace, depth_write: bool, label: String| {
                Self::create_mesh_pbr_pipeline(
                    device,
                    &pipeline_layout,
                    &shader,
                    format,
                    front_face,
                    depth_write,
                    samples,
                    &label,
                    layout,
                )
            };
            [
                make(wgpu::FrontFace::Ccw, true, format!("cad-mesh-pbr-{prefix}")),
                make(
                    wgpu::FrontFace::Cw,
                    true,
                    format!("cad-mesh-pbr-{prefix}-mirrored"),
                ),
                make(
                    wgpu::FrontFace::Ccw,
                    false,
                    format!("cad-mesh-pbr-{prefix}-transparent"),
                ),
                make(
                    wgpu::FrontFace::Cw,
                    false,
                    format!("cad-mesh-pbr-{prefix}-mirrored-transparent"),
                ),
            ]
        };
        (
            family(PbrVertexLayout::Full, ""),
            family(PbrVertexLayout::Uv, "uv"),
            family(PbrVertexLayout::Flat, "flat"),
        )
    }

    /// Create the PBR channel bind-group layout (group 2 of `mesh_pbr`).
    ///
    /// Five `texture_2d<f32>` (filterable) and one filtering sampler — the same
    /// texture/sampler types the image and textured-mesh channels use, so all
    /// three share one upload path and stay WebGL2-safe.
    pub(super) fn create_pbr_texture_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad-pbr-texture-layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                texture_entry(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }

    /// Create the exact multiplicative-identity 1x1 textures (ADR-0039 D6).
    ///
    /// They are `Rgba8Unorm` (linear, no sRGB decode): the identity values are
    /// already in the space the shader compares against, and a *normal* identity
    /// must not pass through an sRGB transform.
    pub(super) fn create_pbr_identities(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> PbrIdentities {
        let make = |label: &str, rgba: [u8; 4]| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            texture
        };
        let white = make("cad-pbr-identity-white", [255, 255, 255, 255]);
        let black = make("cad-pbr-identity-black", [0, 0, 0, 255]);
        // (0.5, 0.5, 1) in linear 8-bit: 128 -> 0.502, 255 -> 1.0. `2*x - 1`
        // yields very close to (0, 0, 1) and normalising restores it exactly.
        let flat_normal = make("cad-pbr-identity-flat-normal", [128, 128, 255, 255]);
        let view =
            |texture: &wgpu::Texture| texture.create_view(&wgpu::TextureViewDescriptor::default());
        PbrIdentities {
            white: view(&white),
            black: view(&black),
            flat_normal: view(&flat_normal),
            _textures: [white, black, flat_normal],
        }
    }

    /// Create the all-identity PBR bind group used when a batch's channel set has
    /// not been resolved (or is entirely absent). Every slot is its channel's
    /// multiplicative identity, so a disabled channel is neutral.
    pub(super) fn create_pbr_identity_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        identities: &PbrIdentities,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-pbr-identity-bind"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&identities.white),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&identities.white),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&identities.flat_normal),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&identities.black),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&identities.white),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Install the freshly created PBR resources into this renderer (one call from
    /// the device-initialisation path; keeps the eight assignments out of it).
    #[allow(clippy::type_complexity)]
    pub(super) fn install_pbr_resources(
        &mut self,
        full: [wgpu::RenderPipeline; 4],
        uv: [wgpu::RenderPipeline; 4],
        flat: [wgpu::RenderPipeline; 4],
        layout: wgpu::BindGroupLayout,
        identities: PbrIdentities,
        identity_bind_group: wgpu::BindGroup,
    ) {
        self.install_pbr_pipelines(full, uv, flat);
        self.pbr_texture_layout = Some(layout);
        self.pbr_identities = Some(identities);
        self.pbr_identity_bind_group = Some(identity_bind_group);
    }

    /// Install only the twelve PBR pipelines (used when rebuilding after an
    /// MSAA sample-count change; the bind-group layout and identity textures
    /// survive that rebuild).
    pub(super) fn install_pbr_pipelines(
        &mut self,
        full: [wgpu::RenderPipeline; 4],
        uv: [wgpu::RenderPipeline; 4],
        flat: [wgpu::RenderPipeline; 4],
    ) {
        let [pbr, mirrored, transparent, mirrored_transparent] = full;
        self.mesh_pipeline_pbr = Some(pbr);
        self.mesh_pipeline_pbr_mirrored = Some(mirrored);
        self.mesh_pipeline_pbr_transparent = Some(transparent);
        self.mesh_pipeline_pbr_mirrored_transparent = Some(mirrored_transparent);
        let [pbr_uv, uv_mirrored, uv_transparent, uv_mirrored_transparent] = uv;
        self.mesh_pipeline_pbr_uv = Some(pbr_uv);
        self.mesh_pipeline_pbr_uv_mirrored = Some(uv_mirrored);
        self.mesh_pipeline_pbr_uv_transparent = Some(uv_transparent);
        self.mesh_pipeline_pbr_uv_mirrored_transparent = Some(uv_mirrored_transparent);
        let [flat, flat_mirrored, flat_transparent, flat_mirrored_transparent] = flat;
        self.mesh_pipeline_pbr_flat = Some(flat);
        self.mesh_pipeline_pbr_flat_mirrored = Some(flat_mirrored);
        self.mesh_pipeline_pbr_flat_transparent = Some(flat_transparent);
        self.mesh_pipeline_pbr_flat_mirrored_transparent = Some(flat_mirrored_transparent);
    }

    /// The `mesh_pbr` pipeline for a batch's mirror/alpha class and vertex layout
    /// variant (ADR-0039 D6).
    ///
    /// `full == true` is the UV0 + tangent family (`vs_main`); `full == false` is
    /// the scalar-only family (`vs_flat`), used when the batch has no tangent
    /// frame so the pipeline never asks for a UV/tangent buffer it cannot supply.
    /// `None` only before device initialisation (or after a device loss).
    fn pbr_mesh_pipeline(
        &self,
        mirrored: bool,
        transparent: bool,
        layout: PbrVertexLayout,
    ) -> Option<&wgpu::RenderPipeline> {
        match (layout, mirrored, transparent) {
            (PbrVertexLayout::Full, false, false) => self.mesh_pipeline_pbr.as_ref(),
            (PbrVertexLayout::Full, true, false) => self.mesh_pipeline_pbr_mirrored.as_ref(),
            (PbrVertexLayout::Full, false, true) => self.mesh_pipeline_pbr_transparent.as_ref(),
            (PbrVertexLayout::Full, true, true) => {
                self.mesh_pipeline_pbr_mirrored_transparent.as_ref()
            }
            (PbrVertexLayout::Uv, false, false) => self.mesh_pipeline_pbr_uv.as_ref(),
            (PbrVertexLayout::Uv, true, false) => self.mesh_pipeline_pbr_uv_mirrored.as_ref(),
            (PbrVertexLayout::Uv, false, true) => self.mesh_pipeline_pbr_uv_transparent.as_ref(),
            (PbrVertexLayout::Uv, true, true) => {
                self.mesh_pipeline_pbr_uv_mirrored_transparent.as_ref()
            }
            (PbrVertexLayout::Flat, false, false) => self.mesh_pipeline_pbr_flat.as_ref(),
            (PbrVertexLayout::Flat, true, false) => self.mesh_pipeline_pbr_flat_mirrored.as_ref(),
            (PbrVertexLayout::Flat, false, true) => {
                self.mesh_pipeline_pbr_flat_transparent.as_ref()
            }
            (PbrVertexLayout::Flat, true, true) => {
                self.mesh_pipeline_pbr_flat_mirrored_transparent.as_ref()
            }
        }
    }

    /// The pipeline, group-2 bind group and vertex-layout variant a PBR batch
    /// draws with, plus the fallback report for declared-but-unresolved channels.
    ///
    /// A channel whose key is not resident is drawn with its **identity** texture
    /// (channel disabled) and named in `fallbacks` — never with a placeholder
    /// image. A normal map additionally requires a CPU-generated tangent frame; if
    /// the material declared one the pack step records
    /// `render.pbr.tangent_unavailable`, and the shader disables that channel.
    pub(super) fn pbr_draw_state<'a>(
        &'a self,
        batch: &GpuBatch,
        transparent: bool,
        fallbacks: &mut Vec<ResourceKey>,
    ) -> Option<(
        &'a wgpu::RenderPipeline,
        &'a wgpu::BindGroup,
        PbrVertexLayout,
    )> {
        let pbr = batch.pbr.as_ref()?;
        let layout = PbrVertexLayout::of(batch.uvs.is_some(), batch.has_tangents);
        let pipeline = self.pbr_mesh_pipeline(batch.mirrored, transparent, layout)?;
        let mut resolve = |key: Option<&ResourceKey>, resident: &dyn Fn(&ResourceKey) -> bool| {
            if let Some(key) = key {
                if !resident(key) && !fallbacks.contains(key) {
                    fallbacks.push(key.clone());
                }
            }
        };
        let colour_resident = |key: &ResourceKey| self.mesh_textures.contains_key(key);
        let data_resident = |key: &ResourceKey| self.pbr_data_textures.contains_key(key);
        resolve(batch.texture.as_ref(), &colour_resident);
        resolve(pbr.metallic_roughness.as_ref(), &data_resident);
        resolve(pbr.emissive_texture.as_ref(), &colour_resident);
        resolve(pbr.occlusion.as_ref(), &data_resident);
        if layout == PbrVertexLayout::Full {
            resolve(pbr.normal.as_ref(), &data_resident);
        }
        let set = PbrTextureSet::of(batch);
        let bind_group = self
            .pbr_bind_groups
            .get(&set)
            .or(self.pbr_identity_bind_group.as_ref())?;
        Some((pipeline, bind_group, layout))
    }

    /// This frame's per-batch uniform: the frame-constant light and camera
    /// position come from the renderer (`last_light`/`last_camera_position`, set at
    /// the top of the frame), so the draw loop passes only the batch's own values.
    ///
    /// `ibl_enabled` is renderer-level frame state (ADR-0044), not per batch: it
    /// rides the shared uniform and needs no pipeline change, so toggling it
    /// only affects the next frame's uniform data.
    pub(super) fn batch_uniform(
        &self,
        batch: &GpuBatch,
        matrix: &[f32; 16],
        tint: [f32; 4],
        key_flags: [u32; 4],
    ) -> [u8; UNIFORM_STRUCT_BYTES as usize] {
        batch_uniform(
            batch,
            matrix,
            tint,
            key_flags,
            self.last_light,
            self.last_camera_position.unwrap_or([0.0; 3]),
            self.ibl_enabled,
            self.fog_words(),
        )
    }

    /// Enable or disable the procedural analytic IBL in the PBR channel
    /// (ADR-0044).
    ///
    /// This only stores the flag: the value travels as per-frame uniform data
    /// (`Camera.pbr_eye.w`), so the next frame picks it up without a pipeline
    /// rebuild, a plan invalidation or a re-upload. `false` restores the legacy
    /// `AMBIENT` floor exactly.
    ///
    /// Storing the flag is not enough on its own: once a progressive walk has
    /// completed it keeps `cursor == ordered.len()`, and the frame key
    /// (`frame_key`) deliberately excludes the IBL flag (it is not a
    /// camera/attachment change). The next render with an unchanged key would
    /// then accept no page, draw nothing into the retained colour target and
    /// present the **pre-toggle** shading — the toggle was invisible after the
    /// first settled frame. The geometry and camera are unchanged, so this
    /// restarts the walk at cursor 0 over the loaded colour (clear depth only),
    /// exactly like a selection-flag toggle (`set_selection_flags_enabled`),
    /// instead of dropping the plan (which would clear the background) or doing a
    /// full scene rebuild.
    /// SketchUp "Shadows": use `sun` (a unit vector toward the sun) instead of
    /// the camera-relative key light for the next frames. `None` restores the
    /// camera key light.
    ///
    /// Per-frame uniform data only; the frame light the id pass reuses is the
    /// same value. A non-finite vector is refused (the previous light stays).
    pub fn set_sun(&mut self, sun: Option<[f32; 3]>) {
        let sun = sun.and_then(|sun| {
            let length = sun[0].hypot(sun[1]).hypot(sun[2]);
            (length.is_finite() && length > 1e-9)
                .then(|| [sun[0] / length, sun[1] / length, sun[2] / length])
        });
        if self.sun == sun {
            return;
        }
        self.sun = sun;
        // A shading change is uniform data, but a progressive frame mid-walk
        // would keep the old light in its retained colour half.
        if let Some(frame) = self.progressive_frame.as_mut() {
            frame.cursor = 0;
            self.preserve_color_on_restart = true;
        }
    }

    /// The frame's distance fog, or `None` to draw unfogged geometry.
    pub fn set_fog(&mut self, fog: Option<FrameFog>) {
        if self.fog == fog {
            return;
        }
        self.fog = fog;
        if let Some(frame) = self.progressive_frame.as_mut() {
            frame.cursor = 0;
            self.preserve_color_on_restart = true;
        }
    }

    /// The shared uniform's fog words for this frame.
    pub(super) fn fog_words(&self) -> [f32; 8] {
        self.fog.map_or([0.0; 8], FrameFog::words)
    }

    /// The shared uniform for one stream/chunk draw: opaque white tint, zero
    /// key, and the frame's fog words plus the eye in that stream's fixed-point
    /// space (the stream origin is already folded into `matrix`).
    pub(super) fn stream_uniform(
        &self,
        matrix: &[f32; 16],
        key_flags: [u32; 4],
        light: [f32; 4],
        origin: [f64; 3],
    ) -> [u8; UNIFORM_STRUCT_BYTES as usize] {
        uniform_bytes_fog(
            matrix,
            [1.0; 4],
            key_flags,
            light,
            self.fog_words(),
            self.eye_local(origin),
        )
    }

    /// The eye position in one stream's fixed-point space (the fog distance
    /// reference; the stream's origin is already folded into its matrix).
    pub(super) fn eye_local(&self, origin: [f64; 3]) -> [f32; 3] {
        let eye = self.last_camera_position.unwrap_or([0.0; 3]);
        [
            eye[0] - origin[0] as f32,
            eye[1] - origin[1] as f32,
            eye[2] - origin[2] as f32,
        ]
    }

    pub fn set_ibl_enabled(&mut self, enabled: bool) {
        if self.ibl_enabled == enabled {
            return;
        }
        self.ibl_enabled = enabled;
        if let Some(frame) = self.progressive_frame.as_mut() {
            frame.cursor = 0;
            self.preserve_color_on_restart = true;
        }
    }
}
