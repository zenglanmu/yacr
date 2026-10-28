//! The wasm-only WebGL2 canvas presenter itself.
//!
//! Only this file needs `web-sys` and a real wgpu surface; every rule it applies
//! lives in [`super::policy`], which is compiled and unit tested on every target.

use super::policy::{
    check_physical_size, check_present_format, check_present_size, check_present_usage,
    preferred_present_format, CanvasError, CanvasTargetKind, SurfaceStatus,
};
use crate::blit::FullscreenBlitter;
use crate::wgpu;

/// The web canvas a presenter owns, before it is turned into a wgpu surface.
///
/// One enum instead of two presenters: `Surface::configure`, the acquire/status
/// policy, the blit and the counters are identical for both carriers, and the only
/// thing that differs is which `wgpu::SurfaceTarget` variant is created (wgpu's
/// GLES WebGL2 backend sizes both canvases from the surface config in the same
/// way). Duplicating the presenter would be two copies of the same rules, which is
/// exactly how the two carriers would drift apart.
enum CanvasTarget {
    /// The page's `HtmlCanvasElement` (main-thread session).
    Html(web_sys::HtmlCanvasElement),
    /// An `OffscreenCanvas` transferred into a Worker (isolated session).
    Offscreen(web_sys::OffscreenCanvas),
}

impl CanvasTarget {
    /// The stable tag recorded in diagnostics and status.
    fn kind(&self) -> CanvasTargetKind {
        match self {
            Self::Html(_) => CanvasTargetKind::HtmlCanvas,
            Self::Offscreen(_) => CanvasTargetKind::OffscreenCanvas,
        }
    }

    /// The `wgpu::SurfaceTarget` for this canvas.
    fn into_surface_target(self) -> wgpu::SurfaceTarget<'static> {
        match self {
            Self::Html(canvas) => wgpu::SurfaceTarget::Canvas(canvas),
            Self::Offscreen(canvas) => wgpu::SurfaceTarget::OffscreenCanvas(canvas),
        }
    }
}

/// What evidence records about the adapter behind a canvas presenter.
///
/// **This is not hardware proof.** WebKitGTK masks the GL renderer string (it
/// reports a generic name), so `name` alone never proves a physical/discrete GPU,
/// and `backend` only says the GLES/WebGL2 backend was selected — the wasm path
/// has no Vulkan/DX12/Metal fallback, because neither target runtime exposes
/// WebGPU (`navigator.gpu` is absent). Record it as a description of the session,
/// never as a GPU capability claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasAdapterInfo {
    /// Adapter name as reported by the browser's GL implementation (may be
    /// masked/generic).
    pub name: String,
    /// Stable lowercase wgpu backend token; `"gl"` for the WebGL2 backend.
    pub backend: &'static str,
    /// The configured surface format (always the format the blit targets).
    pub format: wgpu::TextureFormat,
    /// The adapter's `max_texture_dimension_2d`, the upper bound a physical size
    /// may have.
    pub max_texture_dimension_2d: u32,
    /// Which carrier this presenter draws into: `html-canvas` (main thread) or
    /// `offscreen-canvas` (Worker). See [`CanvasTargetKind`].
    pub target: CanvasTargetKind,
}

/// A WebGL2 canvas presenter: owns the wgpu instance, the canvas surface, the
/// device/queue and the fullscreen blit.
///
/// The host hands in **one of two** web canvas handles and never touches a wgpu
/// object beyond what this crate re-exports; `wgpu` stays inside the renderer
/// crate exactly like the native probe path:
///
/// - [`CanvasPresenter::create`] — an [`web_sys::HtmlCanvasElement`] from the host
///   page (the main-thread carrier, `surfaceMode: dom-canvas` today);
/// - [`CanvasPresenter::create_offscreen`] — a [`web_sys::OffscreenCanvas`]
///   transferred into a Worker (the isolated carrier, ADR 0024 stage ⑧).
///
/// Both produce exactly the same presenter type and behaviour; the carrier is
/// recorded as [`CanvasAdapterInfo::target`]. The measured engine split — Chromium
/// 141 gives WebGL2 on a worker-side `OffscreenCanvas`, WebKitGTK 2.52.6 returns
/// `null` from `getContext("webgl2")` there — is a **browser** fact, not something
/// this crate can work around: on that engine `create_offscreen` fails with an
/// explicit [`CanvasError::SurfaceCreate`] and the host must stay on the
/// main-thread carrier instead of pretending.
///
/// # Sizing contract
///
/// - Every size is **physical** pixels: `CSS px × devicePixelRatio`. CSS pixels
///   would present at the wrong scale on any `devicePixelRatio != 1` display.
/// - `Surface::configure` also sets `canvas.width`/`canvas.height`, so the host
///   must not size the canvas backing store itself after `create`/`resize`.
/// - [`CanvasPresenter::resize`] is idempotent (a repeated size is a no-op) and
///   safe to call between frames; the next `present_texture` then targets the new
///   size.
/// - A `(0, 0)` (or any zero dimension) request is rejected with
///   [`CanvasError::InvalidSize`] — never a panic inside `Surface::configure`.
///
/// # Frame contract
///
/// [`CanvasPresenter::present_texture`] blits the renderer's offscreen frame
/// texture onto the canvas: no CPU readback, no second CAD surface, and no
/// blocking poll (WebGL2 has no waitable fence). The renderer must be attached to
/// [`CanvasPresenter::device_queue`], so the frame texture lives on this device.
pub struct CanvasPresenter {
    /// Kept alive so the backend owning the surface never drops early.
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    info: CanvasAdapterInfo,
    blitter: FullscreenBlitter,
    /// Number of `Surface::configure` calls (`create` is the first one). Evidence
    /// only, but it is what makes the "reconfigure once on Outdated/Lost" rule
    /// observable instead of implied.
    configure_count: u64,
    /// Frames that reached the canvas. Evidence only.
    presented_frames: u64,
}

impl CanvasPresenter {
    /// Create the WebGL2 device/queue for the page's `canvas` element and
    /// configure it at `physical` pixels.
    ///
    /// `physical` is `CSS px × devicePixelRatio`; see the type-level sizing
    /// contract. The instance requests only `Backends::GL`: WebGL2 is the only
    /// backend both target runtimes have (`navigator.gpu` is absent), so asking
    /// for anything else would be dead configuration.
    pub async fn create(
        canvas: web_sys::HtmlCanvasElement,
        physical: (u32, u32),
    ) -> Result<Self, CanvasError> {
        Self::create_for(CanvasTarget::Html(canvas), physical).await
    }

    /// Create the WebGL2 device/queue for an `OffscreenCanvas` (a canvas already
    /// transferred into this Worker with `transferControlToOffscreen()`) and
    /// configure it at `physical` pixels.
    ///
    /// Everything except the surface target is identical to
    /// [`CanvasPresenter::create`]: same instance descriptor (`Backends::GL`), same
    /// WebGL2 device floor, same present-mode/format/alpha policy, same blit. That
    /// is on purpose — one presenter, two carriers.
    ///
    /// Fails with [`CanvasError::SurfaceCreate`] when the engine cannot give this
    /// offscreen canvas a WebGL2 context. That is the measured WebKitGTK 2.52.6
    /// behaviour inside a Worker (`getContext("webgl2")` returns `null`), so the
    /// caller must treat it as "worker rendering is unavailable here" and report
    /// that instead of attempting a blank canvas.
    pub async fn create_offscreen(
        canvas: web_sys::OffscreenCanvas,
        physical: (u32, u32),
    ) -> Result<Self, CanvasError> {
        Self::create_for(CanvasTarget::Offscreen(canvas), physical).await
    }

    /// The one setup path both public constructors share.
    async fn create_for(target: CanvasTarget, physical: (u32, u32)) -> Result<Self, CanvasError> {
        let kind = target.kind();
        // Reject an impossible size before any GPU object exists.
        check_physical_size(physical, u32::MAX)?;

        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::GL;
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(target.into_surface_target())
            .map_err(|error| {
                // The target kind is part of the message: "no WebGL2 here" means
                // one thing for a page canvas and another for a worker-side
                // offscreen canvas, and evidence must be able to tell them apart.
                CanvasError::SurfaceCreate(format!("create_surface({kind}): {error}"))
            })?;

        // On WebGL2 an adapter can only be selected together with a compatible
        // surface: without it, selection fails outright.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| {
                CanvasError::AdapterUnavailable(format!("request_adapter({kind}): {error}"))
            })?;
        let limits = adapter.limits();
        // The adapter limit is known only now, so the upper bound is checked here.
        // (Retained separately: `using_resolution` consumes `limits`.)
        let max_texture_dimension_2d = limits.max_texture_dimension_2d;
        let physical = check_physical_size(physical, max_texture_dimension_2d)?;

        // WebGL2 has no storage buffers and a reduced limit set: ask for exactly
        // the WebGL2 floor (resolved up to what the adapter reports) and no
        // features, so request_device cannot fail on an optional capability.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cad-canvas-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(limits),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| CanvasError::DeviceUnavailable(format!("request_device: {error}")))?;

        let mut config = surface
            .get_default_config(&adapter, physical.0, physical.1)
            .ok_or(CanvasError::NoSurfaceConfig)?;
        // Explicit, not inherited: Fifo cannot tear, and the CAD content is
        // opaque so the canvas must composite opaquely.
        config.present_mode = wgpu::PresentMode::Fifo;
        config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        // The **linear** sibling of the frame format, so the present is
        // wgpu-hal's `blitFramebuffer` copy rather than its `gl_VertexID`-indexed
        // sRGB present shader (measured to draw nothing on the Android WebView;
        // see `preferred_present_format`). The sRGB encode then happens in our own
        // blit stage, so the presented bytes do not change.
        if let Some(format) = preferred_present_format(&surface.get_capabilities(&adapter).formats)
        {
            config.format = format;
        }
        // Also sets `canvas.width`/`canvas.height` from `config` — for both
        // carriers (wgpu-hal's WebGL2 backend configures `HtmlCanvasElement` and
        // `OffscreenCanvas` identically), so the host never sizes the backing
        // store itself.
        surface.configure(&device, &config);

        let blitter = FullscreenBlitter::new(&device, config.format);
        let adapter_info = adapter.get_info();
        let info = CanvasAdapterInfo {
            name: adapter_info.name,
            backend: adapter_info.backend.to_str(),
            format: config.format,
            max_texture_dimension_2d,
            target: kind,
        };
        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
            info,
            blitter,
            configure_count: 1,
            presented_frames: 0,
        })
    }

    /// Reconfigure the canvas for a new physical size (`CSS px ×
    /// devicePixelRatio`).
    ///
    /// Idempotent: a request equal to the configured size returns `Ok(())`
    /// without a reconfigure. Zero or out-of-limit sizes are rejected with
    /// [`CanvasError::InvalidSize`]. Safe between frames; the next presented frame
    /// must match the new size (see [`CanvasPresenter::present_texture`]).
    pub fn resize(&mut self, physical: (u32, u32)) -> Result<(), CanvasError> {
        let (width, height) = check_physical_size(physical, self.info.max_texture_dimension_2d)?;
        if (width, height) == (self.config.width, self.config.height) {
            return Ok(());
        }
        self.config.width = width;
        self.config.height = height;
        self.configure();
        Ok(())
    }

    /// The device/queue the CAD render session must be attached to.
    ///
    /// The host gets wgpu types through this crate's re-exports, so it needs no
    /// direct `wgpu` dependency.
    pub fn device_queue(&self) -> (wgpu::Device, wgpu::Queue) {
        (self.device.clone(), self.queue.clone())
    }

    /// The adapter/session description evidence records (see
    /// [`CanvasAdapterInfo`] — **not** hardware proof).
    pub fn adapter_info(&self) -> CanvasAdapterInfo {
        self.info.clone()
    }

    /// Which carrier this presenter draws into (see [`CanvasTargetKind`]).
    ///
    /// Read from the same record [`CanvasPresenter::adapter_info`] returns, so the
    /// status a host prints and the diagnostics a failure carries cannot disagree
    /// about whether the session is isolated.
    pub fn target_kind(&self) -> CanvasTargetKind {
        self.info.target
    }

    /// The configured physical size.
    pub fn surface_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Number of `Surface::configure` calls since construction (`create` counts as
    /// one). A steady frame loop must not grow it.
    pub fn configure_count(&self) -> u64 {
        self.configure_count
    }

    /// Frames that reached the canvas (a dropped/refused frame never counts).
    pub fn presented_frames(&self) -> u64 {
        self.presented_frames
    }

    /// Number of texture bind groups the blit created (`0` after the first frame);
    /// evidence that the per-frame bind cache works.
    pub fn bind_groups_created(&self) -> u64 {
        self.blitter.binds_created()
    }

    /// Present the renderer's offscreen frame `texture` on the canvas.
    ///
    /// `texture` must live on the device from [`CanvasPresenter::device_queue`],
    /// be the configured physical size, carry the configured surface's format or
    /// its sRGB/linear sibling ([`check_present_format`]), and have
    /// `TEXTURE_BINDING` usage; each violation is an explicit
    /// [`CanvasError::Blit`] naming the failed check rather than a stretched or
    /// colour-shifted frame.
    ///
    /// Acquire failures are explicit too: `Outdated`/`Lost` reconfigure the
    /// surface **once** and then report [`CanvasError::SurfaceOutdated`] /
    /// [`CanvasError::SurfaceLost`], so a blank canvas can never be presented
    /// silently — the host retries the next frame.
    pub fn present_texture(&mut self, texture: &wgpu::Texture) -> Result<(), CanvasError> {
        // Refuse an unusable source before touching the swapchain.
        let size = texture.size();
        check_present_size((size.width, size.height), self.surface_size())?;
        check_present_format(texture.format(), self.config.format)?;
        check_present_usage(texture.usage())?;

        let frame = self.acquire()?;
        {
            // Scoped so the view (and its borrow of the surface texture) is gone
            // before the present call, exactly like the native presenter.
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            self.blitter.blit(&self.device, &self.queue, &view, texture);
        }
        // `Queue::present` is the only present channel in wgpu 30 (a
        // `SurfaceTexture` has no `present` method of its own).
        self.queue.present(frame);
        self.presented_frames += 1;
        Ok(())
    }

    /// Acquire the next canvas frame, reconfiguring once for the statuses whose
    /// documented remedy is a reconfigure.
    fn acquire(&mut self) -> Result<wgpu::SurfaceTexture, CanvasError> {
        match self.surface.get_current_texture() {
            // `Suboptimal` still carries a usable frame; present it and let the
            // host reconfigure on its own terms.
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(frame),
            wgpu::CurrentSurfaceTexture::Timeout => {
                Err(self.acquire_failure(SurfaceStatus::Timeout))
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                Err(self.acquire_failure(SurfaceStatus::Occluded))
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                Err(self.acquire_failure(SurfaceStatus::Outdated))
            }
            wgpu::CurrentSurfaceTexture::Lost => Err(self.acquire_failure(SurfaceStatus::Lost)),
            wgpu::CurrentSurfaceTexture::Validation => {
                Err(self.acquire_failure(SurfaceStatus::Validation))
            }
        }
    }

    /// Apply the reconfigure a failing status calls for, then build the explicit
    /// error for it.
    fn acquire_failure(&mut self, status: SurfaceStatus) -> CanvasError {
        // `Outdated`/`Lost`: the documented remedy is a fresh configure, and it is
        // applied **once** before the error is reported.
        if status.needs_reconfigure() {
            self.configure();
        }
        status.acquire_error().unwrap_or_else(|| {
            CanvasError::SurfaceValidation(format!(
                "internal: {status:?} carries a frame but reached the failure path"
            ))
        })
    }

    /// Re-apply the current configuration and count it.
    fn configure(&mut self) {
        self.surface.configure(&self.device, &self.config);
        self.configure_count += 1;
    }
}
