//! Native GPU contract for mesh-texture **sampler wrap** (ADR-0044 §7).
//!
//! The root cause of the reported "weird texture" on Khronos `DamagedHelmet`:
//! `mesh_sampler` clamped U/V to the edge, while glTF 2.0's default sampler wrap
//! is REPEAT (the asset's `samplers: [{}]`) and its `TEXCOORD_0` V is in
//! `[1.0006, 1.9987]`. Every texel therefore clamped to V = 1.0, i.e. the
//! texture's last row stretched along U — read as 1-D "brushed metal" bands,
//! zero metal, black emissive and flat AO.
//!
//! This fixture pins the difference directly: a 4×4 texture whose last row is
//! red and whose other rows are green, drawn on a full-screen triangle whose
//! V ∈ [1, 2]. The production sampler must sample **REPEAT** — the green rows
//! dominate the frame and the red last row appears once per period (its seam
//! blends with the wrapped first row under bilinear filtering) — while a locally
//! built `ClampToEdge` sampler must stretch the red last row over the whole
//! frame. That clamp result is exactly the defect.
//!
//! A software adapter's result is sampling/geometry evidence only — never a
//! performance or vendor claim. Run serially (these bind a real device):
//!
//! ```text
//! cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 mesh_wrap_gpu_tests
//! ```
use super::mesh_pbr_gpu_tests::{gpu_or_skip, init};
use crate::headless::HeadlessGpu;
use crate::wgpu;

/// The fixture texture is 4×4: three green rows and a red last row.
const SIDE: u32 = 4;
/// The drawn target: one or more pixels per texel, so filtering stays at LOD 0.
const TARGET: u32 = 16;

/// Full-screen triangle whose V spans exactly `[1, 2]` (one REPEAT period).
///
/// `uv.x` stays `[0, 1]`; `uv.y = 2 - (y + 1) / 2` gives V = 1 at the top of the
/// screen and V = 2 at the bottom, so the quad samples one full wrapped period.
const WRAP_SAMPLE_WGSL: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let corner = corners[index];
    var out: VsOut;
    out.position = vec4<f32>(corner, 0.0, 1.0);
    out.uv = vec2<f32>((corner.x + 1.0) * 0.5, 2.0 - (corner.y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // LOD 0 is explicit so the only variable between the two runs is the
    // sampler's address mode, not a driver's implicit-LOD choice.
    return textureSampleLevel(source_texture, source_sampler, in.uv, 0.0);
}
"#;

/// Three green rows and a red last row, top to bottom.
fn last_row_red() -> Vec<u8> {
    let mut rgba = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        let value = if y == SIDE - 1 {
            [255u8, 0, 0, 255]
        } else {
            [0u8, 255, 0, 255]
        };
        for _ in 0..SIDE {
            rgba.extend_from_slice(&value);
        }
    }
    rgba
}

/// Draw `rgba` with `sampler` and return the tightly packed target pixels.
fn sample(gpu: &HeadlessGpu, sampler: &wgpu::Sampler, rgba: &[u8]) -> Vec<u8> {
    let device = &gpu.device;
    let queue = &gpu.queue;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cad-wrap-contract-texture"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // 4 texels × 4 bytes = 16 bytes per row, under the 256-byte copy alignment,
    // so one padded stride is written per level row.
    let row_bytes = (SIDE * 4) as usize;
    let aligned = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let mut padded = vec![0u8; aligned * SIDE as usize];
    for row in 0..SIDE as usize {
        padded[row * aligned..row * aligned + row_bytes]
            .copy_from_slice(&rgba[row * row_bytes..(row + 1) * row_bytes]);
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &padded,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned as u32),
            rows_per_image: Some(SIDE),
        },
        wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
    );

    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cad-wrap-contract-bind-layout"),
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
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cad-wrap-contract-bind"),
        layout: &bind_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cad-wrap-contract-shader"),
        source: wgpu::ShaderSource::Wgsl(WRAP_SAMPLE_WGSL.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cad-wrap-contract-pipeline-layout"),
        bind_group_layouts: &[Some(&bind_layout)],
        immediate_size: 0,
    });
    let target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cad-wrap-contract-pipeline"),
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
                format: target_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cad-wrap-contract-target"),
        size: wgpu::Extent3d {
            width: TARGET,
            height: TARGET,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cad-wrap-contract-encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cad-wrap-contract-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
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
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    let target_row_bytes = TARGET * 4;
    let bytes_per_row = target_row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cad-wrap-contract-readback"),
        size: u64::from(bytes_per_row) * u64::from(TARGET),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(TARGET),
            },
        },
        wgpu::Extent3d {
            width: TARGET,
            height: TARGET,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_result| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll the wrap contract readback");
    let mapped = slice
        .get_mapped_range()
        .expect("map the wrap contract readback");
    let mut pixels = Vec::with_capacity((target_row_bytes * TARGET) as usize);
    for row in 0..TARGET as usize {
        let start = row * bytes_per_row as usize;
        pixels.extend_from_slice(&mapped[start..start + target_row_bytes as usize]);
    }
    pixels
}

/// A pixel whose green channel dominates (the texture's non-last rows).
fn green_dominant(pixel: &[u8]) -> bool {
    pixel[1] > pixel[0]
}

/// A pixel whose red channel dominates (the texture's last row).
fn red_dominant(pixel: &[u8]) -> bool {
    pixel[0] > pixel[1]
}

#[test]
fn mesh_textures_sample_repeat_not_clamp_to_edge() {
    let Some(gpu) = gpu_or_skip("mesh texture sampler wrap") else {
        return;
    };
    let renderer = init(&gpu);
    let production = renderer
        .mesh_sampler()
        .expect("an initialized renderer owns the mesh sampler");
    let clamped = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cad-wrap-contract-clamp"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });

    let rgba = last_row_red();
    let repeated = sample(&gpu, production, &rgba);
    let edge_clamped = sample(&gpu, &clamped, &rgba);

    let count = |pixels: &[u8], predicate: fn(&[u8]) -> bool| {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| predicate(*p))
            .count()
    };
    let total = (TARGET * TARGET) as usize;
    let repeated_green = count(&repeated, green_dominant);
    let repeated_red = count(&repeated, red_dominant);
    let clamped_green = count(&edge_clamped, green_dominant);
    let clamped_red = count(&edge_clamped, red_dominant);
    eprintln!(
        "[mesh-wrap-gpu] V∈[1,2], 4×4 texture (last row red): production sampler \
         green-dominant={repeated_green} red-dominant={repeated_red} of {total}; \
         ClampToEdge green-dominant={clamped_green} red-dominant={clamped_red} of {total}"
    );

    // REPEAT: V∈[1,2] is one full period of the 4×4 texture. Three of its four
    // rows are green, so the green channel must dominate the frame; the red last
    // row appears once per period, and at the V seam its bilinear footprint
    // blends with the wrapped first (green) row — that is why the fixture
    // asserts dominance rather than pure texel colours.
    assert_eq!(
        repeated_green + repeated_red,
        total,
        "the fixture has only two colours, so every pixel is one of them"
    );
    assert!(
        repeated_green > total * 2 / 3,
        "REPEAT must reach the texture's green rows, not the clamped last row: \
         green-dominant={repeated_green} red-dominant={repeated_red}"
    );

    // ClampToEdge: every V ≥ 1 clamps to the last row, so the whole frame is the
    // texture's last (red) row stretched along U. This is the reported defect.
    assert_eq!(
        clamped_red, total,
        "ClampToEdge must stretch the last row over the whole quad (this is the \
         defect the REPEAT sampler fixes): green-dominant={clamped_green} \
         red-dominant={clamped_red}"
    );
    assert_eq!(clamped_green, 0);
}
