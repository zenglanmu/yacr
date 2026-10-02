//! Slint-only presentation adapter. No database access or geometry construction.
use super::runtime::{FrameBinding, PresentedFrame};
use super::*;

#[derive(Default)]
pub(super) struct SlintPresenter {
    bound: Option<FrameBinding>,
}
impl SlintPresenter {
    pub fn reset(&mut self, handle: &UiHandle) -> CadResult<()> {
        handle.set_cad_frame(slint::Image::default())?;
        self.bound = None;
        Ok(())
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
