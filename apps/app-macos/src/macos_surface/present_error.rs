//! Stable present-error classes for the `[probe] cad-present-error` evidence
//! marker, kept in its own module so `macos_surface.rs` stays under the
//! code-file size limit.
//!
//! This mirrors `apps/app-windows/src/win32_surface/present_error.rs` token for
//! token (the evidence tooling keeps one vocabulary across hosts). The
//! composition class is absent because macOS presents straight to the
//! `CAMetalLayer`-backed swapchain and has no DirectComposition step.

use super::*;

/// The stable class of a CAD present failure, for the
/// `[probe] cad-present-error class=…` evidence marker.
///
/// Derived from the **typed** discriminants ([`ProbeError`],
/// `cad_render_wgpu::RenderError`), never from matching the free text, so an
/// evidence run can group a failure mechanically. The token set is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CadPresentClass {
    /// The engine-side pre-render stage (`apply_ready_updates`) or the frame
    /// pipeline could not start (no session/presenter).
    ApplyUpdates,
    /// The engine's render call failed (frame/size/upload/scene, not device loss).
    Render,
    /// The surface or device was lost; the presenter must be rebuilt.
    DeviceLost,
    /// The swapchain could not hand out a frame
    /// (`outdated`/`timeout`/`occluded`/`validation`).
    Acquire,
    /// The native CAD texture failed validation, or its size disagreed with the
    /// surface persistently (the size-guard defect class).
    TextureInvalid,
    /// The submitted/presented frame was rejected by GPU validation.
    FrameValidation,
    /// The child placement or the swapchain reconfigure to the size authority
    /// failed (the surface could not follow the accepted CAD rect).
    SurfaceReconfigure,
    /// The renderer refused an allocation **for memory** (its own application
    /// budget, or the device reporting out-of-memory). The resident scene is
    /// intact and still renders, so this is neither a device loss nor a generic
    /// frame error (`cad_render_wgpu::RenderError::GpuMemory`).
    GpuMemory,
}

/// The stable class token of a GPU-memory refusal (budget or device OOM).
pub const GPU_MEMORY_CLASS: &str = "gpu_memory";

impl CadPresentClass {
    /// The stable marker token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApplyUpdates => "apply_updates",
            Self::Render => "render",
            Self::DeviceLost => "device_lost",
            Self::Acquire => "acquire",
            Self::TextureInvalid => "texture_invalid",
            Self::FrameValidation => "frame_validation",
            Self::SurfaceReconfigure => "surface_reconfigure",
            Self::GpuMemory => GPU_MEMORY_CLASS,
        }
    }

    pub(super) fn from_probe(error: &ProbeError) -> Self {
        match error {
            ProbeError::Acquire(_) => Self::Acquire,
            ProbeError::DeviceLost(_) => Self::DeviceLost,
            ProbeError::Frame(_) => Self::FrameValidation,
            ProbeError::TextureInvalid(_) => Self::TextureInvalid,
            // `TextureNotReady` never reaches the error path (the caller waits),
            // and the remaining variants are construction-time failures that
            // cannot occur mid-frame; they share the frame-validation class so
            // the mapping stays total without inventing a token.
            _ => Self::FrameValidation,
        }
    }

    pub(super) fn from_render(error: &cad_render_wgpu::RenderError) -> Self {
        match error {
            cad_render_wgpu::RenderError::DeviceLost(_) => Self::DeviceLost,
            cad_render_wgpu::RenderError::Frame(_)
            | cad_render_wgpu::RenderError::NotInitialized(_) => Self::Render,
            cad_render_wgpu::RenderError::GpuMemory(_) => Self::GpuMemory,
        }
    }
}

/// A classified CAD present failure: the stable class plus the human text that
/// still lands in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CadPresentError {
    /// Stable class, emitted as `class=…`.
    pub class: CadPresentClass,
    /// Human-readable text (the status note keeps this, unchanged in shape).
    pub text: String,
}

impl CadPresentError {
    /// Build a classified failure.
    pub fn new(class: CadPresentClass, text: impl Into<String>) -> Self {
        Self {
            class,
            text: text.into(),
        }
    }

    pub(super) fn from_probe(error: &ProbeError, text: String) -> Self {
        Self::new(CadPresentClass::from_probe(error), text)
    }

    /// The `text=` token of the marker: one line, so a marker stays greppable.
    pub fn marker_text(&self) -> String {
        self.text.replace(['\n', '\r'], " ")
    }
}

impl std::fmt::Display for CadPresentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

/// The stable class of a render error, for explicit reporting.
pub(super) fn render_error_class(error: &cad_render_wgpu::RenderError) -> &'static str {
    match error {
        cad_render_wgpu::RenderError::DeviceLost(_) => "device_lost",
        cad_render_wgpu::RenderError::Frame(_) => "frame",
        cad_render_wgpu::RenderError::NotInitialized(_) => "not_initialized",
        cad_render_wgpu::RenderError::GpuMemory(_) => GPU_MEMORY_CLASS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marker class tokens are the closed evidence vocabulary.
    #[test]
    fn present_error_classes_have_stable_tokens() {
        for (class, token) in [
            (CadPresentClass::ApplyUpdates, "apply_updates"),
            (CadPresentClass::Render, "render"),
            (CadPresentClass::DeviceLost, "device_lost"),
            (CadPresentClass::Acquire, "acquire"),
            (CadPresentClass::TextureInvalid, "texture_invalid"),
            (CadPresentClass::FrameValidation, "frame_validation"),
            (CadPresentClass::SurfaceReconfigure, "surface_reconfigure"),
            (CadPresentClass::GpuMemory, GPU_MEMORY_CLASS),
        ] {
            assert_eq!(class.as_str(), token);
        }
        assert_eq!(GPU_MEMORY_CLASS, "gpu_memory");
        assert_ne!(GPU_MEMORY_CLASS, "frame");
        assert_ne!(GPU_MEMORY_CLASS, "device_lost");
        // The mapping reads the typed discriminants, not the text.
        assert_eq!(
            CadPresentClass::from_probe(&ProbeError::Acquire("timeout")),
            CadPresentClass::Acquire
        );
        assert_eq!(
            CadPresentClass::from_probe(&ProbeError::TextureInvalid("size".into())),
            CadPresentClass::TextureInvalid
        );
        assert_eq!(
            CadPresentClass::from_render(&cad_render_wgpu::RenderError::Frame("frame".into())),
            CadPresentClass::Render
        );
        // A marker never spans lines.
        assert_eq!(
            CadPresentError::new(CadPresentClass::Render, "a\nb\r\nc").marker_text(),
            "a b  c"
        );
    }
}
