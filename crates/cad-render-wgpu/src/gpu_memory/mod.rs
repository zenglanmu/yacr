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
//! 2. [`GpuBudget`] — the **policy**: a cap, a refusal fraction and the
//!    `YACR_GPU_BUDGET_MB` override. The cap is an **application estimate**, not
//!    the card's VRAM (see the honesty note below).
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
//! **The card's total VRAM is not knowable from wgpu 30.0.1.** `AdapterInfo` has
//! no VRAM field, `Instance::generate_report()` reports no memory budget, and
//! [`Limits`](wgpu::Limits) only carries **per-resource** ceilings
//! (`max_buffer_size`, `max_texture_dimension_2d`). Any app-level cap is
//! therefore labelled an application estimate and is configurable
//! ([`GpuBudget`] / `YACR_GPU_BUDGET_MB`); it is never presented as the card's
//! capacity.
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
//!   why the old 512 MiB default was a measured false refusal). It remains an
//!   application estimate: the card's VRAM is not knowable from wgpu.
//!
//! **Renderer-side stream sizing is a separate follow-up.** The `stream` share is
//! the thing to optimise (fewer/larger streams, tighter merged-chunk packing), and
//! this lane deliberately does not attempt it — the accounting above is how that
//! work will be measured.

mod accounting;
mod budget;

pub(crate) use accounting::{texture_alloc_bytes, AllocatorFacts, GpuCharge, GpuMemoryLedger};
pub use accounting::{
    texture_bytes, GpuMemoryCategory, GpuMemoryRefusal, GpuMemoryRefusalOrigin, GpuMemoryReport,
    GpuMemorySource,
};
pub use budget::{
    apply_memory_budget_thresholds, memory_budget_thresholds, threshold_backends_supported,
    GpuBudget, DEFAULT_GPU_BUDGET_BYTES, DEVICE_LOSS_BUDGET_PERCENT, GPU_BUDGET_ENV,
    MEASURED_FIXTURE_GPU_BYTES, RESOURCE_CREATION_BUDGET_PERCENT,
};

#[cfg(test)]
mod tests;
