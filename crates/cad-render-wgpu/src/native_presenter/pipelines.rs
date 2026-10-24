use super::*;

/// Collect the validation error scope with a bounded, non-blocking wait.
///
/// Polls the device in small steps up to `budget`; if the scope has not resolved
/// by then it logs and returns `None`, so a slow device poll cannot hold the
/// frame. Validation errors that do resolve in time are still reported.
/// The composite-alpha mode a given surface kind must request.
///
/// - `Hwnd`: `Auto` (wgpu picks the only mode an HWND chain advertises: opaque).
/// - `Composition`: **`PreMultiplied`**, which is what a
///   `CreateSwapChainForComposition` chain is specified for. `Auto` maps to
///   `DXGI_ALPHA_MODE_UNSPECIFIED`, which is not a valid composition alpha mode.
///   The CAD content is opaque (`alpha = 1`), so premultiplied and straight
///   compositing are byte-identical here — the choice only makes the contract
///   explicit instead of relying on a default.
pub(super) fn alpha_mode_for(kind: ProbeSurfaceKind) -> wgpu::CompositeAlphaMode {
    match kind {
        ProbeSurfaceKind::Hwnd => wgpu::CompositeAlphaMode::Auto,
        ProbeSurfaceKind::Composition => wgpu::CompositeAlphaMode::PreMultiplied,
    }
}

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

/// Create the probe instance for `backend_policy` (shared by the safe and unsafe
/// paths).
///
/// The descriptor's backend set is exactly the policy's: `vulkan` keeps the
/// historical Vulkan-only instance, `dx12` asks for DX12 only, and `auto` enables
/// both. Backends not compiled for the target are absent from enumeration rather
/// than substituted.
pub(super) fn make_probe_instance(backend_policy: ProbeBackendPolicy) -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backend_policy.backends();
    // The backend's own memory thresholds (80 % creation / 95 % device loss), the
    // same values the headless and renderer instances use; see
    // `gpu_memory::memory_budget_thresholds`.
    crate::gpu_memory::apply_memory_budget_thresholds(&mut descriptor);
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
///
/// Delegates to the shared blit module ([`crate::blit`]) so the native probe and
/// the wasm canvas presenter build the *same* pipeline. Native presentation
/// semantics are unchanged: this module still owns the per-frame binding cache
/// (keyed by [`NativeTextureKey`]) and the texture/surface size fence.
pub(super) fn build_texture_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    crate::blit::build_texture_pipeline(device, format)
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

/// Presence-based, probe-only override that makes CPU/software Vulkan adapters
/// eligible for adapter selection.
///
/// Exactly like `YACR_PROBE_NO_MAXIMIZE`, only **presence** matters: the value is
/// irrelevant (`=1` and `=` are equivalent), and an absent variable keeps the
/// contract default (CPU adapters rejected). This is a host-bring-up/evidence
/// knob for software-Vulkan environments (for example lavapipe in a VM), never a
/// production relaxation.
pub(crate) const ALLOW_CPU_ADAPTER_ENV: &str = "YACR_PROBE_ALLOW_CPU_ADAPTER";

/// Whether the [`ALLOW_CPU_ADAPTER_ENV`] override is present in the environment.
pub(crate) fn allow_cpu_adapter_override() -> bool {
    env_flag_present(std::env::var_os(ALLOW_CPU_ADAPTER_ENV))
}

/// Pure presence test behind [`allow_cpu_adapter_override`].
///
/// Split out so the documented semantics (presence, not truthiness) are testable
/// without mutating the process environment.
pub(super) fn env_flag_present(value: Option<std::ffi::OsString>) -> bool {
    value.is_some()
}

/// Rank adapters the probe is willing to use, most preferred first.
///
/// Eligibility is hard: the adapter's backend must be allowed by
/// `backend_policy`, the device type must not be `cpu` (unless the probe-only
/// [`ALLOW_CPU_ADAPTER_ENV`] override is active), and the adapter must be able to
/// present to the surface. Ordering is soft: a name matching `preferred_names`
/// (case-insensitive substring, in list order) wins, then Vulkan before DX12,
/// then a discrete GPU over an integrated one, then enumeration order. The result
/// is deterministic and stable.
///
/// This is the contract default (CPU/software adapters rejected). The probe-only
/// [`rank_probe_adapters_allow_cpu`] is the opt-in variant behind
/// [`ALLOW_CPU_ADAPTER_ENV`].
pub(crate) fn rank_probe_adapters<S: AsRef<str>>(
    infos: &[ProbeAdapterInfo],
    preferred_names: &[S],
    backend_policy: ProbeBackendPolicy,
) -> Vec<usize> {
    rank_probe_adapters_policy(infos, preferred_names, backend_policy, false)
}

/// [`rank_probe_adapters`] with CPU/software adapters **also** eligible.
///
/// Host-bring-up/evidence only (software Vulkan such as lavapipe). Ranking keeps
/// hardware strictly above CPU/software adapters: a CPU adapter never outranks a
/// hardware adapter even when its name matches `preferred_names`. Inside a
/// device-type tier the existing preference applies unchanged (preferred name
/// first, then Vulkan before DX12, then discrete over integrated, then
/// enumeration order). The backend policy applies exactly as in the strict path.
pub(crate) fn rank_probe_adapters_allow_cpu<S: AsRef<str>>(
    infos: &[ProbeAdapterInfo],
    preferred_names: &[S],
    backend_policy: ProbeBackendPolicy,
) -> Vec<usize> {
    rank_probe_adapters_policy(infos, preferred_names, backend_policy, true)
}

/// Shared ranking core. `allow_cpu` only widens eligibility; the sort key is the
/// same shape for every backend policy, so a Vulkan-only policy (where the
/// backend rank is constant) keeps the historical ordering bit-for-bit.
fn rank_probe_adapters_policy<S: AsRef<str>>(
    infos: &[ProbeAdapterInfo],
    preferred_names: &[S],
    backend_policy: ProbeBackendPolicy,
    allow_cpu: bool,
) -> Vec<usize> {
    let mut eligible: Vec<usize> = infos
        .iter()
        .enumerate()
        .filter(|(_, info)| {
            backend_policy.allows_backend(&info.backend)
                && (allow_cpu || info.device_type != "cpu")
                && info.surface_compatible
        })
        .map(|(index, _)| index)
        .collect();
    eligible.sort_by_key(|&index| {
        let info = &infos[index];
        (
            // Hardware always outranks CPU/software, even a name-preferred one.
            // In the strict path CPU adapters are filtered out before sorting, so
            // this key is constant and the ordering is unchanged.
            u8::from(info.device_type == "cpu"),
            preferred_rank(&info.name, preferred_names),
            backend_rank(&info.backend),
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

/// Sort weight inside a backend policy: Vulkan before DX12, others last.
pub(super) fn backend_rank(backend: &str) -> u8 {
    match backend {
        "vulkan" => 0,
        "dx12" => 1,
        _ => 2,
    }
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
