use super::*;

/// Preferred adapter-name substrings, tried in order, when none is supplied.
///
/// This is a *preference*, never a filter: a machine without any of these still
/// selects another Vulkan hardware adapter.
pub const DEFAULT_PREFERRED_ADAPTER_NAMES: &[&str] = &["P620"];

/// The only backend the probe will use, as wgpu's `Backend::to_str` value.
pub(super) const REQUIRED_BACKEND: &str = "vulkan";

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
