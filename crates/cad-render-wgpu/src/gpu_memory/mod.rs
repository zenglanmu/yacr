//! Device-resident GPU memory: accounting, an explicit application budget, and
//! the explicit out-of-memory classification for the renderer.
//!
//! # Why this exists
//!
//! The wasm canvas host already refuses a scene *before* it aborts on the 4 GiB
//! linear-memory ceiling (`apps/app-web/src/budget.rs`: named constants, a
//! refusal that names the budget, the limit and the measured value, and never a
//! blank canvas). The **native** renderer had no equivalent: a large drawing
//! could ask the driver for more device memory than exists, and the failure
//! arrived as either a wgpu **panic** through the default error handler
//! (`create_buffer: Out of Memory`, see `renderer_impl/alloc.rs`) or as a generic
//! [`RenderError::Frame`](crate::RenderError::Frame) string that a caller could
//! not tell apart from a bad frame.
//!
//! This module mirrors the wasm vocabulary on the GPU side:
//!
//! 1. [`GpuMemoryLedger`] — **accounting**. Every buffer and texture the renderer
//!    creates is charged against one category of [`GpuMemoryCategory`] when it is
//!    created and released when the owning cluster is dropped ([`GpuCharge`] is a
//!    drop guard, so no creation site can leak a charge and no destruction site
//!    can forget one). Current and **peak** bytes are kept, so the transient of a
//!    re-upload (the previous resident scene plus the freshly staged one) is
//!    visible instead of being averaged away.
//! 2. [`GpuBudget`] / [`GpuBudgetConfig`] — the **policy**: a cap, a refusal
//!    fraction and the resolution order explicit bytes > percentage of the probed
//!    VRAM **proxy** (default [`DEFAULT_GPU_BUDGET_PERCENT`] = 90 %) > the
//!    documented estimate fallback. The proxy is the **largest** device-local heap
//!    the backend advertises (see [`VramProbe`]), which is **not** free memory;
//!    when no backend can answer, the cap is the application estimate
//!    [`DEFAULT_GPU_BUDGET_BYTES`] (see the honesty note below).
//! 3. [`GpuMemoryRefusal`] — the **explicit refusal**: `{category, used,
//!    requested, cap}`, carried by
//!    [`RenderError::GpuMemory`](crate::RenderError::GpuMemory) when the
//!    application budget trips, and by the same variant with
//!    [`GpuMemoryRefusalOrigin::DeviceOutOfMemory`] when the *device* refuses an
//!    allocation.
//!
//! # What is measured, and what is not
//!
//! | evidence | what it says | limit |
//! | --- | --- | --- |
//! | [`GpuMemorySource::Accounted`] | exactly what **this renderer** created | not the device total: a shared `Device` also holds the host's own resources, and wgpu's allocator may reserve more than it hands out |
//! | [`GpuMemorySource::AllocatorReport`] | wgpu's `Device::generate_allocator_report()`: allocation and reservation totals for the whole `Device` | `None` on backends without sub-allocation (for example GL), so it is a **cross-check**, never the only source |
//! | [`GpuMemorySource::Estimate`] | no device is attached, so neither measurement is live; the numbers are the last accounting (zero after a teardown) and must be read as an application estimate | owns no evidence at all |
//!
//! **wgpu 30.0.1 has no portable VRAM field.** `AdapterInfo` has no memory entry,
//! `Instance::generate_report()` reports no memory budget, and
//! [`Limits`](wgpu::Limits) only carries **per-resource** ceilings
//! (`max_buffer_size`, `max_texture_dimension_2d`). The closest number a desktop
//! backend gives is its **device-local physical memory heaps**, read through the
//! Vulkan HAL escape hatch (see [`VramProbe`]). The proxy is the **largest single
//! device-local heap**, not their sum: on the reference 4 GiB card an extra
//! host-visible device-local heap made the sum **exceed the card's capacity**, so a
//! summed percentage overstates the budget (it is sized against memory the card
//! does not have). Both numbers are advertised memory, **not free memory** (usage is
//! not subtracted, other clients are unknown, and an integrated GPU's heap is
//! shared system RAM). The default policy is 90 % of the proxy; when the probe
//! cannot answer (non-Vulkan, no HAL handle, no device-local heap) the cap falls
//! back to the measured application estimate [`DEFAULT_GPU_BUDGET_BYTES`] and is
//! labelled [`BudgetSource::Estimate`](crate::BudgetSource::Estimate) — it is never
//! presented as a percentage of the card.
//!
//! # The backend's own thresholds
//!
//! wgpu 30.0.1 can be told to police itself, from
//! [`InstanceDescriptor::memory_budget_thresholds`](wgpu::InstanceDescriptor):
//!
//! > Threshold at which texture, buffer, query set and acceleration structure
//! > creation will start to return OOM errors. This is a percent of the memory
//! > budget reported by native APIs. […] Currently only the D3D12 and
//! > (optionally) Vulkan backends support these options.
//! > — `wgpu-types-30.0.1/src/instance.rs:339-350`
//!
//! This crate installs [`RESOURCE_CREATION_BUDGET_PERCENT`] /
//! [`DEVICE_LOSS_BUDGET_PERCENT`] wherever wgpu documents that the backend honours
//! them — **D3D12 only**, see [`threshold_backends_supported`] for the measured
//! reason the Vulkan half is not a safe default. The wasm canvas presenter is
//! deliberately **not** touched: WebGL2 has no such API.
//!
//! # Measured footprint (the numbers this policy is sized from)
//!
//! Reference run: Quadro P620, NVIDIA 580.178.04, Vulkan,
//! `cad-cli-tools render fixtures/complex-test.dwg --gpu high`:
//!
//! ```text
//! gpu-memory: total=2107964952 buffer=2100592152 texture=7372800 peak=2107964952
//!             budget=4294967296 cap=4294967296 source=accounted
//!             allocator_total=2109386656 reserved=2617245696
//! gpu-memory-detail: vertex=303645612 index=80201976 uniform=4325456
//!             instance=16159896 stream=1695124632 mask=446436
//!             color-target=3686400 depth-target=3686400 image-texture=0
//!             mask-texture=0 other=688144
//! ```
//!
//! * **total 2.11 GB** (2,107,964,952 B) for the authorized fixture, equal to its
//!   peak because a scene upload is one atomic commit;
//! * the driver's own allocator report agrees to **0.07 %**
//!   (2,109,386,656 B vs 2,107,964,952 B) — that is the cross-check the
//!   [`GpuMemorySource`] distinction exists for, and it is the evidence that the
//!   per-resource accounting is neither missing resources nor double-counting the
//!   ones shared between a pulled segment and its streams;
//! * the dominant consumer is **`stream`: 1.70 GB** (1,695,124,632 B — instanced
//!   line and merged-mesh streams), which is more than the wasm heap the same file
//!   needs on the canvas path (1.43 GB, ADR 0025);
//! * the app-level cap is sized from that measurement
//!   ([`DEFAULT_GPU_BUDGET_BYTES`] = 3 GiB, see [`MEASURED_FIXTURE_GPU_BYTES`] for
//!   why the old 512 MiB default was a measured false refusal). On a card whose
//!   device-local memory can be probed the cap is instead
//!   [`DEFAULT_GPU_BUDGET_PERCENT`] % of it; the 3 GiB number is the documented
//!   fallback when the probe cannot answer.
//!
//! **Renderer-side stream sizing is a separate follow-up.** The `stream` share is
//! the thing to optimise (fewer/larger streams, tighter merged-chunk packing), and
//! this lane deliberately does not attempt it — the accounting above is how that
//! work will be measured.

mod accounting;
mod budget;
mod vram;

pub(crate) use accounting::{texture_alloc_bytes, AllocatorFacts, GpuCharge, GpuMemoryLedger};
pub use accounting::{
    texture_bytes, GpuMemoryCategory, GpuMemoryRefusal, GpuMemoryRefusalOrigin, GpuMemoryReport,
    GpuMemorySource,
};
pub use budget::{
    apply_memory_budget_thresholds, memory_budget_thresholds, threshold_backends_supported,
    BudgetSource, GpuBudget, GpuBudgetConfig, ResolvedBudget, DEFAULT_GPU_BUDGET_BYTES,
    DEFAULT_GPU_BUDGET_PERCENT, DEVICE_LOSS_BUDGET_PERCENT, GPU_BUDGET_ENV, GPU_BUDGET_PERCENT_ENV,
    MAX_GPU_BUDGET_PERCENT, MEASURED_FIXTURE_GPU_BYTES, MIN_GPU_BUDGET_PERCENT,
    RESOURCE_CREATION_BUDGET_PERCENT,
};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use vram::probe_vram_bytes_from_device;
pub(crate) use vram::VramCache;
pub use vram::{largest_device_local, probe_vram_bytes, sum_device_local, VramHeap, VramProbe};

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod tests;
