use super::*;

impl Renderer {
    pub(super) fn vertex_buffer(
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        bytes: &[u8],
    ) -> wgpu::Buffer {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad-batch"),
            size: bytes.len().max(12) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(queue) = queue {
            queue.write_buffer(&buffer, 0, bytes);
        }
        buffer
    }

    pub(super) fn index_buffer(
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        indices: &[u32],
    ) -> wgpu::Buffer {
        let bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad-batch-index"),
            size: bytes.len().max(4) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(queue) = queue {
            queue.write_buffer(&buffer, 0, &bytes);
        }
        buffer
    }

    /// Create a `STORAGE | COPY_DST` buffer for the accelerated tier and upload
    /// `bytes` (a multiple of 4; the caller never passes an empty payload).
    pub(super) fn try_storage_buffer(
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        bytes: &[u8],
    ) -> Result<wgpu::Buffer, PulledAllocationError> {
        Self::allocate_storage_buffer(
            device,
            queue,
            "cad-pulled-storage",
            wgpu::BufferUsages::STORAGE,
            bytes,
        )
    }

    /// Create a buffer that is both the accelerated line class's pulled storage
    /// and its per-run instance vertex buffer, so the two paths share one copy of
    /// the geometry.
    pub(super) fn try_vertex_storage_buffer(
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        bytes: &[u8],
    ) -> Result<wgpu::Buffer, PulledAllocationError> {
        Self::allocate_storage_buffer(
            device,
            queue,
            "cad-pulled-line-storage",
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
            bytes,
        )
    }

    /// Allocate one storage buffer and upload its contents, observing **every**
    /// error the device reports for the operation before returning the handle.
    ///
    /// `create_buffer` cannot fail through its return value: on wgpu 30.0.1 a
    /// driver allocation failure (out of memory) yields an **invalid** handle,
    /// and the failure only becomes visible when the handle is next used — as a
    /// *validation* error (`Buffer with '…' label is invalid`), which an
    /// `OutOfMemory` error filter does not capture, so it reached wgpu's default
    /// handler and aborted the process. Both steps therefore run inside
    /// [`allocation_guard`] (which captures both filters) and the create step is
    /// checked **before** the upload: on a failed create the handle is never
    /// written to, bound, or copied from, and the caller aborts its attempt
    /// (see the `?` sites in `rebuild_pulled`).
    pub(super) fn allocate_storage_buffer(
        device: &wgpu::Device,
        queue: Option<&wgpu::Queue>,
        label: &'static str,
        usage: wgpu::BufferUsages,
        bytes: &[u8],
    ) -> Result<wgpu::Buffer, PulledAllocationError> {
        let buffer = allocation_guard(device, || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len().max(4) as wgpu::BufferAddress,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })?;
        if let Some(queue) = queue {
            allocation_guard(device, || queue.write_buffer(&buffer, 0, bytes))?;
        }
        Ok(buffer)
    }

    /// Create a bind group, observing any error the device reports for it.
    ///
    /// The bind group is the *next* resource built from a freshly allocated
    /// buffer, so a buffer that the device silently invalidated would surface
    /// here; guarding it keeps that failure an explicit skip instead of a panic
    /// through wgpu's default handler.
    pub(super) fn guarded_bind_group(
        device: &wgpu::Device,
        descriptor: &wgpu::BindGroupDescriptor<'_>,
    ) -> Result<wgpu::BindGroup, PulledAllocationError> {
        allocation_guard(device, || device.create_bind_group(descriptor))
    }
}

/// Exact bytes one pulled rebuild allocates for the given resident classes.
///
/// Derived from the **real packed types**, never from a tuned constant:
/// `size_of::<PulledLine>()` per line instance, `size_of::<PulledMeshVertex>()`
/// per merged-mesh vertex, 4 bytes per rebased mesh index, and one `[f32; 4]`
/// chunk-table entry per resident stream/chunk (the tables `rebuild_pulled`
/// zero-fills). Segmentation does not change the total: each record and index is
/// written exactly once, into whichever binding-sized buffer it lands.
pub(crate) fn pulled_payload_bytes(
    line_instances: &[usize],
    mesh_vertices: &[usize],
    mesh_indices: &[usize],
) -> u64 {
    let line = line_instances
        .iter()
        .map(|count| *count as u64 * std::mem::size_of::<PulledLine>() as u64)
        .sum::<u64>();
    let line_table = line_instances.len() as u64 * std::mem::size_of::<[f32; 4]>() as u64;
    let mesh = mesh_vertices
        .iter()
        .map(|count| *count as u64 * std::mem::size_of::<PulledMeshVertex>() as u64)
        .sum::<u64>()
        + mesh_indices
            .iter()
            .map(|count| *count as u64 * 4)
            .sum::<u64>();
    let mesh_table = mesh_vertices.len() as u64 * std::mem::size_of::<[f32; 4]>() as u64;
    line + line_table + mesh + mesh_table
}

/// The pre-flight capacity verdict: the pulled organisation may be built only
/// when its exact requirement fits the budget.
///
/// An exact fit is allowed: the budget is the renderer's own ceiling on the
/// allocation it asks the device for, not a device-side hard limit. Pure so the
/// boundary is unit-testable without a GPU.
pub(crate) fn pulled_capacity_fits(required_bytes: u64, budget_bytes: u64) -> bool {
    required_bytes <= budget_bytes
}

/// Budget for **all** whole-scene pulled allocations of one rebuild.
///
/// Derived from the device's own limits, not from a scene or machine constant:
/// the budget is the device's largest single allocation, `max_buffer_size`
/// (the adapter's reported maximum allocation clamped by the shared device's
/// requested limits — on the Vulkan backend,
/// `VkPhysicalDeviceLimits::maxMemoryAllocationSize`, wgpu-hal 30.0.1
/// `src/vulkan/adapter.rs:1494-1514`).
///
/// Policy: the pulled organisation is a **second** copy of every converted
/// instance/vertex, allocated as binding-sized segments on top of the resident
/// per-run buffers. It may claim at most one `max_buffer_size` worth of
/// additional device memory; past that it is many separate allocations
/// competing with the per-chunk mesh buffers, the textures, the desktop
/// compositor sharing the same heap and driver fragmentation, which is exactly
/// the regime that made a ~2 GiB pulled set panic (`create_buffer: Out of
/// Memory`) on a 4 GB adapter.
///
/// A budget below one `max_buffer_size` (for example a share of
/// `max_storage_buffer_binding_size`) was rejected: the device's *usable*
/// bindings are already capped at `min(max_buffer_size,
/// max_storage_buffer_binding_size)` by the segmenter, so a tighter total bound
/// would skip the accelerated organisation for scenes that build and draw it
/// comfortably (the repository's 10 656-entity fixture needs ~172 MiB).
///
/// wgpu 30.0.1 exposes **no** total/free-memory query (`Adapter`/`Device` have
/// none), and this machine's NVIDIA driver exposes no VRAM size through
/// sysfs/procfs, so no tighter memory hint is obtainable here. The asymmetry is
/// deliberate: over-estimating is the failure being fixed, while
/// under-estimating only costs the accelerated organisation, because the
/// universal per-run fallback draws the same pixels.
pub(crate) fn pulled_budget_for(limits: &wgpu::Limits) -> u64 {
    limits.max_buffer_size
}

/// Pop one error scope and return the captured error text, if any.
///
/// The scope is always popped, so the scope stack stays balanced.
///
/// On native backends wgpu-core resolves a popped scope immediately
/// (`wgpu-30.0.1/src/backend/wgpu_core.rs:1860-1897` returns
/// `Box::pin(ready(scope.error))`), so `block_on` cannot stall. The browser tier
/// cannot await the pop on the render path, so it pops without observing the
/// result and relies on the pre-flight gate.
pub(crate) fn pop_captured_error(scope: wgpu::ErrorScopeGuard) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    let captured = futures_lite::future::block_on(scope.pop()).map(|error| error.to_string());
    #[cfg(target_arch = "wasm32")]
    let captured = {
        drop(scope);
        None
    };
    captured
}

/// The device error one allocation region reported, classified by *kind* rather
/// than guessed from its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PulledAllocationError {
    /// wgpu reported `Error::OutOfMemory` (a genuine allocation failure).
    OutOfMemory(String),
    /// wgpu reported something other than an out-of-memory error while the
    /// resource was being built. On this backend a driver-refused
    /// `create_buffer` surfaces as a **validation** error on the next use of the
    /// invalid handle, so this variant usually *is* memory exhaustion — but it is
    /// not provable from what the device reported, so it is never reported as
    /// [`PulledState::SkippedOutOfMemory`].
    Other(String),
}

impl PulledAllocationError {
    /// The captured error text (whitespace already normalised by wgpu's `Display`).
    pub(crate) fn message(&self) -> &str {
        match self {
            PulledAllocationError::OutOfMemory(message) | PulledAllocationError::Other(message) => {
                message
            }
        }
    }

    /// Append a note to the captured text, keeping the original classification.
    ///
    /// Used when the universal fallback also fails: the first failure is the
    /// verdict, the second is evidence that the device could not recover.
    pub(crate) fn with_note(self, note: &str) -> Self {
        let message = format!("{}; {note}", self.message());
        match self {
            PulledAllocationError::OutOfMemory(_) => PulledAllocationError::OutOfMemory(message),
            PulledAllocationError::Other(_) => PulledAllocationError::Other(message),
        }
    }
}

/// Verdict for one allocation region: build, or abandon it with the captured
/// error. Pure so the "did the region report an error?" decision is testable
/// without a GPU, exactly like [`pulled_capacity_fits`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PulledAllocationVerdict {
    Proceed,
    Skip(PulledAllocationError),
}

/// Decide whether an allocation region may be used, from what its error scopes
/// reported.
///
/// * `validation` — the captured `ErrorFilter::Validation` error, if any. This is
///   the filter that sees a driver-refused allocation on wgpu 30.0.1 (an invalid
///   buffer handle is reported as `Buffer with '…' label is invalid` when next
///   used), so the pre-fix code's `OutOfMemory`-only scope never caught it.
/// * `oom` — the captured `ErrorFilter::OutOfMemory` error, if any (a create-time
///   OOM is mapped to `ErrorType::OutOfMemory` by wgpu-core).
/// * `injected` — the test-only synthetic OOM (`Renderer::injected_pulled_oom`).
///
/// An observed out-of-memory error wins the classification: if a region reports
/// both, the validation error is the *consequence* of the failed allocation
/// (invalid handle use), not a second, independent fault. `injected` is only
/// consulted when the device reported nothing, so a synthetic test message can
/// never mask a real captured error.
pub(crate) fn allocation_verdict(
    validation: Option<String>,
    oom: Option<String>,
    injected: Option<String>,
) -> PulledAllocationVerdict {
    // A real captured error is the evidence; the injected test-only message only
    // stands in when the device reported nothing.
    if let Some(message) = oom {
        return PulledAllocationVerdict::Skip(PulledAllocationError::OutOfMemory(message));
    }
    if let Some(message) = validation {
        return PulledAllocationVerdict::Skip(PulledAllocationError::Other(message));
    }
    if let Some(message) = injected {
        return PulledAllocationVerdict::Skip(PulledAllocationError::OutOfMemory(message));
    }
    PulledAllocationVerdict::Proceed
}

/// The explicit skip state for an observed allocation failure.
///
/// `required_bytes`/`budget_bytes` are the pre-flight numbers the failed attempt
/// was working from, so a reader can compare them exactly as for the other skip
/// verdicts; `error` carries the device's own message and decides whether the
/// verdict is provably [`PulledState::SkippedOutOfMemory`] or the weaker
/// [`PulledState::SkippedAllocationFailed`]. Never a generic/empty state.
pub(crate) fn pulled_skip_state(
    required_bytes: u64,
    budget_bytes: u64,
    error: PulledAllocationError,
) -> PulledState {
    match error {
        PulledAllocationError::OutOfMemory(message) => PulledState::SkippedOutOfMemory {
            required_bytes,
            budget_bytes,
            message,
        },
        PulledAllocationError::Other(message) => PulledState::SkippedAllocationFailed {
            required_bytes,
            budget_bytes,
            message,
        },
    }
}

/// Run one device operation inside nested `Validation` + `OutOfMemory` error
/// scopes and return its result only when neither scope captured an error.
///
/// Why both filters: a driver-refused `create_buffer` in wgpu 30.0.1 leaves an
/// **invalid** handle whose first use is reported as a *validation* error (the
/// filtering reason is documented at the call site in `rebuild_pulled`). wgpu
/// delivers each error to the innermost scope with a *matching* filter, so the
/// two scopes capture both kinds without shadowing each other.
///
/// Both scopes are popped on every path (LIFO: `OutOfMemory` was pushed last).
/// The browser tier cannot await a pop on the render path, so it observes
/// nothing and relies on the pre-flight gate — the same documented limitation as
/// [`pop_captured_error`].
pub(crate) fn allocation_guard<R>(
    device: &wgpu::Device,
    operation: impl FnOnce() -> R,
) -> Result<R, PulledAllocationError> {
    let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let oom_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let value = operation();
    let oom_captured = pop_captured_error(oom_scope);
    let validation_captured = pop_captured_error(validation_scope);
    match allocation_verdict(validation_captured, oom_captured, None) {
        PulledAllocationVerdict::Proceed => Ok(value),
        PulledAllocationVerdict::Skip(error) => Err(error),
    }
}
