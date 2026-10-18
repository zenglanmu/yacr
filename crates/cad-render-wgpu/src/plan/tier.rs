use super::*;

/// Decide the draw tier once per device.
///
/// The accelerated tier (one draw per class via vertex-stage storage-buffer
/// pulling) needs a **read-only storage buffer in the vertex stage**. That is
/// core WebGPU but absent from WebGL2, so it is capability-gated here rather
/// than assumed:
///
/// * the backend's capability snapshot must report storage buffers (WebGL2 does
///   not; `caps_for` already encodes this);
/// * `max_storage_buffers_per_shader_stage` must be at least
///   [`MIN_PULLED_STORAGE_BINDINGS`] — the true requirement, not `1`: the pulled
///   merged-mesh pipeline binds three read-only storage buffers in the vertex
///   stage (`mesh_pulled.wgsl`) and the pulled line pipeline two. A device
///   reporting only one or two bindings would pass a `< 1` gate and then fail
///   at bind-group/pipeline creation, so the gate is sized to the real demand;
/// * `max_storage_buffer_binding_size` must be large enough to bind a
///   meaningful pulled buffer.
///
/// The renderer **never requests a feature** from a host-provided device; it
/// only reads `limits()`/`features()` and downgrades silently-but-labelled (the
/// live tier is reported by [`Renderer::render_tier`] and on
/// [`FrameStats::tier`]). Pure, so the decision is unit-testable without a GPU.
pub(crate) fn tier_for(
    storage_buffers: bool,
    limits: &wgpu::Limits,
    storage_support_override: Option<bool>,
) -> RenderTier {
    // Test hook: a caller can strip storage-buffer support to exercise the
    // automatic downgrade without a WebGL2 device.
    if storage_support_override == Some(false) {
        return RenderTier::Universal;
    }
    let forced_on = storage_support_override == Some(true);
    if !forced_on {
        if !storage_buffers {
            return RenderTier::Universal;
        }
        if limits.max_storage_buffers_per_shader_stage < MIN_PULLED_STORAGE_BINDINGS {
            return RenderTier::Universal;
        }
        if limits.max_storage_buffer_binding_size < MIN_PULLED_BINDING_BYTES {
            return RenderTier::Universal;
        }
    }
    RenderTier::Accelerated
}

/// Smallest `max_storage_buffer_binding_size` a device must report for the
/// accelerated tier to be selected. Below this a pulled buffer could not hold
/// even a tiny scene, so the universal per-run path is the honest choice.
pub(crate) const MIN_PULLED_BINDING_BYTES: u64 = 64 * 1024;

/// Number of read-only vertex-stage storage buffers the accelerated tier
/// actually binds in one pipeline: three in `mesh_pulled.wgsl` (vertices,
/// indices, chunk table), two in `line_pulled.wgsl` (instances, chunk table).
/// The tier gate requires this many, so a device that reports fewer cannot
/// select a tier it would then fail to create pipelines for.
pub(crate) const MIN_PULLED_STORAGE_BINDINGS: u32 = 3;

/// Finalise the tier once the pulled resources have been created.
///
/// A device that selected [`RenderTier::Accelerated`] but whose pulled
/// pipelines/layouts could not be built (for example a missing flags layout)
/// must downgrade to [`RenderTier::Universal`]: leaving it labelled Accelerated
/// with a missing pulled path would draw the prepare-time placeholder buffers
/// instead of real geometry.
pub(crate) fn finalize_tier(preliminary: RenderTier, pulled_resources_ready: bool) -> RenderTier {
    if preliminary == RenderTier::Accelerated && !pulled_resources_ready {
        RenderTier::Universal
    } else {
        preliminary
    }
}
