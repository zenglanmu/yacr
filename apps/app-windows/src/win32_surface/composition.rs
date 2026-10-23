//! The DirectComposition tree that makes the CAD content compositable.
//!
//! Split out of `win32_surface.rs` to stay under the code-file size limit. The
//! owner (and the lifetime contract) is documented on the type.

use super::*;

/// Which window the composition target is attached to.
///
/// `TopLevel` is the mandated form: the visual sits in the documented
/// **behind-child-windows** layer slot of the top-level window's composition tree
/// (DOM-on-top by construction), and the CAD content is positioned/clipped to the
/// accepted CAD rect. `Child` is kept as a **typed evidence token** only: it is
/// the form that provably does not reach the composited output in this
/// configuration (see `docs/windows-app.md` §2.2 and the ADR amendment draft), so
/// a build can be labelled without text matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionTargetKind {
    /// Target attached to the top-level (transparent) window. Mandated.
    TopLevel,
    /// Target attached to the CAD child HWND. Not used by the host; kept as a
    /// typed evidence token so a build can be labelled `target=child` without
    /// text matching (this is the form that never reaches the composited output
    /// in this configuration).
    #[allow(dead_code)]
    Child,
}

impl CompositionTargetKind {
    /// The stable `target=…` token for the presenter-ready line.
    pub fn as_token(self) -> &'static str {
        match self {
            Self::TopLevel => "top",
            Self::Child => "child",
        }
    }
}

/// The DirectComposition objects that make the CAD content compositable.
///
/// **Ownership/lifetimes:** this struct owns the device, the target, the visual
/// and the clip. `wgpu` keeps its own COM ref on the visual for the surface's
/// lifetime, but composition only produces pixels while the device and target are
/// alive and a `Commit()` follows each `IDCompositionVisual::SetContent` (issued by
/// hal on every `configure`) and each offset/clip change. The drop order is
/// presenter (surface) → these objects → child HWND, enforced by
/// `release_gpu`/`Drop`.
///
/// **Target choice:** the target is the **top-level** window, not the CAD child.
/// A child-window target is legal per MSDN, but in this configuration
/// (`WS_EX_NOREDIRECTIONBITMAP` top level, full-client `WRY_WEBVIEW` sibling, and
/// `WS_CLIPSIBLINGS` on the child) child-attached content never reaches the
/// composited output. The top-level target's `topmost = FALSE` slot is documented
/// as *behind* child windows, which is exactly the layering we need.
pub(super) struct CompositionSurface {
    device: IDCompositionDevice,
    /// Kept alive only: the target attaches the visual tree to the top-level
    /// window. Dropping it would detach the composed content.
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    /// Clip rectangle, updated in place on every layout: the CAD content only, so
    /// a resize race can never bleed pixels over the surrounding chrome.
    clip: IDCompositionRectangleClip,
    /// Which window the target is attached to (evidence token).
    target_kind: CompositionTargetKind,
}

impl CompositionSurface {
    /// Create the device/target/visual tree rooted at `top_level`.
    ///
    /// The target **must** be the top-level (transparent) window: the visual then
    /// lives in the documented layer slot *behind* child windows (DOM-on-top by
    /// construction), and it is the configuration wgpu and wry document for a
    /// transparent window.
    pub(super) fn new(top_level: HWND) -> Result<Self, String> {
        // `DCompositionCreateDevice(None, …)` lets DirectComposition create its
        // own device (the same call wgpu-hal makes); passing `None` keeps the
        // renderer's D3D12 device out of this contract entirely.
        let device: IDCompositionDevice = unsafe { DCompositionCreateDevice(None) }
            .map_err(|error| format!("DCompositionCreateDevice failed: {error}"))?;
        // `topmost = FALSE`: layer 2 of the window's composition tree — behind all
        // child windows (the WebView), which is DOM-on-top by construction.
        let target = unsafe { device.CreateTargetForHwnd(top_level, false) }.map_err(|error| {
            format!("IDCompositionDevice::CreateTargetForHwnd(top-level) failed: {error}")
        })?;
        let visual = unsafe { device.CreateVisual() }
            .map_err(|error| format!("IDCompositionDevice::CreateVisual failed: {error}"))?;
        let clip = unsafe { device.CreateRectangleClip() }
            .map_err(|error| format!("IDCompositionDevice::CreateRectangleClip failed: {error}"))?;
        unsafe { target.SetRoot(&visual) }
            .map_err(|error| format!("IDCompositionTarget::SetRoot failed: {error}"))?;
        unsafe { visual.SetClip(&clip) }
            .map_err(|error| format!("IDCompositionVisual::SetClip failed: {error}"))?;
        unsafe { device.Commit() }
            .map_err(|error| format!("IDCompositionDevice::Commit (initial) failed: {error}"))?;
        Ok(Self {
            device,
            _target: target,
            visual,
            clip,
            target_kind: CompositionTargetKind::TopLevel,
        })
    }

    /// Position and clip the visual to the accepted CAD rect, in **DIPs**.
    ///
    /// The offset is the logical position verbatim (DirectComposition composes in
    /// DIPs; no scale math here), and the clip is the logical content size from
    /// the visual's own origin. The change becomes visible on the next `Commit`,
    /// which the host issues through its pending-placement bookkeeping.
    pub(super) fn set_placement(&self, offset: (f32, f32), clip: (f32, f32)) -> Result<(), String> {
        unsafe { self.visual.SetOffsetX2(offset.0) }
            .map_err(|error| format!("IDCompositionVisual::SetOffsetX failed: {error}"))?;
        unsafe { self.visual.SetOffsetY2(offset.1) }
            .map_err(|error| format!("IDCompositionVisual::SetOffsetY failed: {error}"))?;
        unsafe { self.clip.SetLeft2(0.0) }
            .map_err(|error| format!("IDCompositionRectangleClip::SetLeft failed: {error}"))?;
        unsafe { self.clip.SetTop2(0.0) }
            .map_err(|error| format!("IDCompositionRectangleClip::SetTop failed: {error}"))?;
        unsafe { self.clip.SetRight2(clip.0) }
            .map_err(|error| format!("IDCompositionRectangleClip::SetRight failed: {error}"))?;
        unsafe { self.clip.SetBottom2(clip.1) }
            .map_err(|error| format!("IDCompositionRectangleClip::SetBottom failed: {error}"))?;
        Ok(())
    }

    /// The `target=…` evidence token.
    pub(super) fn target_kind(&self) -> CompositionTargetKind {
        self.target_kind
    }

    /// The raw `IDCompositionVisual` pointer handed to the renderer.
    ///
    /// The pointer stays valid while this struct is alive; wgpu-hal wraps it
    /// borrowed and keeps its own ref, so the surface never dangles even if this
    /// struct is dropped first (our release order drops the surface first anyway).
    pub(super) fn visual_ptr(&self) -> *mut core::ffi::c_void {
        Interface::as_raw(&self.visual)
    }

    /// Apply pending visual-tree changes.
    pub(super) fn commit(&self) -> Result<(), String> {
        unsafe { self.device.Commit() }
            .map_err(|error| format!("IDCompositionDevice::Commit failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `target=` token is evidence vocabulary: `top` is the mandated form,
    /// `child` stays addressable so a build-to-build comparison can never be
    /// confused by text.
    #[test]
    fn composition_target_tokens_are_stable() {
        assert_eq!(CompositionTargetKind::TopLevel.as_token(), "top");
        assert_eq!(CompositionTargetKind::Child.as_token(), "child");
        assert_ne!(
            CompositionTargetKind::TopLevel.as_token(),
            CompositionTargetKind::Child.as_token()
        );
    }
}
