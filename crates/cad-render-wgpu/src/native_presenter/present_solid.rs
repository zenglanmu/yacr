//! The `YACR_PROBE_SOLID_FRAME` diagnostic present path.
//!
//! Kept in its own module so the presenter stays under the code-file size limit;
//! it is still `impl SurfaceProbePresenter` over the same surface/device state
//! (the fields it touches are `pub(super)`).

use super::*;

impl SurfaceProbePresenter {
    /// Present a **diagnostic solid-colour frame**: acquire → clear to an opaque
    /// colour that alternates magenta/olive per frame → present.
    ///
    /// Selected by the host's presence-based `YACR_PROBE_SOLID_FRAME` knob. It
    /// skips the CAD texture blit **and** the texture/surface size guard
    /// entirely, which is exactly what makes it discriminating: a visible
    /// alternating field proves the swapchain/composition path presents, so a
    /// still-transparent CAD region then isolates the defect to the texture/size
    /// path (the presenter's `TextureInvalid` guard) rather than to the surface.
    /// Because the colour alternates, a stale buffer that is re-presented forever
    /// is visible too.
    ///
    /// Diagnostic only — never acceptance evidence, and never a product path.
    pub fn present_solid(&mut self) -> Result<ProbeFrameStats, ProbeError> {
        let frame_started = std::time::Instant::now();
        if self.stopped {
            return Err(ProbeError::Stopped);
        }
        let mut timings = ProbeStageTimings::default();
        let (frame, configure_us) = self.acquire_frame()?;
        timings.acquire_us = frame_started.elapsed().as_micros() as u64;
        timings.configure_us = configure_us;

        // Opaque and high-contrast, alternating so a never-updated surface shows.
        let color = if self.frame_index % 2 == 0 {
            wgpu::Color {
                r: 1.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            }
        } else {
            wgpu::Color {
                r: 0.5,
                g: 0.5,
                b: 0.0,
                a: 1.0,
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cad-solid-frame-encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad-solid-frame-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let stage = std::time::Instant::now();
        self.queue.submit(Some(encoder.finish()));
        timings.submit_us = stage.elapsed().as_micros() as u64;

        let stage = std::time::Instant::now();
        if let Some(error) = collect_error_scope_bounded(&self.device, scope, SCOPE_COLLECT_BUDGET)
        {
            // The frame is dropped without present, which discards the acquired
            // image; the failure is explicit rather than a silent blank.
            return Err(ProbeError::Frame(format!(
                "solid frame rejected by GPU validation: {error}"
            )));
        }
        timings.scope_pop_us = stage.elapsed().as_micros() as u64;

        timings.present_us = self.present_in_scope(frame)?;
        self.frame_index += 1;
        timings.total_us = frame_started.elapsed().as_micros() as u64;
        Ok(ProbeFrameStats {
            frame_index: self.frame_index,
            marker_offset: 0.0,
            surface_size: self.size,
            timings,
        })
    }
}
