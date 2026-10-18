use super::*;

impl Renderer {
    /// Reset derived GPU resources after a device loss.
    ///
    /// The old device cannot be reused, so this only tears state down and marks
    /// the renderer as lost. The host must call [`Renderer::initialize_with_device`]
    /// and re-upload the scene; batches are *not* silently recreated here.
    pub fn rebuild_device(&mut self, _target: &RenderTarget) -> CadResult<()> {
        self.line_pipeline = None;
        self.line_instanced_pipeline = None;
        self.arc_pipeline = None;
        self.unit_quad_buffer = None;
        self.mesh_pipeline = None;
        self.mesh_pipeline_mirrored = None;
        self.mesh_pipeline_transparent = None;
        self.mesh_pipeline_mirrored_transparent = None;
        self.mesh_pipeline_merged = None;
        self.mesh_pipeline_merged_mirrored = None;
        self.image_pipeline = None;
        self.image_texture_layout = None;
        self.image_sampler = None;
        self.layout = None;
        self.target = None;
        self.target_view = None;
        self.depth_view = None;
        self.batches.clear();
        self.images.clear();
        self.image_textures.clear();
        self.uniforms.reset();
        self.frame_slots.clear();
        self.image_slots.clear();
        self.lineweight_not_drawn.clear();
        self.device = None;
        self.queue = None;
        self.active_backend = None;
        // Measurement resources belong to the dropped device generation.
        self.gpu_time = None;
        self.gpu_time_state = gpu_time::GpuTimeState::Disabled;
        self.device_lost = true;
        self.draw_plan = None;
        self.instance_streams.clear();
        self.arc_streams.clear();
        self.mesh_chunks.clear();
        self.pick_target = None;
        self.pick_view = None;
        self.pick_depth_view = None;
        self.pick_size = (0, 0);
        self.pick_line_pipeline = None;
        self.pick_line_instanced_pipeline = None;
        self.pick_mesh_pipeline = None;
        self.pick_mesh_pipeline_mirrored = None;
        self.pick_mesh_merged_pipeline = None;
        self.pick_mesh_merged_pipeline_mirrored = None;
        self.pick_image_pipeline = None;
        self.flags_layout = None;
        self.flags_texture = None;
        self.flags_view = None;
        self.flags_bind_group = None;
        self.flags_ctl_buffer = None;
        self.pulled_lines = None;
        self.pulled_mesh = None;
        self.pulled_camera_bind_group = None;
        self.invalidate_pulled_prefix();
        // The pulled organisation is gone with the device: report it as
        // unavailable rather than leaving a stale `Active`.
        self.pulled_state = PulledState::Unavailable;
        self.line_pulled_pipeline = None;
        self.mesh_pulled_pipeline = None;
        self.mesh_pulled_mirrored_pipeline = None;
        self.last_projection = None;
        self.last_camera_position = None;
        // A device rebuild is a new device generation: retire the request too.
        self.reset_pick_keys_and_drop_request();
        Err(CadError::GpuFailure(
            "device rebuild requires a new shared device from the host".into(),
        ))
    }

    pub fn switch_backend(&mut self, preference: BackendPreference) -> CadResult<()> {
        self.preference = preference;
        self.clear_batches();
        Ok(())
    }

    /// Mark the device as lost after the host's device-lost callback fires.
    ///
    /// The renderer cannot own the lost callback itself because the device is
    /// shared with the host. The host calls this when wgpu reports
    /// `DeviceLostReason`, or when a backend/surface failure is observed. All
    /// derived GPU resources are dropped; the host must re-initialize and
    /// re-upload the scene. Batches are never silently recreated.
    pub fn note_device_lost(&mut self, detail: impl Into<String>) -> RenderError {
        let detail = detail.into();
        self.line_pipeline = None;
        self.line_instanced_pipeline = None;
        self.arc_pipeline = None;
        self.unit_quad_buffer = None;
        self.mesh_pipeline = None;
        self.mesh_pipeline_mirrored = None;
        self.mesh_pipeline_transparent = None;
        self.mesh_pipeline_mirrored_transparent = None;
        self.mesh_pipeline_merged = None;
        self.mesh_pipeline_merged_mirrored = None;
        self.image_pipeline = None;
        self.image_texture_layout = None;
        self.image_sampler = None;
        self.layout = None;
        self.target = None;
        self.target_view = None;
        self.depth_view = None;
        self.batches.clear();
        self.images.clear();
        self.image_textures.clear();
        self.uniforms.reset();
        self.frame_slots.clear();
        self.image_slots.clear();
        self.lineweight_not_drawn.clear();
        self.active_backend = None;
        self.device_lost = true;
        self.draw_plan = None;
        self.instance_streams.clear();
        self.arc_streams.clear();
        self.mesh_chunks.clear();
        self.pick_target = None;
        self.pick_view = None;
        self.pick_depth_view = None;
        self.pick_size = (0, 0);
        self.flags_layout = None;
        self.flags_texture = None;
        self.flags_view = None;
        self.flags_bind_group = None;
        self.flags_ctl_buffer = None;
        self.last_projection = None;
        self.last_camera_position = None;
        // Device loss is a new device generation: the key space and the retained
        // entity-flag request are both retired with the old device.
        self.reset_pick_keys_and_drop_request();
        // The resident scene is gone with the device; drop its cost knowledge too.
        self.reset_pacing();
        // Derived GPU resources are torn down by the loss; the pulled organisation
        // is no longer active. Reported so a reader never sees a stale `Active`.
        self.pulled_state = PulledState::Unavailable;
        self.last_device_lost = Some(detail.clone());
        RenderError::DeviceLost(detail)
    }
}
