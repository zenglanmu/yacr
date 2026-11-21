//! Native GPU contracts for mesh-texture **mip filtering**.
//!
//! The reported defect: every mesh material texture was uploaded with
//! `mip_level_count: 1` and sampled with a `Nearest`-mip sampler, so a 2048²
//! albedo on a ~750 px mesh aliased into directional striations (the CLI report
//! showed `mesh_textures.bytes == 83 886 080` = 5 × 2048² × 4, i.e. level 0 only).
//!
//! These contracts are the regression fence, in `gpu_or_skip` discipline:
//!
//! 1. aperiodic noise sampled at 64× minification lands on its footprint average
//!    **with** the CPU-generated chain, and measurably does not with
//!    `mip_level_count: 1` (both runs use the renderer's production sampler);
//! 2. the upload report counts every level, so `mesh_textures.bytes` grows by
//!    ~1/3 instead of staying at level 0.
//!
//! A software adapter's result is filtering/geometry evidence only — never a
//! performance or vendor claim.
//!
//! Run serially (these bind a real device):
//!
//! ```text
//! cargo test -p cad-render-wgpu --lib --locked -- --test-threads=1 mesh_mip_gpu_tests
//! ```
use super::mesh_pbr_gpu_tests::{
    gpu_or_skip, init, pbr_quad, scalar, srgb_byte, unit_uvs, BASE_KEY,
};
use crate::headless::HeadlessGpu;
use crate::renderer_impl::{generate_mip_levels, mip_level_count, mipped_rgba8_bytes, MipRole};
use cad_resources::{DecodedImage, DecodedImageCache, ResourceKey, ResourceLimits};
use std::sync::Arc;

// --- mip filtering: a minified mesh texture must not alias -------------------
//
// The reported defect: every mesh material texture was uploaded with
// `mip_level_count: 1` and no mip filtering, so a 2048² albedo on a ~750 px mesh
// aliased into directional striations instead of showing its screws, decals and
// hazard stripes. These contracts sample a **minified** texture with the
// renderer's production sampler and require the mip path to land on the
// footprint average where the one-level texture cannot.

/// Texture side of the minification fixture.
const MIP_TEXTURE_SIZE: u32 = 512;
/// Drawn target side: 8 px, i.e. an exact 64× minification (LOD 6, where both
/// neighbouring levels average whole 32×32 / 64×64 texel blocks).
const MIP_TARGET_SIZE: u32 = 8;

/// A black/white checkerboard of `cell`-texel squares, `side` texels square.
fn checkerboard(side: u32, cell: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let value = if ((x / cell) + (y / cell)).is_multiple_of(2) {
                255u8
            } else {
                0u8
            };
            rgba.extend_from_slice(&[value, value, value, 255]);
        }
    }
    rgba
}

/// Deterministic 50 % black/white noise, plus its exact linear mean.
///
/// Aperiodic on purpose. Periodic fixtures (checkerboards, stripes) are averaged
/// **exactly** by a driver's anisotropic scan — a few evenly spaced taps cover a
/// whole period — so they cannot show what a missing mip level costs. Noise
/// cannot be averaged by 16 taps, which is exactly the residual a real albedo's
/// decals and grime leave when level 0 is sampled from far away.
fn noise_pattern(side: u32) -> (Vec<u8>, f32) {
    let mut state: u32 = 0x9E37_79B9;
    let texels = side * side;
    let mut rgba = Vec::with_capacity((texels * 4) as usize);
    let mut white = 0u64;
    for _ in 0..texels {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let value = if (state >> 16) & 1 == 0 { 255u8 } else { 0u8 };
        if value == 255 {
            white += 1;
        }
        rgba.extend_from_slice(&[value, value, value, 255]);
    }
    // White is linear 1.0 and black is 0.0, so the linear mean is the white
    // fraction.
    (rgba, white as f32 / texels as f32)
}

/// The contract's own sampling shader: nothing but `textureSample` with implicit
/// derivatives, so the LOD is the driver's screen-space footprint exactly as in
/// the mesh pipelines. No lighting, no colour maths — one variable (the mip
/// chain) separates the two runs.
const TEXTURE_SAMPLE_WGSL: &str = r#"
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
    out.uv = vec2<f32>((corner.x + 1.0) * 0.5, 1.0 - (corner.y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(source_texture, source_sampler, in.uv);
}
"#;

/// Write one tightly packed RGBA8 level, padding rows to the 256-byte copy
/// alignment (the same rule the renderer's upload path applies).
fn write_level(
    queue: &crate::wgpu::Queue,
    texture: &crate::wgpu::Texture,
    level: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) {
    let row_bytes = width as usize * 4;
    let aligned = row_bytes.div_ceil(crate::wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
        * crate::wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded;
    let data: &[u8] = if aligned == row_bytes {
        rgba
    } else {
        let mut buffer = vec![0u8; aligned * height as usize];
        for row in 0..height as usize {
            buffer[row * aligned..row * aligned + row_bytes]
                .copy_from_slice(&rgba[row * row_bytes..(row + 1) * row_bytes]);
        }
        padded = buffer;
        &padded
    };
    queue.write_texture(
        crate::wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: crate::wgpu::Origin3d::ZERO,
            aspect: crate::wgpu::TextureAspect::All,
        },
        data,
        crate::wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(aligned as u32),
            rows_per_image: Some(height),
        },
        crate::wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Sample `rgba` (a `MIP_TEXTURE_SIZE`² sRGB texture) minified into a
/// `MIP_TARGET_SIZE`² target with `sampler`.
///
/// `mip_role` is `Some(role)` for the production chain (levels generated by the
/// renderer's own builder and uploaded level by level) and `None` for the
/// one-level comparison texture — the exact configuration the defect shipped.
fn sample_minified(
    gpu: &HeadlessGpu,
    sampler: &crate::wgpu::Sampler,
    rgba: &[u8],
    mip_role: Option<MipRole>,
) -> Vec<u8> {
    use crate::wgpu;

    let levels = mip_role.map_or_else(Vec::new, |role| {
        generate_mip_levels(rgba, MIP_TEXTURE_SIZE, MIP_TEXTURE_SIZE, role)
    });
    if mip_role.is_some() {
        assert_eq!(
            1 + levels.len() as u32,
            mip_level_count(MIP_TEXTURE_SIZE, MIP_TEXTURE_SIZE),
            "the fixture must upload the full chain, not a partial one"
        );
    }
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cad-mip-contract-texture"),
        size: wgpu::Extent3d {
            width: MIP_TEXTURE_SIZE,
            height: MIP_TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1 + levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_level(
        &gpu.queue,
        &texture,
        0,
        MIP_TEXTURE_SIZE,
        MIP_TEXTURE_SIZE,
        rgba,
    );
    for (index, level) in levels.iter().enumerate() {
        write_level(
            &gpu.queue,
            &texture,
            index as u32 + 1,
            level.width,
            level.height,
            &level.rgba,
        );
    }

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad-mip-contract-bind-layout"),
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
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cad-mip-contract-bind"),
        layout: &bind_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad-mip-contract-shader"),
            source: wgpu::ShaderSource::Wgsl(TEXTURE_SAMPLE_WGSL.into()),
        });
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-mip-contract-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
    let target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let pipeline = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad-mip-contract-pipeline"),
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
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cad-mip-contract-target"),
        size: wgpu::Extent3d {
            width: MIP_TARGET_SIZE,
            height: MIP_TARGET_SIZE,
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
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cad-mip-contract-encoder"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cad-mip-contract-pass"),
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
    let row_bytes = MIP_TARGET_SIZE * 4;
    let bytes_per_row =
        row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cad-mip-contract-readback"),
        size: u64::from(bytes_per_row) * u64::from(MIP_TARGET_SIZE),
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
                rows_per_image: Some(MIP_TARGET_SIZE),
            },
        },
        wgpu::Extent3d {
            width: MIP_TARGET_SIZE,
            height: MIP_TARGET_SIZE,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_result| {});
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll the mip contract readback");
    let view = slice
        .get_mapped_range()
        .expect("map the mip contract readback");
    let mut pixels = Vec::with_capacity((row_bytes * MIP_TARGET_SIZE) as usize);
    for row in 0..MIP_TARGET_SIZE as usize {
        let start = row * bytes_per_row as usize;
        pixels.extend_from_slice(&view[start..start + row_bytes as usize]);
    }
    pixels
}

/// Mean absolute deviation of the RGB channels from `expected`.
fn mean_deviation(pixels: &[u8], expected: u8) -> f32 {
    let mut total = 0.0f64;
    let mut samples = 0.0f64;
    for pixel in pixels.as_chunks::<4>().0 {
        for channel in &pixel[..3] {
            total += (f64::from(*channel) - f64::from(expected)).abs();
            samples += 1.0;
        }
    }
    (total / samples) as f32
}

/// The regression fence for the reported defect: noise sampled at a 64×
/// minification must land on its footprint average **with** the uploaded mip
/// chain, and must not without it.
///
/// Both runs use the renderer's production sampler
/// (`Renderer::mesh_sampler`) and differ in exactly one thing: the texture's
/// `mip_level_count`. Noise is the honest fixture here — a periodic pattern is
/// averaged exactly by a driver's anisotropic taps, so it cannot show what a
/// missing mip level costs.
#[test]
fn a_minified_mesh_texture_is_filtered_by_its_mip_chain() {
    let Some(gpu) = gpu_or_skip("mip chain minification") else {
        return;
    };
    let renderer = init(&gpu);
    let sampler = renderer
        .mesh_sampler()
        .expect("an initialized renderer owns the mesh sampler");
    let (pattern, linear_mean) = noise_pattern(MIP_TEXTURE_SIZE);
    // The footprint average of black/white noise is its linear mean, encoded the
    // way an `Rgba8UnormSrgb` target stores it.
    let expected = srgb_byte(linear_mean) as u8;

    let mipped = sample_minified(&gpu, sampler, &pattern, Some(MipRole::SrgbColour));
    let one_level = sample_minified(&gpu, sampler, &pattern, None);

    let mipped_deviation = mean_deviation(&mipped, expected);
    let one_level_deviation = mean_deviation(&one_level, expected);
    eprintln!(
        "[mesh-pbr-gpu] mip chain: {factor:.0}x minification, expected grey \
         {expected}, mip mean deviation {mipped_deviation:.1}, one-level mean \
         deviation {one_level_deviation:.1}",
        factor = f64::from(MIP_TEXTURE_SIZE / MIP_TARGET_SIZE),
    );

    assert!(
        mipped_deviation < 6.0,
        "the mip path must sample the footprint average (grey {expected}), \
         got mean deviation {mipped_deviation:.1}"
    );
    assert!(
        one_level_deviation > 10.0,
        "the one-level texture must alias at this minification (this is the \
         defect the mip chain fixes), got mean deviation \
         {one_level_deviation:.1}"
    );
    assert!(
        one_level_deviation > mipped_deviation * 3.0,
        "the mip chain must remove measurably more aliasing than level 0 alone: \
         mip {mipped_deviation:.1} vs one-level {one_level_deviation:.1}"
    );
}

#[test]
fn the_mesh_texture_upload_reports_every_mip_level() {
    let Some(gpu) = gpu_or_skip("mesh texture mip accounting") else {
        return;
    };
    let mut renderer = init(&gpu);
    let mut batch = pbr_quad([0.0, 0.0, 1.0], unit_uvs(), scalar(0.0, 0.5, [0.0; 3]));
    batch.texture = Some(ResourceKey(BASE_KEY.to_owned()));
    renderer
        .prepare_upload_batches(std::slice::from_ref(&batch))
        .and_then(|staged| renderer.commit_upload(staged, 0))
        .expect("commit the PBR scene");

    let mut cache = DecodedImageCache::new();
    let side = 256u32;
    cache
        .insert(
            ResourceKey(BASE_KEY.to_owned()),
            DecodedImage {
                width: side,
                height: side,
                rgba: Arc::from(checkerboard(side, 4).as_slice()),
            },
            &ResourceLimits::default(),
        )
        .expect("the fixture texture fits the resource budget");
    let report = renderer
        .upload_mesh_textures(&[ResourceKey(BASE_KEY.to_owned())], &cache)
        .expect("upload the mesh texture");

    let level_zero = u64::from(side) * u64::from(side) * 4;
    let chain = mipped_rgba8_bytes(side, side);
    eprintln!(
        "[mesh-pbr-gpu] mip accounting: report.bytes={}, level 0={level_zero}, chain={chain}",
        report.bytes
    );
    assert!(
        report.bytes >= chain,
        "the report must count every level, not just level 0: {} vs chain {chain}",
        report.bytes
    );
    assert!(
        report.bytes > level_zero + level_zero / 4,
        "a mip-chained upload must be ~1/3 larger than level 0 alone: \
         {} vs {level_zero}",
        report.bytes
    );
}
