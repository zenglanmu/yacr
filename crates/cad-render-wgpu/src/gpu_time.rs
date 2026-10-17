//! Measurement-only GPU frame timing (ADR 0013 class 1).
//!
//! This module carries the **explicit opt-in** for bracketing a frame's GPU work
//! with timestamp queries, the pure policy that decides which features to request
//! and what state the request yields, and the pure tick→millisecond conversion.
//!
//! Policy (ADR 0013 class 1, "measurement-only"):
//!
//! * It is requested **only** behind an explicit, documented opt-in. The host
//!   reads the environment and turns it into a [`GpuTimeConfig`]; nothing in this
//!   crate reads the environment itself.
//! * It must never become a runtime dependency, must not change rendering or
//!   pixels, and must not enter product defaults. When the opt-in is off the
//!   device requests **no** feature and `FrameStats::gpu_ms` stays `None` — an
//!   estimate is never substituted.
//! * If the opt-in is set but the adapter lacks the needed feature, the state is
//!   reported as [`GpuTimeState::Unsupported`] **naming the missing feature**;
//!   there is no silent fallback that pretends measurement worked.
//!
//! Honesty (ADR 0013 / iron rule 6): a timestamp readback **perturbs** the frame
//! (an extra `resolve_query_set` + buffer copy on the GPU and a map/unmap on the
//! CPU). The measured device is therefore **not** the product configuration, and
//! no vendor/golden/performance claim follows from it.

use crate::wgpu;

/// The minimum feature set a frame-bracketing timestamp query needs.
///
/// `TIMESTAMP_QUERY` enables the query type; `TIMESTAMP_QUERY_INSIDE_ENCODERS`
/// enables the `CommandEncoder::write_timestamp` calls that bracket the frame's
/// single command encoder. The writes happen on the encoder (not inside a render
/// pass), so `TIMESTAMP_QUERY_INSIDE_PASSES` is deliberately **not** requested.
pub const GPU_TIME_FEATURES: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// Explicit opt-in configuration for GPU frame-time measurement.
///
/// Defaults to disabled: a caller that does not opt in gets the product device
/// (no features requested, `gpu_ms = None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuTimeConfig {
    /// Whether frame-time measurement was explicitly requested.
    pub enabled: bool,
}

impl GpuTimeConfig {
    /// The product default: no measurement, no feature request.
    pub const fn disabled() -> Self {
        Self { enabled: false }
    }

    /// The measurement opt-in.
    pub const fn enabled() -> Self {
        Self { enabled: true }
    }
}

/// The measurement state token carried on every frame and probe line.
///
/// This is the honest, explicit verdict — never a success by default:
///
/// * [`GpuTimeState::Disabled`] — the opt-in was off (the product default), or
///   the platform cannot measure (`wasm32`).
/// * [`GpuTimeState::Enabled`] — the opt-in was on and the device granted the
///   feature set, so `gpu_ms` is a real measurement.
/// * [`GpuTimeState::Unsupported`] — the opt-in was on but the adapter lacks the
///   named feature; `gpu_ms` stays `None` and the missing feature is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GpuTimeState {
    /// Measurement was not requested (or is unavailable on this platform).
    #[default]
    Disabled,
    /// Measurement is active; `gpu_ms` is a real measurement.
    Enabled,
    /// The opt-in was set but this adapter lacks the named feature.
    Unsupported(&'static str),
}

impl GpuTimeState {
    /// The stable, whitespace-free token appended to probe logs.
    ///
    /// Shape: `enabled`, `disabled` or `unsupported(<FEATURE_NAME>)`.
    pub fn as_token(&self) -> String {
        match self {
            GpuTimeState::Disabled => "disabled".to_string(),
            GpuTimeState::Enabled => "enabled".to_string(),
            GpuTimeState::Unsupported(feature) => format!("unsupported({feature})"),
        }
    }
}

/// The features a device-creation request asked for, what it was granted, and the
/// resulting measurement state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuTimeReport {
    /// Features requested at device creation for measurement (`empty` when off).
    pub requested: wgpu::Features,
    /// Features the device actually granted (a subset of `requested`).
    pub granted: wgpu::Features,
    /// The explicit measurement state.
    pub state: GpuTimeState,
}

/// Pure policy: which features to request for `config`, given the adapter's
/// supported features, and the resulting state.
///
/// Disabled ⇒ request nothing and report [`GpuTimeState::Disabled`]. Enabled ⇒
/// request [`GPU_TIME_FEATURES`] intersected with the adapter's support; a
/// missing feature yields [`GpuTimeState::Unsupported`] naming it (the first
/// missing one), never a silent downgrade.
pub fn negotiate_gpu_time(adapter: wgpu::Features, config: GpuTimeConfig) -> GpuTimeReport {
    if !config.enabled {
        return GpuTimeReport {
            requested: wgpu::Features::empty(),
            granted: wgpu::Features::empty(),
            state: GpuTimeState::Disabled,
        };
    }
    let missing = GPU_TIME_FEATURES.difference(adapter);
    if !missing.is_empty() {
        // Name the first missing feature so the state is actionable.
        let feature = if missing.contains(wgpu::Features::TIMESTAMP_QUERY) {
            "TIMESTAMP_QUERY"
        } else {
            "TIMESTAMP_QUERY_INSIDE_ENCODERS"
        };
        return GpuTimeReport {
            requested: GPU_TIME_FEATURES,
            granted: wgpu::Features::empty(),
            state: GpuTimeState::Unsupported(feature),
        };
    }
    GpuTimeReport {
        requested: GPU_TIME_FEATURES,
        granted: GPU_TIME_FEATURES,
        state: GpuTimeState::Enabled,
    }
}

/// Convert raw timestamp ticks into milliseconds.
///
/// `period_ns` is `Queue::get_timestamp_period()` (nanoseconds per tick). A
/// zero period (timestamps unsupported) yields `0.0`; ticks are monotonic so the
/// caller passes an already-differenced, non-negative tick count.
pub fn ticks_to_ms(ticks: u64, period_ns: f32) -> f64 {
    ticks as f64 * f64::from(period_ns) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_requests_no_features() {
        // Fence 1: with the flag off the request set is empty and the state is
        // disabled — no feature is requested on that path.
        let report = negotiate_gpu_time(wgpu::Features::all(), GpuTimeConfig::disabled());
        assert_eq!(report.requested, wgpu::Features::empty());
        assert_eq!(report.granted, wgpu::Features::empty());
        assert_eq!(report.state, GpuTimeState::Disabled);
        assert_eq!(report.state.as_token(), "disabled");
    }

    #[test]
    fn disabled_stays_disabled_even_without_adapter_support() {
        let report = negotiate_gpu_time(wgpu::Features::empty(), GpuTimeConfig::disabled());
        assert_eq!(report.state, GpuTimeState::Disabled);
    }

    #[test]
    fn enabled_with_full_support_grants_the_feature_set() {
        let report = negotiate_gpu_time(GPU_TIME_FEATURES, GpuTimeConfig::enabled());
        assert_eq!(report.requested, GPU_TIME_FEATURES);
        assert_eq!(report.granted, GPU_TIME_FEATURES);
        assert_eq!(report.state, GpuTimeState::Enabled);
        assert_eq!(report.state.as_token(), "enabled");
    }

    #[test]
    fn enabled_with_superset_adapter_grants_only_the_needed_set() {
        let report = negotiate_gpu_time(wgpu::Features::all(), GpuTimeConfig::enabled());
        assert_eq!(report.granted, GPU_TIME_FEATURES);
        assert_eq!(report.state, GpuTimeState::Enabled);
    }

    #[test]
    fn enabled_without_support_names_the_missing_feature() {
        // Fence 3 (pure half): the opt-in is on but the adapter lacks the
        // feature — the state names it instead of silently disabling.
        let report = negotiate_gpu_time(wgpu::Features::empty(), GpuTimeConfig::enabled());
        assert_eq!(report.granted, wgpu::Features::empty());
        assert_eq!(
            report.state,
            GpuTimeState::Unsupported("TIMESTAMP_QUERY"),
            "the first missing feature is named"
        );
        assert_eq!(report.state.as_token(), "unsupported(TIMESTAMP_QUERY)");

        // Only the encoder feature missing: it is the one named.
        let partial = wgpu::Features::TIMESTAMP_QUERY;
        let report = negotiate_gpu_time(partial, GpuTimeConfig::enabled());
        assert_eq!(
            report.state,
            GpuTimeState::Unsupported("TIMESTAMP_QUERY_INSIDE_ENCODERS")
        );
    }

    #[test]
    fn ticks_to_ms_converts_with_the_period() {
        // 1 ns/tick is the WebGPU-on-web convention; 1e6 ticks = 1 ms.
        assert_eq!(ticks_to_ms(1_000_000, 1.0), 1.0);
        // A 1.0 tick period: 500 ticks = 500 ns = 0.0005 ms.
        assert_eq!(ticks_to_ms(500, 1.0), 0.0005);
        // NVIDIA-class periods are ~1 ns; a 0.5 ns period halves the result.
        assert_eq!(ticks_to_ms(2_000_000, 0.5), 1.0);
    }

    #[test]
    fn ticks_to_ms_zero_period_is_zero() {
        assert_eq!(ticks_to_ms(123_456, 0.0), 0.0);
    }

    #[test]
    fn ticks_to_ms_is_monotonic_in_ticks() {
        // Ordering: more ticks never yields less time for a fixed positive period.
        let a = ticks_to_ms(1_000, 1.0);
        let b = ticks_to_ms(2_000, 1.0);
        let c = ticks_to_ms(9_999_999, 1.0);
        assert!(a < b && b < c);
        assert_eq!(ticks_to_ms(0, 1.0), 0.0);
    }
}
