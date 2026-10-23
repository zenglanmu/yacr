use super::*;

fn info(
    backend: &str,
    device_type: &str,
    name: &str,
    surface_compatible: bool,
) -> ProbeAdapterInfo {
    ProbeAdapterInfo {
        backend: backend.into(),
        name: name.into(),
        device_type: device_type.into(),
        driver: "test-driver".into(),
        driver_info: String::new(),
        surface_compatible,
    }
}

#[test]
fn rank_rejects_cpu_non_vulkan_and_incompatible() {
    let infos = vec![
        info("vulkan", "cpu", "P620", true),
        info("gl", "discrete_gpu", "P620", true),
        info("vulkan", "discrete_gpu", "P620", false),
    ];
    assert!(
        rank_probe_adapters(&infos, &["P620"], ProbeBackendPolicy::Vulkan).is_empty(),
        "under the vulkan policy, cpu, non-Vulkan and surface-incompatible adapters must all be rejected"
    );
}

#[test]
fn rank_prefers_named_adapter_before_other_discrete() {
    let infos = vec![
        info("vulkan", "discrete_gpu", "Quadro RTX 5000", true),
        info("vulkan", "integrated_gpu", "Intel UHD Graphics", true),
        info("vulkan", "discrete_gpu", "Quadro P620", true),
    ];
    assert_eq!(
        rank_probe_adapters(&infos, &["P620"], ProbeBackendPolicy::Vulkan),
        vec![2, 0, 1],
        "the preferred name wins, then discrete before integrated"
    );
}

#[test]
fn rank_preferred_match_is_case_insensitive_substring() {
    let infos = vec![
        info("vulkan", "discrete_gpu", "NVIDIA Quadro p620", true),
        info("vulkan", "discrete_gpu", "Other Discrete", true),
    ];
    assert_eq!(
        rank_probe_adapters(&infos, &["P620"], ProbeBackendPolicy::Auto),
        vec![0, 1]
    );
}

#[test]
fn rank_falls_back_when_no_preferred_name_matches() {
    let infos = vec![
        info("vulkan", "integrated_gpu", "Intel UHD Graphics", true),
        info("vulkan", "discrete_gpu", "Quadro RTX", true),
    ];
    assert_eq!(
        rank_probe_adapters(&infos, &["P620"], ProbeBackendPolicy::Auto),
        vec![1, 0],
        "no P620 present: a discrete GPU is still preferred over an integrated one"
    );
}

#[test]
fn rank_falls_back_to_integrated_when_only_integrated() {
    let infos = vec![info("vulkan", "integrated_gpu", "Intel UHD Graphics", true)];
    assert_eq!(
        rank_probe_adapters(&infos, &["P620"], ProbeBackendPolicy::Auto),
        vec![0],
        "a lone integrated Vulkan GPU is still a valid hardware target"
    );
}

#[test]
fn rank_is_stable_for_equal_keys() {
    let infos = vec![
        info("vulkan", "discrete_gpu", "A", true),
        info("vulkan", "discrete_gpu", "B", true),
    ];
    let none: [&str; 0] = [];
    assert_eq!(
        rank_probe_adapters(&infos, &none, ProbeBackendPolicy::Vulkan),
        vec![0, 1]
    );
}

/// `vulkan` policy: an otherwise perfect DX12 adapter is ineligible, so a
/// Vulkan-only machine keeps exactly the historical adapter set.
#[test]
fn vulkan_policy_rejects_dx12_backend_adapters() {
    let infos = vec![
        info("dx12", "discrete_gpu", "NVIDIA GeForce RTX 5000", true),
        info("vulkan", "integrated_gpu", "Intel UHD Graphics", true),
    ];
    assert_eq!(
        rank_probe_adapters(&infos, &["RTX"], ProbeBackendPolicy::Vulkan),
        vec![1],
        "the DX12 adapter stays ineligible even when its name is preferred"
    );
}

/// `dx12` policy: a Vulkan adapter is ineligible, so the same machine selects the
/// DX12 adapter instead of silently presenting through Vulkan.
#[test]
fn dx12_policy_rejects_vulkan_backend_adapters() {
    let infos = vec![
        info("vulkan", "discrete_gpu", "Quadro P620", true),
        info("dx12", "integrated_gpu", "Intel UHD Graphics", true),
    ];
    assert_eq!(
        rank_probe_adapters(&infos, &["P620"], ProbeBackendPolicy::Dx12),
        vec![1],
        "the Vulkan adapter stays ineligible even when its name is preferred"
    );
}

/// `auto` policy: Vulkan is preferred over DX12 when both adapters are otherwise
/// equally ranked (same name, same device type), and the preference is stable in
/// either enumeration order.
#[test]
fn auto_policy_prefers_vulkan_within_a_tier() {
    let none: [&str; 0] = [];
    let infos = vec![
        info("dx12", "discrete_gpu", "Same GPU", true),
        info("vulkan", "discrete_gpu", "Same GPU", true),
    ];
    assert_eq!(
        rank_probe_adapters(&infos, &none, ProbeBackendPolicy::Auto),
        vec![1, 0],
        "Vulkan outranks DX12 inside the same name/device tier"
    );
}

/// Parsing is exact: only the three documented wire values are meaningful, and
/// everything else — absent, empty, wrong case, an unknown token — is `auto`.
#[test]
fn backend_policy_parsing_is_exact() {
    assert_eq!(
        ProbeBackendPolicy::parse(None),
        ProbeBackendPolicy::Auto,
        "an absent preference is auto"
    );
    for raw in [
        "", "auto", "Auto", "DX12", "Vulkan", "gl", "vulkan ", " dx12",
    ] {
        assert_eq!(
            ProbeBackendPolicy::parse(Some(raw)),
            ProbeBackendPolicy::Auto,
            "{raw:?} is not a wire value and must resolve to auto"
        );
    }
    assert_eq!(
        ProbeBackendPolicy::parse(Some("vulkan")),
        ProbeBackendPolicy::Vulkan
    );
    assert_eq!(
        ProbeBackendPolicy::parse(Some("dx12")),
        ProbeBackendPolicy::Dx12
    );
}

/// The wire token and the wgpu backend set are pinned per policy: the log line,
/// the instance descriptor and the enumerator all read them from here.
#[test]
fn backend_policy_backends_and_tokens_are_pinned() {
    assert_eq!(ProbeBackendPolicy::Auto.as_token(), "auto");
    assert_eq!(ProbeBackendPolicy::Vulkan.as_token(), "vulkan");
    assert_eq!(ProbeBackendPolicy::Dx12.as_token(), "dx12");
    assert_eq!(
        ProbeBackendPolicy::Auto.backends(),
        wgpu::Backends::VULKAN | wgpu::Backends::DX12
    );
    assert_eq!(
        ProbeBackendPolicy::Vulkan.backends(),
        wgpu::Backends::VULKAN
    );
    assert_eq!(ProbeBackendPolicy::Dx12.backends(), wgpu::Backends::DX12);
    assert_eq!(ProbeBackendPolicy::default(), ProbeBackendPolicy::Auto);
    assert!(ProbeBackendPolicy::Auto.allows_backend("vulkan"));
    assert!(ProbeBackendPolicy::Auto.allows_backend("dx12"));
    assert!(!ProbeBackendPolicy::Vulkan.allows_backend("dx12"));
    assert!(!ProbeBackendPolicy::Dx12.allows_backend("vulkan"));
}

/// DX12 is a Windows-only target capability: elsewhere the probe refuses the
/// policy explicitly (exit path is `ProbeError::UnsupportedBackendPolicy`), and
/// on Windows it is accepted. No fallback backend is ever tried.
#[test]
fn dx12_policy_support_is_explicit_per_target() {
    if cfg!(target_os = "windows") {
        assert_eq!(ProbeBackendPolicy::Dx12.ensure_supported(), Ok(()));
    } else {
        assert_eq!(
            ProbeBackendPolicy::Dx12.ensure_supported(),
            Err(ProbeError::UnsupportedBackendPolicy { policy: "dx12" })
        );
        assert_eq!(ProbeBackendPolicy::Auto.ensure_supported(), Ok(()));
        assert_eq!(ProbeBackendPolicy::Vulkan.ensure_supported(), Ok(()));
    }
}

/// Probe-only override: a CPU/software adapter is eligible, but hardware still
/// ranks strictly above it — even though the CPU adapter carries the preferred
/// name.
#[test]
fn rank_allow_cpu_ranks_hardware_before_cpu_adapter() {
    let infos = vec![
        info("vulkan", "cpu", "P620", true),
        info("vulkan", "discrete_gpu", "Other Discrete", true),
    ];
    assert_eq!(
        rank_probe_adapters_allow_cpu(&infos, &["P620"], ProbeBackendPolicy::Auto),
        vec![1, 0],
        "the hardware adapter wins, then the name-preferred CPU adapter"
    );
}

/// A software-Vulkan host (lavapipe in a VM) has no hardware adapter at all: the
/// override must then rank the presentable CPU adapter rather than fail.
#[test]
fn rank_allow_cpu_accepts_lone_presentable_cpu_adapter() {
    let infos = vec![info(
        "vulkan",
        "cpu",
        "llvmpipe (LLVM 19.1.7, 256 bits)",
        true,
    )];
    assert_eq!(
        rank_probe_adapters_allow_cpu(&infos, &["P620"], ProbeBackendPolicy::Auto),
        vec![0]
    );
}

/// The override widens eligibility for `cpu` only: non-Vulkan and
/// surface-incompatible adapters stay ineligible, and the backend policy still
/// applies.
#[test]
fn rank_allow_cpu_keeps_non_vulkan_and_incompatible_ineligible() {
    let infos = vec![
        info("gl", "cpu", "llvmpipe", true),
        info("vulkan", "cpu", "llvmpipe", false),
        info("dx12", "cpu", "Microsoft Basic Render Driver", true),
    ];
    assert_eq!(
        rank_probe_adapters_allow_cpu(&infos, &["P620"], ProbeBackendPolicy::Vulkan),
        Vec::<usize>::new(),
        "only Vulkan + surface-compatible adapters become eligible"
    );
    assert_eq!(
        rank_probe_adapters_allow_cpu(&infos, &["P620"], ProbeBackendPolicy::Dx12),
        vec![2],
        "the DX12 WARP adapter is eligible under the dx12 policy"
    );
}

/// `YACR_PROBE_ALLOW_CPU_ADAPTER` is presence-based (like
/// `YACR_PROBE_NO_MAXIMIZE`): any value — including an empty one — enables it,
/// and an absent variable disables it.
#[test]
fn allow_cpu_override_is_presence_based() {
    assert!(!env_flag_present(None), "absent must keep CPU rejected");
    assert!(
        env_flag_present(Some(std::ffi::OsString::new())),
        "an empty value still counts as present"
    );
    assert!(
        env_flag_present(Some(std::ffi::OsString::from("0"))),
        "the value is irrelevant; only presence counts"
    );
}

#[test]
fn choose_format_prefers_srgb_when_available() {
    let formats = [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ];
    assert_eq!(
        choose_surface_format(&formats),
        Some(wgpu::TextureFormat::Rgba8UnormSrgb)
    );
}

#[test]
fn choose_format_prefers_bgra_srgb_over_non_srgb() {
    let formats = [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ];
    assert_eq!(
        choose_surface_format(&formats),
        Some(wgpu::TextureFormat::Bgra8UnormSrgb)
    );
}

#[test]
fn choose_format_falls_back_to_first_supported() {
    let formats = [
        wgpu::TextureFormat::Rgb10a2Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
    ];
    // Bgra8Unorm is in the sRGB/8-bit preference list, so it wins here.
    assert_eq!(
        choose_surface_format(&formats),
        Some(wgpu::TextureFormat::Bgra8Unorm)
    );
    let unknown = [wgpu::TextureFormat::Rgb10a2Unorm];
    assert_eq!(
        choose_surface_format(&unknown),
        Some(wgpu::TextureFormat::Rgb10a2Unorm),
        "with no known format, the surface's first listed format is used"
    );
}

#[test]
fn choose_format_none_when_empty() {
    assert_eq!(choose_surface_format(&[]), None);
}

/// The unsafe raw-handle constructor's signature is the next lane's contract.
/// Pinning it here (by reference, never called) makes an accidental change a
/// compile error in this crate rather than a silent break for the caller.
///
/// Deliberately updated when the backend policy was threaded through: the
/// trailing `ProbeBackendPolicy` parameter is the only signature change.
#[test]
#[allow(clippy::type_complexity)] // the whole point is pinning this exact signature
fn raw_handle_constructor_signature_is_stable() {
    let _constructor: unsafe fn(
        wgpu::rwh::RawDisplayHandle,
        wgpu::rwh::RawWindowHandle,
        u32,
        u32,
        ProbePresentMode,
        crate::gpu_time::GpuTimeConfig,
        ProbeBackendPolicy,
    ) -> Result<SurfaceProbePresenter, ProbeError> = SurfaceProbePresenter::new_from_raw_handles;
}

#[test]
fn binding_cache_requires_key_and_texture_identity() {
    let a = NativeTextureKey {
        device_epoch: 1,
        texture_revision: 1,
        size: (64, 64),
    };
    let b = NativeTextureKey {
        device_epoch: 1,
        texture_revision: 1,
        size: (64, 64),
    };
    // Same key + same texture -> reusable.
    assert!(binding_cache_is_reusable(&a, &b, true));
    // Same key + different texture identity -> NOT reusable (the collision
    // the key alone cannot detect).
    assert!(!binding_cache_is_reusable(&a, &b, false));
    let c = NativeTextureKey {
        device_epoch: 2,
        texture_revision: 1,
        size: (64, 64),
    };
    assert!(!binding_cache_is_reusable(&a, &c, true));
}

#[test]
fn texture_key_validation() {
    assert!(validate_texture_key(
        &NativeTextureKey {
            device_epoch: 0,
            texture_revision: 0,
            size: (0, 10)
        },
        8192
    )
    .is_err());
    assert!(validate_texture_key(
        &NativeTextureKey {
            device_epoch: 0,
            texture_revision: 0,
            size: (9000, 10)
        },
        8192
    )
    .is_err());
    assert!(validate_texture_key(
        &NativeTextureKey {
            device_epoch: 1,
            texture_revision: 2,
            size: (960, 439)
        },
        8192
    )
    .is_ok());
}

#[test]
fn texture_format_support_is_explicit() {
    for format in [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
    ] {
        assert!(
            texture_format_is_supported(format),
            "{format:?} must be supported"
        );
    }
    assert!(!texture_format_is_supported(
        wgpu::TextureFormat::Rgba16Float
    ));
    assert!(!texture_format_is_supported(
        wgpu::TextureFormat::Depth32Float
    ));
}

#[test]
fn texture_shader_is_valid_wgsl() {
    // The shared blit shader (native presenter + wasm canvas presenter).
    let module =
        naga::front::wgsl::parse_str(crate::blit::TEXTURE_SHADER).expect("texture shader parses");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    validator
        .validate(&module)
        .expect("texture shader validates");
    assert!(module
        .entry_points
        .iter()
        .any(|e| e.name == "vs_main" && e.stage == naga::ShaderStage::Vertex));
    assert!(module
        .entry_points
        .iter()
        .any(|e| e.name == "fs_main" && e.stage == naga::ShaderStage::Fragment));
}

#[test]
fn validate_size_rejects_zero_dimensions() {
    assert_eq!(
        validate_size(0, 100, 8192),
        Err(ProbeError::InvalidSize {
            width: 0,
            height: 100
        })
    );
    assert_eq!(
        validate_size(100, 0, 8192),
        Err(ProbeError::InvalidSize {
            width: 100,
            height: 0
        })
    );
}

#[test]
fn validate_size_rejects_over_limit() {
    assert_eq!(
        validate_size(9000, 100, 8192),
        Err(ProbeError::SizeExceedsLimit {
            width: 9000,
            height: 100,
            limit: 8192
        })
    );
    assert_eq!(
        validate_size(100, 8193, 8192),
        Err(ProbeError::SizeExceedsLimit {
            width: 100,
            height: 8193,
            limit: 8192
        })
    );
}

#[test]
fn validate_size_accepts_boundary_and_unit() {
    assert_eq!(validate_size(1, 1, 8192), Ok(()));
    assert_eq!(
        validate_size(8192, 8192, 8192),
        Ok(()),
        "the adapter limit itself is allowed"
    );
}

#[test]
fn sanitize_marker_rejects_non_finite() {
    assert!(matches!(
        sanitize_marker(f32::NAN),
        Err(ProbeError::InvalidMarker(value)) if value.is_nan()
    ));
    assert!(matches!(
        sanitize_marker(f32::INFINITY),
        Err(ProbeError::InvalidMarker(_))
    ));
    assert!(matches!(
        sanitize_marker(f32::NEG_INFINITY),
        Err(ProbeError::InvalidMarker(_))
    ));
}

#[test]
fn sanitize_marker_wraps_into_unit_range() {
    assert_eq!(sanitize_marker(0.25), Ok(0.25));
    assert_eq!(sanitize_marker(1.25), Ok(0.25));
    assert_eq!(sanitize_marker(-0.25), Ok(0.75));
    assert_eq!(sanitize_marker(0.0), Ok(0.0));
}

/// P5 gate F2 (transient): a native-texture/surface size disagreement that
/// clears within a couple of ticks is a wait, not an error — so a mid-resize
/// frame (or a DOM-derived size rounded one pixel off the integer allocation)
/// never surfaces a user-visible "present failed".
#[test]
fn a_transient_size_disagreement_waits_instead_of_erroring() {
    let mut wait = SizeMismatchWait::default();
    assert!(
        wait.record_mismatch(),
        "the first disagreeing tick must wait (not ready yet)"
    );
    assert_eq!(wait.consecutive(), 1);
    assert!(
        wait.record_mismatch(),
        "the second disagreeing tick must still wait"
    );
    assert_eq!(wait.consecutive(), 2);

    // The resize settles: the next present matches, so the wait is over and a
    // later transient starts its own bounded budget.
    wait.record_match();
    assert_eq!(wait.consecutive(), 0);
    assert!(
        wait.record_mismatch(),
        "a fresh transient after a matching present must wait again"
    );
}

/// P5 gate F2 (persistent): a disagreement that does not clear stops being a
/// wait; the guard surfaces the explicit error it exists for, so a genuine
/// drawing-area/surface mismatch still fails loudly.
#[test]
fn a_persistent_size_disagreement_becomes_an_explicit_error() {
    let mut wait = SizeMismatchWait::default();
    let mut waited = 0;
    let mut errored = false;
    for _ in 0..SIZE_MISMATCH_WAIT_TICKS {
        if wait.record_mismatch() {
            waited += 1;
        } else {
            errored = true;
        }
    }
    assert_eq!(
        waited,
        SIZE_MISMATCH_WAIT_TICKS - 1,
        "the wait is bounded (it does not swallow a persistent mismatch)"
    );
    assert!(
        errored,
        "the persistent mismatch is an explicit error on the {}th tick",
        SIZE_MISMATCH_WAIT_TICKS
    );
    assert!(
        !wait.record_mismatch(),
        "it stays an error while it persists"
    );
    assert_eq!(wait.consecutive(), SIZE_MISMATCH_WAIT_TICKS + 1);
}

/// The composition path is not cosmetic: it is the only surface kind that
/// composites a `WS_CHILD` under a transparent top-level window on Windows, and
/// it must request the composite-alpha mode that
/// `CreateSwapChainForComposition` is specified for. `Auto` maps to
/// `DXGI_ALPHA_MODE_UNSPECIFIED`, which is not a valid composition alpha mode, so
/// pinning this pairing keeps a future refactor from silently regressing it.
#[test]
fn surface_kind_selects_its_composite_alpha_mode_and_token() {
    assert_eq!(
        alpha_mode_for(ProbeSurfaceKind::Hwnd),
        wgpu::CompositeAlphaMode::Auto,
        "an HWND chain keeps wgpu's automatic (opaque) choice"
    );
    assert_eq!(
        alpha_mode_for(ProbeSurfaceKind::Composition),
        wgpu::CompositeAlphaMode::PreMultiplied,
        "a CreateSwapChainForComposition chain must not be UNSPECIFIED"
    );
    assert_ne!(
        alpha_mode_for(ProbeSurfaceKind::Hwnd),
        alpha_mode_for(ProbeSurfaceKind::Composition)
    );
    // The ready line's `surface=` token is the evidence vocabulary.
    assert_eq!(ProbeSurfaceKind::Hwnd.as_token(), "hwnd");
    assert_eq!(ProbeSurfaceKind::Composition.as_token(), "composition");
}
