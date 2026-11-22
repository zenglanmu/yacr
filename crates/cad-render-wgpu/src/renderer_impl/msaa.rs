use super::*;

/// The WebGPU-guaranteed MSAA counts for the renderer's colour format.
const SPEC_SAMPLES: [u32; 2] = [1, 4];

/// Resolve a requested sample count against what the adapter reports.
///
/// Only `1`, `2` and `4` are ever requested by the settings surface. A count
/// the adapter does not support is lowered to the greatest supported count that
/// does not exceed the request (so a `2x` request on a `{1,4}` device becomes
/// `1x`, never a more expensive `4x`). `1x` is always available.
pub(crate) fn effective_samples(requested: u32, supported: &[u32]) -> u32 {
    let requested = match requested {
        2 => 2,
        4 => 4,
        _ => 1,
    };
    if requested == 1 {
        return 1;
    }
    let supported: &[u32] = if supported.is_empty() {
        &SPEC_SAMPLES
    } else {
        supported
    };
    if supported.contains(&requested) {
        return requested;
    }
    supported
        .iter()
        .copied()
        .filter(|count| *count > 1 && *count <= requested)
        .max()
        .unwrap_or(1)
}

/// Stable diagnostic when the requested count had to be lowered.
pub(crate) fn clamp_diagnostic(requested: u32, effective: u32) -> Option<String> {
    (requested != effective).then(|| {
        format!("render.msaa.unsupported_samples: requested={requested} effective={effective}")
    })
}

impl Renderer {
    /// Request the initial MSAA sample count for the next
    /// [`Renderer::initialize_with_device`].
    ///
    /// The host sets this from the persisted `view.msaa.mode` preference before
    /// attaching. Values other than `1`/`2`/`4` are treated as off.
    pub fn set_requested_msaa_samples(&mut self, samples: u32) {
        self.requested_msaa_samples = match samples {
            2 => 2,
            4 => 4,
            _ => 1,
        };
    }

    /// Tell the renderer which MSAA sample counts the attached adapter really
    /// supports. Empty means the WebGPU guarantee (`1` and `4`).
    ///
    /// The host reads this from its presenter's adapter (a `wgpu::Device` cannot
    /// answer it), and re-applies it after every device attach because a new
    /// adapter may differ.
    pub fn set_supported_msaa_samples(&mut self, supported: Vec<u32>) {
        let mut supported: Vec<u32> = supported
            .into_iter()
            .filter(|count| matches!(count, 1 | 2 | 4))
            .collect();
        supported.sort_unstable();
        supported.dedup();
        self.supported_msaa_samples = supported;
    }

    /// The effective sample count the current/last frame used (`1` = off).
    pub fn msaa_samples(&self) -> u32 {
        self.msaa_samples
    }

    /// Pipelines that draw into the persistent base target use this state; the
    /// overlay/mask passes deliberately stay at 1×.
    pub(super) fn msaa_state(&self) -> wgpu::MultisampleState {
        wgpu::MultisampleState {
            count: self.msaa_samples.max(1),
            ..Default::default()
        }
    }

    /// The requested sample count (what the settings asked for).
    pub fn requested_msaa_samples(&self) -> u32 {
        self.requested_msaa_samples
    }

    /// The labelled downgrade when the request could not be honoured.
    pub fn msaa_diagnostic(&self) -> Option<&str> {
        self.msaa_diagnostic.as_deref()
    }

    /// Reconcile the frame's requested sample count with the live device.
    ///
    /// Called from [`Renderer::ensure_target`] before allocation. A change
    /// rebuilds the base/overlay pipeline families (never the batches or the
    /// resident scene), drops the offscreen targets so the next allocation uses
    /// the new sample count, and bumps `texture_revision` so a progressive page
    /// walk restarts instead of loading a reallocated base.
    pub(super) fn reconcile_msaa(&mut self, requested: u32) -> Result<(), RenderError> {
        self.requested_msaa_samples = match requested {
            2 => 2,
            4 => 4,
            _ => 1,
        };
        let effective =
            effective_samples(self.requested_msaa_samples, &self.supported_msaa_samples);
        if effective != self.msaa_samples {
            let device = self
                .device
                .as_ref()
                .ok_or_else(|| RenderError::NotInitialized("renderer not initialized".into()))?
                .clone();
            self.msaa_samples = effective;
            self.rebuild_pipelines_for_msaa(&device)?;
        }
        self.msaa_diagnostic = clamp_diagnostic(self.requested_msaa_samples, self.msaa_samples);
        if let Some(diagnostic) = self.msaa_diagnostic.as_deref() {
            // One line per actual change; never per frame.
            if self.last_msaa_diagnostic.as_deref() != Some(diagnostic) {
                eprintln!("[render] {diagnostic}");
            }
        }
        self.last_msaa_diagnostic = self.msaa_diagnostic.clone();
        if self.target_samples != self.msaa_samples {
            // The old attachments cannot be reused across a sample-count change.
            self.target = None;
            self.target_view = None;
            self.depth_view = None;
            self.overlay_target = None;
            self.overlay_view = None;
            self.overlay_depth = None;
            self.overlay_depth_view = None;
            self.target_size = (0, 0);
            self.target_samples = 0;
            self.texture_revision = self.texture_revision.saturating_add(1);
        }
        Ok(())
    }

    /// Rebuild every pipeline that can draw into the base colour attachment.
    ///
    /// Mask pipelines stay at 1× by design (masks draw on the presented target),
    /// and pick pipelines stay at 1× (picking is exact). Textured/PBR pipelines
    /// are base-only; overlay batches are solid highlight/UI geometry.
    fn rebuild_pipelines_for_msaa(&mut self, device: &wgpu::Device) -> Result<(), RenderError> {
        let layout = self
            .layout
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("uniform layout missing".into()))?
            .clone();
        let flags_layout = self
            .flags_layout
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("flags layout missing".into()))?
            .clone();
        let image_texture_layout = self
            .image_texture_layout
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("image texture layout missing".into()))?
            .clone();
        let pbr_texture_layout = self
            .pbr_texture_layout
            .as_ref()
            .ok_or_else(|| RenderError::NotInitialized("pbr texture layout missing".into()))?
            .clone();
        let image_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cad-image-pipeline-layout"),
                bind_group_layouts: &[
                    Some(&layout),
                    Some(&flags_layout),
                    Some(&image_texture_layout),
                ],
                immediate_size: 0,
            });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad-pipeline-layout"),
            bind_group_layouts: &[Some(&layout), Some(&flags_layout)],
            immediate_size: 0,
        });
        let samples = self.msaa_samples;
        let accelerated = self.render_tier == RenderTier::Accelerated;
        let geometry = Self::create_geometry_pipelines(
            device,
            &pipeline_layout,
            &image_pipeline_layout,
            self.target_format,
            accelerated,
            samples,
        );
        self.line_pipeline = Some(geometry.line);
        self.line_instanced_pipeline = Some(geometry.line_instanced);
        self.arc_pipeline = Some(geometry.arc);
        self.mesh_pipeline = Some(geometry.mesh);
        self.mesh_pipeline_mirrored = Some(geometry.mesh_mirrored);
        self.mesh_pipeline_transparent = Some(geometry.mesh_transparent);
        self.mesh_pipeline_mirrored_transparent = Some(geometry.mesh_mirrored_transparent);
        self.mesh_pipeline_merged = Some(geometry.mesh_merged);
        self.mesh_pipeline_merged_mirrored = Some(geometry.mesh_merged_mirrored);
        self.image_pipeline = Some(geometry.image);

        let (textured, textured_mirrored, textured_transparent, textured_mirrored_transparent) =
            Self::create_mesh_textured_pipelines(
                device,
                &layout,
                &flags_layout,
                &image_texture_layout,
                self.target_format,
                samples,
            );
        self.mesh_pipeline_textured = Some(textured);
        self.mesh_pipeline_textured_mirrored = Some(textured_mirrored);
        self.mesh_pipeline_textured_transparent = Some(textured_transparent);
        self.mesh_pipeline_textured_mirrored_transparent = Some(textured_mirrored_transparent);

        let (full, uv, flat) = Self::create_mesh_pbr_pipelines(
            device,
            &layout,
            &flags_layout,
            &pbr_texture_layout,
            self.target_format,
            samples,
        );
        self.install_pbr_pipelines(full, uv, flat);
        self.rebuild_pulled_pipelines(device);

        self.ui_geometry = (samples > 1).then(|| {
            Self::create_geometry_pipelines(
                device,
                &pipeline_layout,
                &image_pipeline_layout,
                self.target_format,
                accelerated,
                1,
            )
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_samples_never_exceeds_the_request() {
        assert_eq!(effective_samples(4, &[]), 4);
        assert_eq!(effective_samples(2, &[]), 1);
        assert_eq!(effective_samples(4, &[1, 2, 4]), 4);
        assert_eq!(effective_samples(4, &[1, 2]), 2);
        assert_eq!(effective_samples(2, &[1, 4]), 1);
        assert_eq!(effective_samples(4, &[1]), 1);
        assert_eq!(effective_samples(0, &[1, 2, 4]), 1);
        assert_eq!(effective_samples(8, &[1, 2, 4]), 1);
    }

    #[test]
    fn a_clamp_is_named_and_an_exact_request_is_not() {
        assert_eq!(clamp_diagnostic(4, 4), None);
        assert_eq!(clamp_diagnostic(1, 1), None);
        assert_eq!(
            clamp_diagnostic(4, 1).as_deref(),
            Some("render.msaa.unsupported_samples: requested=4 effective=1")
        );
    }
}
