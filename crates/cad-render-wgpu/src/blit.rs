//! Shared fullscreen-textured blit used by every surface presentation path.
//!
//! Both surface paths present the renderer's offscreen CAD frame the same way:
//! one fullscreen triangle samples the frame texture straight onto the
//! swapchain/canvas surface. No pixels are read back on either path and no extra
//! CAD surface is created. The WGSL source and the pipeline builder therefore
//! live here, so the native probe presenter and the wasm canvas presenter cannot
//! drift apart.
//!
//! One fragment stage per target *kind*, not per caller: an sRGB target gets the
//! pass-through stage ([`TEXTURE_SHADER`]), a linear target gets the
//! sRGB-encoding one ([`TEXTURE_SHADER_LINEAR_TARGET`]) so the presented bytes are
//! the same either way. The vertex stage is shared and deliberately contains no
//! array lookup — see the [`texture_shader!`] macro for the measured reason.
//!
//! [`FullscreenBlitter`] is the wasm-side stateful wrapper (pipeline plus a
//! one-texture bind-group cache). The native probe deliberately keeps its own
//! binding cache (keyed by `NativeTextureKey`) and its texture/surface size
//! fence: this module shares the shader and the pipeline construction only, so no
//! native presentation semantics change.

use crate::wgpu;

/// Assemble a complete texture shader from the shared vertex stage and a fragment
/// stage, so the two target kinds cannot drift apart.
///
/// # No array lookup for the corner (measured, Android WebView)
///
/// The vertex stage computes the corner from `vertex_index` **arithmetically**.
/// The obvious form — `var corners = array<vec2<f32>, 3>(…); corners[vertex_index]`
/// — reads as *zero for every vertex* on `com.google.android.webview
/// 124.0.6367.219` running on the emulator GLES translator + Google SwiftShader:
/// the draw rasterizes nothing and reports **no GL error**, so the frame is
/// silently dropped. The defect is the dynamic array index in the vertex stage,
/// not the geometry: the same three clip positions selected with `if`/`else` or
/// with arithmetic rasterize a full screen, and so does an attribute-fed triangle
/// (evidence: `docs/validation-android-present-2026-10-28.md`). wgpu-hal's own
/// sRGB present shader has exactly the indexed-array shape, which is why the
/// canvas presenter configures a **linear** surface and presents with
/// `blitFramebuffer` (see [`crate::canvas`]) instead of that shader.
macro_rules! texture_shader {
    ($fragment:literal) => {
        concat!(
            r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var cad_texture: texture_2d<f32>;
@group(0) @binding(1) var cad_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    // The covering triangle (-1,-1) (3,-1) (-1,3), written without an array so
    // no dynamic index reaches the GLSL ES 300 vertex stage.
    let right = f32(vertex_index == 1u);
    let top = f32(vertex_index == 2u);
    let xy = vec2<f32>(-1.0 + 4.0 * right, -1.0 + 4.0 * top);
    var out: VsOut;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    // Texture origin is top-left: clip y=+1 is the screen top, so v must be
    // (1 - y)/2. Getting this wrong flips the image vertically.
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}
"#,
            $fragment,
        )
    };
}

/// Fullscreen textured shader for an **sRGB target**: samples a CAD texture
/// directly to the surface (no CPU readback, no extra CAD surface). Opaque
/// display; the CAD texture is expected to be opaque (premultiplied alpha is not
/// used here).
///
/// The triangle covers the whole target with no aspect correction and no colour
/// conversion, which is why both callers pass a source texture whose size and
/// format match the configured surface: an sRGB frame texture into an sRGB target
/// is encoded by the hardware on write.
pub(crate) const TEXTURE_SHADER: &str = texture_shader!(
    r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(cad_texture, cad_sampler, in.uv);
}
"#
);

/// Fullscreen textured shader for a **linear target** (the wasm canvas surface,
/// see [`crate::canvas::preferred_present_format`]): the same triangle, but the
/// sRGB transfer function is applied in the fragment stage.
///
/// A linear present target stores the shader's output as-is, so sampling the
/// renderer's `Rgba8UnormSrgb` frame (which the hardware decodes to linear) and
/// writing it unchanged would present a visibly washed-out/too-dark frame. This
/// stage re-encodes, so the surface texture ends up holding exactly the bytes the
/// sRGB target would have stored, and wgpu-hal's `blitFramebuffer` present copies
/// them to the canvas untouched.
pub(crate) const TEXTURE_SHADER_LINEAR_TARGET: &str = texture_shader!(
    r#"
/// IEC 61966-2-1 linear -> sRGB, the same curve wgpu-hal's own present shader
/// uses (piecewise, not a `pow(1/2.2)` approximation).
fn linear_to_srgb(value: f32) -> f32 {
    let over = 1.055 * pow(max(value, 0.0), 0.41666) - 0.055;
    let under = 12.92 * value;
    return select(over, under, value <= 0.0031308);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let linear = textureSample(cad_texture, cad_sampler, in.uv);
    return vec4<f32>(
        linear_to_srgb(linear.r),
        linear_to_srgb(linear.g),
        linear_to_srgb(linear.b),
        linear.a,
    );
}
"#
);

/// The shader for `format`: pass-through into an sRGB target, encoded into a
/// linear one. Exposed so a test can assert both variants exist for the formats
/// the presenter can be configured with.
pub(crate) fn texture_shader_for(format: wgpu::TextureFormat) -> &'static str {
    if format.is_srgb() {
        TEXTURE_SHADER
    } else {
        TEXTURE_SHADER_LINEAR_TARGET
    }
}

/// Build the fullscreen textured pipeline, its bind-group layout and sampler.
///
/// `format` is the **configured target format** (surface format, or the format a
/// test renders into); never assume BGRA — canvas formats are RGBA on every
/// browser backend.
///
/// The fragment stage follows the target kind ([`texture_shader_for`]): an sRGB
/// target is written by the hardware's encoder, a linear target is encoded by the
/// shader. The native probe targets a real swapchain format and therefore keeps
/// the sRGB path; the wasm canvas presents through a linear surface (see
/// [`crate::canvas::preferred_present_format`] for the measured reason).
pub(crate) fn build_texture_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cad-texture-shader"),
        source: wgpu::ShaderSource::Wgsl(texture_shader_for(format).into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cad-texture-layout"),
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
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cad-texture-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cad-texture-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
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
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                // Opaque display: the CAD texture is expected to be opaque.
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cad-texture-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    (pipeline, layout, sampler)
}

/// Stateful wasm-side blitter: the textured pipeline plus a bind-group cache for
/// the single source texture of the last frame.
///
/// Wasm only on purpose: the native probe already owns an equivalent cache keyed
/// by its `NativeTextureKey`, and compiling a second, unused copy there would be
/// dead code. Both use [`build_texture_pipeline`] and the shaders above.
#[cfg(target_arch = "wasm32")]
pub(crate) struct FullscreenBlitter {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The last bound source texture and its bind group. The cache is keyed by
    /// **texture identity** (a `wgpu::Texture` comparison is a stable identity
    /// comparison), because a renderer session may reuse a size across sessions.
    bound: Option<(wgpu::Texture, wgpu::BindGroup)>,
    /// Bind groups created since construction (evidence only: a steady frame loop
    /// presents the same frame texture, so this must stay flat).
    binds_created: u64,
}

#[cfg(target_arch = "wasm32")]
impl FullscreenBlitter {
    /// Build the pipeline, layout and sampler for `format` (the configured
    /// surface format).
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let (pipeline, layout, sampler) = build_texture_pipeline(device, format);
        Self {
            pipeline,
            layout,
            sampler,
            bound: None,
            binds_created: 0,
        }
    }

    /// Number of bind groups created since construction.
    pub(crate) fn binds_created(&self) -> u64 {
        self.binds_created
    }

    /// Draw `texture` across the whole `target` and submit the pass.
    ///
    /// Callers validate the source (size/format/usage) and their own error
    /// channel first; this helper performs the GPU work only. No readback, no
    /// blocking poll: the caller presents the acquired surface texture after the
    /// submit.
    pub(crate) fn blit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        texture: &wgpu::Texture,
    ) {
        self.ensure_bound(device, texture);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cad-canvas-blit-encoder"),
        });
        {
            let bind = &self
                .bound
                .as_ref()
                .expect("ensure_bound just cached the source texture")
                .1;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-canvas-blit-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }

    /// Ensure the bind-group cache holds a group for `texture`'s identity.
    fn ensure_bound(&mut self, device: &wgpu::Device, texture: &wgpu::Texture) {
        let reusable = self
            .bound
            .as_ref()
            .is_some_and(|(cached, _)| cached == texture);
        if reusable {
            return;
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-canvas-texture-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.bound = Some((texture.clone(), bind));
        self.binds_created += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every textured-blit variant, by the target kind it serves.
    fn variants() -> [(&'static str, &'static str); 2] {
        [
            ("sRGB target", TEXTURE_SHADER),
            ("linear target", TEXTURE_SHADER_LINEAR_TARGET),
        ]
    }

    /// The regression guard for the measured Android WebView defect: a **dynamic
    /// array index in the vertex stage** rasterizes nothing there (every element
    /// reads as zero, no GL error), so the shader text must not introduce one. The
    /// corner is `vertex_index` arithmetic.
    #[test]
    fn textured_blit_vertex_stage_has_no_array_lookup() {
        for (name, shader) in variants() {
            assert!(
                !shader.contains("array<"),
                "{name}: the vertex stage must not declare an array; a dynamic index into one \
                 draws nothing on the Android WebView GLES implementation"
            );
            assert!(
                shader.contains("vertex_index == 1u") && shader.contains("vertex_index == 2u"),
                "{name}: the covering triangle must be computed from vertex_index arithmetic"
            );
        }
    }

    /// Both variants compile to GLSL ES 3.00 with the naga that wgpu embeds
    /// (WebGL2 is the only wasm backend), and the vertex stage back-translates
    /// without an indexed array — the shape wgpu-hal's own present shader has and
    /// this driver cannot draw.
    #[test]
    fn textured_blit_shaders_back_translate_to_glsl_es_300() {
        for (name, source) in variants() {
            let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| {
                panic!("{name}: WGSL parse error: {}", e.emit_to_string(source))
            });
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: WGSL validation error: {e:?}"));
            for (entry, stage) in [
                ("vs_main", naga::ShaderStage::Vertex),
                ("fs_main", naga::ShaderStage::Fragment),
            ] {
                let options = naga::back::glsl::Options {
                    version: naga::back::glsl::Version::new_gles(300),
                    ..Default::default()
                };
                let mut glsl = String::new();
                naga::back::glsl::Writer::new(
                    &mut glsl,
                    &module,
                    &info,
                    &options,
                    &naga::back::glsl::PipelineOptions {
                        shader_stage: stage,
                        entry_point: entry.to_string(),
                        multiview: None,
                    },
                    naga::proc::BoundsCheckPolicies::default(),
                )
                .unwrap_or_else(|e| panic!("{name} {entry}: GLSL ES 300 writer error: {e:?}"))
                .write()
                .unwrap_or_else(|e| panic!("{name} {entry}: GLSL ES 300 write error: {e:?}"));
                assert!(
                    glsl.contains("#version 300 es"),
                    "{name} {entry}: expected a GLSL ES 300 header, got:\n{glsl}"
                );
                if entry == "vs_main" {
                    assert!(
                        !glsl.contains("gl_VertexID]"),
                        "{name} {entry}: the vertex stage back-translated to a gl_VertexID array \
                         lookup, which draws nothing on the Android WebView:\n{glsl}"
                    );
                }
            }
        }
    }

    /// The variant is chosen by the **target** kind: an sRGB target keeps the
    /// hardware encoder, a linear target is encoded by the shader.
    #[test]
    fn textured_blit_shader_follows_the_target_kind() {
        assert_eq!(
            texture_shader_for(wgpu::TextureFormat::Rgba8UnormSrgb),
            TEXTURE_SHADER
        );
        assert_eq!(
            texture_shader_for(wgpu::TextureFormat::Rgba8Unorm),
            TEXTURE_SHADER_LINEAR_TARGET
        );
        // The encoding stage is the only difference between the two.
        assert!(TEXTURE_SHADER_LINEAR_TARGET.contains("linear_to_srgb"));
        assert!(!TEXTURE_SHADER.contains("linear_to_srgb"));
        for (name, shader) in variants() {
            assert!(
                shader.contains("fn vs_main") && shader.contains("fn fs_main"),
                "{name}: both entry points must be present"
            );
        }
    }
}
