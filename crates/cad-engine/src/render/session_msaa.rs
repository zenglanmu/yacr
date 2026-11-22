use super::*;

impl RenderSessionHandle {
    /// Request the MSAA sample count for the next device attach (`1`, `2` or
    /// `4`; anything else is treated as off).
    ///
    /// A live session also applies it on the next frame through the view
    /// snapshot (`RenderTarget::samples`), which reconciles the renderer's
    /// pipelines and attachments without a device re-attach.
    pub fn set_requested_msaa_samples(&self, samples: u32) {
        self.0.borrow_mut().msaa_samples = match samples {
            2 => 2,
            4 => 4,
            _ => 1,
        };
    }

    /// Record the adapter's supported MSAA sample counts for the next attach.
    /// An empty list means the WebGPU guarantee (`1`/`4`).
    pub fn set_msaa_supported_samples(&self, supported: Vec<u32>) {
        self.0.borrow_mut().msaa_supported_samples = supported;
    }
}
