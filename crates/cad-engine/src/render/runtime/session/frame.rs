//! Frame presentation for the resident scene.
//!
//! Split out of `session.rs`; behaviour is unchanged.

use super::super::*;

impl CadRenderRuntime {
    /// Render the resident scene. `completeness` is the caller's honest label for
    /// what this frame represents: [`FrameCompleteness::CurrentScene`] when the
    /// latest published scene is committed, [`FrameCompleteness::PendingOverlayInputs`]
    /// when newer transient overlay inputs are still preparing. The frame always
    /// shows the **whole** resident scene — the overlay suffix is never truncated
    /// (a snapshot/preview update must not drop the tint it is updating). The
    /// renderer-owned cursor is fed separately (see [`Self::render_frame`]) and is
    /// never part of that suffix.
    pub fn render_if_dirty(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
        view_revision: u64,
        completeness: FrameCompleteness,
        pause_after_page: bool,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        self.render_frame(
            view,
            logical,
            scale,
            view_revision,
            completeness,
            pause_after_page,
        )
    }

    fn render_frame(
        &mut self,
        view: &ViewSnapshot,
        logical: [f64; 2],
        scale: f64,
        view_revision: u64,
        completeness: FrameCompleteness,
        pause_after_page: bool,
    ) -> Result<Option<PresentedFrame>, RenderError> {
        if !matches!(self.lifecycle, RenderLifecycle::Ready { .. }) {
            return Ok(None);
        }
        if logical.iter().any(|v| !v.is_finite() || *v <= 0.0) || !scale.is_finite() || scale <= 0.0
        {
            return Err(RenderError::Frame("invalid CAD surface dimensions".into()));
        }
        // The one size authority: the same function a native host uses to size
        // its CAD child surface (see `render_target_size`'s contract).
        let size = render_target_size(logical, scale);
        if self.target_size != Some(size) {
            self.dirty.invalidate();
        }
        let renderer = self.renderer.as_mut().expect("ready renderer");
        // An attachment can change at the same size. Its identity is inspected
        // for presentation even when CAD pixels do not need redrawing.
        if self.dirty.dirty() {
            // Feed the renderer-owned transient cursor (ADR 0028) before the
            // camera/render call. This is the only per-move renderer input: it
            // writes the tiny transient instance buffer and never touches the
            // committed batches, revisions, planning or pacing.
            renderer.set_cursor_transient(self.cursor_transient.clone());
            // Per-frame PBR environment switch (ADR-0044): uniform data only, no
            // pipeline/scene rebuild.
            renderer.set_ibl_enabled(view.ibl_enabled);
            // SketchUp Shadows/Fog: per-frame uniform data only, like the IBL
            // switch above. Cast shadows are not implemented.
            renderer.set_sun(
                view.sun
                    .map(|sun| [sun[0] as f32, sun[1] as f32, sun[2] as f32]),
            );
            renderer.set_fog(view.fog.map(|fog| cad_render_wgpu::FrameFog {
                color: fog.color,
                start: fog.start as f32,
                end: fog.end as f32,
            }));
            // MSAA is a 3D-view quality setting: the 2D path already has
            // analytic 1 px line coverage (ADR-0009) and must keep its exact
            // pixels at any requested level. A view-mode switch is a real
            // sample-count change, so the renderer reconfigures once.
            let msaa_samples = if view.mode == ProjectionKind::ThreeD {
                view.msaa_samples
            } else {
                1
            };
            let target = RenderTarget::new(size.0, size.1)
                .with_clear_color(view.clear_color)
                .with_samples(msaa_samples);
            let stats = match view.mode {
                ProjectionKind::TwoD => {
                    let params = view
                        .camera
                        .camera2d_params()
                        .map_err(|e| RenderError::Frame(e.to_string()))?;
                    let mut camera = camera2d_from_params(params);
                    // Full-window hosts place the camera on the visible rect.
                    if let Some(origin) = view.canvas_origin {
                        let target = [f64::from(size.0) / scale, f64::from(size.1) / scale];
                        camera = place_canvas_camera2d(camera, target, origin, view.canvas_size)
                            .ok_or_else(|| RenderError::Frame("placement invalid".into()))?;
                    }
                    camera.world_per_px /= scale;
                    // A changed camera restarts the walk; a paused frame uses
                    // the retained-base variant (cursor-only overlay).
                    renderer.render_navigation(camera, &target, pause_after_page)?
                }
                ProjectionKind::ThreeD => {
                    let params = view
                        .camera
                        .camera3d_params()
                        .map_err(|e| RenderError::Frame(e.to_string()))?;
                    renderer.render_3d_navigation(
                        camera3d_from_params(params),
                        &target,
                        pause_after_page,
                    )?
                }
            };
            self.target_size = Some(size);
            let progressive_complete = !renderer.frame_pending();
            let progressive = renderer.frame_progress();
            // One page per gesture change; `presented_revision` stays honest.
            if progressive_complete || pause_after_page {
                self.dirty.complete();
            }
            self.frames_rendered += 1;
            self.presented_revision = if progressive_complete {
                self.applied_revision
            } else {
                None
            };
            // Capture the attachment identity/texture, ending the `renderer` field
            // borrow, then record the **actual** renderer invocation (the stats
            // that would otherwise be dropped). Retained/reused frames never reach
            // here.
            let texture_revision = renderer.texture_revision();
            let texture = renderer.frame_texture().cloned();
            self.record_render_observation(stats, view_revision, progressive, progressive_complete);
            return Ok(texture.map(|texture| PresentedFrame {
                binding: FrameBinding {
                    device_epoch: self.epoch,
                    texture_revision,
                    size,
                },
                view_revision,
                completeness,
                texture,
            }));
        }
        Ok(renderer.frame_texture().map(|texture| PresentedFrame {
            binding: FrameBinding {
                device_epoch: self.epoch,
                texture_revision: renderer.texture_revision(),
                size,
            },
            view_revision,
            completeness,
            texture: texture.clone(),
        }))
    }
}
