//! The application GPU memory budget: named constants, the environment override
//! and the pure verdict arithmetic.
//!
//! Everything here is plain arithmetic over `u64`/`f32`, so the refusal boundary
//! and the environment parsing are unit-tested on the host (`cargo test -p
//! cad-render-wgpu`), exactly like the wasm heap budget
//! (`apps/app-web/src/budget.rs`) is.
//!
//! # The cap is an application estimate, not the card's VRAM
//!
//! wgpu 30.0.1 exposes **no** total/free-memory query: `AdapterInfo` has no VRAM
//! field, `Instance::generate_report()` reports no memory budget, and
//! [`Limits`](wgpu::Limits) only carries per-resource ceilings
//! (`max_buffer_size`, `max_texture_dimension_2d`). A portable VRAM total
//! therefore does not exist, and [`DEFAULT_GPU_BUDGET_BYTES`] is the renderer's
//! own **application estimate** of how much device memory a CAD scene may claim —
//! sized from a measurement ([`MEASURED_FIXTURE_GPU_BYTES`]: 2.11 GB for
//! `fixtures/complex-test.dwg` on the reference machine, cross-checked against the
//! driver's allocator report to within 0.07 %), and overridable per machine with
//! `--gpu-budget-mb` / `YACR_GPU_BUDGET_MB` or
//! [`Renderer::set_gpu_budget`](crate::Renderer::set_gpu_budget) for a bigger card
//! or a smaller one.

/// The environment variable that overrides [`GpuBudget::cap_bytes`], in MiB.
///
/// Read once, when the renderer is constructed ([`GpuBudget::resolve`]). `0` on
/// its own **explicitly disables** the cap (reported as `cap_bytes: None`, never
/// silently treated as a one-megabyte cap); an unparseable value is reported on
/// stderr and ignored in favour of the default, so a typo can never leave a
/// session unbounded by accident.
pub const GPU_BUDGET_ENV: &str = "YACR_GPU_BUDGET_MB";

/// One mebibyte, the unit [`GPU_BUDGET_ENV`] is expressed in.
const MIB: u64 = 1024 * 1024;

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
/// Another drawing, another driver revision or another scene shape can need more;
/// a host that knows its own working set should raise or lower the cap instead of
/// treating this number as a promise. It exists so [`DEFAULT_GPU_BUDGET_BYTES`] can
/// be *checked* against a real file (see
/// `the_default_budget_admits_the_measured_fixture`), not so callers compare
/// against it at runtime.
pub const MEASURED_FIXTURE_GPU_BYTES: u64 = 2_107_964_952;

/// The default application GPU memory cap: **3 GiB**.
///
/// Rationale, from measurement rather than a round number picked for looks:
///
/// * the authorized fixture (`fixtures/complex-test.dwg`, 145,947 batches) needs
///   **2,107,964,952 bytes** of device memory on the reference machine
///   ([`MEASURED_FIXTURE_GPU_BYTES`]), with a peak equal to its total because the
///   upload is one atomic commit. The dominant consumer is the `stream` category
///   (`1,695,124,632 bytes` — instanced line and merged mesh streams), which is
///   **larger than the wasm heap the same file needs on the canvas path**
///   (1.43 GB, `docs/adr/0025-wasm-memory-budget-and-worker-isolation.md`).
///   3 GiB leaves ~0.9 GiB above that peak, so a somewhat larger drawing is still
///   admitted instead of being refused at upload.
/// * The previous **512 MiB** default was *wrong by measurement*: it refused the
///   same legitimate drawing with
///   `gpu-memory-refused: origin=application-budget category=mask
///   used=1573159200 requested=144 cap=536870912` — a false refusal, because the
///   renderer was already holding 1.57 GB when it asked for another 144 bytes. A
///   budget that refuses the repository's own authorized fixture is worse than no
///   budget: it turns a working session into a broken one while claiming to
///   protect it.
/// * 3 GiB still stays below a 4 GB card (the P620 this was measured on), so the
///   renderer's own refusal keeps its intended job of tripping *before* the driver
///   starts evicting other clients — and the driver-side thresholds are the second
///   line of defence where the backend honours them
///   ([`apply_memory_budget_thresholds`]).
/// * It is a **policy** number, not a hardware fact. A per-machine override is
///   expected: `--gpu-budget-mb <n>` on the CLI / `YACR_GPU_BUDGET_MB=<n>` in the
///   environment / [`Renderer::set_gpu_budget`](crate::Renderer::set_gpu_budget)
///   for a bigger card or a deliberately tighter session. Nothing in the renderer
///   assumes the default.
///
/// This is an **application estimate**. It is never reported as the card's VRAM
/// (wgpu 30.0.1 cannot report that; see the module docs).
///
/// Renderer-side **stream sizing is a separate follow-up**: the 1.70 GB `stream`
/// share says the instanced-line/merged-mesh representation is the thing to
/// optimise next, and this lane deliberately does not touch it (the accounting is
/// how that work will be measured).
pub const DEFAULT_GPU_BUDGET_BYTES: u64 = 3 * 1024 * MIB;

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

/// The GPU memory budget policy: a cap and the fraction of it at which staging is
/// refused.
///
/// Both fields are plain configuration so a host can construct one without wgpu
/// types. [`Default`] is [`GpuBudget::estimated_default`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuBudget {
    /// The cap in bytes. `None` **explicitly** disables the cap: the renderer then
    /// allocates until the device or the driver refuses, which is exactly what
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
        Self::estimated_default()
    }
}

impl GpuBudget {
    /// The default policy: [`DEFAULT_GPU_BUDGET_BYTES`] and `refuse_at = 1.0`.
    pub const fn estimated_default() -> Self {
        Self {
            cap_bytes: Some(DEFAULT_GPU_BUDGET_BYTES),
            refuse_at: 1.0,
        }
    }

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
    /// and parseable, [`GpuBudget::estimated_default`] otherwise.
    ///
    /// A malformed value is reported on stderr and ignored — an invalid override
    /// must not silently uncap the renderer, and must not silently look applied.
    pub fn from_env() -> Self {
        Self::resolve(std::env::var(GPU_BUDGET_ENV).ok().as_deref())
    }

    /// [`GpuBudget::from_env`] over an explicit value, so the parsing rules are
    /// unit-testable without touching the process environment.
    ///
    /// * `None` (unset) → the default cap;
    /// * `""` or a non-numeric value → the default cap, with a stderr note;
    /// * `"0"` → [`GpuBudget::uncapped`] (an explicit opt-out);
    /// * `"<n>"` MiB → `n × 1 MiB`, or the default cap with a stderr note if the
    ///   multiplication overflows.
    pub fn resolve(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::estimated_default();
        };
        match parse_budget_mib(raw) {
            Ok(Some(bytes)) => Self::estimated_default().with_cap(Some(bytes)),
            Ok(None) => Self::uncapped(),
            Err(reason) => {
                eprintln!(
                    "yacr: ignoring {GPU_BUDGET_ENV}={raw:?}: {reason}; \
                     using the default application estimate of {} MiB",
                    DEFAULT_GPU_BUDGET_BYTES / MIB
                );
                Self::estimated_default()
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
