//! The host-facing accelerated-GPU-storage policy on the render session.
//!
//! Split out of `session_impl.rs` so both files stay inside the file-size gate.
//! The draw tier is chosen once per device, so this is a recorded policy that
//! every fresh `CadRenderRuntime` attach installs before `initialize_with_device`.

use super::*;

impl RenderSessionHandle {
    /// Allow or refuse the accelerated vertex-stage storage copy of the scene
    /// (`view.gpuStorage.disabled` inverted; default **allowed** on the engine
    /// handle).
    ///
    /// The host records the user's choice here; the renderer selects the draw
    /// tier once per device, so the value is installed on the renderer **before**
    /// `initialize_with_device` on every attach. Calling this on a live session
    /// does not switch the current device's tier mid-frame — the next attach
    /// does (the Options help text says the change needs a restart).
    pub fn set_gpu_storage_enabled(&self, enabled: bool) {
        self.0.borrow_mut().gpu_storage_enabled = enabled;
    }

    /// The recorded accelerated-GPU-storage choice (see
    /// [`Self::set_gpu_storage_enabled`]). Reported so a host test can prove the
    /// persisted setting reached the renderer seam without a device.
    pub fn gpu_storage_enabled(&self) -> bool {
        self.0.borrow().gpu_storage_enabled
    }
}
