//! The DirectComposition surface constructor (Windows only).
//!
//! Split out of `presenter.rs` so that file stays under the code-file size
//! limit; the constructor is still `impl SurfaceProbePresenter` and reuses the
//! shared `build_with_surface` factory (adapter ranking, device request,
//! pipeline, present mode, configure).

use super::*;

impl SurfaceProbePresenter {
    /// Create a presenter over a **DirectComposition** visual.
    ///
    /// The surface is built from
    /// [`wgpu::SurfaceTargetUnsafe::CompositionVisual`], which wgpu-hal 30 turns
    /// into an `IDXGIFactory2::CreateSwapChainForComposition` swapchain and binds
    /// with `IDCompositionVisual::SetContent` on every configure — the only swap
    /// chain kind that composites under a transparent
    /// (`WS_EX_NOREDIRECTIONBITMAP`) top-level window.
    ///
    /// The **visual's target is the top-level window** (with `topmost = FALSE`,
    /// the documented layer *behind* child windows), and the host positions/clips
    /// that visual to the accepted CAD rect in DIPs. The renderer deliberately
    /// knows nothing about either: it only wraps the visual pointer.
    ///
    /// # Safety
    ///
    /// - `visual` must be a live `IDCompositionVisual` (`IDCompositionDevice::
    ///   CreateVisual`) whose `IDCompositionTarget` targets the **top-level**
    ///   window the CAD content is shown in, positioned/clipped by the caller.
    /// - The **caller owns** the composition device, target, visual and clip and
    ///   must keep them alive until this presenter (and therefore its surface) is
    ///   dropped; wgpu keeps its own ref on the visual, but content appears only
    ///   while the device/target stay alive and a `Commit()` follows each
    ///   `configure_generation()` change *and* each offset/clip change.
    /// - The visual must be the root (or an ancestor-mounted child) of that
    ///   target.
    #[cfg(target_os = "windows")]
    pub unsafe fn new_from_composition_visual(
        visual: *mut core::ffi::c_void,
        width: u32,
        height: u32,
        present_mode: ProbePresentMode,
        gpu_time: crate::gpu_time::GpuTimeConfig,
        backend_policy: ProbeBackendPolicy,
    ) -> Result<Self, ProbeError> {
        backend_policy.ensure_supported()?;
        let preferred_names = DEFAULT_PREFERRED_ADAPTER_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        let instance = make_probe_instance(backend_policy);
        // SAFETY: forwarded from the caller's contract documented above; the
        // visual stays alive for at least as long as this surface (hal keeps its
        // own COM ref on it).
        let surface: wgpu::Surface<'static> = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(visual))
        }
        .map_err(|error| ProbeError::SurfaceCreation(error.to_string()))?;
        Self::build_with_surface(
            instance,
            surface,
            (width, height),
            preferred_names,
            present_mode,
            gpu_time,
            backend_policy,
            ProbeSurfaceKind::Composition,
        )
    }
}
