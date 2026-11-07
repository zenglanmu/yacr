//! The additive `cad_status.performance` block of the performance/diagnostics
//! panel.
//!
//! Every field is `Option`: a metric the host (or its backend) cannot measure is
//! `None` and serializes as `null`. An unavailable number is **never** reported
//! as `0`, because a fabricated zero is indistinguishable from a real
//! measurement.
//!
//! The GPU numbers are the **renderer's own accounting**, not device-wide usage
//! reported by a driver: they say how many bytes *this renderer* charges against
//! the GPU device it owns. [`CadPerformance::vram_total_bytes`] is the
//! **advertised device-local VRAM proxy** (the largest device-local heap), not
//! free memory — usage is not subtracted from it, and an integrated GPU's heap is
//! shared with the CPU.
//!
//! This module carries no host state and performs no sampling: the host's
//! sampler (for example `cad_desktop_shell::performance::PerformanceState`) fills
//! the block and the session projects it onto the status.

use serde::Serialize;

/// Host-reported performance metrics of the performance/diagnostics panel.
///
/// Wire shape: every key is always present (no `skip_serializing_if`), with
/// `null` for a metric the carrier cannot measure. The host publishes the whole
/// block as `performance: null` when it has no sampler at all.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadPerformance {
    /// Process CPU over the last sample window, as a percentage of **one** core
    /// (so a fully busy process on a 4-core machine reports about `100`, not
    /// `25`).
    pub cpu_percent: Option<f64>,
    /// Resident set size of this process, in bytes.
    pub process_rss_bytes: Option<u64>,
    /// Virtual address space of this process, in bytes.
    pub process_virtual_bytes: Option<u64>,
    /// Total physical memory of the machine, in bytes.
    pub system_total_bytes: Option<u64>,
    /// Currently available physical memory of the machine, in bytes.
    pub system_available_bytes: Option<u64>,
    /// Bytes this renderer currently charges against the GPU device.
    ///
    /// This is the **renderer's own accounted total** (buffers + textures), not
    /// device-wide usage: other processes and the host's own allocations outside
    /// the renderer are not included.
    pub gpu_memory_used_bytes: Option<u64>,
    /// Highest [`Self::gpu_memory_used_bytes`] observed since the renderer
    /// started, including re-upload transients.
    pub gpu_memory_peak_bytes: Option<u64>,
    /// Bytes the GPU allocator reserved from the driver (whole `Device`,
    /// including regions it has not handed out), when the backend sub-allocates.
    pub gpu_memory_reserved_bytes: Option<u64>,
    /// Advertised device-local VRAM proxy (the largest device-local heap), in
    /// bytes.
    ///
    /// This is **advertised capacity, not free memory**: usage is not subtracted,
    /// and it is `None` on backends that cannot answer (non-Vulkan, or no
    /// device-local heap).
    pub vram_total_bytes: Option<u64>,
}
