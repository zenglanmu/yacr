//! The public GPU-memory surface: accounting, budget and the explicit refusal.
//!
//! Everything here is a thin projection of [`gpu_memory`]: hosts never touch a
//! `wgpu` type to read it, and the accounting itself lives in the drop guards
//! described there (so no code path here can drift from the real allocations).

use super::*;
use gpu_memory::{AllocatorFacts, GpuCharge, GpuMemoryCategory};

impl Renderer {
    /// The device-resident GPU memory this renderer holds, with its peak and the
    /// budget policy in force.
    ///
    /// Read [`GpuMemoryReport::source`]:
    ///
    /// * [`GpuMemorySource::Accounted`] — the numbers are this crate's own measured
    ///   per-resource accounting (exact for what the renderer created; a *lower
    ///   bound* for a `Device` shared with the host);
    /// * [`GpuMemorySource::AllocatorReport`] — no accounting is live, so the
    ///   numbers come from wgpu's allocator report;
    /// * [`GpuMemorySource::Estimate`] — neither is live; no measurement is
    ///   claimed.
    ///
    /// The allocator report (`Device::generate_allocator_report`, `None` on a
    /// backend without sub-allocation, for example GL) is attached as
    /// `reserved_bytes`/`allocator_total_bytes` and is **never** substituted for
    /// the accounting.
    ///
    /// Read [`GpuMemoryReport::budget_source`] for the **policy**: the cap is a
    /// percentage of the probed VRAM **proxy**
    /// (`GpuMemoryReport::vram_bytes`, the largest device-local heap — advertised
    /// device-local memory, *not* free memory; the sum of all device-local heaps is
    /// reported separately as `vram_all_heaps_bytes` because summing exceeds the
    /// card's physical capacity on the reference machine; see the validation
    /// record), an explicit byte cap, or **no cap**
    /// ([`BudgetSource::Unprobed`](crate::BudgetSource::Unprobed)) when no probe
    /// answered — the renderer runs unbudgeted rather than guess. The probe runs at
    /// most once per device (`Renderer::vram_report`).
    ///
    /// Cheap by construction: a handful of atomic loads plus one allocator report
    /// call, never a scan of the resident scene. It is a diagnostics call, not a
    /// frame-path dependency.
    pub fn gpu_memory_report(&self) -> GpuMemoryReport {
        self.refresh_gpu_budget_from_probe();
        self.gpu_memory
            .report(self.device.is_some(), self.allocator_facts())
    }

    /// The memory budget policy in force (see [`Renderer::set_gpu_budget`] and
    /// [`Renderer::set_gpu_budget_config`]).
    pub fn gpu_budget(&self) -> GpuBudget {
        self.refresh_gpu_budget_from_probe();
        self.gpu_memory.budget()
    }

    /// The VRAM-relative configuration in force (see
    /// [`Renderer::set_gpu_budget_config`]).
    pub fn gpu_budget_config(&self) -> GpuBudgetConfig {
        self.gpu_budget_config
    }

    /// Replace the memory budget policy.
    ///
    /// The accounting and the resident scene are untouched: a tighter budget
    /// governs the **next** staging request, never the memory already resident
    /// (retroactively refusing resident bytes would mean tearing down a scene the
    /// user can currently see). Takes effect immediately for
    /// [`Renderer::try_prepare_upload_batches`] and the `CadError` staging path.
    ///
    /// This is the low-level shape: the policy is labelled
    /// [`BudgetSource::ExplicitBytes`](crate::BudgetSource::ExplicitBytes) because
    /// the caller chose the number, and it mirrors into
    /// [`Renderer::gpu_budget_config`]'s `explicit_bytes` so a later VRAM probe
    /// cannot silently override the caller's choice.
    pub fn set_gpu_budget(&mut self, budget: GpuBudget) {
        self.gpu_budget_config.explicit_bytes = Some(budget.cap_bytes.unwrap_or(0));
        self.gpu_budget_config.refuse_at = budget.refuse_at;
        self.gpu_memory.set_budget(budget);
    }

    /// Replace the VRAM-relative policy (see [`GpuBudgetConfig`]).
    ///
    /// Takes effect with the next staging request (resident memory is never
    /// re-litigated). If the device's VRAM has not been probed yet this probes it
    /// now; otherwise the cached probe is reused, so a policy change never adds a
    /// driver query.
    pub fn set_gpu_budget_config(&mut self, config: GpuBudgetConfig) {
        self.gpu_budget_config = config;
        self.refresh_gpu_budget_from_probe();
    }

    /// The probed device-local VRAM description, or `None` when the backend cannot
    /// answer.
    ///
    /// Probes lazily on the first call and caches the result for this device
    /// generation (a failed probe is cached too). `total_bytes` is the VRAM
    /// **proxy** (the largest device-local heap): **advertised** device-local
    /// memory, not free memory — usage is not subtracted and an integrated GPU's
    /// heap is shared system RAM. `all_device_local_bytes` is the sum of every
    /// device-local heap, kept for transparency (summing **overestimates**: on the
    /// reference card the sum exceeded physical VRAM; see the validation record).
    pub fn vram_report(&self) -> Option<VramProbe> {
        self.ensure_vram_probe()
    }

    /// Resolve [`Self::gpu_budget_config`] against the cached (or freshly probed)
    /// VRAM total and install the result on the ledger.
    ///
    /// Called after device attach, on every report/budget read, and on a policy
    /// change; the probe itself runs at most once per device.
    pub(crate) fn refresh_gpu_budget_from_probe(&self) {
        let probe = self.ensure_vram_probe();
        let vram = probe.as_ref().map(VramProbe::summary);
        let proxy = vram.map(|vram| vram.total_bytes);
        self.gpu_memory
            .set_resolved(self.gpu_budget_config.resolve(proxy), vram);
    }

    /// The cached probe, probing this device on first use.
    fn ensure_vram_probe(&self) -> Option<VramProbe> {
        // Cloned before locking so the closure does not borrow `self` while the
        // cache guard is held; a `wgpu::Device` clone is a cheap handle bump.
        let device = self.device.clone();
        let mut cache = self
            .gpu_vram
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.get_or_probe(|| {
            #[cfg(not(target_arch = "wasm32"))]
            {
                device
                    .as_ref()
                    .and_then(gpu_memory::probe_vram_bytes_from_device)
            }
            #[cfg(target_arch = "wasm32")]
            {
                // The browser tier has no HAL/VRAM query; the wasm heap budget is
                // its memory story (see `gpu_memory::vram`).
                let _ = &device;
                None
            }
        })
    }

    /// The most recent GPU memory refusal, if no later staging attempt cleared it.
    ///
    /// This is the structured counterpart of
    /// [`RenderError::GpuMemory`] for callers on a `CadResult` path (the crate
    /// cannot widen [`cad_domain::CadError`], so `upload`/`prepare_upload_batches`
    /// keep their signature and report the same refusal here). [`None`] means no
    /// refusal is pending.
    ///
    /// It also reports a refusal that did **not** fail the call: when the
    /// accelerated pulled organisation is skipped because its *second* copy of the
    /// geometry would cross the budget, the universal per-run path still draws the
    /// same pixels and the upload succeeds — the recorded refusal is the structured
    /// reason for the downgrade (see
    /// [`Renderer::pulled_state`](crate::Renderer::pulled_state)).
    pub fn gpu_budget_refusal(&self) -> Option<GpuMemoryRefusal> {
        self.gpu_memory.refusal()
    }

    /// Charge `bytes` against `category` and return the drop guard that releases
    /// them.
    ///
    /// Call this **immediately after** creating the resource, so the ledger's peak
    /// covers the transient of a re-upload.
    pub(crate) fn gpu_charge(&self, category: GpuMemoryCategory, bytes: u64) -> GpuCharge {
        self.gpu_memory.charge(category, bytes)
    }

    /// Admit `parts` against the budget, mapping a refusal onto the crate's
    /// `CadResult` path.
    ///
    /// The refusal itself is recorded on the ledger, so the caller's caller can
    /// read [`Renderer::gpu_budget_refusal`] instead of parsing the message. The
    /// message is the refusal's own sentence, so the `CadError` still names the
    /// category, the request, the accounting and the cap.
    pub(crate) fn admit_gpu_memory(
        &self,
        parts: &[(GpuMemoryCategory, u64)],
    ) -> Result<(), CadError> {
        self.gpu_memory
            .admit(parts)
            .map_err(|refusal| CadError::GpuFailure(refusal.message().to_string()))
    }

    /// Admit `parts` against the budget, keeping the explicit
    /// [`RenderError::GpuMemory`] classification.
    pub(crate) fn try_admit_gpu_memory(
        &self,
        parts: &[(GpuMemoryCategory, u64)],
    ) -> Result<(), RenderError> {
        self.gpu_memory.admit(parts).map_err(RenderError::GpuMemory)
    }

    /// The device allocator's own report as plain numbers, if the backend has one.
    ///
    /// Native only: the wasm canvas path never asks for it (the browser tier has
    /// no allocator inspection and its budget story is the wasm heap budget), so
    /// this cannot change browser behaviour.
    fn allocator_facts(&self) -> Option<AllocatorFacts> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let report = self.device.as_ref()?.generate_allocator_report()?;
            Some(AllocatorFacts {
                allocated_bytes: report.total_allocated_bytes,
                reserved_bytes: report.total_reserved_bytes,
            })
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Release every standalone charge whose resource is being torn down.
    ///
    /// Only the resources that are **not** owned by a resident cluster need this:
    /// the unit quad, the offscreen colour/depth targets, the id target and its
    /// depth, the selection-flags table and control buffer, and the pulled camera
    /// uniform. Cluster-owned charges release themselves when the cluster drops.
    pub(crate) fn clear_standalone_charges(&mut self) {
        self.unit_quad_charge = None;
        self.target_charge = None;
        self.depth_charge = None;
        self.pick_target_charge = None;
        self.pick_depth_charge = None;
        self.flags_texture_charge = None;
        self.flags_ctl_charge = None;
        self.pulled_camera_charge = None;
    }

    /// Create a texture, classifying a device out-of-memory report as the explicit
    /// [`RenderError::GpuMemory`].
    ///
    /// `create_texture` has no error return value in wgpu 30.0.1: a refused
    /// allocation leaves an **invalid** handle whose first use is reported as a
    /// validation error, which is exactly what reached wgpu's default handler and
    /// aborted the process before this guard existed (see
    /// `renderer_impl/alloc.rs`). Both filters are observed here, so an
    /// out-of-memory report becomes [`GpuMemoryRefusalOrigin::DeviceOutOfMemory`]
    /// and a genuinely invalid descriptor becomes a named frame error — never a
    /// generic `Frame(...)` string for a memory failure.
    ///
    /// On the browser tier the pop cannot be awaited (no blocking on the render
    /// path), so nothing is captured and this is a plain create — the wasm budget
    /// story is the session heap budget, not a device OOM report.
    pub(crate) fn create_texture_checked(
        &self,
        device: &wgpu::Device,
        descriptor: &wgpu::TextureDescriptor<'_>,
        category: GpuMemoryCategory,
        requested_bytes: u64,
    ) -> Result<wgpu::Texture, RenderError> {
        let label = descriptor.label.unwrap_or("texture");
        allocation_guard(device, || device.create_texture(descriptor)).map_err(
            |error| match error {
                PulledAllocationError::OutOfMemory(message) => RenderError::GpuMemory(
                    self.gpu_memory
                        .device_out_of_memory(category, requested_bytes, message),
                ),
                PulledAllocationError::Other(message) => RenderError::Frame(format!(
                    "creating the {} texture '{label}' failed: {message}",
                    category.label()
                )),
            },
        )
    }

    /// Drop a recorded refusal (a staging attempt that succeeded must not leave a
    /// stale verdict for the next status read).
    pub(crate) fn clear_gpu_memory_refusal(&self) {
        self.gpu_memory.clear_refusal();
    }
}
