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
    /// the accounting. No field is the card's total VRAM: wgpu 30.0.1 cannot
    /// report it, so [`GpuMemoryReport::budget_bytes`] is an **application
    /// estimate** (see [`GpuBudget`]) sized from a measurement — the authorized
    /// fixture needs 2.11 GB, dominated by `stream` at 1.70 GB
    /// ([`MEASURED_FIXTURE_GPU_BYTES`]).
    ///
    /// Cheap by construction: a handful of atomic loads plus one allocator report
    /// call, never a scan of the resident scene. It is a diagnostics call, not a
    /// frame-path dependency.
    pub fn gpu_memory_report(&self) -> GpuMemoryReport {
        self.gpu_memory
            .report(self.device.is_some(), self.allocator_facts())
    }

    /// The memory budget policy in force (see [`Renderer::set_gpu_budget`]).
    pub fn gpu_budget(&self) -> GpuBudget {
        self.gpu_memory.budget()
    }

    /// Replace the memory budget policy.
    ///
    /// The accounting and the resident scene are untouched: a tighter budget
    /// governs the **next** staging request, never the memory already resident
    /// (retroactively refusing resident bytes would mean tearing down a scene the
    /// user can currently see). Takes effect immediately for
    /// [`Renderer::try_prepare_upload_batches`] and the `CadError` staging path.
    pub fn set_gpu_budget(&mut self, budget: GpuBudget) {
        self.gpu_memory.set_budget(budget);
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
