//! The application GPU memory budget: named constants, the environment override
//! and the pure verdict arithmetic.
//!
//! Everything here is plain arithmetic over `u64`/`f32`, so the refusal boundary
//! and the environment parsing are unit-tested on the host (`cargo test -p
//! cad-render-wgpu`), exactly like the wasm heap budget
//! (`apps/app-web/src/budget.rs`) is.
//!
//! # The cap is a percentage of the card's advertised memory — and absent when
//! # VRAM cannot be probed
//!
//! wgpu 30.0.1 exposes **no** portable VRAM field: `AdapterInfo` has no memory
//! entry, `Instance::generate_report()` reports no budget, and
//! [`Limits`](wgpu::Limits) only carries per-resource ceilings (`max_buffer_size`,
//! `max_texture_dimension_2d`). The closest number a desktop backend will give is
//! its **device-local physical memory heaps**, which this crate reads through the
//! Vulkan HAL escape hatch ([`super::vram`]) — Vulkan only: GL/Metal/DX12/the web
//! backend expose no such query and therefore run **unbudgeted**. The cap uses the
//! **largest single device-local heap** as the VRAM proxy — **not** the sum of the
//! heaps, which a host-visible device-local heap inflated past the card's physical
//! capacity on the reference machine (see the validation record). Both are
//! *advertised device-local memory*, **not free memory** (usage is not subtracted,
//! other clients are unknown, and on an integrated GPU it is shared system RAM).
//!
//! [`GpuBudgetConfig`] is therefore the policy, resolved in this order:
//!
//! 1. [`GpuBudgetConfig::explicit_bytes`] — a host/CLI choice
//!    (`--gpu-budget-mb` / `YACR_GPU_BUDGET_MB`; `0` = explicitly uncapped);
//! 2. `percent_of_vram` (default [`DEFAULT_GPU_BUDGET_PERCENT`] = 90 %) of the
//!    probed proxy (`YACR_GPU_BUDGET_PERCENT`);
//! 3. **no cap at all** ([`BudgetSource::Unprobed`]) when neither an explicit cap
//!    nor a VRAM probe exists. There is deliberately **no guessed fallback**: a cap
//!    invented without evidence can refuse legitimate drawings. This repository
//!    already measured that failure with the old 512 MiB default, which refused the
//!    authorized `fixtures/complex-test.dwg` while the renderer legitimately held
//!    1.57 GB (`gpu-memory-refused: … used=1573159200 requested=144
//!    cap=536870912`). Running unbudgeted is honest; a false refusal is not.
//!
//! Every level is overridable per machine with the flags/env above or
//! [`Renderer::set_gpu_budget_config`](crate::Renderer::set_gpu_budget_config) /
//! [`Renderer::set_gpu_budget`](crate::Renderer::set_gpu_budget). [`MEASURED_FIXTURE_GPU_BYTES`]
//! stays a *measurement* used by a test that a probed 4 GiB card admits the fixture —
//! never a default.
//!
//! The effective cap is still an **application** policy: 90 % of advertised
//! device-local memory leaves headroom for the compositor, the host's own
//! resources on the shared `Device`, allocator fragmentation and the driver's own
//! reservations. It is never a claim that the renderer may fill the card.

/// The environment variable that overrides [`GpuBudget::cap_bytes`], in MiB.
///
/// Read once, when the renderer is constructed ([`GpuBudgetConfig::from_env`]).
/// `0` on its own **explicitly disables** the cap (reported as `cap_bytes: None`,
/// never silently treated as a one-megabyte cap); an unparseable value is reported
/// on stderr and ignored in favour of the percentage path (or no cap at all when
/// VRAM cannot be probed), so a bad setting never looks applied.
pub const GPU_BUDGET_ENV: &str = "YACR_GPU_BUDGET_MB";

/// The environment variable that sets [`GpuBudgetConfig::percent_of_vram`].
///
/// Read once, when the renderer is constructed ([`GpuBudgetConfig::from_env`]).
/// It applies only when [`GPU_BUDGET_ENV`] is unset: an explicit byte cap always
/// wins. A value outside [`MIN_GPU_BUDGET_PERCENT`]..=[`MAX_GPU_BUDGET_PERCENT`]
/// (`0`, `5`, `101`, …) or a non-numeric value is reported on stderr and ignored
/// in favour of [`DEFAULT_GPU_BUDGET_PERCENT`] — the same honesty rule as the MB
/// override: a bad setting must neither look applied nor silently change the
/// policy.
pub const GPU_BUDGET_PERCENT_ENV: &str = "YACR_GPU_BUDGET_PERCENT";

/// The default share of the probed VRAM proxy the renderer may claim: **90 %**.
///
/// The remaining tenth is deliberate headroom for the compositor, the host's own
/// resources on the shared `Device`, allocator fragmentation and the driver's own
/// reservations. It is a policy default, not a hardware fact.
pub const DEFAULT_GPU_BUDGET_PERCENT: u8 = 90;

/// The smallest accepted VRAM percentage.
///
/// Below this the cap would be so tight that ordinary CAD scenes would be refused
/// on any card; a host that genuinely wants a tiny fixed cap should pass an
/// explicit byte value instead.
pub const MIN_GPU_BUDGET_PERCENT: u8 = 10;

/// The largest accepted VRAM percentage.
///
/// 100 % is allowed on purpose (a caller may decide it owns the whole card) but is
/// never the default: it leaves no room for the compositor or the driver.
pub const MAX_GPU_BUDGET_PERCENT: u8 = 100;

/// One mebibyte, the unit [`GPU_BUDGET_ENV`] is expressed in.
pub(crate) const MIB: u64 = 1024 * 1024;

/// The authorized fixture's measured device footprint: **2,107,964,952 bytes**
/// (2.11 GB decimal, 1.96 GiB).
///
/// This is a **measurement, not a guarantee**. Taken on the project's reference
/// machine (Quadro P620, NVIDIA 580.178.04, Vulkan) with
/// `cad-cli-tools render fixtures/complex-test.dwg --gpu high`, the run that also
/// cross-validated the accounting against the driver:
///
/// ```text
/// gpu-memory: total=2107964952 buffer=2100592152 texture=7372800 peak=2107964952
///             budget=4294967296 cap=4294967296 source=accounted
///             allocator_total=2109386656 reserved=2617245696
/// gpu-memory-detail: vertex=303645612 index=80201976 uniform=4325456
///             instance=16159896 stream=1695124632 mask=446436
///             color-target=3686400 depth-target=3686400 image-texture=0
///             mask-texture=0 other=688144
/// ```
///
/// Another drawing, another driver revision or another scene shape can need more,
/// so this is a **measurement, not a promise** and not a budget default. It is kept
/// so a *probed* budget can be checked against a real file (see
/// `a_probed_four_gib_card_admits_the_measured_fixture`), not so callers compare
/// against it at runtime.
pub const MEASURED_FIXTURE_GPU_BYTES: u64 = 2_107_964_952;

/// Percent of the backend's own reported memory budget at which wgpu starts
/// returning OOM from resource creation.
///
/// Applied to every **native** `wgpu::Instance` this crate creates (see
/// [`apply_memory_budget_thresholds`]). 80 % leaves a fifth of the driver's own
/// budget as headroom for the compositor, the host's own resources on the shared
/// `Device`, and allocator fragmentation, so the renderer is refused an allocation
/// *before* the driver starts sacrificing other clients.
pub const RESOURCE_CREATION_BUDGET_PERCENT: u8 = 80;

/// Percent of the backend's own reported memory budget at which wgpu will lose the
/// device to memory pressure.
///
/// 95 % — a hard stop that must never be reached in normal operation: crossing
/// [`RESOURCE_CREATION_BUDGET_PERCENT`] first makes the offending allocation fail
/// explicitly, and this second threshold exists so a runaway that bypasses
/// resource creation (driver-side growth, another client) ends in a *reported*
/// device loss instead of an unrecoverable process.
pub const DEVICE_LOSS_BUDGET_PERCENT: u8 = 95;

/// The `memory_budget_thresholds` value this crate installs on native instances.
///
/// wgpu 30.0.1 documents the fields as percentages of "the memory budget reported
/// by native APIs", supported by **D3D12 and (optionally) Vulkan**
/// (`wgpu-types-30.0.1/src/instance.rs:339-350`). Backends that do not support
/// them ignore the value, which is why the renderer also keeps its own accounting
/// and budget — the thresholds are a second line of defence, never the only one.
pub const fn memory_budget_thresholds() -> wgpu::MemoryBudgetThresholds {
    wgpu::MemoryBudgetThresholds {
        for_resource_creation: Some(RESOURCE_CREATION_BUDGET_PERCENT),
        for_device_loss: Some(DEVICE_LOSS_BUDGET_PERCENT),
    }
}

/// Whether this descriptor's backend set is one where wgpu 30.0.1 applies the
/// thresholds to **resource creation** as documented.
///
/// Only **D3D12**: wgpu's own doc says "Currently only the D3D12 and
/// (optionally) Vulkan backends support these options"
/// (`wgpu-types-30.0.1/src/instance.rs:334-350`), and the Vulkan half is
/// measured to be unsafe as a default here:
///
/// * reference machine: NVIDIA 580.178.04, Quadro P620, Vulkan with
///   `VK_EXT_memory_budget` (the extension wgpu-hal reads for these thresholds,
///   `wgpu-hal-30.0.1/src/vulkan/device.rs:848-975`);
/// * the driver's reported heap usage is **process-wide**, so the first device's
///   `~192 MiB` of reservations (measured: `total_reserved_bytes =
///   201_326_592` for a trivial device) already puts a *second* device in the
///   same process past an 80 % creation threshold;
/// * result: that second device's very first 72-byte `queue.write_buffer` fails
///   with `wgpu error: Out of Memory` — 3/3 runs, while the identical binary
///   passes with the thresholds unset. Two devices in one process is a normal
///   shape here (the crate's own headless tests, and any host with a probe
///   presenter beside its CAD device).
///
/// A threshold that turns a working session into a dead one is not a safety net,
/// so it is installed only where the backend documents support. The portable line
/// of defence is this crate's own accounting plus [`GpuBudget`] (see
/// [`super::GpuMemoryLedger`]) and the explicit
/// [`RenderError::GpuMemory`](crate::RenderError::GpuMemory) classification.
pub const fn threshold_backends_supported(descriptor: &wgpu::InstanceDescriptor) -> bool {
    matches!(descriptor.backends, wgpu::Backends::DX12)
}

/// Install [`memory_budget_thresholds`] on a native `InstanceDescriptor`, where
/// [`threshold_backends_supported`] says the backend honours them.
///
/// Call this at every native `wgpu::Instance` creation in this crate. It is
/// deliberately **not** applied to the wasm canvas presenter: WebGL2 has no such
/// API and the browser session has its own heap budget instead.
pub const fn apply_memory_budget_thresholds(descriptor: &mut wgpu::InstanceDescriptor) {
    if threshold_backends_supported(descriptor) {
        descriptor.memory_budget_thresholds = memory_budget_thresholds();
    }
}

/// Which rule produced the effective cap in a report.
///
/// This is the **policy** source, deliberately separate from
/// [`super::GpuMemorySource`] (which describes where the *measured bytes* came
/// from). A report can legitimately say `source: Accounted` (the bytes are this
/// renderer's own counters) and `budget_source: VramPercent` (the cap is 90 % of
/// what the card advertised).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BudgetSource {
    /// `percent_of_vram` × the probed VRAM proxy (the largest device-local heap).
    VramPercent,
    /// An explicit byte cap (`--gpu-budget-mb` / `YACR_GPU_BUDGET_MB` /
    /// [`GpuBudget`] / [`GpuBudgetConfig::explicit_bytes`]). `cap_bytes: None`
    /// with this source means an explicit "uncapped".
    ExplicitBytes,
    /// **No budget**: no VRAM probe answered (non-Vulkan backend, no HAL handle,
    /// no device-local heap) and no explicit byte cap was configured. The renderer
    /// then runs unbudgeted ([`GpuBudget::uncapped`]) instead of guessing a cap —
    /// a guess can falsely refuse legitimate drawings (see the module docs).
    ///
    /// This is also the [`Default`] value, because "we have no evidence" is the
    /// only honest default.
    #[default]
    Unprobed,
}

impl BudgetSource {
    /// Stable lowercase token for messages and diagnostics (never a UI string).
    pub const fn label(self) -> &'static str {
        match self {
            Self::VramPercent => "vram-percent",
            Self::ExplicitBytes => "explicit-bytes",
            Self::Unprobed => "unprobed",
        }
    }

    /// Compact encoding for the ledger's atomics.
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::VramPercent => 0,
            Self::ExplicitBytes => 1,
            Self::Unprobed => 2,
        }
    }

    /// Decode [`Self::code`]; an unknown code is treated as [`Self::Unprobed`]
    /// (the honest "no budget evidence" label) rather than panicking in a report.
    pub(crate) const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::VramPercent,
            1 => Self::ExplicitBytes,
            _ => Self::Unprobed,
        }
    }
}

/// The VRAM-relative budget policy: the share of the card the renderer may claim,
/// the explicit override, and the refusal fraction.
///
/// Plain configuration data (no wgpu types), so a host can build one and a unit
/// test can resolve it without a GPU. [`Default`] is 90 % of probed VRAM with no
/// explicit cap and `refuse_at = 1.0`; if VRAM cannot be probed it resolves to **no
/// cap** ([`BudgetSource::Unprobed`]) rather than a guessed number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuBudgetConfig {
    /// Share of the probed VRAM proxy to use as the cap, in percent.
    ///
    /// The default is [`DEFAULT_GPU_BUDGET_PERCENT`] (90 %). It applies **only**
    /// when a probe answered (Vulkan only): with no probe this setting cannot be
    /// applied and the policy is unbudgeted. Values outside
    /// [`MIN_GPU_BUDGET_PERCENT`]..=[`MAX_GPU_BUDGET_PERCENT`] are **clamped** at
    /// resolution (documented, so a programmatic `255` can never become a cap
    /// above the card); the environment variable is rejected loudly instead (see
    /// [`GPU_BUDGET_PERCENT_ENV`]).
    pub percent_of_vram: u8,
    /// An explicit cap in bytes, which always wins over the percentage and works
    /// even when VRAM cannot be probed.
    ///
    /// `None` = "no explicit cap: use the percentage (or no cap if unprobed)".
    /// `Some(0)` is the documented explicit **uncapped** sentinel (mirroring
    /// `--gpu-budget-mb 0`); any other value is used verbatim.
    pub explicit_bytes: Option<u64>,
    /// The fraction of the effective cap at which staging is refused, in `(0, 1]`.
    /// Semantics are [`GpuBudget::refuse_at`]'s; default `1.0`.
    pub refuse_at: f32,
}

impl Default for GpuBudgetConfig {
    fn default() -> Self {
        Self {
            percent_of_vram: DEFAULT_GPU_BUDGET_PERCENT,
            explicit_bytes: None,
            refuse_at: 1.0,
        }
    }
}

impl GpuBudgetConfig {
    /// Resolve the configuration from the environment:
    ///
    /// * [`GPU_BUDGET_ENV`] (`YACR_GPU_BUDGET_MB`) → [`Self::explicit_bytes`];
    /// * [`GPU_BUDGET_PERCENT_ENV`] (`YACR_GPU_BUDGET_PERCENT`) →
    ///   [`Self::percent_of_vram`].
    ///
    /// Malformed values are reported on stderr and ignored (the default kept), so
    /// an invalid override never looks applied — and never invents a cap: with no
    /// explicit bytes and no probe the renderer runs unbudgeted.
    pub fn from_env() -> Self {
        Self::resolve_env(
            std::env::var(GPU_BUDGET_ENV).ok().as_deref(),
            std::env::var(GPU_BUDGET_PERCENT_ENV).ok().as_deref(),
        )
    }

    /// [`Self::from_env`] over explicit values, so the parsing rules are
    /// unit-testable without touching the process environment.
    pub fn resolve_env(megabytes: Option<&str>, percent: Option<&str>) -> Self {
        let explicit_bytes = match megabytes {
            None => None,
            Some(raw) => match parse_budget_mib(raw) {
                // `parse_budget_mib`'s `None` is the explicit-disable sentinel.
                Ok(Some(bytes)) => Some(bytes),
                Ok(None) => Some(0),
                Err(reason) => {
                    eprintln!(
                        "yacr: ignoring {GPU_BUDGET_ENV}={raw:?}: {reason}; \
                         using the {} % VRAM policy instead (unbudgeted if VRAM cannot be probed)",
                        DEFAULT_GPU_BUDGET_PERCENT
                    );
                    None
                }
            },
        };
        let mut config = Self {
            explicit_bytes,
            ..Self::default()
        };
        if let Some(raw) = percent {
            match parse_budget_percent(raw) {
                Ok(value) => config.percent_of_vram = value,
                Err(reason) => {
                    eprintln!(
                        "yacr: ignoring {GPU_BUDGET_PERCENT_ENV}={raw:?}: {reason}; \
                         using the default {DEFAULT_GPU_BUDGET_PERCENT} % of probed device-local \
                         VRAM"
                    );
                }
            }
        }
        config
    }

    /// Resolve the effective cap against a probed VRAM proxy.
    ///
    /// Precedence (documented in the module docs): explicit bytes **>** percentage
    /// of the proxy **>** **no cap** ([`BudgetSource::Unprobed`]) when the probe is
    /// `None` or zero and no explicit cap was configured. There is no guessed
    /// fallback: an unprobed backend runs unbudgeted rather than risk a false
    /// refusal.
    ///
    /// The percentage is floored (`total × percent / 100`, integer arithmetic), so
    /// the cap never claims more than the configured share.
    pub fn resolve(&self, vram_bytes: Option<u64>) -> ResolvedBudget {
        if let Some(total) = vram_bytes.filter(|total| *total > 0) {
            if let Some(explicit) = self.explicit_bytes {
                // An explicit choice always beats the probe, including `Some(0)` =
                // the explicit "uncapped" sentinel.
                return ResolvedBudget {
                    budget: GpuBudget {
                        cap_bytes: (explicit != 0).then_some(explicit),
                        refuse_at: self.refuse_at,
                    },
                    source: BudgetSource::ExplicitBytes,
                    percent: None,
                };
            }
            let percent = self.effective_percent();
            return ResolvedBudget {
                budget: GpuBudget {
                    cap_bytes: Some(percent_of(total, percent)),
                    refuse_at: self.refuse_at,
                },
                source: BudgetSource::VramPercent,
                percent: Some(percent),
            };
        }
        if let Some(explicit) = self.explicit_bytes {
            return ResolvedBudget {
                budget: GpuBudget {
                    cap_bytes: (explicit != 0).then_some(explicit),
                    refuse_at: self.refuse_at,
                },
                source: BudgetSource::ExplicitBytes,
                percent: None,
            };
        }
        // No explicit cap and no probe: unbudgeted, labelled as such (never as a
        // percentage of a card nobody measured, and never a guessed number).
        ResolvedBudget {
            budget: GpuBudget {
                cap_bytes: None,
                refuse_at: self.refuse_at,
            },
            source: BudgetSource::Unprobed,
            percent: None,
        }
    }

    /// [`Self::percent_of_vram`] clamped to the accepted range.
    pub const fn effective_percent(&self) -> u8 {
        if self.percent_of_vram < MIN_GPU_BUDGET_PERCENT {
            MIN_GPU_BUDGET_PERCENT
        } else if self.percent_of_vram > MAX_GPU_BUDGET_PERCENT {
            MAX_GPU_BUDGET_PERCENT
        } else {
            self.percent_of_vram
        }
    }
}

/// A [`GpuBudgetConfig`] resolved against one VRAM probe.
///
/// The renderer keeps this as the ledger's policy, and the report exposes
/// `budget_source`/`budget_percent` from it, so a reader can tell a percentage
/// cap from an explicit one and from **no cap** ([`BudgetSource::Unprobed`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedBudget {
    /// The policy the ledger enforces.
    pub budget: GpuBudget,
    /// Which rule produced [`Self::budget`]'s cap.
    pub source: BudgetSource,
    /// The percentage applied, `Some` only for [`BudgetSource::VramPercent`].
    pub percent: Option<u8>,
}

impl ResolvedBudget {
    /// A policy taken verbatim from a [`GpuBudget`] (the legacy
    /// [`Renderer::set_gpu_budget`](crate::Renderer::set_gpu_budget) shape).
    pub const fn explicit(budget: GpuBudget) -> Self {
        Self {
            budget,
            source: BudgetSource::ExplicitBytes,
            percent: None,
        }
    }
}

/// `total × percent / 100`, floored, saturating at `u64::MAX`.
pub(crate) fn percent_of(total: u64, percent: u8) -> u64 {
    let bytes = (u128::from(total) * u128::from(percent)) / 100;
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// Parse [`GPU_BUDGET_PERCENT_ENV`]'s value: a percentage in
/// [`MIN_GPU_BUDGET_PERCENT`]..=[`MAX_GPU_BUDGET_PERCENT`].
fn parse_budget_percent(raw: &str) -> Result<u8, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("the value is empty".to_string());
    }
    let percent: u8 = trimmed
        .parse()
        .map_err(|_| "expected a whole percentage".to_string())?;
    if !(MIN_GPU_BUDGET_PERCENT..=MAX_GPU_BUDGET_PERCENT).contains(&percent) {
        return Err(format!(
            "expected {MIN_GPU_BUDGET_PERCENT}..={MAX_GPU_BUDGET_PERCENT}"
        ));
    }
    Ok(percent)
}

/// The GPU memory budget policy: a cap and the fraction of it at which staging is
/// refused.
///
/// Both fields are plain configuration so a host can construct one without wgpu
/// types. [`Default`] is [`GpuBudget::uncapped`]: with no evidence about VRAM there
/// is deliberately no default cap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuBudget {
    /// The cap in bytes. `None` means **no cap**: either explicitly (an opt-out)
    /// or because no VRAM probe answered. The renderer then allocates until the
    /// device or the driver refuses, which is exactly what
    /// [`super::GpuMemorySource`] says about the numbers it reports.
    pub cap_bytes: Option<u64>,
    /// The fraction of the cap at which a staging request is refused, in
    /// `(0, 1]`. Default `1.0`.
    ///
    /// A value below 1 keeps headroom: the renderer refuses once the resident
    /// scene plus the request would cross `cap × refuse_at`, so the last part of
    /// the cap is never used at upload time. Non-finite or non-positive values are
    /// clamped to `1.0` (documented, not silently multiplied into the cap).
    pub refuse_at: f32,
}

impl Default for GpuBudget {
    fn default() -> Self {
        Self::uncapped()
    }
}

impl GpuBudget {
    /// An uncapped policy (never refuses on the renderer's own account).
    pub const fn uncapped() -> Self {
        Self {
            cap_bytes: None,
            refuse_at: 1.0,
        }
    }

    /// The same policy with a different cap.
    pub const fn with_cap(self, cap_bytes: Option<u64>) -> Self {
        Self { cap_bytes, ..self }
    }

    /// The same policy with a different refusal fraction.
    pub const fn with_refuse_at(self, refuse_at: f32) -> Self {
        Self { refuse_at, ..self }
    }

    /// Resolve the policy from the environment: [`GPU_BUDGET_ENV`] when it is set
    /// and parseable, [`GpuBudget::uncapped`] otherwise.
    ///
    /// This is the **low-level** shape: it has no VRAM evidence, so unset means
    /// "no cap" ([`GpuBudgetConfig::from_env`] + [`GpuBudgetConfig::resolve`] is the
    /// percentage-aware path). A malformed value is reported on stderr and ignored —
    /// an invalid override must not silently look applied, and the resulting policy
    /// is still uncapped rather than a guessed number.
    pub fn from_env() -> Self {
        Self::resolve(std::env::var(GPU_BUDGET_ENV).ok().as_deref())
    }

    /// [`GpuBudget::from_env`] over an explicit value, so the parsing rules are
    /// unit-testable without touching the process environment.
    ///
    /// * `None` (unset) → [`GpuBudget::uncapped`];
    /// * `""` or a non-numeric value → [`GpuBudget::uncapped`], with a stderr note;
    /// * `"0"` → [`GpuBudget::uncapped`] (an explicit opt-out);
    /// * `"<n>"` MiB → `n × 1 MiB`, or [`GpuBudget::uncapped`] with a stderr note if
    ///   the multiplication overflows.
    pub fn resolve(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::uncapped();
        };
        match parse_budget_mib(raw) {
            Ok(Some(bytes)) => Self::uncapped().with_cap(Some(bytes)),
            Ok(None) => Self::uncapped(),
            Err(reason) => {
                eprintln!(
                    "yacr: ignoring {GPU_BUDGET_ENV}={raw:?}: {reason}; \
                     running without an explicit GPU byte cap"
                );
                Self::uncapped()
            }
        }
    }

    /// The byte threshold a staging request is compared against, or `None` when
    /// the policy is uncapped.
    ///
    /// `refuse_at` is clamped to `(0, 1]`: non-finite, zero and negative values
    /// mean "refuse at the cap" rather than a nonsense threshold, and a value
    /// above 1 would promise memory the cap does not cover.
    pub fn effective_limit_bytes(&self) -> Option<u64> {
        let cap = self.cap_bytes?;
        let fraction = if self.refuse_at.is_finite() && self.refuse_at > 0.0 {
            f64::from(self.refuse_at.min(1.0))
        } else {
            1.0
        };
        Some((cap as f64 * fraction) as u64)
    }

    /// Whether `requested` more bytes on top of `used` would be refused.
    pub fn would_exceed(&self, used: u64, requested: u64) -> bool {
        self.effective_limit_bytes()
            .is_some_and(|limit| used.saturating_add(requested) > limit)
    }
}

/// Parse [`GPU_BUDGET_ENV`]'s value: `Ok(Some(bytes))`, `Ok(None)` for the
/// explicit disable, `Err(reason)` when the value is not usable.
fn parse_budget_mib(raw: &str) -> Result<Option<u64>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("the value is empty".to_string());
    }
    let mib: u64 = trimmed
        .parse()
        .map_err(|_| "expected a whole number of MiB".to_string())?;
    if mib == 0 {
        return Ok(None);
    }
    mib.checked_mul(MIB)
        .map(Some)
        .ok_or_else(|| "the value is too large to be expressed in bytes".to_string())
}
