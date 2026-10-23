//! Framework-free policy of the wasm canvas presenter.
//!
//! Deliberately compiled on **every** target: no `web-sys`, no wgpu device and no
//! surface here. That keeps the size and format rules, the acquire-status
//! classification and the error text unit-testable in the ordinary (native)
//! `cargo test` run — a browser is never needed, and no test in this module can
//! touch a GPU.

use crate::wgpu;

/// Everything the wasm canvas path can fail with.
///
/// Every variant names the step that failed in its [`std::fmt::Display`] text, so
/// a host can log the error verbatim and evidence never has to guess which call
/// broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasError {
    /// A physical size the presenter cannot configure: a zero dimension, or one
    /// above the adapter's `max_texture_dimension_2d`. Rejected explicitly
    /// instead of letting `Surface::configure` panic on a zero size.
    InvalidSize {
        /// Requested physical width in device pixels.
        width: u32,
        /// Requested physical height in device pixels.
        height: u32,
        /// Why the size was refused.
        detail: String,
    },
    /// `Instance::create_surface(SurfaceTarget::Canvas(..))` failed: the browser
    /// refused a WebGL2 context for this canvas, or the canvas is already
    /// attached to another context.
    SurfaceCreate(String),
    /// `Surface::get_default_config` returned `None`: the surface advertises no
    /// usable configuration for the selected adapter (no format or present mode).
    NoSurfaceConfig,
    /// `Instance::request_adapter` failed. On WebGL2 an adapter can only be
    /// selected together with a compatible surface, so this is also the symptom
    /// of a canvas without a usable WebGL2 context.
    AdapterUnavailable(String),
    /// `Adapter::request_device` failed.
    DeviceUnavailable(String),
    /// Acquire reported `Outdated`: the surface was reconfigured once and this
    /// frame was dropped. Retry on the next frame.
    SurfaceOutdated,
    /// Acquire reported `Lost`: the surface was reconfigured once and this frame
    /// was dropped. If it repeats, the host must drop the presenter and build a
    /// new one.
    SurfaceLost,
    /// Acquire reported `Timeout`: no frame is available yet; skip this frame.
    SurfaceTimeout,
    /// Acquire reported `Occluded`: the canvas is hidden or zero-sized; skip this
    /// frame.
    SurfaceOccluded,
    /// Acquire was rejected by wgpu validation.
    SurfaceValidation(String),
    /// The blit was refused **before** submission: zero-sized source texture,
    /// missing `TEXTURE_BINDING` usage, or a source whose size/format does not
    /// match the configured surface (the fullscreen triangle has neither aspect
    /// correction nor colour conversion).
    Blit(String),
}

impl std::fmt::Display for CanvasError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSize {
                width,
                height,
                detail,
            } => write!(
                formatter,
                "canvas: invalid physical size {width}x{height}: {detail}"
            ),
            Self::SurfaceCreate(detail) => {
                write!(formatter, "canvas: create_surface failed: {detail}")
            }
            Self::NoSurfaceConfig => write!(
                formatter,
                "canvas: get_default_config returned no configuration for this surface"
            ),
            Self::AdapterUnavailable(detail) => {
                write!(formatter, "canvas: request_adapter failed: {detail}")
            }
            Self::DeviceUnavailable(detail) => {
                write!(formatter, "canvas: request_device failed: {detail}")
            }
            Self::SurfaceOutdated => write!(
                formatter,
                "canvas: get_current_texture reported Outdated; the surface was reconfigured \
                 once, retry the frame"
            ),
            Self::SurfaceLost => write!(
                formatter,
                "canvas: get_current_texture reported Lost; the surface was reconfigured once, \
                 retry the frame"
            ),
            Self::SurfaceTimeout => write!(
                formatter,
                "canvas: get_current_texture reported Timeout; the frame was skipped"
            ),
            Self::SurfaceOccluded => write!(
                formatter,
                "canvas: get_current_texture reported Occluded (canvas hidden or zero-sized); \
                 the frame was skipped"
            ),
            Self::SurfaceValidation(detail) => write!(
                formatter,
                "canvas: get_current_texture was rejected by validation: {detail}"
            ),
            Self::Blit(detail) => write!(formatter, "canvas: blit failed: {detail}"),
        }
    }
}

impl std::error::Error for CanvasError {}

/// One-to-one mirror of `wgpu::CurrentSurfaceTexture`'s variants.
///
/// The wgpu enum carries the acquired frame inside two of its variants, so it
/// cannot be constructed (and therefore cannot be unit tested) without a real
/// surface. This mirror is what the policy functions take, which makes the
/// status → error mapping testable without a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceStatus {
    /// A frame was acquired.
    Success,
    /// A frame was acquired, but the surface configuration is no longer optimal.
    Suboptimal,
    /// No frame is available yet.
    Timeout,
    /// The canvas is hidden or zero-sized.
    Occluded,
    /// The surface configuration is stale and must be re-applied.
    Outdated,
    /// The surface (or the whole device) was lost.
    Lost,
    /// Acquire was rejected by validation.
    Validation,
}

impl SurfaceStatus {
    /// Whether wgpu returned a usable frame for this status.
    pub fn is_frame(self) -> bool {
        matches!(self, Self::Success | Self::Suboptimal)
    }

    /// Whether the documented remedy is to reconfigure the surface.
    pub fn needs_reconfigure(self) -> bool {
        matches!(self, Self::Outdated | Self::Lost)
    }

    /// The explicit error for this status, or `None` when it carries a frame.
    ///
    /// For [`SurfaceStatus::needs_reconfigure`] statuses the caller has already
    /// re-applied the configuration when it reports the error (the presenter does
    /// that unconditionally), so the message states it. A failure is always
    /// reported: the canvas path never presents a blank frame silently.
    pub fn acquire_error(self) -> Option<CanvasError> {
        match self {
            Self::Success | Self::Suboptimal => None,
            Self::Timeout => Some(CanvasError::SurfaceTimeout),
            Self::Occluded => Some(CanvasError::SurfaceOccluded),
            Self::Outdated => Some(CanvasError::SurfaceOutdated),
            Self::Lost => Some(CanvasError::SurfaceLost),
            Self::Validation => Some(CanvasError::SurfaceValidation(
                "the surface acquire failed validation; see the browser console for the GPU message"
                    .to_string(),
            )),
        }
    }
}

/// Validate a **physical** (device-pixel) size before it reaches the surface.
///
/// The sizing contract: callers pass physical pixels, i.e. `CSS px ×
/// devicePixelRatio`, never CSS pixels. `Surface::configure` sizes the canvas
/// backing store from this value and **panics** on a zero dimension, so `(0, 0)`
/// (and any zero dimension) is rejected here with [`CanvasError::InvalidSize`].
/// A size above `max_dimension` (the adapter's `max_texture_dimension_2d`) is
/// rejected the same way rather than failing later inside wgpu.
pub fn check_physical_size(
    physical: (u32, u32),
    max_dimension: u32,
) -> Result<(u32, u32), CanvasError> {
    let (width, height) = physical;
    if width == 0 || height == 0 {
        return Err(CanvasError::InvalidSize {
            width,
            height,
            detail: "width and height must both be non-zero; the caller passes physical pixels \
                     (CSS px x devicePixelRatio) and a zero dimension cannot be configured"
                .to_string(),
        });
    }
    if width > max_dimension || height > max_dimension {
        return Err(CanvasError::InvalidSize {
            width,
            height,
            detail: format!("exceeds the adapter's max_texture_dimension_2d ({max_dimension})"),
        });
    }
    Ok((width, height))
}

/// Pick the surface format to present with from the adapter's advertised formats.
///
/// An sRGB format is preferred because the renderer's offscreen frame texture is
/// `Rgba8UnormSrgb`: blitting an sRGB texture into a **linear** surface format
/// would store the sRGB-decoded (linear) value and the display would encode it
/// again, i.e. a visibly washed-out/wrong-contrast frame. No channel order is
/// hardcoded — `Bgra8Unorm` is a native-only order and canvas formats are RGBA —
/// the first advertised format is the fallback when no sRGB format is offered.
pub fn preferred_present_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats
        .iter()
        .copied()
        .find(|format| format.is_srgb())
        .or_else(|| formats.first().copied())
}

/// The blit performs no colour conversion of its own, so the presented frame
/// texture must be the same format as the configured surface.
///
/// A mismatch is an explicit [`CanvasError::Blit`], never a silent tone shift.
pub fn check_present_format(
    source: wgpu::TextureFormat,
    surface: wgpu::TextureFormat,
) -> Result<(), CanvasError> {
    if source == surface {
        return Ok(());
    }
    Err(CanvasError::Blit(format!(
        "present_texture: source frame format {source:?} does not match the configured surface \
         format {surface:?}; the blit has no colour conversion, so the frame is refused instead \
         of being presented with a colour shift"
    )))
}

/// The fullscreen triangle stretches whatever it samples across the whole
/// surface (there is no aspect correction), so the presented frame texture must
/// be exactly the configured physical size.
///
/// A mismatch is an explicit [`CanvasError::Blit`], never a silent anisotropic
/// stretch.
pub fn check_present_size(source: (u32, u32), surface: (u32, u32)) -> Result<(), CanvasError> {
    if source == surface {
        return Ok(());
    }
    Err(CanvasError::Blit(format!(
        "present_texture: source frame is {}x{} but the surface is {}x{}; refusing to stretch it \
         (pass the same physical size to `create`/`resize` and the renderer's offscreen target)",
        source.0, source.1, surface.0, surface.1
    )))
}

/// The blit samples the frame as a filterable 2D texture, so its usage must
/// include `TEXTURE_BINDING`.
pub fn check_present_usage(usage: wgpu::TextureUsages) -> Result<(), CanvasError> {
    if usage.contains(wgpu::TextureUsages::TEXTURE_BINDING) {
        return Ok(());
    }
    Err(CanvasError::Blit(format!(
        "present_texture: source frame usage {usage:?} does not include TEXTURE_BINDING; the blit \
         cannot sample it"
    )))
}
