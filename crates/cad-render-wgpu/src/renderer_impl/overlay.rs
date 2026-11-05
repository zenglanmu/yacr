//! The transient-overlay compositing path: draw-order lists, per-batch uniform
//! slots, the base→overlay copy and the overlay render pass.
//!
//! The base scene (persistent drawing + reference grid) owns the base colour and
//! depth targets. The transient overlay (cursor crosshair/pickbox, snap hints,
//! previews, sub-element highlight) is never part of the base page plan: on every
//! render the base colour is copied into the presented overlay colour and the
//! overlay batches are drawn on top with an independently cleared depth. This is
//! what keeps a suffix commit from clearing or re-paging the base and keeps the
//! cursor visible over a partially drawn base page.

use super::*;

/// The overlay's draw-order lists plus the identity cull its pass uses.
///
/// Built from the resident batches after `base_end`; overlay batches are staged
/// legacy, so each contributes exactly one per-batch draw and needs one uniform
/// slot.
pub(super) struct OverlayPlan {
    pub(super) opaque: Vec<usize>,
    pub(super) transparent: Vec<usize>,
    pub(super) invisible: usize,
    cull: ChunkCull,
}

impl OverlayPlan {
    /// Number of overlay batches that will be drawn (opaque + transparent).
    pub(super) fn drawn_batches(&self) -> usize {
        self.opaque.len() + self.transparent.len()
    }
}

/// Counters produced by the overlay composite pass, merged into `FrameStats`.
pub(super) struct OverlayOutcome {
    pub(super) submission: wgpu::SubmissionIndex,
    pub(super) draw_calls: usize,
    pub(super) opaque_batches: usize,
    pub(super) transparent_batches: usize,
    pub(super) invisible_batches: usize,
    pub(super) encode_ms: f64,
    pub(super) set_pipeline_calls: u64,
    pub(super) set_bind_group_calls: u64,
    pub(super) set_vertex_buffer_calls: u64,
    pub(super) set_index_buffer_calls: u64,
}

impl Renderer {
    /// Build the overlay draw-order lists from the resident overlay range.
    ///
    /// Every batch after `base_end`, in the scene's visual order (opaque then
    /// transparent), with the identity cull (an overlay batch is always drawn;
    /// its geometry is never stream/chunk/conic-converted).
    pub(super) fn plan_overlay(&self, camera_position: [f32; 3]) -> OverlayPlan {
        let overlay: Vec<usize> = (self.base_end..self.batches.len()).collect();
        let entries: Vec<BatchOrderEntry> = overlay
            .iter()
            .map(|&index| {
                let batch = &self.batches[index];
                BatchOrderEntry {
                    draw_order: batch.draw_order,
                    alpha: batch.alpha,
                    centroid: batch.centroid,
                }
            })
            .collect();
        let passes = draw_passes(&overlay, &entries, Some(camera_position));
        OverlayPlan {
            opaque: passes.opaque,
            transparent: passes.transparent,
            invisible: passes.invisible,
            cull: ChunkCull::all_visible(),
        }
    }

    /// Write one per-batch uniform slot for every drawn overlay batch, starting
    /// at `first_slot`; returns the next free slot.
    ///
    /// The shape is identical to the base legacy loop so the same shaders and
    /// flag tint apply. Overlay batches own no stream/chunk/arc slots. Takes the
    /// disjoint arena/slot/batch state (not `&mut self`) because `render` holds
    /// immutable view/pipeline borrows of other `Renderer` fields.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn write_overlay_uniforms(
        batches: &[GpuBatch],
        frame_slots: &mut [u32],
        uniforms: &mut UniformArena,
        projection: &FrameProjection,
        light: [f32; 4],
        line_wpp_bits: u32,
        first_slot: usize,
        plan: &OverlayPlan,
    ) -> usize {
        let mut next_slot = first_slot;
        for &index in plan.opaque.iter().chain(plan.transparent.iter()) {
            let batch = &batches[index];
            let (m, [r, g, b], alpha) = (
                projection.batch_matrix(batch.origin),
                renderer_color(batch.color),
                clamp_alpha(batch.alpha),
            );
            let uniform = uniform_bytes(
                &m,
                [r, g, b, alpha],
                [batch.pick_key, batch.instance_flags(), line_wpp_bits, 0],
                light,
            );
            uniforms.write_slot(next_slot, &uniform);
            frame_slots[index] = next_slot as u32;
            next_slot += 1;
        }
        next_slot
    }

    /// Copy the base colour into the presented overlay colour.
    ///
    /// Both attachments exist after `ensure_target`; the base colour carries
    /// `COPY_SRC` and the overlay `COPY_DST`. The copy runs on every render (even
    /// with no overlay batches) so `frame_texture` is always the overlay.
    pub(super) fn copy_base_to_overlay(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: self.target.as_ref().expect("base target"),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: self.overlay_target.as_ref().expect("overlay target"),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.target_size.0.max(1),
                height: self.target_size.1.max(1),
                depth_or_array_layers: 1,
            },
        );
    }

    /// Encode and submit the overlay composite pass into the presented target.
    ///
    /// Runs in a submission **after** the base page + base→overlay copy, so queue
    /// order makes the copied base visible. The overlay depth is cleared every
    /// frame (overlay geometry always draws on top); the base depth is untouched.
    /// No images, masks, stream/chunk/arc slots or pulled organisation are used.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode_overlay_pass(
        &self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        overlay_view: &wgpu::TextureView,
        overlay_depth_view: &wgpu::TextureView,
        background: [f32; 4],
        plan: &OverlayPlan,
        line_pipeline: &wgpu::RenderPipeline,
        line_instanced_pipeline: &wgpu::RenderPipeline,
        arc_pipeline: &wgpu::RenderPipeline,
        mesh_pipeline: &wgpu::RenderPipeline,
        mesh_pipeline_mirrored: &wgpu::RenderPipeline,
        mesh_pipeline_transparent: &wgpu::RenderPipeline,
        mesh_pipeline_mirrored_transparent: &wgpu::RenderPipeline,
        mesh_pipeline_merged: &wgpu::RenderPipeline,
        mesh_pipeline_merged_mirrored: &wgpu::RenderPipeline,
        image_pipeline: &wgpu::RenderPipeline,
        unit_quad_buffer: &wgpu::Buffer,
    ) -> OverlayOutcome {
        let outcome = self.encode_frame(
            device,
            overlay_view,
            overlay_depth_view,
            false,
            true,
            background,
            false,
            &plan.opaque,
            &plan.transparent,
            &[],
            &plan.cull,
            &[],
            &[],
            &[],
            &[],
            line_pipeline,
            line_instanced_pipeline,
            arc_pipeline,
            mesh_pipeline,
            mesh_pipeline_mirrored,
            mesh_pipeline_transparent,
            mesh_pipeline_mirrored_transparent,
            mesh_pipeline_merged,
            mesh_pipeline_merged_mirrored,
            image_pipeline,
            unit_quad_buffer,
        );
        let submission = queue.submit(Some(outcome.encoder.finish()));
        OverlayOutcome {
            submission,
            draw_calls: plan.drawn_batches(),
            opaque_batches: plan.opaque.len(),
            transparent_batches: plan.transparent.len(),
            invisible_batches: plan.invisible,
            encode_ms: outcome.encode_ms,
            set_pipeline_calls: outcome.encode_state.tally.set_pipeline_calls,
            set_bind_group_calls: outcome.encode_state.tally.set_bind_group_calls,
            set_vertex_buffer_calls: outcome.encode_state.tally.set_vertex_buffer_calls,
            set_index_buffer_calls: outcome.encode_state.tally.set_index_buffer_calls,
        }
    }
}
