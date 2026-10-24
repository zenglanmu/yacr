//! Host-run contracts for the GPU memory **budget policy**: the percentage
//! arithmetic, the resolution precedence and the environment parsing.
//!
//! Split from `budget.rs` to keep that module focused on the policy types
//! (and under the repository's per-file line limit). Like
//! `gpu_memory/tests.rs`, nothing here needs a GPU: every case is plain data.

use super::budget::*;

#[test]
fn the_default_budget_is_named_and_estimated() {
    // Named constant, not an inline literal at a call site.
    assert_eq!(DEFAULT_GPU_BUDGET_BYTES, 3 * 1024 * 1024 * 1024);
    let budget = GpuBudget::estimated_default();
    assert_eq!(budget.cap_bytes, Some(DEFAULT_GPU_BUDGET_BYTES));
    assert_eq!(budget.refuse_at, 1.0);
    assert_eq!(GpuBudget::default(), budget);
    assert_eq!(budget.effective_limit_bytes(), Some(3 * 1024 * 1024 * 1024));
}

#[test]
fn the_default_budget_admits_the_measured_fixture() {
    // The default must not refuse the repository's own authorized fixture. This
    // pins a **measured** total (`fixtures/complex-test.dwg` on the reference
    // P620: 2,107,964,952 bytes) against the policy, so shrinking the default
    // below a real drawing fails here instead of at a user's upload.
    let budget = GpuBudget::estimated_default();
    assert_eq!(MEASURED_FIXTURE_GPU_BYTES, 2_107_964_952);
    assert!(
        !budget.would_exceed(MEASURED_FIXTURE_GPU_BYTES, 0),
        "the default cap {} must admit the measured fixture total {}",
        DEFAULT_GPU_BUDGET_BYTES,
        MEASURED_FIXTURE_GPU_BYTES
    );
    assert!(
        !budget.would_exceed(MEASURED_FIXTURE_GPU_BYTES, 144),
        "a staged buffer on top of the resident fixture must still be admitted"
    );
    // ... with real room above it, not a one-byte margin: the measurement is a
    // data point, not the ceiling.
    const {
        assert!(
            DEFAULT_GPU_BUDGET_BYTES - MEASURED_FIXTURE_GPU_BYTES >= 900 * MIB,
            "the default must leave measurable headroom above the fixture"
        )
    };
    // It also stays under the reference card's 4 GB, so the renderer's own
    // refusal keeps tripping before the driver has to.
    const { assert!(DEFAULT_GPU_BUDGET_BYTES < 4 * 1024 * 1024 * 1024) };
    // The previous 512 MiB default is documented as a measured false refusal.
    assert!(GpuBudget::estimated_default()
        .with_cap(Some(512 * MIB))
        .would_exceed(1_573_159_200, 144));
}

#[test]
fn an_uncapped_budget_never_refuses() {
    let budget = GpuBudget::uncapped();
    assert_eq!(budget.effective_limit_bytes(), None);
    assert!(!budget.would_exceed(u64::MAX - 1, u64::MAX));
    assert!(!budget.would_exceed(0, u64::MAX));
}

#[test]
fn the_limit_is_inclusive_at_the_cap() {
    let budget = GpuBudget::estimated_default();
    let cap = DEFAULT_GPU_BUDGET_BYTES;
    // Exactly at the cap is admitted; one byte over is refused.
    assert!(!budget.would_exceed(cap, 0));
    assert!(!budget.would_exceed(cap - 1, 1));
    assert!(budget.would_exceed(cap, 1));
    assert!(budget.would_exceed(cap - 1, 2));
}

#[test]
fn refuse_at_scales_the_threshold_down_and_never_up() {
    let budget = GpuBudget::estimated_default().with_refuse_at(0.75);
    assert_eq!(
        budget.effective_limit_bytes(),
        Some(DEFAULT_GPU_BUDGET_BYTES * 3 / 4)
    );
    // A fraction above 1 would promise memory the cap does not cover: clamp.
    assert_eq!(
        budget.with_refuse_at(4.0).effective_limit_bytes(),
        Some(DEFAULT_GPU_BUDGET_BYTES)
    );
    // Zero, negative and non-finite mean "refuse at the cap", never a zero cap.
    for nonsense in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            budget.with_refuse_at(nonsense).effective_limit_bytes(),
            Some(DEFAULT_GPU_BUDGET_BYTES),
            "refuse_at {nonsense} must clamp to the cap"
        );
    }
}

#[test]
fn saturating_arithmetic_never_wraps_a_huge_request() {
    let budget = GpuBudget::estimated_default();
    assert!(budget.would_exceed(u64::MAX, u64::MAX));
    assert!(budget.would_exceed(DEFAULT_GPU_BUDGET_BYTES, u64::MAX));
}

#[test]
fn the_environment_override_parses_mib_and_the_explicit_off() {
    assert_eq!(GpuBudget::resolve(Some("2048")).cap_bytes, Some(2048 * MIB));
    assert_eq!(GpuBudget::resolve(Some(" 64 ")).cap_bytes, Some(64 * MIB));
    // `0` is the documented explicit disable, not a 1 MiB cap.
    let disabled = GpuBudget::resolve(Some("0"));
    assert_eq!(disabled.cap_bytes, None);
    assert_eq!(disabled.effective_limit_bytes(), None);
    // Unset keeps the default estimate.
    assert_eq!(
        GpuBudget::resolve(None).cap_bytes,
        Some(DEFAULT_GPU_BUDGET_BYTES)
    );
}

#[test]
fn a_malformed_override_falls_back_to_the_default_instead_of_uncapping() {
    for malformed in ["", "  ", "lots", "-1", "1.5", "1e9", "0x10"] {
        let budget = GpuBudget::resolve(Some(malformed));
        assert_eq!(
            budget.cap_bytes,
            Some(DEFAULT_GPU_BUDGET_BYTES),
            "{malformed:?} must not change the cap"
        );
    }
    // An overflowing value is refused loudly instead of wrapping to a tiny cap.
    assert_eq!(
        GpuBudget::resolve(Some("18446744073709551615")).cap_bytes,
        Some(DEFAULT_GPU_BUDGET_BYTES)
    );
}

#[test]
fn the_backend_thresholds_are_the_documented_percentages() {
    // The two numbers the task fixes: 80 % resource creation, 95 % device loss.
    assert_eq!(RESOURCE_CREATION_BUDGET_PERCENT, 80);
    assert_eq!(DEVICE_LOSS_BUDGET_PERCENT, 95);
    let thresholds = memory_budget_thresholds();
    assert_eq!(thresholds.for_resource_creation, Some(80));
    assert_eq!(thresholds.for_device_loss, Some(95));
    // Creation must trip strictly before device loss, otherwise the device is
    // lost before the renderer's explicit refusal path can run.
    const { assert!(RESOURCE_CREATION_BUDGET_PERCENT < DEVICE_LOSS_BUDGET_PERCENT) };
}

#[test]
fn applying_the_thresholds_sets_exactly_the_documented_value_on_d3d12() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    assert_eq!(
        descriptor.memory_budget_thresholds.for_resource_creation, None,
        "the wgpu default must be overridden, not relied on"
    );
    apply_memory_budget_thresholds(&mut descriptor);
    // `MemoryBudgetThresholds` is not `PartialEq`, so compare field by field
    // against the documented numbers (not against a second call to the same
    // function, which would only prove self-consistency).
    assert_eq!(
        descriptor.memory_budget_thresholds.for_resource_creation,
        Some(80)
    );
    assert_eq!(
        descriptor.memory_budget_thresholds.for_device_loss,
        Some(95)
    );
}

#[test]
fn a_vulkan_instance_keeps_the_wgpu_default_because_the_threshold_is_measured_unsafe() {
    // Documented deviation, not an oversight: see
    // `threshold_backends_supported` for the measurement that made an 80 %
    // creation threshold kill a second device in the same process on NVIDIA.
    for backends in [
        wgpu::Backends::VULKAN,
        wgpu::Backends::VULKAN | wgpu::Backends::GL,
        wgpu::Backends::VULKAN | wgpu::Backends::DX12,
        wgpu::Backends::GL,
    ] {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        apply_memory_budget_thresholds(&mut descriptor);
        assert_eq!(
            descriptor.memory_budget_thresholds.for_resource_creation, None,
            "{backends:?} must keep the wgpu default"
        );
        assert_eq!(descriptor.memory_budget_thresholds.for_device_loss, None);
    }
}

#[test]
fn the_default_config_is_ninety_percent_of_probed_vram() {
    let config = GpuBudgetConfig::default();
    assert_eq!(config.percent_of_vram, DEFAULT_GPU_BUDGET_PERCENT);
    assert_eq!(DEFAULT_GPU_BUDGET_PERCENT, 90);
    assert_eq!(config.explicit_bytes, None);
    assert_eq!(config.refuse_at, 1.0);
    let resolved = config.resolve(Some(8 * 1024 * MIB));
    assert_eq!(resolved.source, BudgetSource::VramPercent);
    assert_eq!(resolved.percent, Some(90));
    assert_eq!(
        resolved.budget.cap_bytes,
        Some(8 * 1024 * MIB * 9 / 10),
        "90 % of the advertised device-local total"
    );
}

#[test]
fn the_percentage_arithmetic_is_floored_for_every_supported_step() {
    // 10 GiB is divisible by 10, so the round steps are exact.
    let total = 10 * 1024 * MIB;
    for (percent, expected) in [
        (10u8, total / 10),
        (50, total / 2),
        (90, total * 9 / 10),
        (100, total),
    ] {
        let config = GpuBudgetConfig {
            percent_of_vram: percent,
            ..GpuBudgetConfig::default()
        };
        let resolved = config.resolve(Some(total));
        assert_eq!(resolved.source, BudgetSource::VramPercent);
        assert_eq!(resolved.percent, Some(percent));
        assert_eq!(resolved.budget.cap_bytes, Some(expected), "{percent} %");
    }
    // Non-round totals floor: the cap never claims more than the share.
    assert_eq!(percent_of(999, 90), 899);
    assert_eq!(percent_of(999, 10), 99);
    assert_eq!(percent_of(1, 90), 0);
    assert_eq!(percent_of(u64::MAX, 100), u64::MAX);
}

#[test]
fn an_out_of_range_percentage_is_clamped_at_resolution() {
    let total = 1_000u64;
    for (configured, effective) in [
        (0u8, 10u8),
        (5, 10),
        (9, 10),
        (10, 10),
        (90, 90),
        (100, 100),
        (101, 100),
        (255, 100),
    ] {
        let config = GpuBudgetConfig {
            percent_of_vram: configured,
            ..GpuBudgetConfig::default()
        };
        assert_eq!(config.effective_percent(), effective, "{configured}");
        let resolved = config.resolve(Some(total));
        assert_eq!(resolved.percent, Some(effective), "{configured}");
        assert_eq!(
            resolved.budget.cap_bytes,
            Some(u64::from(effective) * total / 100)
        );
    }
}

#[test]
fn the_cap_resolution_precedence_is_explicit_then_percent_then_estimate() {
    let vram = 16 * 1024 * MIB;

    // 1. Explicit bytes beat the probe.
    let explicit = GpuBudgetConfig {
        explicit_bytes: Some(2 * 1024 * MIB),
        ..GpuBudgetConfig::default()
    };
    let resolved = explicit.resolve(Some(vram));
    assert_eq!(resolved.budget.cap_bytes, Some(2 * 1024 * MIB));
    assert_eq!(resolved.source, BudgetSource::ExplicitBytes);
    assert_eq!(resolved.percent, None);

    // 1b. An explicit `0` is "uncapped" and still beats the probe.
    let uncapped = GpuBudgetConfig {
        explicit_bytes: Some(0),
        ..GpuBudgetConfig::default()
    };
    let resolved = uncapped.resolve(Some(vram));
    assert_eq!(resolved.budget.cap_bytes, None);
    assert_eq!(resolved.source, BudgetSource::ExplicitBytes);

    // 2. No explicit cap: the percentage of the probed total.
    let resolved = GpuBudgetConfig::default().resolve(Some(vram));
    assert_eq!(resolved.budget.cap_bytes, Some(vram * 9 / 10));
    assert_eq!(resolved.source, BudgetSource::VramPercent);
    assert_eq!(resolved.percent, Some(DEFAULT_GPU_BUDGET_PERCENT));

    // 3. No explicit cap and no usable probe: the documented estimate.
    for unknown in [None, Some(0)] {
        let resolved = GpuBudgetConfig::default().resolve(unknown);
        assert_eq!(resolved.budget.cap_bytes, Some(DEFAULT_GPU_BUDGET_BYTES));
        assert_eq!(resolved.source, BudgetSource::Estimate);
        assert_eq!(resolved.percent, None);
    }

    // `refuse_at` travels through every branch untouched.
    let resolved = GpuBudgetConfig {
        refuse_at: 0.5,
        ..GpuBudgetConfig::default()
    }
    .resolve(Some(vram));
    assert_eq!(resolved.budget.refuse_at, 0.5);
    // 90 % of the probed total, then half of that (the refusal threshold).
    assert_eq!(resolved.budget.effective_limit_bytes(), Some(vram * 9 / 20));
}

#[test]
fn the_environment_percentage_is_validated_and_the_default_kept() {
    assert_eq!(
        GpuBudgetConfig::resolve_env(None, Some("50")).percent_of_vram,
        50
    );
    assert_eq!(
        GpuBudgetConfig::resolve_env(None, Some("10")).percent_of_vram,
        MIN_GPU_BUDGET_PERCENT
    );
    assert_eq!(
        GpuBudgetConfig::resolve_env(None, Some(" 100 ")).percent_of_vram,
        MAX_GPU_BUDGET_PERCENT
    );
    // Unset keeps the default.
    assert_eq!(
        GpuBudgetConfig::resolve_env(None, None).percent_of_vram,
        DEFAULT_GPU_BUDGET_PERCENT
    );
    // `0`, below the minimum, above 100 and non-numeric values are rejected
    // with a stderr note and the default kept — never a silent 0 % cap.
    for bad in ["0", "9", "101", "255", "abc", "", "  ", "-1", "1.5", "1e2"] {
        assert_eq!(
            GpuBudgetConfig::resolve_env(None, Some(bad)).percent_of_vram,
            DEFAULT_GPU_BUDGET_PERCENT,
            "{bad:?} must not change the percentage"
        );
    }
    // The rest of the config is untouched by a bad percentage.
    let config = GpuBudgetConfig::resolve_env(None, Some("0"));
    assert_eq!(config.explicit_bytes, None);
    assert_eq!(config.refuse_at, 1.0);
}

#[test]
fn the_mb_override_maps_to_explicit_bytes_and_beats_the_percentage() {
    let config = GpuBudgetConfig::resolve_env(Some("2048"), Some("50"));
    assert_eq!(config.explicit_bytes, Some(2048 * MIB));
    assert_eq!(config.percent_of_vram, 50);
    let resolved = config.resolve(Some(8 * 1024 * MIB));
    assert_eq!(resolved.budget.cap_bytes, Some(2048 * MIB));
    assert_eq!(resolved.source, BudgetSource::ExplicitBytes);

    // `0` MiB is the explicit uncapped sentinel (same rule as `GpuBudget`).
    let uncapped = GpuBudgetConfig::resolve_env(Some("0"), None);
    assert_eq!(uncapped.explicit_bytes, Some(0));
    assert_eq!(
        uncapped.resolve(Some(8 * 1024 * MIB)).budget.cap_bytes,
        None
    );

    // A malformed MB value falls back to the percentage path, not to uncapped.
    for bad in ["", "lots", "-1", "1.5", "18446744073709551615"] {
        let config = GpuBudgetConfig::resolve_env(Some(bad), Some("75"));
        assert_eq!(config.explicit_bytes, None, "{bad:?}");
        assert_eq!(config.percent_of_vram, 75, "{bad:?}");
        assert_eq!(
            config.resolve(Some(1_000)).source,
            BudgetSource::VramPercent,
            "{bad:?}"
        );
    }
}

#[test]
fn the_legacy_gpu_budget_keeps_working_and_is_labelled_explicit() {
    // `GpuBudget` is the low-level shape hosts and the CLI already use: it
    // still resolves from the MB env, and `ResolvedBudget::explicit` preserves
    // its cap verbatim, labelled as an explicit choice.
    let budget = GpuBudget::resolve(Some("1024"));
    let resolved = ResolvedBudget::explicit(budget);
    assert_eq!(resolved.budget, budget);
    assert_eq!(resolved.source, BudgetSource::ExplicitBytes);
    assert_eq!(resolved.percent, None);
    assert_eq!(budget.cap_bytes, Some(1024 * MIB));
    // The uncapped shape survives too.
    let resolved = ResolvedBudget::explicit(GpuBudget::uncapped());
    assert_eq!(resolved.budget.cap_bytes, None);
    assert_eq!(resolved.source, BudgetSource::ExplicitBytes);
}
