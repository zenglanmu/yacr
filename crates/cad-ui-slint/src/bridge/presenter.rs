//! Slint-only presentation adapter. No database access or geometry construction.
use super::runtime::{FrameBinding, PresentedFrame};
use super::*;

#[derive(Default)]
pub(super) struct SlintPresenter {
    bound: Option<FrameBinding>,
}

impl SlintPresenter {
    /// Explicit host reset, outside rendering lifecycle callbacks.
    pub fn reset(&mut self, handle: &UiHandle) -> CadResult<()> {
        handle.set_cad_frame(slint::Image::default())?;
        self.invalidate();
        Ok(())
    }

    pub fn invalidate(&mut self) {
        // Lifecycle callbacks may run while the window is mutably borrowed.
        // Keep the old image alive until BeforeRendering replaces it.
        self.bound = None;
    }

    pub fn bind_frame_if_changed(
        &mut self,
        handle: &UiHandle,
        frame: PresentedFrame,
    ) -> CadResult<()> {
        if self.bound == Some(frame.binding) {
            return Ok(());
        }
        let image = slint::Image::try_from(frame.texture)
            .map_err(|e| CadError::GpuFailure(format!("Slint texture import: {e}")))?;
        handle.set_cad_frame(image)?;
        // A failed import/binding remains retryable on the next UI frame.
        self.bound = Some(frame.binding);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_invalidation_needs_no_window_and_forces_rebinding() {
        let mut presenter = SlintPresenter {
            bound: Some(FrameBinding {
                device_epoch: 1,
                texture_revision: 1,
                size: (10, 10),
            }),
        };
        presenter.invalidate();
        assert!(presenter.bound.is_none());
        presenter.invalidate();
        assert!(presenter.bound.is_none());
    }
}
