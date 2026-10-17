//! Native-only **surface probe** presenter for host bring-up.
//!
//! This is deliberately **not** the production CAD presenter. It proves that a
//! host (a Tauri window, or an X11 child handle) can hand a window to the
//! renderer crate, get a real swapchain back, render onto it with wgpu, and
//! present every tick on a real hardware GPU. To make that unambiguous the probe
//! shader draws only a recognisable grid, a translating colour block and two
//! corner markers — never anything that could be mistaken for CAD geometry.
//!
//! Contract (spec §5.2/§5.3; `docs/adr/0002-gpu-ui-composition.md`):
//! - Native only (`#[cfg(not(target_arch = "wasm32"))]`); the browser path gets
//!   its device from the host canvas instead.
//! - All `wgpu` instance/device/pass/submit/present work lives in this module.
//!   `wgpu` handle objects never escape it; only plain data (adapter
//!   description, texture format, size, present mode) is exposed.
//! - The instance forces `Backends::VULKAN`, and adapter selection rejects
//!   `cpu` (software/fallback) adapters. A preferred adapter *name* (default
//!   `P620`) is only a preference, never a filter: any Vulkan hardware adapter
//!   that can present to the surface is accepted.
//! - No GPU→CPU readback is ever performed.
//! - The renderer never owns an event loop; the host owns the single loop and
//!   calls [`SurfaceProbePresenter::frame`] once per tick.
//! - Lifecycle is destroy-and-rebuild: drop the presenter to release the
//!   surface, construct a new one to rebuild. Every failure is an explicit
//!   [`ProbeError`], never a silent success.
//!
//! The constructor takes an *owning* window handle (`W: 'static`). The surface
//! is `Surface<'static>` and keeps that handle alive, so the window outlives the
//! surface by construction (see `wgpu::Surface::_handle_source`).
//!
//! For hosts that own a non-`Send` native widget (for example a GTK child), the
//! safe owning constructors cannot be used.
//! [`SurfaceProbePresenter::new_from_raw_handles`] is an explicit `unsafe`
//! alternative whose lifetime contract is entirely the caller's (see its
//! `# Safety`); it reuses the same factory as the safe path.

use crate::wgpu;
use futures_lite::future::block_on;

/// Preferred adapter-name substrings, tried in order, when none is supplied.
///
/// This is a *preference*, never a filter: a machine without any of these still
/// selects another Vulkan hardware adapter.
pub const DEFAULT_PREFERRED_ADAPTER_NAMES: &[&str] = &["P620"];

/// The only backend the probe will use, as wgpu's `Backend::to_str` value.
const REQUIRED_BACKEND: &str = "vulkan";

/// Marker-offset wrapping range. The probe wraps offsets into `[0, 1)`.
const MARKER_PERIOD: f32 = 1.0;

/// Bounded budget for collecting the validation error scope each frame.
///
/// The scope is resolved by device callbacks, which need a device poll. Blocking
/// on it indefinitely (the previous behaviour) let a slow device poll hold the
/// frame; this bounds it without dropping validation reporting.
const SCOPE_COLLECT_BUDGET: std::time::Duration = std::time::Duration::from_millis(100);

/// A read-only, owned description of an adapter the probe considered.
///
/// Strings mirror wgpu's stable lowercase spellings (`vulkan`, `discrete_gpu`,
/// …) so callers can log or assert without touching `wgpu` types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeAdapterInfo {
    /// Stable lowercase backend name (`"vulkan"` for a selected adapter).
    pub backend: String,
    /// Adapter/driver-reported name (for example `"Quadro P620"`).
    pub name: String,
    /// Stable lowercase device type: `"cpu"`, `"integrated_gpu"`,
    /// `"discrete_gpu"`, `"virtual_gpu"` or `"other"`.
    pub device_type: String,
    /// Driver name.
    pub driver: String,
    /// Free-form driver info string.
    pub driver_info: String,
    /// Whether the adapter reported at least one surface format for this
    /// surface. Always `true` for a selected adapter.
    pub surface_compatible: bool,
}

/// Per-stage wall-clock timings (microseconds) for one probe frame.
///
/// The probe previously reported only a total, which could not distinguish a
/// slow `write_buffer` from a slow acquire/submit/present. These stages are
/// measured individually so a stall can be attributed without guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProbeStageTimings {
    /// `Queue::write_buffer` of the params uniform.
    pub write_buffer_us: u64,
    /// `Surface::get_current_texture` (and any `Outdated` reconfigure retry).
    pub acquire_us: u64,
    /// `Surface::configure` calls made inside this frame (retry/outdated).
    pub configure_us: u64,
    /// `Queue::submit` of the encoded pass.
    pub submit_us: u64,
    /// Bounded, non-blocking collection of the validation error scope.
    pub scope_pop_us: u64,
    /// `Queue::present` of the acquired frame.
    pub present_us: u64,
    /// Total `frame` wall-clock time.
    pub total_us: u64,
}

/// Statistics for one presented probe frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeFrameStats {
    /// Monotonic probe frame counter, starting at `1` for the first frame.
    pub frame_index: u64,
    /// The sanitised marker offset actually drawn (always in `[0.0, 1.0)`).
    pub marker_offset: f32,
    /// Surface size at presentation time, in physical pixels.
    pub surface_size: (u32, u32),
    /// Per-stage timings for this frame.
    pub timings: ProbeStageTimings,
}

/// Cache/identity key for a native CAD texture presented on the surface.
///
/// The app builds this from `cad-engine`'s `PresentedFrame.binding`: the device
/// epoch, the texture revision and the physical size. Changing any field forces
/// a rebind (the cached bind group is keyed by the whole struct).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeTextureKey {
    /// Renderer device generation the texture belongs to.
    pub device_epoch: u64,
    /// Revision of the texture contents.
    pub texture_revision: u64,
    /// Physical size in pixels.
    pub size: (u32, u32),
}

/// Validate a texture key against the adapter's maximum texture dimension.
pub(crate) fn validate_texture_key(
    key: &NativeTextureKey,
    max_dimension: u32,
) -> Result<(), ProbeError> {
    if key.size.0 == 0 || key.size.1 == 0 {
        return Err(ProbeError::TextureInvalid(format!(
            "size {}x{} must be non-zero",
            key.size.0, key.size.1
        )));
    }
    if key.size.0 > max_dimension || key.size.1 > max_dimension {
        return Err(ProbeError::TextureInvalid(format!(
            "size {}x{} exceeds the adapter maximum {max_dimension}",
            key.size.0, key.size.1
        )));
    }
    Ok(())
}

/// Formats this presenter can sample for a native CAD texture (8-bit RGBA/BGRA,
/// sRGB or linear). Anything else is an explicit error rather than a guess.
pub(crate) fn texture_format_is_supported(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8UnormSrgb
            | wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Bgra8Unorm
    )
}

/// Present-mode selection for the probe surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProbePresentMode {
    /// `Fifo` (vsync) — the default and the only mode wgpu guarantees.
    #[default]
    Fifo,
    /// Use `Mailbox` when the surface reports it, otherwise `Immediate`.
    /// The requested, supported and chosen modes are logged explicitly; there is
    /// no silent fallback to a different mode.
    MailboxElseImmediate,
}

/// A probe failure. Every variant is a distinct, explicit cause; the probe never
/// converts a failure into a silent success.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeError {
    /// A requested surface width or height was zero.
    InvalidSize { width: u32, height: u32 },
    /// A requested surface dimension exceeds the adapter's texture limit.
    SizeExceedsLimit { width: u32, height: u32, limit: u32 },
    /// A marker offset was not finite (`NaN`/infinite).
    InvalidMarker(f32),
    /// `Instance::create_surface` failed.
    SurfaceCreation(String),
    /// No Vulkan hardware adapter could present to the surface.
    NoVulkanAdapter(String),
    /// `Adapter::request_device` failed.
    DeviceRequest(String),
    /// The surface reported no supported texture format.
    NoSupportedFormat,
    /// The surface could not hand out a frame (`timeout`/`occluded`/`outdated`/
    /// `validation`).
    Acquire(&'static str),
    /// The surface (or its device) was lost; rebuild the presenter.
    DeviceLost(String),
    /// The submitted frame was rejected (for example GPU validation).
    Frame(String),
    /// The presenter was stopped; no further frames are drawn.
    Stopped,
    /// A native texture passed to `present_texture` failed validation.
    TextureInvalid(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::InvalidSize { width, height } => {
                write!(f, "probe size {width}x{height} must be non-zero")
            }
            ProbeError::SizeExceedsLimit {
                width,
                height,
                limit,
            } => write!(
                f,
                "probe size {width}x{height} exceeds the adapter's maximum texture dimension {limit}"
            ),
            ProbeError::InvalidMarker(offset) => {
                write!(f, "probe marker offset {offset} must be finite")
            }
            ProbeError::SurfaceCreation(message) => {
                write!(f, "surface creation failed: {message}")
            }
            ProbeError::NoVulkanAdapter(message) => {
                write!(f, "no usable Vulkan hardware adapter: {message}")
            }
            ProbeError::DeviceRequest(message) => {
                write!(f, "wgpu device request failed: {message}")
            }
            ProbeError::NoSupportedFormat => {
                write!(f, "surface reported no supported texture format")
            }
            ProbeError::Acquire(kind) => write!(f, "surface acquisition failed: {kind}"),
            ProbeError::DeviceLost(message) => write!(f, "device lost: {message}"),
            ProbeError::Frame(message) => write!(f, "probe frame rejected: {message}"),
            ProbeError::Stopped => write!(f, "probe presenter is stopped"),
            ProbeError::TextureInvalid(message) => {
                write!(f, "native texture rejected: {message}")
            }
        }
    }
}

impl std::error::Error for ProbeError {}

/// A native, Vulkan-only surface probe.
///
/// Construct one per window, call [`SurfaceProbePresenter::frame`] once per host
/// tick, and drop it to release the surface. Rebuilding is just constructing a
/// new instance (possibly with a new window handle).
pub struct SurfaceProbePresenter {
    /// Kept alive for the lifetime of the presenter so the backend instance
    /// owning the surface never drops early.
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: ProbeAdapterInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    max_dimension: u32,
    size: (u32, u32),
    preferred_names: Vec<String>,
    /// Requested present-mode policy (the concrete mode lives in `config`).
    present_mode_request: ProbePresentMode,
    /// Wall-clock microseconds of the most recent `Surface::configure`.
    last_configure_us: u64,
    frame_index: u64,
    stopped: bool,
    /// Lazily built fullscreen textured pipeline (surface format bound).
    texture_pipeline: Option<wgpu::RenderPipeline>,
    texture_layout: Option<wgpu::BindGroupLayout>,
    texture_sampler: Option<wgpu::Sampler>,
    /// Cached bind group for the last presented texture, keyed by the full key.
    texture_bind: Option<(NativeTextureKey, wgpu::Texture, wgpu::BindGroup)>,
    /// The limits this device was created with (see
    /// [`crate::headless::negotiated_device_limits`]), including the degraded
    /// fallback verdict.
    device_limits: crate::headless::DeviceLimitsReport,
}

impl SurfaceProbePresenter {
    /// Create and configure a Vulkan-only surface for `window`.
    ///
    /// `window` is an owning `raw-window-handle` provider (for example a Tauri
    /// window or an X11 child handle); the resulting `Surface<'static>` keeps it
    /// alive, so it must outlive nothing beyond the returned presenter.
    ///
    /// Uses [`DEFAULT_PREFERRED_ADAPTER_NAMES`] as the adapter preference. See
    /// [`SurfaceProbePresenter::new_with_preferred_names`] to override it.
    pub fn new<W>(window: W, width: u32, height: u32) -> Result<Self, ProbeError>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static,
    {
        let preferred_names = DEFAULT_PREFERRED_ADAPTER_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        Self::build(
            window,
            width,
            height,
            preferred_names,
            ProbePresentMode::Fifo,
        )
    }

    /// Create a probe with an explicit present-mode policy.
    ///
    /// [`ProbePresentMode::MailboxElseImmediate`] is the single allowed
    /// present-mode comparison; it never silently falls back.
    pub fn new_with_present_mode<W>(
        window: W,
        width: u32,
        height: u32,
        present_mode: ProbePresentMode,
    ) -> Result<Self, ProbeError>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static,
    {
        let preferred_names = DEFAULT_PREFERRED_ADAPTER_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        Self::build(window, width, height, preferred_names, present_mode)
    }

    /// Create a probe with an explicit adapter-name preference list.
    ///
    /// Empty entries are ignored by the matcher. The list is only a preference;
    /// a machine with none of the names still gets a Vulkan hardware adapter.
    pub fn new_with_preferred_names<W>(
        window: W,
        width: u32,
        height: u32,
        preferred_names: &[&str],
    ) -> Result<Self, ProbeError>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static,
    {
        let preferred_names = preferred_names
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        Self::build(
            window,
            width,
            height,
            preferred_names,
            ProbePresentMode::Fifo,
        )
    }

    fn build<W>(
        window: W,
        width: u32,
        height: u32,
        preferred_names: Vec<String>,
        present_mode_request: ProbePresentMode,
    ) -> Result<Self, ProbeError>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + Send + Sync + 'static,
    {
        let instance = make_probe_instance();
        // `window` is owned for `'static`, so the surface is `Surface<'static>`
        // and keeps the window handle alive.
        let surface = instance
            .create_surface(window)
            .map_err(|error| ProbeError::SurfaceCreation(error.to_string()))?;
        Self::build_with_surface(
            instance,
            surface,
            width,
            height,
            preferred_names,
            present_mode_request,
        )
    }

    /// Create a probe surface from **raw** display/window handles.
    ///
    /// This is the entry point for hosts that own a non-`Send` native widget
    /// (for example a GTK child) and therefore cannot use the safe owning
    /// constructors: the surface does **not** own the window, and its lifetime
    /// is entirely the caller's responsibility. `wgpu`'s unsafe surface creation
    /// stays inside this crate; no `wgpu` object escapes.
    ///
    /// # Safety
    ///
    /// The caller must guarantee, for the entire lifetime of the returned
    /// presenter (until it is **fully dropped**):
    ///
    /// - `display` and `window` are a **matched, live** pair identifying the
    ///   same drawable. Two backends satisfy this: an **Xlib/Xcb** pair for the
    ///   same X server/window, **or** a **Wayland** pair of the `wl_display` and
    ///   the **app-owned** `wl_surface` presented through that display. The raw
    ///   handles are not validated here, so a mismatched or stale pair is
    ///   undefined behaviour;
    /// - the native display and window stay alive and keep their identity until
    ///   the presenter is dropped: they are never destroyed, unrealized,
    ///   re-created or given a new XID, and no surface role/lifetime change
    ///   (e.g. a Wayland surface re-role) happens while it exists;
    /// - this presenter is the **single presentation owner** of that drawable:
    ///   no other Vulkan presenter/surface writes to the same window, and for a
    ///   Wayland `wl_surface` the surface is app-owned — it is **never** a
    ///   GTK/GDK-owned surface that GDK itself attaches/commits to (no double
    ///   writer against GDK's own composition);
    /// - construction, every `frame`/`resize`, and the final drop all happen on
    ///   the host's **main thread** (the same thread that owns the native
    ///   window), matching the safe path's semantics.
    ///
    /// The returned `Surface<'static>` only *claims* `'static`; it does not keep
    /// the handles alive. Violating the above is undefined behaviour. On any
    /// error the GPU objects created so far are dropped before returning.
    pub unsafe fn new_from_raw_handles(
        display: wgpu::rwh::RawDisplayHandle,
        window: wgpu::rwh::RawWindowHandle,
        width: u32,
        height: u32,
        present_mode: ProbePresentMode,
    ) -> Result<Self, ProbeError> {
        let preferred_names = DEFAULT_PREFERRED_ADAPTER_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        let instance = make_probe_instance();
        // SAFETY: forwarded from the caller's contract documented above.
        let surface: wgpu::Surface<'static> = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display),
                raw_window_handle: window,
            })
        }
        .map_err(|error| ProbeError::SurfaceCreation(error.to_string()))?;
        Self::build_with_surface(
            instance,
            surface,
            width,
            height,
            preferred_names,
            present_mode,
        )
    }

    /// Shared factory used by the safe and unsafe constructors.
    ///
    /// Everything after the surface exists lives here, so both paths reuse the
    /// exact same adapter selection, device request, pipeline build, present-mode
    /// choice, configure and timing code. On any error path the surface, instance
    /// and (if created) device are dropped before returning, so a failed call
    /// leaves no GPU resource behind.
    fn build_with_surface(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        preferred_names: Vec<String>,
        present_mode_request: ProbePresentMode,
    ) -> Result<Self, ProbeError> {
        let adapters = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
        if adapters.is_empty() {
            return Err(ProbeError::NoVulkanAdapter(
                "no Vulkan adapter was enumerated".into(),
            ));
        }
        let infos: Vec<ProbeAdapterInfo> = adapters
            .iter()
            .map(|adapter| probe_info(adapter, &surface))
            .collect();
        let ranked = rank_probe_adapters(&infos, &preferred_names);
        let index = ranked.first().copied().ok_or_else(|| {
            ProbeError::NoVulkanAdapter(format!(
                "no Vulkan hardware adapter can present to the surface (enumerated: {})",
                infos
                    .iter()
                    .map(|info| format!("{}/{}", info.backend, info.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        let adapter = adapters[index].clone();
        let adapter_info = infos[index].clone();

        let capabilities = surface.get_capabilities(&adapter);
        let format =
            choose_surface_format(&capabilities.formats).ok_or(ProbeError::NoSupportedFormat)?;
        let max_dimension = adapter.limits().max_texture_dimension_2d;
        validate_size(width, height, max_dimension)?;

        let negotiated = crate::headless::request_device_with_negotiated_limits(
            &adapter,
            "cad-surface-probe-device",
        )
        .map_err(ProbeError::DeviceRequest)?;
        let device = negotiated.device;
        let queue = negotiated.queue;
        // One-off startup diagnostic (never per frame): the negotiated capacity
        // is what explains the pulled budget in `[probe] actual-render`'s
        // `pulled=…` token, which reports the same `max_buffer_size` as
        // `budget_bytes`. A degraded line means the device fell back to the
        // WebGPU defaults and the pulled budget is 256 MiB again.
        eprintln!("[probe] device-limits {}", negotiated.limits.summary());

        let (pipeline, bind_group, uniform) = build_probe_pipeline(&device, format);

        let supported_present_modes = capabilities.present_modes.clone();
        let chosen_present_mode = match present_mode_request {
            ProbePresentMode::Fifo => wgpu::PresentMode::Fifo,
            ProbePresentMode::MailboxElseImmediate => {
                if supported_present_modes.contains(&wgpu::PresentMode::Mailbox) {
                    wgpu::PresentMode::Mailbox
                } else {
                    wgpu::PresentMode::Immediate
                }
            }
        };
        eprintln!(
            "[probe] present mode requested={present_mode_request:?} supported={supported_present_modes:?} chosen={chosen_present_mode:?}"
        );

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: chosen_present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };
        let configure_started = std::time::Instant::now();
        surface.configure(&device, &config);
        let last_configure_us = configure_started.elapsed().as_micros() as u64;
        eprintln!("[probe] initial configure took {last_configure_us} us");

        Ok(Self {
            _instance: instance,
            surface,
            adapter: adapter_info,
            device,
            queue,
            config,
            pipeline,
            bind_group,
            uniform,
            max_dimension,
            size: (width, height),
            preferred_names,
            present_mode_request,
            last_configure_us,
            frame_index: 0,
            stopped: false,
            texture_pipeline: None,
            texture_layout: None,
            texture_sampler: None,
            texture_bind: None,
            device_limits: negotiated.limits,
        })
    }

    /// Reconfigure the surface for a new physical size.
    ///
    /// Both dimensions must be non-zero and within the adapter's texture limit;
    /// otherwise an explicit [`ProbeError`] is returned instead of a configure
    /// panic. A no-op resize leaves the surface untouched.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), ProbeError> {
        validate_size(width, height, self.max_dimension)?;
        if (width, height) == self.size {
            return Ok(());
        }
        self.config.width = width;
        self.config.height = height;
        let started = std::time::Instant::now();
        self.surface.configure(&self.device, &self.config);
        self.last_configure_us = started.elapsed().as_micros() as u64;
        eprintln!(
            "[probe] resize configure {width}x{height} took {} us",
            self.last_configure_us
        );
        self.size = (width, height);
        Ok(())
    }

    /// Wall-clock microseconds of the most recent `Surface::configure`.
    pub fn last_configure_us(&self) -> u64 {
        self.last_configure_us
    }

    /// Render and present one probe frame with the given marker offset.
    ///
    /// `marker_offset` must be finite; it is wrapped into `[0, 1)` and used to
    /// translate the colour block, so frames are visually distinguishable without
    /// any readback. A recoverable `Outdated` acquire is retried once after a
    /// reconfigure; every other acquire failure is returned explicitly. Drawing
    /// after [`SurfaceProbePresenter::stop`] returns [`ProbeError::Stopped`].
    pub fn frame(&mut self, marker_offset: f32) -> Result<ProbeFrameStats, ProbeError> {
        let frame_started = std::time::Instant::now();
        if self.stopped {
            return Err(ProbeError::Stopped);
        }
        let marker = sanitize_marker(marker_offset)?;
        let (width, height) = self.size;
        let mut timings = ProbeStageTimings::default();

        // Params uniform: resolution (vec2, 8 bytes) + marker (f32) + pad (f32).
        let mut params = [0u8; 16];
        params[0..4].copy_from_slice(&(width as f32).to_le_bytes());
        params[4..8].copy_from_slice(&(height as f32).to_le_bytes());
        params[8..12].copy_from_slice(&marker.to_le_bytes());
        let stage = std::time::Instant::now();
        self.queue.write_buffer(&self.uniform, 0, &params);
        timings.write_buffer_us = stage.elapsed().as_micros() as u64;

        let acquire_started = std::time::Instant::now();
        let mut configure_us = 0u64;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // `Outdated` is recoverable: the documented remedy is to reconfigure
            // and acquire again. Bound it to a single retry so a persistent
            // mismatch still fails explicitly rather than looping.
            wgpu::CurrentSurfaceTexture::Outdated => {
                let stage = std::time::Instant::now();
                self.surface.configure(&self.device, &self.config);
                configure_us += stage.elapsed().as_micros() as u64;
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        return Err(ProbeError::Acquire("outdated"))
                    }
                    wgpu::CurrentSurfaceTexture::Timeout => {
                        return Err(ProbeError::Acquire("timeout"))
                    }
                    wgpu::CurrentSurfaceTexture::Occluded => {
                        return Err(ProbeError::Acquire("occluded"))
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        return Err(ProbeError::DeviceLost(
                            "surface lost; rebuild the presenter".into(),
                        ))
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        return Err(ProbeError::Acquire("validation"))
                    }
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout => return Err(ProbeError::Acquire("timeout")),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(ProbeError::Acquire("occluded")),
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(ProbeError::DeviceLost(
                    "surface lost; rebuild the presenter".into(),
                ))
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(ProbeError::Acquire("validation"))
            }
        };
        timings.acquire_us = acquire_started.elapsed().as_micros() as u64;
        timings.configure_us = configure_us;

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cad-surface-probe-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-surface-probe-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.03,
                            b: 0.05,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let stage = std::time::Instant::now();
        self.queue.submit(Some(encoder.finish()));
        timings.submit_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        if let Some(error) = collect_error_scope_bounded(&self.device, scope, SCOPE_COLLECT_BUDGET)
        {
            // `frame` is dropped without present, which discards the swapchain
            // image; the error is surfaced rather than hidden.
            return Err(ProbeError::Frame(format!(
                "probe frame rejected by GPU validation: {error}"
            )));
        }
        timings.scope_pop_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        self.queue.present(frame);
        timings.present_us = stage.elapsed().as_micros() as u64;

        self.frame_index += 1;
        timings.total_us = frame_started.elapsed().as_micros() as u64;
        Ok(ProbeFrameStats {
            frame_index: self.frame_index,
            marker_offset: marker,
            surface_size: (width, height),
            timings,
        })
    }

    /// Clone the device/queue this presenter uses.
    ///
    /// A host uses this to attach its own session to the **same** device (the
    /// app gets `wgpu` types through this crate's re-exports, so it needs no
    /// direct `wgpu` dependency).
    pub fn device_queue(&self) -> (wgpu::Device, wgpu::Queue) {
        (self.device.clone(), self.queue.clone())
    }

    /// Present a native CAD texture directly on the surface (no CPU readback, no
    /// extra CAD surface, no second event loop).
    ///
    /// `texture` must live on the **same device** as this presenter; the app
    /// guarantees that by attaching its session through [`Self::device_queue`].
    /// The bind group is cached by `key`; a changed `device_epoch`,
    /// `texture_revision` or `size` rebinds. Validation (format/usage/size) and
    /// any GPU validation error are explicit [`ProbeError`]s.
    pub fn present_texture(
        &mut self,
        texture: &wgpu::Texture,
        key: NativeTextureKey,
    ) -> Result<ProbeFrameStats, ProbeError> {
        let frame_started = std::time::Instant::now();
        if self.stopped {
            return Err(ProbeError::Stopped);
        }
        validate_texture_key(&key, self.max_dimension)?;
        validate_texture(texture, &key)?;
        let mut timings = ProbeStageTimings::default();

        let stage = std::time::Instant::now();
        self.ensure_texture_bind_group(texture, &key);
        timings.write_buffer_us = stage.elapsed().as_micros() as u64;

        let (frame, configure_us) = self.acquire_frame()?;
        timings.acquire_us = frame_started.elapsed().as_micros() as u64;
        timings.configure_us = configure_us;

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cad-texture-encoder"),
            });
        {
            let pipeline = self
                .texture_pipeline
                .as_ref()
                .ok_or_else(|| ProbeError::Frame("texture pipeline missing".into()))?;
            let bind = self
                .texture_bind
                .as_ref()
                .map(|(_, _, bind)| bind)
                .ok_or_else(|| ProbeError::Frame("texture bind group missing".into()))?;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-texture-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        let stage = std::time::Instant::now();
        self.queue.submit(Some(encoder.finish()));
        timings.submit_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        if let Some(error) = collect_error_scope_bounded(&self.device, scope, SCOPE_COLLECT_BUDGET)
        {
            return Err(ProbeError::Frame(format!(
                "native texture frame rejected by GPU validation: {error}"
            )));
        }
        timings.scope_pop_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        self.queue.present(frame);
        timings.present_us = stage.elapsed().as_micros() as u64;

        self.frame_index += 1;
        timings.total_us = frame_started.elapsed().as_micros() as u64;
        Ok(ProbeFrameStats {
            frame_index: self.frame_index,
            marker_offset: 0.0,
            surface_size: self.size,
            timings,
        })
    }

    /// Acquire a surface frame, retrying once after a reconfigure on `Outdated`.
    /// Returns the frame and the microseconds spent reconfiguring.
    fn acquire_frame(&self) -> Result<(wgpu::SurfaceTexture, u64), ProbeError> {
        let mut configure_us = 0u64;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated => {
                let stage = std::time::Instant::now();
                self.surface.configure(&self.device, &self.config);
                configure_us += stage.elapsed().as_micros() as u64;
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Outdated => {
                        return Err(ProbeError::Acquire("outdated"))
                    }
                    wgpu::CurrentSurfaceTexture::Timeout => {
                        return Err(ProbeError::Acquire("timeout"))
                    }
                    wgpu::CurrentSurfaceTexture::Occluded => {
                        return Err(ProbeError::Acquire("occluded"))
                    }
                    wgpu::CurrentSurfaceTexture::Lost => {
                        return Err(ProbeError::DeviceLost(
                            "surface lost; rebuild the presenter".into(),
                        ))
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        return Err(ProbeError::Acquire("validation"))
                    }
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout => return Err(ProbeError::Acquire("timeout")),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(ProbeError::Acquire("occluded")),
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(ProbeError::DeviceLost(
                    "surface lost; rebuild the presenter".into(),
                ))
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(ProbeError::Acquire("validation"))
            }
        };
        Ok((frame, configure_us))
    }

    /// Build the textured pipeline on first use and (re)bind the CAD texture.
    /// The bind group is cached by the full [`NativeTextureKey`].
    fn ensure_texture_bind_group(&mut self, texture: &wgpu::Texture, key: &NativeTextureKey) {
        // Reuse only when BOTH the key and the texture identity match: the key
        // alone is not unique across engine sessions, and `wgpu::Texture`
        // equality is a stable identity comparison.
        let reusable = self
            .texture_bind
            .as_ref()
            .is_some_and(|(cached_key, cached_texture, _)| {
                binding_cache_is_reusable(cached_key, key, cached_texture == texture)
            });
        if reusable {
            return;
        }
        if self.texture_pipeline.is_none() {
            let (pipeline, layout, sampler) =
                build_texture_pipeline(&self.device, self.config.format);
            self.texture_pipeline = Some(pipeline);
            self.texture_layout = Some(layout);
            self.texture_sampler = Some(sampler);
        }
        let layout = self
            .texture_layout
            .as_ref()
            .expect("texture layout exists once the pipeline is built");
        let sampler = self
            .texture_sampler
            .as_ref()
            .expect("texture sampler exists once the pipeline is built");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad-texture-bind-group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        self.texture_bind = Some((*key, texture.clone(), bind));
    }

    /// Drop the cached native-texture bind group.
    ///
    /// A host calls this when it replaces the render session or after a failure,
    /// so a later frame can never reuse a binding that belongs to a dead device
    /// or a different document.
    pub fn clear_binding_cache(&mut self) {
        self.texture_bind = None;
    }

    /// Stop drawing. Subsequent [`SurfaceProbePresenter::frame`] calls return
    /// [`ProbeError::Stopped`]. Rebuilding requires a new presenter.
    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// Whether [`SurfaceProbePresenter::stop`] has been called.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// The adapter the probe selected (backend, name, device type, driver).
    pub fn adapter(&self) -> &ProbeAdapterInfo {
        &self.adapter
    }

    /// The negotiated device limits and whether the fallback to WebGPU defaults
    /// was taken (see [`crate::headless::DeviceLimitsReport`]).
    pub fn device_limits(&self) -> &crate::headless::DeviceLimitsReport {
        &self.device_limits
    }

    /// The configured surface texture format.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// The current surface size, in physical pixels.
    pub fn surface_size(&self) -> (u32, u32) {
        self.size
    }

    /// The configured present mode.
    pub fn present_mode(&self) -> wgpu::PresentMode {
        self.config.present_mode
    }

    /// The requested present-mode policy (as constructed).
    pub fn present_mode_request(&self) -> ProbePresentMode {
        self.present_mode_request
    }

    /// The adapter-name preference list, as constructed.
    pub fn preferred_adapter_names(&self) -> &[String] {
        &self.preferred_names
    }
}

impl std::fmt::Debug for SurfaceProbePresenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceProbePresenter")
            .field("adapter", &self.adapter)
            .field("surface_format", &self.config.format)
            .field("surface_size", &self.size)
            .field("present_mode", &self.config.present_mode)
            .field("frame_index", &self.frame_index)
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

/// Collect the validation error scope with a bounded, non-blocking wait.
///
/// Polls the device in small steps up to `budget`; if the scope has not resolved
/// by then it logs and returns `None`, so a slow device poll cannot hold the
/// frame. Validation errors that do resolve in time are still reported.
fn collect_error_scope_bounded(
    device: &wgpu::Device,
    guard: wgpu::ErrorScopeGuard,
    budget: std::time::Duration,
) -> Option<wgpu::Error> {
    let mut pop = std::pin::pin!(guard.pop());
    let deadline = std::time::Instant::now() + budget;
    loop {
        // `poll_once` returns `None` while the inner future is still pending, so
        // this never blocks on the scope.
        if let Some(error) = block_on(futures_lite::future::poll_once(pop.as_mut())) {
            return error;
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("[probe] error scope did not resolve within {budget:?}; skipping collection");
            return None;
        }
        // Give the device a bounded chance to run its callbacks.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_millis(2)),
        });
    }
}

/// Create the Vulkan-only probe instance (shared by the safe and unsafe paths).
fn make_probe_instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    wgpu::Instance::new(descriptor)
}

/// Whether a cached native-texture binding may be reused: the key must match
/// **and** the texture identity must be the same. The key alone is not unique
/// across engine sessions.
pub(crate) fn binding_cache_is_reusable(
    cached_key: &NativeTextureKey,
    key: &NativeTextureKey,
    same_texture: bool,
) -> bool {
    cached_key == key && same_texture
}

/// Validate a native texture against its key (format/usage/size).
pub(crate) fn validate_texture(
    texture: &wgpu::Texture,
    key: &NativeTextureKey,
) -> Result<(), ProbeError> {
    let format = texture.format();
    if !texture_format_is_supported(format) {
        return Err(ProbeError::TextureInvalid(format!(
            "unsupported format {format:?}; expected 8-bit RGBA/BGRA (sRGB or linear)"
        )));
    }
    if !texture
        .usage()
        .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Err(ProbeError::TextureInvalid(
            "usage must include TEXTURE_BINDING".into(),
        ));
    }
    let size = texture.size();
    if (size.width, size.height) != key.size {
        return Err(ProbeError::TextureInvalid(format!(
            "texture is {}x{} but key says {}x{}",
            size.width, size.height, key.size.0, key.size.1
        )));
    }
    Ok(())
}

/// Build the fullscreen textured pipeline, its bind-group layout and sampler.
fn build_texture_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cad-texture-shader"),
        source: wgpu::ShaderSource::Wgsl(TEXTURE_SHADER.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cad-texture-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cad-texture-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cad-texture-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                // Opaque display: the CAD texture is expected to be opaque.
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cad-texture-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    (pipeline, layout, sampler)
}

/// Build the probe shader, render pipeline, uniform buffer and bind group.
fn build_probe_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroup, wgpu::Buffer) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cad-surface-probe-shader"),
        source: wgpu::ShaderSource::Wgsl(PROBE_SHADER.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cad-surface-probe-uniform-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cad-surface-probe-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cad-surface-probe-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        // The probe is a flat 2D overlay; it needs no depth attachment.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cad-surface-probe-uniform"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cad-surface-probe-bind-group"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    (pipeline, bind_group, uniform)
}

/// Describe `adapter` and whether it can present to `surface`.
fn probe_info(adapter: &wgpu::Adapter, surface: &wgpu::Surface<'_>) -> ProbeAdapterInfo {
    let info = adapter.get_info();
    let capabilities = surface.get_capabilities(adapter);
    ProbeAdapterInfo {
        backend: info.backend.to_str().to_string(),
        name: info.name,
        device_type: device_type_str(info.device_type).to_string(),
        driver: info.driver,
        driver_info: info.driver_info,
        surface_compatible: !capabilities.formats.is_empty(),
    }
}

fn device_type_str(device_type: wgpu::DeviceType) -> &'static str {
    match device_type {
        wgpu::DeviceType::Cpu => "cpu",
        wgpu::DeviceType::IntegratedGpu => "integrated_gpu",
        wgpu::DeviceType::DiscreteGpu => "discrete_gpu",
        wgpu::DeviceType::VirtualGpu => "virtual_gpu",
        wgpu::DeviceType::Other => "other",
    }
}

/// Rank adapters the probe is willing to use, most preferred first.
///
/// Eligibility is hard: the backend must be Vulkan, the device type must not be
/// `cpu`, and the adapter must be able to present to the surface. Ordering is
/// soft: a name matching `preferred_names` (case-insensitive substring, in list
/// order) wins, then a discrete GPU over an integrated one, then enumeration
/// order. The result is deterministic and stable.
pub(crate) fn rank_probe_adapters<S: AsRef<str>>(
    infos: &[ProbeAdapterInfo],
    preferred_names: &[S],
) -> Vec<usize> {
    let mut eligible: Vec<usize> = infos
        .iter()
        .enumerate()
        .filter(|(_, info)| {
            info.backend == REQUIRED_BACKEND && info.device_type != "cpu" && info.surface_compatible
        })
        .map(|(index, _)| index)
        .collect();
    eligible.sort_by_key(|&index| {
        let info = &infos[index];
        (
            preferred_rank(&info.name, preferred_names),
            device_rank(&info.device_type),
            index,
        )
    });
    eligible
}

/// Position of the first preferred name contained in `name`, or `usize::MAX`.
fn preferred_rank<S: AsRef<str>>(name: &str, preferred_names: &[S]) -> usize {
    let lowered = name.to_ascii_lowercase();
    preferred_names
        .iter()
        .position(|candidate| {
            let needle = candidate.as_ref().to_ascii_lowercase();
            !needle.is_empty() && lowered.contains(&needle)
        })
        .unwrap_or(usize::MAX)
}

/// Sort weight inside a backend: discrete GPUs before integrated, others last.
fn device_rank(device_type: &str) -> u8 {
    match device_type {
        "discrete_gpu" => 0,
        "integrated_gpu" => 1,
        _ => 2,
    }
}

/// Deterministic surface-format policy.
///
/// Prefer an 8-bit sRGB format (RGBA then BGRA); otherwise fall back to a
/// non-sRGB 8-bit format; otherwise take whatever the surface listed first.
/// `None` means the surface reported no usable format.
pub(crate) fn choose_surface_format(
    formats: &[wgpu::TextureFormat],
) -> Option<wgpu::TextureFormat> {
    const PREFERRED: [wgpu::TextureFormat; 4] = [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8Unorm,
    ];
    PREFERRED
        .iter()
        .copied()
        .find(|format| formats.contains(format))
        .or_else(|| formats.first().copied())
}

/// Validate a requested surface size against the adapter's texture limit.
pub(crate) fn validate_size(width: u32, height: u32, max_dimension: u32) -> Result<(), ProbeError> {
    if width == 0 || height == 0 {
        return Err(ProbeError::InvalidSize { width, height });
    }
    if width > max_dimension || height > max_dimension {
        return Err(ProbeError::SizeExceedsLimit {
            width,
            height,
            limit: max_dimension,
        });
    }
    Ok(())
}

/// Validate and wrap a marker offset into `[0, 1)`.
pub(crate) fn sanitize_marker(offset: f32) -> Result<f32, ProbeError> {
    if !offset.is_finite() {
        return Err(ProbeError::InvalidMarker(offset));
    }
    Ok(offset.rem_euclid(MARKER_PERIOD))
}

/// The probe shader: a fullscreen triangle with a grid, a translating colour
/// block and two corner markers. It deliberately draws no CAD geometry.
/// Fullscreen textured shader: samples a native CAD texture directly to the
/// surface (no CPU readback, no extra CAD surface). Opaque display; the CAD
/// texture is expected to be opaque (premultiplied alpha is not used here).
const TEXTURE_SHADER: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var cad_texture: texture_2d<f32>;
@group(0) @binding(1) var cad_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vertex_index];
    var out: VsOut;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    // Texture origin is top-left: clip y=+1 is the screen top, so v must be
    // (1 - y)/2. Getting this wrong flips the image vertically.
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(cad_texture, cad_sampler, in.uv);
}
"#;

const PROBE_SHADER: &str = r#"
struct ProbeParams {
    resolution: vec2<f32>,
    marker: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> params: ProbeParams;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vertex_index];
    var out: VsOut;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    out.uv = (xy + vec2<f32>(1.0, 1.0)) * 0.5;
    return out;
}

fn grid_line(coord: f32) -> f32 {
    let width = fwidth(coord);
    let distance = abs(fract(coord) - 0.5);
    return 1.0 - smoothstep(0.5 - width * 1.5, 0.5, distance);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let divisions = 10.0;
    let grid = clamp(
        grid_line(in.uv.x * divisions) + grid_line(in.uv.y * divisions),
        0.0,
        1.0,
    );
    var color = mix(vec3<f32>(0.06, 0.07, 0.10), vec3<f32>(0.30, 0.35, 0.45), grid);

    // Translating colour block: identifies frame progress, not drawing content.
    let marker = vec2<f32>(params.marker, 0.5);
    let block = step(abs(in.uv.x - marker.x), 0.06) * step(abs(in.uv.y - marker.y), 0.06);
    color = mix(color, vec3<f32>(1.0, 0.55, 0.10), block);

    // Static corner markers for an orientation check.
    let top_left = step(in.uv.x, 0.06) * step(in.uv.y, 0.06);
    let bottom_right = step(0.94, in.uv.x) * step(0.94, in.uv.y);
    color = mix(color, vec3<f32>(0.90, 0.10, 0.10), top_left);
    color = mix(color, vec3<f32>(0.10, 0.80, 0.30), bottom_right);

    return vec4<f32>(color, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn info(
        backend: &str,
        device_type: &str,
        name: &str,
        surface_compatible: bool,
    ) -> ProbeAdapterInfo {
        ProbeAdapterInfo {
            backend: backend.into(),
            name: name.into(),
            device_type: device_type.into(),
            driver: "test-driver".into(),
            driver_info: String::new(),
            surface_compatible,
        }
    }

    #[test]
    fn rank_rejects_cpu_non_vulkan_and_incompatible() {
        let infos = vec![
            info("vulkan", "cpu", "P620", true),
            info("gl", "discrete_gpu", "P620", true),
            info("vulkan", "discrete_gpu", "P620", false),
        ];
        assert!(
            rank_probe_adapters(&infos, &["P620"]).is_empty(),
            "cpu, non-Vulkan and surface-incompatible adapters must all be rejected"
        );
    }

    #[test]
    fn rank_prefers_named_adapter_before_other_discrete() {
        let infos = vec![
            info("vulkan", "discrete_gpu", "Quadro RTX 5000", true),
            info("vulkan", "integrated_gpu", "Intel UHD Graphics", true),
            info("vulkan", "discrete_gpu", "Quadro P620", true),
        ];
        assert_eq!(
            rank_probe_adapters(&infos, &["P620"]),
            vec![2, 0, 1],
            "the preferred name wins, then discrete before integrated"
        );
    }

    #[test]
    fn rank_preferred_match_is_case_insensitive_substring() {
        let infos = vec![
            info("vulkan", "discrete_gpu", "NVIDIA Quadro p620", true),
            info("vulkan", "discrete_gpu", "Other Discrete", true),
        ];
        assert_eq!(rank_probe_adapters(&infos, &["P620"]), vec![0, 1]);
    }

    #[test]
    fn rank_falls_back_when_no_preferred_name_matches() {
        let infos = vec![
            info("vulkan", "integrated_gpu", "Intel UHD Graphics", true),
            info("vulkan", "discrete_gpu", "Quadro RTX", true),
        ];
        assert_eq!(
            rank_probe_adapters(&infos, &["P620"]),
            vec![1, 0],
            "no P620 present: a discrete GPU is still preferred over an integrated one"
        );
    }

    #[test]
    fn rank_falls_back_to_integrated_when_only_integrated() {
        let infos = vec![info("vulkan", "integrated_gpu", "Intel UHD Graphics", true)];
        assert_eq!(
            rank_probe_adapters(&infos, &["P620"]),
            vec![0],
            "a lone integrated Vulkan GPU is still a valid hardware target"
        );
    }

    #[test]
    fn rank_is_stable_for_equal_keys() {
        let infos = vec![
            info("vulkan", "discrete_gpu", "A", true),
            info("vulkan", "discrete_gpu", "B", true),
        ];
        let none: [&str; 0] = [];
        assert_eq!(rank_probe_adapters(&infos, &none), vec![0, 1]);
    }

    #[test]
    fn choose_format_prefers_srgb_when_available() {
        let formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ];
        assert_eq!(
            choose_surface_format(&formats),
            Some(wgpu::TextureFormat::Rgba8UnormSrgb)
        );
    }

    #[test]
    fn choose_format_prefers_bgra_srgb_over_non_srgb() {
        let formats = [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ];
        assert_eq!(
            choose_surface_format(&formats),
            Some(wgpu::TextureFormat::Bgra8UnormSrgb)
        );
    }

    #[test]
    fn choose_format_falls_back_to_first_supported() {
        let formats = [
            wgpu::TextureFormat::Rgb10a2Unorm,
            wgpu::TextureFormat::Bgra8Unorm,
        ];
        // Bgra8Unorm is in the sRGB/8-bit preference list, so it wins here.
        assert_eq!(
            choose_surface_format(&formats),
            Some(wgpu::TextureFormat::Bgra8Unorm)
        );
        let unknown = [wgpu::TextureFormat::Rgb10a2Unorm];
        assert_eq!(
            choose_surface_format(&unknown),
            Some(wgpu::TextureFormat::Rgb10a2Unorm),
            "with no known format, the surface's first listed format is used"
        );
    }

    #[test]
    fn choose_format_none_when_empty() {
        assert_eq!(choose_surface_format(&[]), None);
    }

    /// The unsafe raw-handle constructor's signature is the next lane's contract.
    /// Pinning it here (by reference, never called) makes an accidental change a
    /// compile error in this crate rather than a silent break for the caller.
    #[test]
    fn raw_handle_constructor_signature_is_stable() {
        let _constructor: unsafe fn(
            wgpu::rwh::RawDisplayHandle,
            wgpu::rwh::RawWindowHandle,
            u32,
            u32,
            ProbePresentMode,
        ) -> Result<SurfaceProbePresenter, ProbeError> =
            SurfaceProbePresenter::new_from_raw_handles;
    }

    #[test]
    fn binding_cache_requires_key_and_texture_identity() {
        let a = NativeTextureKey {
            device_epoch: 1,
            texture_revision: 1,
            size: (64, 64),
        };
        let b = NativeTextureKey {
            device_epoch: 1,
            texture_revision: 1,
            size: (64, 64),
        };
        // Same key + same texture -> reusable.
        assert!(binding_cache_is_reusable(&a, &b, true));
        // Same key + different texture identity -> NOT reusable (the collision
        // the key alone cannot detect).
        assert!(!binding_cache_is_reusable(&a, &b, false));
        let c = NativeTextureKey {
            device_epoch: 2,
            texture_revision: 1,
            size: (64, 64),
        };
        assert!(!binding_cache_is_reusable(&a, &c, true));
    }

    #[test]
    fn texture_key_validation() {
        assert!(validate_texture_key(
            &NativeTextureKey {
                device_epoch: 0,
                texture_revision: 0,
                size: (0, 10)
            },
            8192
        )
        .is_err());
        assert!(validate_texture_key(
            &NativeTextureKey {
                device_epoch: 0,
                texture_revision: 0,
                size: (9000, 10)
            },
            8192
        )
        .is_err());
        assert!(validate_texture_key(
            &NativeTextureKey {
                device_epoch: 1,
                texture_revision: 2,
                size: (960, 439)
            },
            8192
        )
        .is_ok());
    }

    #[test]
    fn texture_format_support_is_explicit() {
        for format in [
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Bgra8Unorm,
        ] {
            assert!(
                texture_format_is_supported(format),
                "{format:?} must be supported"
            );
        }
        assert!(!texture_format_is_supported(
            wgpu::TextureFormat::Rgba16Float
        ));
        assert!(!texture_format_is_supported(
            wgpu::TextureFormat::Depth32Float
        ));
    }

    #[test]
    fn texture_shader_is_valid_wgsl() {
        let module = naga::front::wgsl::parse_str(TEXTURE_SHADER).expect("texture shader parses");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        validator
            .validate(&module)
            .expect("texture shader validates");
        assert!(module
            .entry_points
            .iter()
            .any(|e| e.name == "vs_main" && e.stage == naga::ShaderStage::Vertex));
        assert!(module
            .entry_points
            .iter()
            .any(|e| e.name == "fs_main" && e.stage == naga::ShaderStage::Fragment));
    }

    #[test]
    fn validate_size_rejects_zero_dimensions() {
        assert_eq!(
            validate_size(0, 100, 8192),
            Err(ProbeError::InvalidSize {
                width: 0,
                height: 100
            })
        );
        assert_eq!(
            validate_size(100, 0, 8192),
            Err(ProbeError::InvalidSize {
                width: 100,
                height: 0
            })
        );
    }

    #[test]
    fn validate_size_rejects_over_limit() {
        assert_eq!(
            validate_size(9000, 100, 8192),
            Err(ProbeError::SizeExceedsLimit {
                width: 9000,
                height: 100,
                limit: 8192
            })
        );
        assert_eq!(
            validate_size(100, 8193, 8192),
            Err(ProbeError::SizeExceedsLimit {
                width: 100,
                height: 8193,
                limit: 8192
            })
        );
    }

    #[test]
    fn validate_size_accepts_boundary_and_unit() {
        assert_eq!(validate_size(1, 1, 8192), Ok(()));
        assert_eq!(
            validate_size(8192, 8192, 8192),
            Ok(()),
            "the adapter limit itself is allowed"
        );
    }

    #[test]
    fn sanitize_marker_rejects_non_finite() {
        assert!(matches!(
            sanitize_marker(f32::NAN),
            Err(ProbeError::InvalidMarker(value)) if value.is_nan()
        ));
        assert!(matches!(
            sanitize_marker(f32::INFINITY),
            Err(ProbeError::InvalidMarker(_))
        ));
        assert!(matches!(
            sanitize_marker(f32::NEG_INFINITY),
            Err(ProbeError::InvalidMarker(_))
        ));
    }

    #[test]
    fn sanitize_marker_wraps_into_unit_range() {
        assert_eq!(sanitize_marker(0.25), Ok(0.25));
        assert_eq!(sanitize_marker(1.25), Ok(0.25));
        assert_eq!(sanitize_marker(-0.25), Ok(0.75));
        assert_eq!(sanitize_marker(0.0), Ok(0.0));
    }
}

/// Real-GPU evidence: opens an Xlib window on the host display, drives the probe
/// and checks it presents on a hardware Vulkan adapter.
///
/// Ignored by default (needs a real X11/XWayland window + a hardware GPU).
/// Run once, serially:
/// `cargo test -p cad-render-wgpu --lib probe_presents_grid_and_rebuilds -- --ignored --test-threads=1 --nocapture`
#[cfg(all(test, target_os = "linux"))]
mod linux_gpu_tests {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use wgpu::rwh::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
        RawWindowHandle, WindowHandle, XlibDisplayHandle, XlibWindowHandle,
    };

    #[link(name = "X11")]
    extern "C" {
        fn XOpenDisplay(name: *const i8) -> *mut c_void;
        fn XDefaultRootWindow(display: *mut c_void) -> u64;
        fn XCreateSimpleWindow(
            display: *mut c_void,
            parent: u64,
            x: i32,
            y: i32,
            width: u32,
            height: u32,
            border_width: u32,
            border: u64,
            background: u64,
        ) -> u64;
        fn XMapWindow(display: *mut c_void, window: u64) -> i32;
        fn XResizeWindow(display: *mut c_void, window: u64, width: u32, height: u32) -> i32;
        fn XFlush(display: *mut c_void) -> i32;
        fn XDestroyWindow(display: *mut c_void, window: u64) -> i32;
        fn XCloseDisplay(display: *mut c_void) -> i32;
    }

    /// A minimal owning Xlib connection + window, exposed through
    /// raw-window-handle so `SurfaceProbePresenter` can own it.
    struct XlibWindow {
        display: *mut c_void,
        window: u64,
        screen: i32,
    }

    // The connection is only used from the single test thread that owns the
    // window; the probe requires `Send + Sync` providers, so assert it here.
    unsafe impl Send for XlibWindow {}
    unsafe impl Sync for XlibWindow {}

    impl XlibWindow {
        fn open(width: u32, height: u32) -> Option<Self> {
            unsafe {
                let display = XOpenDisplay(std::ptr::null());
                if display.is_null() {
                    return None;
                }
                let root = XDefaultRootWindow(display);
                let window = XCreateSimpleWindow(display, root, 0, 0, width, height, 0, 0, 0);
                if window == 0 {
                    XCloseDisplay(display);
                    return None;
                }
                XMapWindow(display, window);
                XFlush(display);
                Some(Self {
                    display,
                    window,
                    screen: 0,
                })
            }
        }
    }

    impl Drop for XlibWindow {
        fn drop(&mut self) {
            unsafe {
                XDestroyWindow(self.display, self.window);
                XCloseDisplay(self.display);
            }
        }
    }

    impl HasWindowHandle for XlibWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = RawWindowHandle::Xlib(XlibWindowHandle::new(self.window));
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    impl HasDisplayHandle for XlibWindow {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            let raw = RawDisplayHandle::Xlib(XlibDisplayHandle::new(
                NonNull::new(self.display),
                self.screen,
            ));
            Ok(unsafe { DisplayHandle::borrow_raw(raw) })
        }
    }

    #[test]
    #[ignore = "requires a real X11/XWayland window and a hardware Vulkan GPU"]
    fn probe_presents_grid_and_rebuilds() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("SKIP: DISPLAY is not set (no X11/XWayland window available)");
            return;
        }
        let window = match XlibWindow::open(320, 240) {
            Some(window) => window,
            None => {
                eprintln!("SKIP: could not open an Xlib display");
                return;
            }
        };
        let raw_display = window.display;
        let raw_window = window.window;

        let mut presenter = SurfaceProbePresenter::new(window, 320, 240)
            .expect("the surface probe must initialize on a real GPU");
        eprintln!(
            "probe adapter: backend={} name={} device_type={}",
            presenter.adapter().backend,
            presenter.adapter().name,
            presenter.adapter().device_type
        );
        assert_eq!(
            presenter.adapter().backend,
            "vulkan",
            "the probe forces Vulkan"
        );
        assert_ne!(
            presenter.adapter().device_type,
            "cpu",
            "the probe must reject software/fallback adapters"
        );
        assert_eq!(presenter.surface_size(), (320, 240));

        let first = presenter
            .frame(0.0)
            .expect("first probe frame must present");
        let second = presenter
            .frame(0.5)
            .expect("second probe frame must present");
        assert_eq!(second.frame_index, first.frame_index + 1);
        assert_eq!(second.marker_offset, 0.5);

        // A real host resizes its window together with the surface; keep them in
        // step so the swapchain is not reported `Outdated`.
        unsafe {
            XResizeWindow(raw_display, raw_window, 640, 480);
            XFlush(raw_display);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        presenter
            .resize(640, 480)
            .expect("resize must reconfigure the surface");
        let resized = presenter
            .frame(0.25)
            .expect("frame after resize must present");
        assert_eq!(resized.surface_size, (640, 480));

        assert!(
            presenter.resize(0, 480).is_err(),
            "a zero dimension must be an explicit error, not a configure panic"
        );

        // Destroy and rebuild the whole lifecycle from a fresh window.
        drop(presenter);
        let window = XlibWindow::open(160, 120).expect("second Xlib window");
        let mut rebuilt =
            SurfaceProbePresenter::new(window, 160, 120).expect("the probe must rebuild cleanly");
        assert!(rebuilt.frame(0.1).is_ok());

        rebuilt.stop();
        assert!(rebuilt.is_stopped());
        assert!(matches!(rebuilt.frame(0.2), Err(ProbeError::Stopped)));
    }

    /// Stable control: run continuously for 10 s on a plain Xlib window and
    /// report per-stage timings. At least 300 frames must present (>=30 fps).
    #[test]
    #[ignore = "requires a real X11/XWayland window and a hardware Vulkan GPU"]
    fn probe_continuous_frames_stage_timing() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("SKIP: DISPLAY is not set (no X11/XWayland window available)");
            return;
        }
        let window = match XlibWindow::open(320, 240) {
            Some(window) => window,
            None => {
                eprintln!("SKIP: could not open an Xlib display");
                return;
            }
        };
        let present_mode = match std::env::var("YACR_PROBE_PRESENT_MODE").as_deref() {
            Ok("mailbox") => ProbePresentMode::MailboxElseImmediate,
            _ => ProbePresentMode::Fifo,
        };
        let mut presenter =
            SurfaceProbePresenter::new_with_present_mode(window, 320, 240, present_mode)
                .expect("the surface probe must initialize on a real GPU");
        eprintln!(
            "[xlib-continuous] adapter={} device_type={} present_mode={:?}",
            presenter.adapter().name,
            presenter.adapter().device_type,
            presenter.present_mode()
        );

        fn summarize(name: &str, mut samples: Vec<u64>) {
            samples.sort_unstable();
            let n = samples.len();
            let median = if n == 0 { 0 } else { samples[n / 2] };
            let p90 = if n == 0 { 0 } else { samples[(n * 9) / 10] };
            let max = samples.last().copied().unwrap_or(0);
            eprintln!(
                "[xlib-continuous] {name}: n={n} median_us={median} p90_us={p90} max_us={max}"
            );
        }

        let duration = std::time::Duration::from_secs(10);
        let started = std::time::Instant::now();
        let mut frames = 0u64;
        let mut marker = 0.0f32;
        let mut write = Vec::new();
        let mut acquire = Vec::new();
        let mut submit = Vec::new();
        let mut scope = Vec::new();
        let mut present = Vec::new();
        let mut total = Vec::new();
        while started.elapsed() < duration {
            let stats = presenter
                .frame(marker)
                .expect("continuous frame must present");
            frames += 1;
            write.push(stats.timings.write_buffer_us);
            acquire.push(stats.timings.acquire_us);
            submit.push(stats.timings.submit_us);
            scope.push(stats.timings.scope_pop_us);
            present.push(stats.timings.present_us);
            total.push(stats.timings.total_us);
            marker = (marker + 0.01).rem_euclid(1.0);
        }
        eprintln!(
            "[xlib-continuous] frames={frames} in {:?}",
            started.elapsed()
        );
        summarize("write_buffer", write);
        summarize("acquire", acquire);
        summarize("submit", submit);
        summarize("scope_pop", scope);
        summarize("present", present);
        summarize("total", total);
        assert!(
            frames >= 300,
            "expected >=300 frames in 10s on a plain Xlib window, got {frames}"
        );
    }

    /// A synthetic checkerboard texture on the **same device** is presented.
    /// This is a texture-present check, **not** CAD acceptance and not a
    /// visual-accuracy claim; no GPU->CPU readback is used.
    #[test]
    #[ignore = "requires a real X11/XWayland window and a hardware Vulkan GPU"]
    fn present_native_texture_checkerboard() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("SKIP: DISPLAY is not set (no X11/XWayland window available)");
            return;
        }
        let window = match XlibWindow::open(320, 240) {
            Some(window) => window,
            None => {
                eprintln!("SKIP: could not open an Xlib display");
                return;
            }
        };
        let mut presenter =
            SurfaceProbePresenter::new(window, 320, 240).expect("presenter must initialize");
        let (device, queue) = presenter.device_queue();
        let size = 64u32;
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        for y in 0..size {
            for x in 0..size {
                let c = if (x / 8 + y / 8) % 2 == 0 { 255 } else { 0 };
                let i = ((y * size + x) * 4) as usize;
                pixels[i] = c;
                pixels[i + 1] = c;
                pixels[i + 2] = c;
                pixels[i + 3] = 255;
            }
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("checkerboard"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );

        let key = NativeTextureKey {
            device_epoch: 1,
            texture_revision: 1,
            size: (size, size),
        };
        let first = presenter
            .present_texture(&texture, key)
            .expect("checkerboard must present");
        assert_eq!(first.frame_index, 1);
        let second = presenter
            .present_texture(&texture, key)
            .expect("second checkerboard frame must present");
        assert_eq!(second.frame_index, 2);

        // A changed key forces a rebind (same texture, new epoch).
        let rebound = NativeTextureKey {
            device_epoch: 2,
            texture_revision: 1,
            size: (size, size),
        };
        assert!(presenter.present_texture(&texture, rebound).is_ok());

        // A key whose size disagrees with the texture is an explicit error.
        let bad = NativeTextureKey {
            device_epoch: 2,
            texture_revision: 2,
            size: (128, 128),
        };
        assert!(matches!(
            presenter.present_texture(&texture, bad),
            Err(ProbeError::TextureInvalid(_))
        ));
    }
}
