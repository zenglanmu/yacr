//! How much **device-local** memory the card advertises, when the backend can be
//! asked at all.
//!
//! # Why this is not a wgpu API
//!
//! wgpu 30.0.1 has no VRAM query: [`AdapterInfo`](wgpu::AdapterInfo) carries no
//! memory field, `Instance::generate_report()` reports no memory budget, and
//! [`Limits`](wgpu::Limits) are **per-resource** ceilings (`max_buffer_size`,
//! `max_texture_dimension_2d`), never totals. The one portable-ish route is the
//! backend's own physical-device description, reached through the HAL escape
//! hatch (`Adapter::as_hal` / `Device::as_hal`, `#[cfg(wgpu_core)]`):
//!
//! * `wgpu_hal::vulkan::Adapter::raw_physical_device()` and
//!   `wgpu_hal::vulkan::Device::raw_physical_device()` are public, and
//!   `InstanceShared::raw_instance()`/`entry()` expose the `ash` handles;
//! * wgpu-hal's own `MemoryProperties` type is private, so this module binds
//!   `ash` directly (pinned to the exact version wgpu-hal 30.0.1 resolves).
//!
//! # What the number means — and what it does not
//!
//! [`VramProbe::total_bytes`] is the **largest single heap flagged
//! `VK_MEMORY_HEAP_DEVICE_LOCAL_BIT`**. That is the closest portable VRAM proxy a
//! desktop backend exposes, and it is deliberately **not** the sum of the
//! device-local heaps:
//!
//! * a discrete card commonly exposes one large device-local heap *plus* a
//!   smaller host-visible device-local heap (a resizable-BAR/spill heap the driver
//!   reports with the same flag). Summing them **overestimates** the card: the
//!   reference Quadro P620 is a 4 GiB card whose device-local heaps summed to
//!   **more than 4 GiB** (see the validation record for the measured numbers), so a
//!   percentage of the sum overstates the usable budget — it is sized against
//!   memory the card does not have. The largest heap is the proxy;
//!   [`VramProbe::all_device_local_bytes`] keeps the sum only for transparency
//!   (`GpuMemoryReport::vram_all_heaps_bytes`), never for the cap.
//! * it is **not free memory** — current usage is not subtracted, because usage
//!   is process/allocator-dependent and a "free" number would be stale the moment
//!   it was read. This applies to both the proxy and the sum;
//! * it can include a small amount the driver reserves for itself, and it does
//!   not know about other clients of the same card;
//! * on an integrated GPU the "device-local" heap *is* system RAM shared with the
//!   CPU, so the number is an address-space budget, not a dedicated pool.
//!
//! Consequently the budget policy derived from it
//! ([`GpuBudgetConfig`](super::GpuBudgetConfig), default 90 %) is still an
//! **application** policy with deliberate headroom, never a claim that the
//! renderer may fill the card. The exact heap layout of the reference machine is
//! recorded in the validation record rather than hard-coded here.
//!
//! # Failure is `None`, never a panic
//!
//! Every path that cannot answer returns [`None`]: a non-Vulkan adapter, an
//! `as_hal` callback that returns `None` (GL/Metal/DX12/noop/webgpu), a heap list
//! with no device-local memory, or a loader that cannot resolve the entry point.
//! The caller then falls back to the documented application estimate
//! ([`DEFAULT_GPU_BUDGET_BYTES`](super::DEFAULT_GPU_BUDGET_BYTES)) and the report
//! labels the cap `Estimate`.

/// One physical memory heap, as the backend reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VramHeap {
    /// Heap index in the backend's own list (`vkPhysicalDeviceMemoryProperties`).
    pub index: u32,
    /// Heap size in bytes.
    pub size: u64,
    /// `VK_MEMORY_HEAP_DEVICE_LOCAL_BIT`: the heap the GPU prefers for
    /// device-resident resources. Only the **largest** such heap is the VRAM proxy
    /// ([`VramProbe::total_bytes`]); every device-local heap is included in
    /// [`VramProbe::all_device_local_bytes`].
    pub device_local: bool,
}

/// The card's device-local memory, as far as the backend can report it.
///
/// See the module docs for what `total_bytes` is (the largest device-local heap)
/// and what it is **not** (free memory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VramProbe {
    /// The VRAM **proxy**: the largest device-local heap in bytes (never zero: a
    /// probe with no device-local memory is reported as `None`).
    ///
    /// The budget percentage is applied to this number. It is *not* the sum of the
    /// heaps — see [`Self::all_device_local_bytes`] and the module docs.
    pub total_bytes: u64,
    /// The sum of **every** device-local heap, for transparency.
    ///
    /// Kept because it is still useful diagnostics (and shows how much a summed
    /// proxy would overestimate), but it is deliberately **not** the cap basis.
    pub all_device_local_bytes: u64,
    /// Every heap the backend listed, device-local or not, for diagnostics.
    pub heaps: Vec<VramHeap>,
    /// The backend that answered. Only `"vulkan"` exists today; the field is
    /// kept so a report can state its evidence instead of implying more.
    pub backend: &'static str,
}

/// Sum the device-local heaps.
///
/// This is the **transparency** number (`VramProbe::all_device_local_bytes`), not
/// the VRAM proxy: on the reference card the extra host-visible device-local heap
/// made the sum exceed physical VRAM (see the module docs). Pure (no wgpu/ash
/// types), so the exclusion of non-device-local heaps is unit-tested on a synthetic
/// heap list without a GPU. Saturating: a bogus heap list can never wrap into a
/// tiny "total".
pub fn sum_device_local(heaps: &[VramHeap]) -> u64 {
    heaps
        .iter()
        .filter(|heap| heap.device_local)
        .fold(0u64, |sum, heap| sum.saturating_add(heap.size))
}

/// The VRAM proxy: the **largest** device-local heap, or `None` when the list has
/// no device-local heap at all.
///
/// Ties are deterministic (the largest `size`; the first heap wins on an exact
/// tie, but equal sizes are indistinguishable in the result). Pure, so the
/// "largest heap, not the sum" rule is unit-tested without a GPU.
pub fn largest_device_local(heaps: &[VramHeap]) -> Option<u64> {
    heaps
        .iter()
        .filter(|heap| heap.device_local)
        .map(|heap| heap.size)
        .max()
}

/// The two VRAM numbers the renderer caches and reports (plain data, no heap list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct VramSummary {
    /// The proxy: largest device-local heap (`VramProbe::total_bytes`).
    pub total_bytes: u64,
    /// The transparency number: sum of all device-local heaps.
    pub all_device_local_bytes: u64,
}

impl VramProbe {
    /// The reportable/cached view of this probe.
    pub(crate) fn summary(&self) -> VramSummary {
        VramSummary {
            total_bytes: self.total_bytes,
            all_device_local_bytes: self.all_device_local_bytes,
        }
    }
}

/// Probe the adapter's device-local VRAM proxy (the largest device-local heap).
///
/// Returns `None` (never panics) when:
///
/// * the adapter is not Vulkan (the only backend whose memory heaps wgpu 30.0.1
///   lets a caller read);
/// * `Adapter::as_hal::<Vulkan>()` returns `None`;
/// * no physical device is reachable;
/// * the backend reports zero heaps, or no device-local heap at all;
/// * the loader cannot resolve `vkGetPhysicalDeviceMemoryProperties`.
///
/// The result is a **point-in-time heap view**, not free memory: see the module
/// docs.
#[cfg(target_arch = "wasm32")]
pub fn probe_vram_bytes(_adapter: &wgpu::Adapter) -> Option<VramProbe> {
    // The browser tier has no `wgpu_core`/HAL and no memory-heap query (WebGL2
    // exposes no VRAM figure). Its budget story is the wasm heap budget; the
    // native percentage policy simply has no evidence on this path.
    None
}

/// Probe the adapter's device-local VRAM proxy (the largest device-local heap).
///
/// Returns `None` (never panics) when:
///
/// * the adapter is not Vulkan (the only backend whose memory heaps wgpu 30.0.1
///   lets a caller read);
/// * `Adapter::as_hal::<Vulkan>()` returns `None`;
/// * no physical device is reachable;
/// * the backend reports zero heaps, or no device-local heap at all;
/// * the loader cannot resolve `vkGetPhysicalDeviceMemoryProperties`.
///
/// The result is a **point-in-time heap view**, not free memory: see the module
/// docs.
#[cfg(not(target_arch = "wasm32"))]
pub fn probe_vram_bytes(adapter: &wgpu::Adapter) -> Option<VramProbe> {
    if adapter.get_info().backend != wgpu::Backend::Vulkan {
        return None;
    }
    // SAFETY: `adapter.as_hal` is safe to *call* here because no resource is
    // produced or destroyed: the guard only borrows the HAL adapter for the
    // duration of this function, and every query below is read-only. The guard is
    // dropped before `adapter` can be dropped (it borrows `adapter`).
    let hal_adapter = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
    let shared = hal_adapter.shared_instance();
    read_memory_properties(
        shared.entry(),
        shared.raw_instance(),
        hal_adapter.raw_physical_device(),
    )
}

/// The same probe for a [`wgpu::Device`], so the renderer can resolve its budget
/// from the device it was actually handed.
///
/// A host provides a `Device`/`Queue`, never an adapter (see
/// [`Renderer::initialize_with_device`](crate::Renderer::initialize_with_device)),
/// so the renderer has no adapter to ask; `wgpu_hal::vulkan::Device` exposes the
/// same physical device and shared instance. Native only (and `None` on a
/// non-Vulkan device, exactly like [`probe_vram_bytes`]).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn probe_vram_bytes_from_device(device: &wgpu::Device) -> Option<VramProbe> {
    // SAFETY: identical rationale as `probe_vram_bytes`; the HAL guard borrows
    // `device` for this call only and every statement below is a read-only query.
    let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
    let shared = hal_device.shared_instance();
    read_memory_properties(
        shared.entry(),
        shared.raw_instance(),
        hal_device.raw_physical_device(),
    )
}

/// Read `VkPhysicalDeviceMemoryProperties` through the loader and turn it into a
/// [`VramProbe`].
///
/// The entry point is resolved from `vkGetInstanceProcAddr` by name rather than
/// read out of `ash`'s pre-loaded table: a table slot whose load failed is a
/// **panicking stub** in `ash` 0.38, so using it would abort instead of returning
/// `None`. A `null` result from the loader is the honest "cannot answer".
#[cfg(not(target_arch = "wasm32"))]
fn read_memory_properties(
    entry: &ash::Entry,
    instance: &ash::Instance,
    physical_device: ash::vk::PhysicalDevice,
) -> Option<VramProbe> {
    let name = c"vkGetPhysicalDeviceMemoryProperties";
    // SAFETY: `entry` and `instance` belong to the live wgpu-hal instance that
    // created `physical_device`; `vkGetInstanceProcAddr` is a pure query that
    // returns either a valid function pointer for this instance or null.
    let raw = unsafe { entry.get_instance_proc_addr(instance.handle(), name.as_ptr()) };
    let raw = raw?;
    // SAFETY: the loader returned the address of
    // `vkGetPhysicalDeviceMemoryProperties` resolved for this instance, whose
    // signature is exactly `PFN_vkGetPhysicalDeviceMemoryProperties`.
    let query = unsafe {
        std::mem::transmute::<
            unsafe extern "system" fn(),
            ash::vk::PFN_vkGetPhysicalDeviceMemoryProperties,
        >(raw)
    };
    let mut properties = ash::vk::PhysicalDeviceMemoryProperties::default();
    // SAFETY: `physical_device` is a valid handle from the same instance, and the
    // out-parameter points at a live, correctly typed, fully initialized value.
    // The command is a read-only core-1.0 query.
    unsafe { query(physical_device, &mut properties) };

    let count = (properties.memory_heap_count as usize).min(properties.memory_heaps.len());
    if count == 0 {
        return None;
    }
    let heaps: Vec<VramHeap> = properties.memory_heaps[..count]
        .iter()
        .enumerate()
        .map(|(index, heap)| VramHeap {
            index: index as u32,
            size: heap.size,
            device_local: heap.flags.contains(ash::vk::MemoryHeapFlags::DEVICE_LOCAL),
        })
        .collect();
    let all_device_local_bytes = sum_device_local(&heaps);
    let Some(total_bytes) = largest_device_local(&heaps) else {
        // Heaps exist but none is device-local: nothing usable to base a VRAM
        // percentage on, so this is "unknown", not "zero VRAM".
        return None;
    };
    Some(VramProbe {
        total_bytes,
        all_device_local_bytes,
        heaps,
        backend: "vulkan",
    })
}

/// The renderer's one-per-device VRAM probe cache.
///
/// `probed` distinguishes "not asked yet" from "asked and the backend could not
/// answer (`None`)": a failed probe is cached too, so a report never re-queries
/// the loader per call.
#[derive(Debug, Clone, Default)]
pub(crate) struct VramCache {
    probed: bool,
    probe: Option<VramProbe>,
}

impl VramCache {
    /// Forget the previous device's answer (a new device generation re-probes).
    pub(crate) fn reset(&mut self) {
        self.probed = false;
        self.probe = None;
    }

    /// Probe at most once, then answer from the cache.
    pub(crate) fn get_or_probe(
        &mut self,
        probe: impl FnOnce() -> Option<VramProbe>,
    ) -> Option<VramProbe> {
        if !self.probed {
            self.probe = probe();
            self.probed = true;
        }
        self.probe.clone()
    }

    /// The cached probe, without probing (`None` also while never probed).
    #[cfg(test)]
    pub(crate) fn cached(&self) -> Option<&VramProbe> {
        self.probe.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heap(index: u32, size: u64, device_local: bool) -> VramHeap {
        VramHeap {
            index,
            size,
            device_local,
        }
    }

    #[test]
    fn only_device_local_heaps_are_summed() {
        // Typical discrete-card layout: one device-local heap, then a host-visible
        // "system" heap that the GPU can *also* address (a resizable BAR / spill
        // heap). Summing both would overstate the card's memory by even more.
        let heaps = [
            heap(0, 4 * 1024 * 1024 * 1024, true),
            heap(1, 16 * 1024 * 1024 * 1024, false),
            heap(2, 256 * 1024 * 1024, true),
        ];
        assert_eq!(
            sum_device_local(&heaps),
            4 * 1024 * 1024 * 1024 + 256 * 1024 * 1024
        );
        // A list with no device-local heap is zero, not the host total.
        assert_eq!(
            sum_device_local(&[heap(0, 8 * 1024 * 1024 * 1024, false)]),
            0
        );
        assert_eq!(sum_device_local(&[]), 0);
    }

    #[test]
    fn the_sum_is_saturating_not_wrapping() {
        let heaps = [heap(0, u64::MAX, true), heap(1, 1, true)];
        assert_eq!(sum_device_local(&heaps), u64::MAX);
    }

    #[test]
    fn the_proxy_is_the_largest_device_local_heap_not_the_sum() {
        const GIB: u64 = 1024 * 1024 * 1024;
        const MIB: u64 = 1024 * 1024;
        // The measured trap: one big device-local heap plus a small host-visible
        // device-local heap. Summing them overstates physical VRAM; the proxy must
        // be the big heap alone.
        let heaps = [heap(0, 4 * GIB, true), heap(1, 246 * MIB, true)];
        assert_eq!(largest_device_local(&heaps), Some(4 * GIB));
        assert_eq!(sum_device_local(&heaps), 4 * GIB + 246 * MIB);
        // Non-device-local heaps never become the proxy, however large.
        assert_eq!(
            largest_device_local(&[heap(0, 4 * GIB, true), heap(1, 16 * GIB, false)]),
            Some(4 * GIB)
        );
        // Mixed sizes: the maximum wins regardless of index order.
        assert_eq!(
            largest_device_local(&[
                heap(0, GIB, true),
                heap(1, 8 * GIB, true),
                heap(2, 2 * GIB, true)
            ]),
            Some(8 * GIB)
        );
        // Ties are deterministic: equal sizes yield that size.
        assert_eq!(
            largest_device_local(&[heap(7, 3 * GIB, true), heap(3, 3 * GIB, true)]),
            Some(3 * GIB)
        );
        // No device-local heap at all is "unknown", not zero.
        assert_eq!(largest_device_local(&[]), None);
        assert_eq!(
            largest_device_local(&[heap(0, 8 * GIB, false), heap(1, 1, false)]),
            None
        );
    }

    #[test]
    fn ninety_percent_of_the_proxy_stays_below_a_four_gib_card() {
        // Regression shape from the reference machine: a 4 GiB card that also
        // exposes a 258 MiB host-visible device-local heap. The sum exceeds the
        // card's physical capacity, so it must never be the cap basis.
        const GIB: u64 = 1024 * 1024 * 1024;
        const MIB: u64 = 1024 * 1024;
        let heaps = [heap(0, 4 * GIB, true), heap(1, 258 * MIB, true)];
        let proxy = largest_device_local(&heaps).expect("a device-local heap exists");
        assert_eq!(proxy, 4 * GIB);
        let proxy_cap = super::super::budget::percent_of(proxy, 90);
        assert!(
            proxy_cap < 4 * GIB,
            "90 % of the proxy {proxy_cap} must stay under the card's 4 GiB"
        );
        // The summed number is larger than the card itself and inflates the cap by
        // the same extra heap; it is reported for transparency only.
        let summed = sum_device_local(&heaps);
        assert_eq!(summed, 4 * GIB + 258 * MIB);
        assert!(summed > 4 * GIB);
        let summed_cap = super::super::budget::percent_of(summed, 90);
        assert!(
            summed_cap > proxy_cap,
            "the old summed proxy overstates the budget"
        );
        // Concretely: the sum-based cap is 90 % of the extra heap larger.
        assert_eq!(summed_cap - proxy_cap, 258 * MIB * 9 / 10);
    }

    #[test]
    fn the_cache_probes_exactly_once() {
        let mut cache = VramCache::default();
        let mut calls = 0;
        let probe = VramProbe {
            total_bytes: 1024,
            all_device_local_bytes: 1536,
            heaps: vec![heap(0, 1024, true), heap(1, 512, true)],
            backend: "vulkan",
        };
        for _ in 0..3 {
            let found = cache.get_or_probe(|| {
                calls += 1;
                Some(probe.clone())
            });
            assert_eq!(found, Some(probe.clone()));
        }
        assert_eq!(calls, 1, "one probe per cached answer");
        assert_eq!(cache.cached(), Some(&probe));

        // A failed probe is cached too: no repeated loader calls per report.
        let mut failing = VramCache::default();
        let mut failed_calls = 0;
        for _ in 0..3 {
            assert_eq!(
                failing.get_or_probe(|| {
                    failed_calls += 1;
                    None
                }),
                None
            );
        }
        assert_eq!(failed_calls, 1);

        // A new device generation resets the cache.
        let mut reset = VramCache::default();
        reset.get_or_probe(|| Some(probe.clone()));
        reset.reset();
        assert_eq!(reset.cached(), None);
        assert_eq!(reset.get_or_probe(|| None), None, "reset re-probes");
    }
}
