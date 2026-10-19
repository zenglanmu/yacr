//! Presentation, sizing, session binding and teardown methods of
//! [`GtkOverlayOwner`], plus its `Drop`.

use cad_engine::render::RenderSessionHandle;
use cad_render_wgpu::{NativeTextureKey, ProbeFrameStats};
use gtk::prelude::*;

use super::native::{
    apply_input_passthrough, checked_physical_size, render_error_class, wayland_buffer_scale,
};
use super::{BridgeState, CadFrameObservation, FrameCadStages, GtkOverlayOwner, LayoutGeometry};

impl GtkOverlayOwner {
    /// Force the overlay to unrealize (evidence-only lifecycle driver). Runs the
    /// registered unrealize hook, which releases the GPU **before** the native
    /// widgets are destroyed. Never called on a normal product path.
    pub fn force_unrealize(&mut self) {
        self.overlay.unrealize();
        eprintln!(
            "[probe] bridge: forced overlay unrealize -> state={:?} generation={}",
            self.state, self.generation
        );
    }

    /// Re-realize and re-map the overlay after a forced unrealize, re-asserting
    /// the input pass-through so the next tick can create a new GPU generation.
    pub fn force_realize(&mut self) {
        // `realize()` alone does not map; re-map the overlay (and the native
        // drawing area) so the next tick can classify a steady native window.
        self.overlay.show_all();
        self.overlay.realize();
        self.overlay.map();
        self.drawing_area.realize();
        self.drawing_area.map();
        apply_input_passthrough(&self.overlay, &self.drawing_area);
        eprintln!(
            "[probe] bridge: forced overlay realize/map -> state={:?} generation={}",
            self.state, self.generation
        );
    }

    /// The presenter's current surface size, if initialized.
    pub fn surface_size(&self) -> Option<(u32, u32)> {
        self.presenter.as_ref().map(|p| p.surface_size())
    }

    /// Adapter description, if initialized.
    pub fn adapter(&self) -> Option<&cad_render_wgpu::ProbeAdapterInfo> {
        self.presenter.as_ref().map(|p| p.adapter())
    }

    /// Present mode, if initialized.
    pub fn present_mode(&self) -> Option<cad_render_wgpu::wgpu::PresentMode> {
        self.presenter.as_ref().map(|p| p.present_mode())
    }

    /// Resize the surface (allocation * scale).
    pub fn resize_to(&mut self, phys_w: u32, phys_h: u32) -> Result<(), String> {
        // Pre-presentation native-health check (Wayland): a released/replaced
        // surface is never resized or presented on.
        self.enforce_wayland_health()?;
        // Keep the Wayland subsurface's logical position/scale in step with the
        // drawing area even when the physical size is unchanged (a pure layout
        // shift), before the size short-circuit below.
        self.sync_wayland_layout();
        let Some(presenter) = self.presenter.as_mut() else {
            return Ok(());
        };
        if presenter.surface_size() == (phys_w, phys_h) {
            return Ok(());
        }
        presenter
            .resize(phys_w, phys_h)
            .map_err(|error| format!("probe resize failed: {error}"))?;
        self.size = (phys_w, phys_h);
        Ok(())
    }

    /// Render one frame.
    pub fn frame(
        &mut self,
        marker: f32,
    ) -> Option<Result<(u64, cad_render_wgpu::ProbeStageTimings), String>> {
        // Native health before presenting (Wayland); a released surface yields an
        // explicit error instead of a stale submit.
        if let Err(error) = self.enforce_wayland_health() {
            return Some(Err(error));
        }
        let presenter = self.presenter.as_mut()?;
        Some(match presenter.frame(marker) {
            Ok(stats) => Ok((stats.frame_index, stats.timings)),
            Err(error) => Err(format!("probe frame failed: {error}")),
        })
    }

    /// Bind (or clear) the CAD render session for this owner.
    ///
    /// A live presenter is first detached from the old session and the retained
    /// frame cache is cleared, then the new session is attached to the current
    /// device via [`SurfaceProbePresenter::device_queue`]. With no presenter the
    /// session is stored and attached on the next successful `try_init_gpu`.
    pub fn set_render_session(
        &mut self,
        session: Option<RenderSessionHandle>,
    ) -> Result<(), String> {
        if let Some(previous) = self.session.take() {
            previous.detach_device();
        }
        // Replacing the session invalidates any cached native-texture binding.
        self.clear_presenter_cache();
        let Some(session) = session else {
            return Ok(());
        };
        if let Some(presenter) = self.presenter.as_ref() {
            let (device, queue) = presenter.device_queue();
            if let Err(error) = session.attach_device_with_gpu_time(device, queue, self.gpu_time) {
                session.fail_device(format!("attach_device: {error}"), false);
                return Err(format!("render session attach failed: {error}"));
            }
        }
        self.session = Some(session);
        Ok(())
    }

    /// Draw one CAD frame through the bound render session and present it.
    ///
    /// Requires an attached session and a live presenter. `Ok(None)` means the
    /// engine has no new frame **and** no reusable retained frame: the caller
    /// waits — no blank or fake grid is drawn. Frame-state failures are explicit;
    /// a render error marks the session device as failed and does not loop.
    pub fn frame_cad(
        &mut self,
        logical: [f64; 2],
        scale: f64,
    ) -> Result<Option<ProbeFrameStats>, String> {
        // Native health before presenting (Wayland); a released/replaced surface
        // is never drawn into.
        self.enforce_wayland_health()?;
        let Some(session) = self.session.clone() else {
            return Err("no render session attached".into());
        };
        if self.presenter.is_none() {
            return Err("no GPU presenter (bridge not ready)".into());
        }
        // `apply_ready_updates` is a CadError: any failure fails the session
        // device and clears the binding cache, so an old frame can never be
        // replayed as a clean success.
        let apply_started = std::time::Instant::now();
        if let Err(error) = session.apply_ready_updates() {
            session.fail_device(format!("apply_ready_updates: {error:?}"), false);
            self.clear_presenter_cache();
            return Err(format!("apply_ready_updates failed: {error:?}"));
        }
        let apply_us = apply_started.elapsed().as_micros() as u64;
        let render_started = std::time::Instant::now();
        let presented = match session.render(logical, scale) {
            Ok(presented) => presented,
            Err(error) => {
                // Only a device loss fails the session device. Camera/size/
                // upload/scene errors keep the device attached and are returned
                // with their own class.
                let device_loss = error.is_device_loss();
                if device_loss {
                    session.fail_device(format!("render device lost: {error}"), true);
                }
                self.clear_presenter_cache();
                return Err(format!(
                    "render failed ({}) : {error}",
                    render_error_class(&error)
                ));
            }
        };

        match presented {
            Some(frame) => {
                let key = NativeTextureKey {
                    device_epoch: frame.binding.device_epoch,
                    texture_revision: frame.binding.texture_revision,
                    size: frame.binding.size,
                };
                let present_started = std::time::Instant::now();
                let result = self
                    .presenter
                    .as_mut()
                    .expect("presenter checked above")
                    .present_texture(&frame.texture, key);
                let present_us = present_started.elapsed().as_micros() as u64;
                self.last_frame_stages = Some(FrameCadStages {
                    apply_us,
                    render_us: render_started.elapsed().as_micros() as u64,
                    present_us,
                    presented: result.is_ok(),
                });
                match result {
                    Ok(stats) => {
                        // Record only the actually presented frame's metadata; a
                        // `None`/error never overwrites it, and the host consumes
                        // it once per presented frame.
                        self.last_cad_frame = Some(CadFrameObservation {
                            rendered_view_revision: frame.view_revision,
                            completeness: frame.completeness,
                            device_epoch: frame.binding.device_epoch,
                            texture_revision: frame.binding.texture_revision,
                            size: frame.binding.size,
                            frame_id: stats.frame_index,
                            timings: stats.timings,
                        });
                        Ok(Some(stats))
                    }
                    Err(error) => {
                        self.clear_presenter_cache();
                        Err(format!("present_texture failed: {error}"))
                    }
                }
            }
            // The engine returns its cached texture whenever the runtime is
            // `Ready`, so `None` means "not ready": wait. Never replay an old
            // frame and never draw a blank/fake grid.
            None => {
                // Record the waiting bracket too (a slow `render` while waiting is
                // exactly the host-frame gap to localize); no present happened.
                self.last_frame_stages = Some(FrameCadStages {
                    apply_us,
                    render_us: render_started.elapsed().as_micros() as u64,
                    present_us: 0,
                    presented: false,
                });
                Ok(None)
            }
        }
    }

    /// The last actually presented CAD frame's metadata (read-only peek).
    ///
    /// Stale (from an earlier successful present) after a `None`/error frame;
    /// the host must use [`Self::take_last_cad_frame_observation`] to log it
    /// exactly once per presented frame.
    pub fn last_cad_frame_observation(&self) -> Option<CadFrameObservation> {
        self.last_cad_frame
    }

    /// Take (consume) the last presented CAD frame's metadata, so a host logs it
    /// exactly once per presented frame and never replays a stale observation.
    pub fn take_last_cad_frame_observation(&mut self) -> Option<CadFrameObservation> {
        self.last_cad_frame.take()
    }

    /// Take (consume) the last `frame_cad` stage timings, so a host logs them at
    /// most once per call and never replays a stale value.
    pub fn take_frame_cad_stages(&mut self) -> Option<FrameCadStages> {
        self.last_frame_stages.take()
    }

    /// Drop the presenter's cached native-texture binding.
    fn clear_presenter_cache(&mut self) {
        if let Some(presenter) = self.presenter.as_mut() {
            presenter.clear_binding_cache();
        }
    }

    /// The logical size of the GTK container that **receives the CAD margins**
    /// (the overlay the React WebView fills), or `None` while its allocation is
    /// not settled (zero). These logical px are the frontend's DOM/CSS coordinate
    /// space, so the host must use them as the reference bounds for the viewport
    /// insets — never the Tauri window `inner_size()`.
    pub fn logical_container_bounds(&self) -> Option<(f32, f32)> {
        let width = self.overlay.allocated_width();
        let height = self.overlay.allocated_height();
        if width <= 0 || height <= 0 {
            return None;
        }
        Some((width as f32, height as f32))
    }

    /// Logical geometry snapshot for the bounded geometry telemetry (overlay /
    /// drawing-area allocation, translated origin, integer scale).
    pub fn layout_geometry(&self) -> LayoutGeometry {
        LayoutGeometry {
            container: (
                self.overlay.allocated_width(),
                self.overlay.allocated_height(),
            ),
            container_origin: self.overlay.translate_coordinates(&self.top_level, 0, 0),
            drawing_area: (
                self.drawing_area.allocated_width(),
                self.drawing_area.allocated_height(),
            ),
            origin: self
                .drawing_area
                .translate_coordinates(&self.top_level, 0, 0),
            scale: wayland_buffer_scale(self.drawing_area.scale_factor()),
        }
    }

    /// Current physical size the surface should be (allocation × integer scale).
    ///
    /// Uses the same checked rule as GPU init; an invalid/overflowing size is
    /// reported as `0x0` (the host skips the frame) with an explicit log — never a
    /// wrapping or saturating fake value.
    pub fn desired_size(&self) -> (u32, u32) {
        let scale = wayland_buffer_scale(self.drawing_area.scale_factor());
        match checked_physical_size(
            self.drawing_area.allocated_width(),
            self.drawing_area.allocated_height(),
            scale,
        ) {
            Ok(size) => size,
            Err(error) => {
                eprintln!("[probe] bridge desired_size invalid: {error}; reporting 0x0");
                (0, 0)
            }
        }
    }

    /// Whether GPU init failed (explicit, never a fallback).
    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            BridgeState::Failed(message) => Some(message.as_str()),
            _ => None,
        }
    }

    /// Stop and drop the GPU presenter **before** any native widget teardown.
    /// Take any live presenter, stop and drop it, and log (B1).
    ///
    /// There is no permanent `released` latch: the slot is simply emptied, so a
    /// later re-init is safe and a repeated release is a no-op.
    pub fn release_gpu(&mut self) {
        // Detach the render session before any GPU object is dropped, then clear
        // the retained frame (it belongs to the dying device).
        if let Some(session) = self.session.as_ref() {
            session.detach_device();
            eprintln!("[probe] bridge: render session detached (before GPU drop)");
        }
        self.clear_presenter_cache();
        self.last_cad_frame = None;
        if let Some(mut presenter) = self.presenter.take() {
            presenter.stop();
            drop(presenter);
            eprintln!(
                "[probe] bridge: GPU released (generation {}; surface before native widgets)",
                self.generation
            );
        }
        // Ordering invariant: the GPU surface is already gone; now the app-owned
        // child `wl_surface`/`wl_subsurface`. This runs from the unrealize hooks,
        // i.e. before GTK destroys/unrealizes the borrowed parent surface.
        self.release_wayland_child();
        if self.state != BridgeState::Closing && !matches!(self.state, BridgeState::Failed(_)) {
            self.state = BridgeState::Suspended;
        }
    }

    /// Enter the terminal closing state; never inits again.
    pub fn close(&mut self) {
        self.state = BridgeState::Closing;
        self.release_gpu();
    }

    /// The current lifecycle state.
    pub fn state(&self) -> &BridgeState {
        &self.state
    }

    /// The current GPU generation (incremented per successful init).
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for GtkOverlayOwner {
    fn drop(&mut self) {
        // Guarantee ordering even if a hook did not fire: GPU first, then GTK.
        self.release_gpu();
    }
}
