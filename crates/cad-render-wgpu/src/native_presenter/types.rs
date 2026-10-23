use super::*;

/// Preferred adapter-name substrings, tried in order, when none is supplied.
///
/// This is a *preference*, never a filter: a machine without any of these still
/// selects another Vulkan hardware adapter.
pub const DEFAULT_PREFERRED_ADAPTER_NAMES: &[&str] = &["P620"];

/// Which wgpu backend(s) the native probe may use.
///
/// This is the renderer half of the durable user preference `wgpu_backend`. The
/// accepted wire values are **exactly** `"auto"`, `"vulkan"` and `"dx12"`;
/// anything else — an absent value, an empty string, an unknown token — resolves
/// to [`ProbeBackendPolicy::Auto`]. There is no silent third choice and no
/// fallback from an explicit policy to another backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProbeBackendPolicy {
    /// Vulkan and DX12, with Vulkan preferred inside a ranking tier.
    ///
    /// The default: on a target where one of the two backends is not compiled
    /// in, wgpu simply has no adapters for it, so `auto` degrades to the
    /// available one without pretending the other was tried.
    #[default]
    Auto,
    /// Vulkan only — the historical probe contract.
    Vulkan,
    /// DirectX 12 only. Windows-only: any other target fails explicitly with
    /// [`ProbeError::UnsupportedBackendPolicy`] (never a silent Vulkan fallback).
    Dx12,
}

impl ProbeBackendPolicy {
    /// Parse the durable `wgpu_backend` value.
    ///
    /// Accepted spellings are exactly `"vulkan"` and `"dx12"`; `"auto"`, an
    /// absent value, an empty value and every unknown token are
    /// [`ProbeBackendPolicy::Auto`] (the documented default). Matching is
    /// case-sensitive: `"DX12"` is not a wire value and therefore resolves to
    /// `auto`, never to DX12 by accident.
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("vulkan") => Self::Vulkan,
            Some("dx12") => Self::Dx12,
            _ => Self::Auto,
        }
    }

    /// The stable wire token (`auto`/`vulkan`/`dx12`), for logs and evidence.
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Vulkan => "vulkan",
            Self::Dx12 => "dx12",
        }
    }

    /// Human-readable policy summary used in the one-line startup log.
    pub fn summary(self) -> &'static str {
        match self {
            Self::Auto => "vulkan preferred",
            Self::Vulkan => "vulkan only",
            Self::Dx12 => "dx12 only",
        }
    }

    /// The wgpu backends this policy enables for instance creation and
    /// enumeration.
    pub fn backends(self) -> wgpu::Backends {
        match self {
            Self::Auto => wgpu::Backends::VULKAN | wgpu::Backends::DX12,
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Dx12 => wgpu::Backends::DX12,
        }
    }

    /// Whether an adapter reporting `backend` (wgpu's `Backend::to_str` value) is
    /// eligible under this policy.
    pub fn allows_backend(self, backend: &str) -> bool {
        match self {
            Self::Auto => backend == "vulkan" || backend == "dx12",
            Self::Vulkan => backend == "vulkan",
            Self::Dx12 => backend == "dx12",
        }
    }

    /// Fail explicitly when this policy cannot work on the current target.
    ///
    /// Only [`ProbeBackendPolicy::Dx12`] has a target restriction: wgpu compiles
    /// its DX12 backend on Windows only, so elsewhere the probe refuses the
    /// policy by name instead of enumerating nothing or silently using Vulkan.
    pub(crate) fn ensure_supported(self) -> Result<(), ProbeError> {
        match self {
            Self::Dx12 if !cfg!(target_os = "windows") => {
                Err(ProbeError::UnsupportedBackendPolicy {
                    policy: self.as_token(),
                })
            }
            _ => Ok(()),
        }
    }
}

/// Marker-offset wrapping range. The probe wraps offsets into `[0, 1)`.
pub(super) const MARKER_PERIOD: f32 = 1.0;

/// Bounded budget for collecting the validation error scope each frame.
///
/// The scope is resolved by device callbacks, which need a device poll. Blocking
/// on it indefinitely (the previous behaviour) let a slow device poll hold the
/// frame; this bounds it without dropping validation reporting.
pub(super) const SCOPE_COLLECT_BUDGET: std::time::Duration = std::time::Duration::from_millis(100);

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

/// How many consecutive native-texture/surface size disagreements are treated
/// as a transient "not ready yet" before the guard surfaces the explicit error.
///
/// Three ticks is ~50 ms at 60 Hz: longer than a mid-resize frame or a
/// one-pixel DOM rounding transient, short enough that a genuine drawing-area
/// mismatch still fails loudly within a few frames.
pub const SIZE_MISMATCH_WAIT_TICKS: u32 = 3;

/// Bounded wait for a native-texture/surface size disagreement.
///
/// The blit stretches the texture across the whole surface (no aspect
/// correction) and pointer input maps through the DOM CAD rect, so a
/// disagreeing texture must never be presented. It can still be a **transient**:
/// a mid-resize frame, or a fractional DOM rect whose rounding lands one pixel
/// off the integer allocation-derived surface size (more likely at `scale >= 2`).
/// This latch keeps the "never stretch" rule and turns those first few ticks into
/// a wait ([`ProbeError::TextureNotReady`]) instead of a user-visible failure: it
/// waits the first `SIZE_MISMATCH_WAIT_TICKS - 1` consecutive disagreements, and
/// only the disagreement that reaches the limit becomes the explicit
/// [`ProbeError::TextureInvalid`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SizeMismatchWait {
    consecutive: u32,
}

impl SizeMismatchWait {
    /// Record one disagreement: `true` means "wait this frame", `false` means the
    /// disagreement has persisted and must be surfaced as an explicit error.
    pub fn record_mismatch(&mut self) -> bool {
        self.consecutive = self.consecutive.saturating_add(1);
        self.consecutive < SIZE_MISMATCH_WAIT_TICKS
    }

    /// Record a matching presentation: the disagreement did not persist.
    pub fn record_match(&mut self) {
        self.consecutive = 0;
    }

    /// Consecutive disagreements recorded so far.
    pub fn consecutive(&self) -> u32 {
        self.consecutive
    }
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

/// Which kind of native surface a presenter is presenting into.
///
/// This is evidence, not decoration: on Windows a **HWND-targeted** DXGI flip
/// swapchain cannot composite a `WS_CHILD` under a transparent (`noRedirectionBitmap`)
/// top-level window — wgpu-hal 30 hardcodes `DXGI_SWAP_EFFECT_FLIP_DISCARD`, and a
/// flip chain binds to the top-level DWM visual, which a child window does not
/// have. The Windows host therefore presents through **DirectComposition**
/// (`CreateSwapChainForComposition` over an `IDCompositionVisual`); the ready line
/// names the kind so an evidence run can never confuse the two paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSurfaceKind {
    /// Surface created from a raw `HWND` (the historical path; still used by the
    /// Linux/GTK host and by callers that own an opaque top-level window).
    Hwnd,
    /// Surface created over an `IDCompositionVisual`
    /// (`SurfaceTargetUnsafe::CompositionVisual`), composed by DirectComposition.
    Composition,
}

impl ProbeSurfaceKind {
    /// The stable `surface=…` token for the presenter-ready evidence line.
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Hwnd => "hwnd",
            Self::Composition => "composition",
        }
    }
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
    /// No Vulkan/DX12 adapter matching the selected [`ProbeBackendPolicy`] could
    /// present to the surface. The message names the policy's backend(s) and the
    /// adapters that were enumerated.
    NoVulkanAdapter(String),
    /// The requested [`ProbeBackendPolicy`] cannot work on this target (DX12
    /// outside Windows). Nothing was tried; there is no fallback backend.
    UnsupportedBackendPolicy { policy: &'static str },
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
    /// The native texture's size disagrees with the surface's **for a bounded
    /// number of consecutive frames**: a not-ready-yet transient (a mid-resize
    /// tick, or a DOM-derived size whose rounding is one pixel off) rather than
    /// the persistent defect [`ProbeError::TextureInvalid`] reports. The texture
    /// is still never presented (no stretch); the caller waits, exactly like the
    /// "no frame" path. If the disagreement persists, the same guard becomes an
    /// explicit [`ProbeError::TextureInvalid`].
    TextureNotReady(String),
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
                write!(f, "no usable GPU adapter: {message}")
            }
            ProbeError::UnsupportedBackendPolicy { policy } => write!(
                f,
                "probe backend policy '{policy}' is not supported on this target; nothing was tried (use 'auto' or 'vulkan')"
            ),
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
            ProbeError::TextureNotReady(message) => {
                write!(f, "native texture not ready yet: {message}")
            }
        }
    }
}

impl std::error::Error for ProbeError {}
