use super::*;

/// Collect the validation error scope with a bounded, non-blocking wait.
///
/// Polls the device in small steps up to `budget`; if the scope has not resolved
/// by then it logs and returns `None`, so a slow device poll cannot hold the
/// frame. Validation errors that do resolve in time are still reported.
pub(super) fn collect_error_scope_bounded(
    device: &wgpu::Device,
    guard: wgpu::ErrorScopeGuard,
    budget: std::time::Duration,
) -> Option<wgpu::Error> {
    let mut pop = std::pin::pin!(guard.pop());
    let deadline = std::time::Instant::now() + budget;
    loop {
        // `poll_once` returns `None` while the inner future is still pending, so
        // this never blocks on the scope.
        if let Some(error) = block_on(futures_lite::future::poll_once(pop.as_mut())) {
            return error;
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("[probe] error scope did not resolve within {budget:?}; skipping collection");
            return None;
        }
        // Give the device a bounded chance to run its callbacks.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_millis(2)),
        });
    }
}

/// Create the Vulkan-only probe instance (shared by the safe and unsafe paths).
pub(super) fn make_probe_instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    wgpu::Instance::new(descriptor)
}

/// Whether a cached native-texture binding may be reused: the key must match
/// **and** the texture identity must be the same. The key alone is not unique
/// across engine sessions.
pub(crate) fn binding_cache_is_reusable(
    cached_key: &NativeTextureKey,
    key: &NativeTextureKey,
    same_texture: bool,
) -> bool {
    cached_key == key && same_texture
}

/// Validate a native texture against its key (format/usage/size).
pub(crate) fn validate_texture(
    texture: &wgpu::Texture,
    key: &NativeTextureKey,
) -> Result<(), ProbeError> {
    let format = texture.format();
    if !texture_format_is_supported(format) {
        return Err(ProbeError::TextureInvalid(format!(
            "unsupported format {format:?}; expected 8-bit RGBA/BGRA (sRGB or linear)"
        )));
    }
    if !texture
        .usage()
        .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Err(ProbeError::TextureInvalid(
            "usage must include TEXTURE_BINDING".into(),
        ));
    }
    let size = texture.size();
    if (size.width, size.height) != key.size {
        return Err(ProbeError::TextureInvalid(format!(
            "texture is {}x{} but key says {}x{}",
            size.width, size.height, key.size.0, key.size.1
        )));
    }
    Ok(())
}

/// Build the fullscreen textured pipeline, its bind-group layout and sampler.
pub(super) fn build_texture_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cad-texture-shader"),
        source: wgpu::ShaderSource::Wgsl(TEXTURE_SHADER.into()),
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

/// Build the probe shader, render pipeline, uniform buffer and bind group.
pub(super) fn build_probe_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroup, wgpu::Buffer) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cad-surface-probe-shader"),
        source: wgpu::ShaderSource::Wgsl(PROBE_SHADER.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cad-surface-probe-uniform-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cad-surface-probe-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cad-surface-probe-pipeline"),
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
        // The probe is a flat 2D overlay; it needs no depth attachment.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cad-surface-probe-uniform"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cad-surface-probe-bind-group"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    (pipeline, bind_group, uniform)
}

/// Describe `adapter` and whether it can present to `surface`.
pub(super) fn probe_info(adapter: &wgpu::Adapter, surface: &wgpu::Surface<'_>) -> ProbeAdapterInfo {
    let info = adapter.get_info();
    let capabilities = surface.get_capabilities(adapter);
    ProbeAdapterInfo {
        backend: info.backend.to_str().to_string(),
        name: info.name,
        device_type: device_type_str(info.device_type).to_string(),
        driver: info.driver,
        driver_info: info.driver_info,
        surface_compatible: !capabilities.formats.is_empty(),
    }
}

pub(super) fn device_type_str(device_type: wgpu::DeviceType) -> &'static str {
    match device_type {
        wgpu::DeviceType::Cpu => "cpu",
        wgpu::DeviceType::IntegratedGpu => "integrated_gpu",
        wgpu::DeviceType::DiscreteGpu => "discrete_gpu",
        wgpu::DeviceType::VirtualGpu => "virtual_gpu",
        wgpu::DeviceType::Other => "other",
    }
}

/// Rank adapters the probe is willing to use, most preferred first.
///
/// Eligibility is hard: the backend must be Vulkan, the device type must not be
/// `cpu`, and the adapter must be able to present to the surface. Ordering is
/// soft: a name matching `preferred_names` (case-insensitive substring, in list
/// order) wins, then a discrete GPU over an integrated one, then enumeration
/// order. The result is deterministic and stable.
pub(crate) fn rank_probe_adapters<S: AsRef<str>>(
    infos: &[ProbeAdapterInfo],
    preferred_names: &[S],
) -> Vec<usize> {
    let mut eligible: Vec<usize> = infos
        .iter()
        .enumerate()
        .filter(|(_, info)| {
            info.backend == REQUIRED_BACKEND && info.device_type != "cpu" && info.surface_compatible
        })
        .map(|(index, _)| index)
        .collect();
    eligible.sort_by_key(|&index| {
        let info = &infos[index];
        (
            preferred_rank(&info.name, preferred_names),
            device_rank(&info.device_type),
            index,
        )
    });
    eligible
}

/// Position of the first preferred name contained in `name`, or `usize::MAX`.
pub(super) fn preferred_rank<S: AsRef<str>>(name: &str, preferred_names: &[S]) -> usize {
    let lowered = name.to_ascii_lowercase();
    preferred_names
        .iter()
        .position(|candidate| {
            let needle = candidate.as_ref().to_ascii_lowercase();
            !needle.is_empty() && lowered.contains(&needle)
        })
        .unwrap_or(usize::MAX)
}

/// Sort weight inside a backend: discrete GPUs before integrated, others last.
pub(super) fn device_rank(device_type: &str) -> u8 {
    match device_type {
        "discrete_gpu" => 0,
        "integrated_gpu" => 1,
        _ => 2,
    }
}

/// Deterministic surface-format policy.
///
/// Prefer an 8-bit sRGB format (RGBA then BGRA); otherwise fall back to a
/// non-sRGB 8-bit format; otherwise take whatever the surface listed first.
/// `None` means the surface reported no usable format.
pub(crate) fn choose_surface_format(
    formats: &[wgpu::TextureFormat],
) -> Option<wgpu::TextureFormat> {
    const PREFERRED: [wgpu::TextureFormat; 4] = [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
    ];
    PREFERRED
        .iter()
        .copied()
        .find(|format| formats.contains(format))
        .or_else(|| formats.first().copied())
}

/// Validate a requested surface size against the adapter's texture limit.
pub(crate) fn validate_size(width: u32, height: u32, max_dimension: u32) -> Result<(), ProbeError> {
    if width == 0 || height == 0 {
        return Err(ProbeError::InvalidSize { width, height });
    }
    if width > max_dimension || height > max_dimension {
        return Err(ProbeError::SizeExceedsLimit {
            width,
            height,
            limit: max_dimension,
        });
    }
    Ok(())
}

/// Validate and wrap a marker offset into `[0, 1)`.
pub(crate) fn sanitize_marker(offset: f32) -> Result<f32, ProbeError> {
    if !offset.is_finite() {
        return Err(ProbeError::InvalidMarker(offset));
    }
    Ok(offset.rem_euclid(MARKER_PERIOD))
}
