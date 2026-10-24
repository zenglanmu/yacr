use super::*;

/// One shared uniform buffer plus its bind group (group 0).
///
/// The bind group binds exactly one slot (`slot_size` bytes); the per-draw
/// dynamic offset selects which slot, so a single bind group serves every batch
/// resident in the page.
pub(crate) struct UniformPage {
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) bind_group: wgpu::BindGroup,
    /// GPU memory charge of this page's uniform buffer (released when the arena is
    /// reset or the device is lost).
    // Held only for its `Drop` impl, which releases the ledger charge.
    #[allow(dead_code)]
    pub(crate) buffer_charge: GpuCharge,
}

/// Shared per-frame uniform storage.
///
/// Every accepted geometry batch (and drawn image) writes its uniform
/// into a slot of a shared uniform buffer and draws it with the slot's byte
/// offset as a *dynamic* offset. This replaces one `queue.write_buffer` per
/// batch with one per page: a page holds `slots_per_page` slots and its total
/// bytes are kept within `max_uniform_buffer_binding_size`, so each page stays a
/// legal uniform binding even on the WebGPU default 64 KiB limit.
///
/// Slots are assigned per frame in accepted order; the arena is reused across
/// frames (it only grows).
pub(crate) struct UniformArena {
    /// Stride between slots: the uniform struct size rounded up to the device's
    /// `min_uniform_buffer_offset_alignment` (dynamic offsets must be aligned).
    pub(crate) slot_size: u32,
    /// Slots per page: `max_uniform_buffer_binding_size / slot_size`, capped so
    /// an adapter reporting a very large binding size cannot allocate an
    /// unbounded buffer.
    pub(crate) slots_per_page: usize,
    pub(crate) pages: Vec<UniformPage>,
    /// CPU staging laid out exactly like the GPU slots (`slot * slot_size`), so
    /// each page uploads with one contiguous `write_buffer`.
    pub(crate) staging: Vec<u8>,
}

impl UniformArena {
    pub(crate) fn new(limits: &wgpu::Limits) -> Self {
        let (slot_size, slots_per_page) = uniform_layout_params(limits);
        UniformArena {
            slot_size,
            slots_per_page,
            pages: Vec::new(),
            staging: Vec::new(),
        }
    }

    /// Drop every GPU page (device loss / re-init).
    pub(crate) fn reset(&mut self) {
        self.pages.clear();
        self.staging.clear();
    }

    /// Grow to hold `needed` slots. Pages are created lazily and never shrink.
    pub(crate) fn ensure_slots(
        &mut self,
        needed: usize,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        ledger: &Arc<GpuMemoryLedger>,
    ) {
        let pages_needed = needed.div_ceil(self.slots_per_page);
        while self.pages.len() < pages_needed {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cad-uniform-page"),
                size: (self.slots_per_page * self.slot_size as usize) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cad-uniform-page-bind"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        // One slot; the dynamic offset slides this window over
                        // the page, which is why the binding itself stays tiny.
                        size: wgpu::BufferSize::new(u64::from(self.slot_size)),
                    }),
                }],
            });
            let buffer_charge = ledger.charge(GpuMemoryCategory::Uniform, buffer.size());
            self.pages.push(UniformPage {
                buffer,
                bind_group,
                buffer_charge,
            });
        }
        let needed_bytes = needed * self.slot_size as usize;
        if self.staging.len() < needed_bytes {
            self.staging.resize(needed_bytes, 0);
        }
    }

    /// Copy one batch/image uniform into its staging slot.
    pub(crate) fn write_slot(&mut self, slot: usize, uniform: &[u8]) {
        let start = slot * self.slot_size as usize;
        self.staging[start..start + uniform.len()].copy_from_slice(uniform);
    }

    /// The bind group and dynamic offset a draw must bind for `slot`.
    pub(crate) fn binding(&self, slot: usize) -> (&wgpu::BindGroup, u32) {
        let (page, offset) = slot_location(slot, self.slots_per_page, self.slot_size);
        (&self.pages[page].bind_group, offset)
    }

    /// Upload the first `used_slots` slots, one `write_buffer` per page, and
    /// return the number of calls issued (this frame's `uniform_writes`).
    pub(crate) fn flush(&self, queue: &wgpu::Queue, used_slots: usize) -> u64 {
        let mut writes = 0u64;
        let mut start = 0usize;
        while start < used_slots {
            let in_page = (used_slots - start).min(self.slots_per_page);
            let byte_offset = start * self.slot_size as usize;
            let byte_len = in_page * self.slot_size as usize;
            let page = start / self.slots_per_page;
            queue.write_buffer(
                &self.pages[page].buffer,
                0,
                &self.staging[byte_offset..byte_offset + byte_len],
            );
            writes += 1;
            start += in_page;
        }
        writes
    }
}
