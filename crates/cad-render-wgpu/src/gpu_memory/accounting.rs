//! The accounting ledger, the drop guard that keeps it honest, and the public
//! report/refusal types.
//!
//! # How the counters stay correct without a per-frame scan
//!
//! Every GPU resource cluster the renderer owns ([`crate::GpuBatch`], an
//! instance/arc stream, a merged mesh chunk, a pulled segment, a mask, an image
//! quad, a texture, a uniform page, an offscreen target, …) holds one or two
//! [`GpuCharge`] values. A charge
//!
//! * adds its bytes to its category (and to the total, and to the peak) when it
//!   is constructed, next to the `create_buffer`/`create_texture` call, and
//! * **subtracts them again when it is dropped**, which happens at exactly the
//!   same moment the `wgpu::Buffer`/`wgpu::Texture` handle it charged is dropped.
//!
//! That symmetry is the whole point: the release cannot be forgotten at one of
//! the many destruction paths (`clear_batches`, a suffix commit's truncation, the
//! garbage collectors, a discarded [`PreparedUpload`](crate::PreparedUpload), a
//! device loss teardown), because Rust runs it for every one of them. Nothing
//! walks the resident scene at report time or per frame — a report is a handful
//! of atomic loads.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::budget::GpuBudget;

/// Every device-resident class this renderer accounts for.
///
/// The categories are the *owner* classes, not wgpu usage flags: a batch's
/// positions, normals, colours and anti-aliased line quads are all
/// [`Vertex`](GpuMemoryCategory::Vertex) because they are all per-vertex data,
/// while its triangle indices are [`Index`](GpuMemoryCategory::Index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GpuMemoryCategory {
    /// Per-vertex buffers: batch positions/normals/colours, anti-aliased line
    /// quads, merged mesh chunk vertices, image quad vertices and the shared unit
    /// quad.
    Vertex,
    /// Triangle/index buffers (batch, merged mesh chunk).
    Index,
    /// The per-frame uniform arena pages and the flags control uniform.
    Uniform,
    /// Per-run instanced line buffers and SDF conic instance streams (the
    /// universal tier's own copy of the converted geometry).
    Instance,
    /// Whole-scene reorganisation buffers: the accelerated tier's pulled storage
    /// segments and their per-chunk translation tables (the *second* copy of the
    /// converted geometry, on top of the per-run [`Instance`](GpuMemoryCategory::Instance)
    /// buffers).
    Stream,
    /// WIPEOUT mask geometry (a mask is a tessellated fill, not a texture).
    Mask,
    /// The offscreen colour target and the id (pick) target.
    ColorTarget,
    /// The depth targets of the colour and id passes.
    DepthTarget,
    /// One texture per resolved image [`ResourceKey`](cad_resources::ResourceKey).
    ImageTexture,
    /// **Reserved: always zero today.** This renderer's WIPEOUT masks are
    /// triangle geometry ([`Mask`](GpuMemoryCategory::Mask)), so no mask texture
    /// exists. The category is kept in the breakdown so a future mask sampling
    /// path cannot silently allocate outside the ledger.
    MaskTexture,
    /// Anything else with real device cost: the selection-flags table texture and
    /// the measurement-only timestamp resolve/readback buffers (ADR 0013 class 1).
    Other,
}

impl GpuMemoryCategory {
    /// Number of categories (the length of every category array).
    pub const COUNT: usize = 11;

    /// Every category, in index order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Vertex,
        Self::Index,
        Self::Uniform,
        Self::Instance,
        Self::Stream,
        Self::Mask,
        Self::ColorTarget,
        Self::DepthTarget,
        Self::ImageTexture,
        Self::MaskTexture,
        Self::Other,
    ];

    /// Index of this category in a category array.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Stable lowercase token for messages and diagnostics (never a UI string).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Index => "index",
            Self::Uniform => "uniform",
            Self::Instance => "instance",
            Self::Stream => "stream",
            Self::Mask => "mask",
            Self::ColorTarget => "color-target",
            Self::DepthTarget => "depth-target",
            Self::ImageTexture => "image-texture",
            Self::MaskTexture => "mask-texture",
            Self::Other => "other",
        }
    }

    /// Whether this category is a texture (as opposed to a buffer).
    pub const fn is_texture(self) -> bool {
        matches!(
            self,
            Self::ColorTarget | Self::DepthTarget | Self::ImageTexture | Self::MaskTexture
        )
    }
}

/// Which measurement the reported totals come from.
///
/// The distinction is the honesty contract of this module: `Accounted` is a
/// measured upper bound for *this renderer's* own resources, `AllocatorReport` is
/// the device allocator's own cross-check (whole `Device`, absent on backends
/// without sub-allocation), and `Estimate` means neither is live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMemorySource {
    /// This crate's own per-resource counters: measured, exact for what the
    /// renderer created (a lower bound for a `Device` shared with the host).
    Accounted,
    /// wgpu's `Device::generate_allocator_report()`, used only when the counters
    /// are not live (no device attached).
    AllocatorReport,
    /// No measurement at all is available; the numbers are the last accounting
    /// (zero after a teardown) and must be read as an application estimate.
    Estimate,
}

/// Why the renderer refused an allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMemoryRefusalOrigin {
    /// The renderer's own [`GpuBudget`] cap would be crossed. Nothing was
    /// allocated and the resident scene is untouched.
    ApplicationBudget,
    /// The **device** refused an allocation (wgpu reported
    /// `Error::OutOfMemory`). The requested resource does not exist.
    DeviceOutOfMemory,
}

/// One refused GPU allocation: the category, the accounting at the moment of the
/// refusal, the request, and the cap that decided it.
///
/// Carried by [`RenderError::GpuMemory`](crate::RenderError::GpuMemory) so a
/// caller can tell a budget/OOM refusal apart from a device loss
/// ([`RenderError::DeviceLost`](crate::RenderError::DeviceLost)) or a rejected
/// frame ([`RenderError::Frame`](crate::RenderError::Frame)) by *type*, not by
/// matching a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuMemoryRefusal {
    /// The class of the refused allocation.
    pub category: GpuMemoryCategory,
    /// Renderer-accounted device bytes at the moment of the refusal.
    pub used_bytes: u64,
    /// Bytes the refused request would have added.
    pub requested_bytes: u64,
    /// The configured application cap, when one is configured.
    pub cap_bytes: Option<u64>,
    /// The threshold the verdict was taken against (`cap × refuse_at`), when a
    /// cap is configured.
    pub limit_bytes: Option<u64>,
    /// Which rule refused the allocation.
    pub origin: GpuMemoryRefusalOrigin,
    /// The device's own message for a [`GpuMemoryRefusalOrigin::DeviceOutOfMemory`]
    /// refusal; `None` for an application-budget refusal.
    pub device_message: Option<String>,
    message: String,
}

impl GpuMemoryRefusal {
    /// An application-budget refusal.
    fn budget(
        category: GpuMemoryCategory,
        used_bytes: u64,
        requested_bytes: u64,
        budget: &GpuBudget,
        limit_bytes: u64,
    ) -> Self {
        let message = format!(
            "gpu memory budget: a {requested_bytes}-byte {} allocation would take the renderer \
             from {used_bytes} to {} bytes, over the {} cap {limit_bytes} bytes \
             (application estimate, refuse_at {}); nothing was allocated and the resident \
             scene is unchanged",
            category.label(),
            used_bytes.saturating_add(requested_bytes),
            super::budget::GPU_BUDGET_ENV,
            budget.refuse_at,
        );
        Self {
            category,
            used_bytes,
            requested_bytes,
            cap_bytes: budget.cap_bytes,
            limit_bytes: Some(limit_bytes),
            origin: GpuMemoryRefusalOrigin::ApplicationBudget,
            device_message: None,
            message,
        }
    }

    /// A refusal caused by the device reporting out-of-memory.
    fn device_oom(
        category: GpuMemoryCategory,
        used_bytes: u64,
        requested_bytes: u64,
        budget: &GpuBudget,
        device_message: String,
    ) -> Self {
        let message = format!(
            "gpu memory: the device refused a {requested_bytes}-byte {} allocation (out of \
             memory: {device_message}); renderer-accounted bytes {used_bytes}, application cap \
             {:?}; the resource was not created and the resident scene is unchanged",
            category.label(),
            budget.cap_bytes,
        );
        Self {
            category,
            used_bytes,
            requested_bytes,
            cap_bytes: budget.cap_bytes,
            limit_bytes: budget.effective_limit_bytes(),
            origin: GpuMemoryRefusalOrigin::DeviceOutOfMemory,
            device_message: Some(device_message),
            message,
        }
    }

    /// The stable refusal sentence naming category, request, accounting and cap.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the renderer's own budget (not the device) refused the request.
    pub fn is_budget_refusal(&self) -> bool {
        self.origin == GpuMemoryRefusalOrigin::ApplicationBudget
    }

    /// Whether the device itself reported out-of-memory.
    pub fn is_device_out_of_memory(&self) -> bool {
        self.origin == GpuMemoryRefusalOrigin::DeviceOutOfMemory
    }
}

impl std::fmt::Display for GpuMemoryRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GpuMemoryRefusal {}

/// A snapshot of the renderer's device-resident GPU memory.
///
/// Read [`source`](GpuMemoryReport::source) before comparing numbers:
/// `Accounted` is this renderer's measured footprint, and **no field here is the
/// card's total VRAM** (wgpu 30.0.1 cannot report it — see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct GpuMemoryReport {
    /// Bytes held by buffers (every non-texture category).
    pub buffer_bytes: u64,
    /// Bytes held by textures (colour/depth targets, image and flags tables).
    pub texture_bytes: u64,
    /// `buffer_bytes + texture_bytes`.
    pub total_bytes: u64,
    /// Highest `total_bytes` observed since the ledger was created, including a
    /// re-upload's transient (previous resident scene **plus** the staged one).
    pub peak_bytes: u64,
    /// `AllocatorReport::total_reserved_bytes`: memory the allocator reserved from
    /// the driver, including regions it has not handed out. `None` when the
    /// backend does not sub-allocate (for example GL).
    pub reserved_bytes: Option<u64>,
    /// `AllocatorReport::total_allocated_bytes`: live allocations of the whole
    /// `Device` (this renderer **and** its host). `None` as above.
    pub allocator_total_bytes: Option<u64>,
    /// The threshold the budget gate compares against (`cap × refuse_at`) in
    /// bytes. This is an **application estimate**, not the card's capacity.
    pub budget_bytes: Option<u64>,
    /// The configured application cap in bytes (`None` when the budget is
    /// explicitly disabled).
    pub cap_bytes: Option<u64>,
    /// Which measurement the numbers above come from.
    pub source: GpuMemorySource,
    /// Per-category breakdown, indexed by [`GpuMemoryCategory::index`].
    pub category_bytes: [u64; GpuMemoryCategory::COUNT],
}

impl GpuMemoryReport {
    /// Bytes charged against one category.
    pub fn category(&self, category: GpuMemoryCategory) -> u64 {
        self.category_bytes[category.index()]
    }

    /// Budget headroom left, or `None` when no cap is configured. Saturating:
    /// a report taken after an over-budget transient never wraps.
    pub fn headroom_bytes(&self) -> Option<u64> {
        self.budget_bytes
            .map(|limit| limit.saturating_sub(self.total_bytes))
    }

    /// Whether the renderer is at or past its own budget threshold.
    pub fn over_budget(&self) -> bool {
        self.budget_bytes
            .is_some_and(|limit| self.total_bytes >= limit)
    }
}

/// Facts from wgpu's allocator report, kept free of wgpu types so the ledger and
/// the report stay constructible in a unit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct AllocatorFacts {
    /// `AllocatorReport::total_allocated_bytes`.
    pub allocated_bytes: u64,
    /// `AllocatorReport::total_reserved_bytes`.
    pub reserved_bytes: u64,
}

/// The renderer's GPU memory ledger: live category counters, the peak, the budget
/// policy and the last refusal.
///
/// Shared through an [`Arc`] so every [`GpuCharge`] can release itself on drop.
/// All counters are relaxed atomics: they report bytes, they do not synchronise
/// anything.
#[derive(Debug)]
pub(crate) struct GpuMemoryLedger {
    categories: [AtomicU64; GpuMemoryCategory::COUNT],
    total_bytes: AtomicU64,
    peak_bytes: AtomicU64,
    /// Configured cap; `u64::MAX` encodes [`None`].
    cap_bytes: AtomicU64,
    /// `refuse_at` as `f32` bits.
    refuse_at_bits: AtomicU32,
    last_refusal: Mutex<Option<GpuMemoryRefusal>>,
}

impl GpuMemoryLedger {
    /// A ledger policing `budget` (see [`GpuBudget::resolve`]).
    pub(crate) fn new(budget: GpuBudget) -> Self {
        Self {
            categories: std::array::from_fn(|_| AtomicU64::new(0)),
            total_bytes: AtomicU64::new(0),
            peak_bytes: AtomicU64::new(0),
            cap_bytes: AtomicU64::new(budget.cap_bytes.unwrap_or(u64::MAX)),
            refuse_at_bits: AtomicU32::new(budget.refuse_at.to_bits()),
            last_refusal: Mutex::new(None),
        }
    }

    /// Charge `bytes` against `category` and return the drop guard that releases
    /// it again.
    ///
    /// Called immediately after the resource is created, so the ledger's peak
    /// includes the transient of a re-upload (old resident scene + staged one).
    pub(crate) fn charge(self: &Arc<Self>, category: GpuMemoryCategory, bytes: u64) -> GpuCharge {
        self.add(category, bytes);
        GpuCharge {
            ledger: Arc::clone(self),
            category,
            bytes,
        }
    }

    fn add(&self, category: GpuMemoryCategory, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.categories[category.index()].fetch_add(bytes, Ordering::Relaxed);
        let total = self.total_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak_bytes.fetch_max(total, Ordering::Relaxed);
    }

    fn release(&self, category: GpuMemoryCategory, bytes: u64) {
        if bytes == 0 {
            return;
        }
        // Saturating: a release can never drive the counter below zero, even if a
        // future charge site is ever mismatched. A saturated counter is a bug in
        // the charge sites, not a reason to underflow in a status report.
        saturating_sub(&self.categories[category.index()], bytes);
        saturating_sub(&self.total_bytes, bytes);
    }

    /// Renderer-accounted device bytes right now.
    pub(crate) fn used_bytes(&self) -> u64 {
        self.total_bytes.load(Ordering::Relaxed)
    }

    /// Highest renderer-accounted total observed so far.
    pub(crate) fn peak_bytes(&self) -> u64 {
        self.peak_bytes.load(Ordering::Relaxed)
    }

    /// Per-category accounting, indexed by [`GpuMemoryCategory::index`].
    pub(crate) fn category_bytes(&self) -> [u64; GpuMemoryCategory::COUNT] {
        std::array::from_fn(|index| self.categories[index].load(Ordering::Relaxed))
    }

    /// The live budget policy.
    pub(crate) fn budget(&self) -> GpuBudget {
        let cap = self.cap_bytes.load(Ordering::Relaxed);
        GpuBudget {
            cap_bytes: (cap != u64::MAX).then_some(cap),
            refuse_at: f32::from_bits(self.refuse_at_bits.load(Ordering::Relaxed)),
        }
    }

    /// Replace the budget policy (see [`Renderer::set_gpu_budget`]).
    ///
    /// [`Renderer::set_gpu_budget`]: crate::Renderer::set_gpu_budget
    pub(crate) fn set_budget(&self, budget: GpuBudget) {
        self.cap_bytes
            .store(budget.cap_bytes.unwrap_or(u64::MAX), Ordering::Relaxed);
        self.refuse_at_bits
            .store(budget.refuse_at.to_bits(), Ordering::Relaxed);
    }

    /// Admit (or refuse) an allocation of `parts` **before** it is created.
    ///
    /// `parts` are the `(category, bytes)` pieces the caller is about to allocate.
    /// The verdict is taken against `used + Σ bytes > limit`, where `limit` is the
    /// effective threshold of the configured budget; an uncapped budget always
    /// admits.
    ///
    /// A refusal is recorded ([`Self::refusal`]) so a caller on a `Result<_,
    /// CadError>` path (where the crate cannot widen the error type) can still
    /// read the structured facts instead of parsing a message.
    pub(crate) fn admit(&self, parts: &[(GpuMemoryCategory, u64)]) -> Result<(), GpuMemoryRefusal> {
        let budget = self.budget();
        let Some(limit_bytes) = budget.effective_limit_bytes() else {
            return Ok(());
        };
        let requested_bytes = parts
            .iter()
            .fold(0u64, |sum, (_, bytes)| sum.saturating_add(*bytes));
        if requested_bytes == 0 {
            // Nothing is being allocated, so there is nothing to refuse: a ledger
            // already past a tightened cap must not reject every empty request
            // (a converted batch owns no per-batch buffers at all).
            return Ok(());
        }
        let used_bytes = self.used_bytes();
        if used_bytes.saturating_add(requested_bytes) <= limit_bytes {
            return Ok(());
        }
        // Name the heaviest part: that is the allocation a reader should look at.
        let category = parts
            .iter()
            .max_by_key(|(_, bytes)| *bytes)
            .map(|(category, _)| *category)
            .unwrap_or(GpuMemoryCategory::Other);
        let refusal =
            GpuMemoryRefusal::budget(category, used_bytes, requested_bytes, &budget, limit_bytes);
        self.record_refusal(refusal.clone());
        Err(refusal)
    }

    /// Record a device-reported out-of-memory refusal and return it.
    ///
    /// That is the classification of a wgpu `Error::OutOfMemory` (wgpu-core maps
    /// its internal `CreateBufferError::OutOfMemory` / `CreateTextureError::OutOfMemory`
    /// to `ErrorType::OutOfMemory`, wgpu-core-30.0.1
    /// `src/device/resource.rs:712-717`), so it is never reported as a generic
    /// frame error.
    pub(crate) fn device_out_of_memory(
        &self,
        category: GpuMemoryCategory,
        requested_bytes: u64,
        device_message: String,
    ) -> GpuMemoryRefusal {
        let refusal = GpuMemoryRefusal::device_oom(
            category,
            self.used_bytes(),
            requested_bytes,
            &self.budget(),
            device_message,
        );
        self.record_refusal(refusal.clone());
        refusal
    }

    /// The most recent refusal (application budget or device OOM), if the
    /// following operations have not cleared it.
    pub(crate) fn refusal(&self) -> Option<GpuMemoryRefusal> {
        self.last_refusal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Drop the recorded refusal (called when a staging attempt succeeds, so a
    /// stale verdict is never reported as the current one).
    pub(crate) fn clear_refusal(&self) {
        *self
            .last_refusal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    fn record_refusal(&self, refusal: GpuMemoryRefusal) {
        *self
            .last_refusal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(refusal);
    }

    /// Build the public report.
    ///
    /// `device_present` selects [`GpuMemorySource::Accounted`];
    /// `allocator` (when the backend produced one) is attached as the cross-check
    /// for the whole `Device`, never substituted for the accounting.
    pub(crate) fn report(
        &self,
        device_present: bool,
        allocator: Option<AllocatorFacts>,
    ) -> GpuMemoryReport {
        let category_bytes = self.category_bytes();
        let texture_bytes: u64 = GpuMemoryCategory::ALL
            .iter()
            .filter(|category| category.is_texture())
            .map(|category| category_bytes[category.index()])
            .sum();
        let total_bytes = self.used_bytes();
        let budget = self.budget();
        let source = if device_present {
            GpuMemorySource::Accounted
        } else if allocator.is_some() {
            GpuMemorySource::AllocatorReport
        } else {
            GpuMemorySource::Estimate
        };
        GpuMemoryReport {
            buffer_bytes: total_bytes.saturating_sub(texture_bytes),
            texture_bytes,
            total_bytes,
            peak_bytes: self.peak_bytes(),
            reserved_bytes: allocator.map(|facts| facts.reserved_bytes),
            allocator_total_bytes: allocator.map(|facts| facts.allocated_bytes),
            budget_bytes: budget.effective_limit_bytes(),
            cap_bytes: budget.cap_bytes,
            source,
            category_bytes,
        }
    }
}

/// `atomic -= bytes`, saturating at zero (the standard library's `fetch_update`
/// equivalent without the deprecated name).
fn saturating_sub(atomic: &AtomicU64, bytes: u64) {
    let mut current = atomic.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_sub(bytes);
        match atomic.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

/// A charge against one ledger category, released when dropped.
///
/// Holds the bytes it added, so the release is exactly symmetric with the charge
/// and never re-derives a size from a resource that may have been resized. It is
/// deliberately **not** `Clone`: one GPU allocation has exactly one owner, and a
/// cloned handle (an image quad's copy of its shared texture handle, or an
/// accelerated stream pointing at a segment's buffer) must not double-count it.
#[derive(Debug)]
pub(crate) struct GpuCharge {
    ledger: Arc<GpuMemoryLedger>,
    category: GpuMemoryCategory,
    bytes: u64,
}

impl Drop for GpuCharge {
    fn drop(&mut self) {
        self.ledger.release(self.category, self.bytes);
    }
}

/// Exact device bytes one texture occupies: `block copy size × blocks × layers ×
/// mip levels × samples`.
///
/// Derived from the **created handle** (`Texture::size`/`format`/…), so it cannot
/// drift from the descriptor that produced it.
pub fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    texture_alloc_bytes(
        texture.format(),
        texture.size(),
        texture.mip_level_count(),
        texture.sample_count(),
    )
}

/// The same arithmetic over a descriptor's parts, for a **pre-flight** budget
/// request (before the texture exists).
pub(crate) fn texture_alloc_bytes(
    format: wgpu::TextureFormat,
    size: wgpu::Extent3d,
    mip_level_count: u32,
    sample_count: u32,
) -> u64 {
    let Some(block_bytes) = format.block_copy_size(None) else {
        return 0;
    };
    let (block_width, block_height) = format.block_dimensions();
    let width_blocks = u64::from(size.width).div_ceil(u64::from(block_width));
    let height_blocks = u64::from(size.height).div_ceil(u64::from(block_height));
    width_blocks
        .saturating_mul(height_blocks)
        .saturating_mul(u64::from(block_bytes))
        .saturating_mul(u64::from(size.depth_or_array_layers))
        .saturating_mul(u64::from(mip_level_count))
        .saturating_mul(u64::from(sample_count))
}
