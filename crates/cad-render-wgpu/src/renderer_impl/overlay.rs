//! The transient-overlay compositing path: draw-order lists, per-batch uniform
//! slots, the base→overlay copy and the overlay render pass.
//!
//! The base scene (the persistent drawing: entities, images, wipeout masks) owns
//! the base colour and depth targets. The transient overlay (reference grid/axes,
//! cursor crosshair/pickbox, snap hints, previews, sub-element highlight) is never
//! part of the base page plan: on every
//! render the base colour is copied into the presented overlay colour and the
//! overlay batches are drawn on top with an independently cleared depth. This is
//! what keeps a suffix commit from clearing or re-paging the base and keeps the
//! cursor visible over a partially drawn base page.

use super::*;

/// The overlay's draw-order lists plus the identity cull its pass uses.
///
/// Built from the resident batches after `base_end` plus, when present, the
/// renderer-owned transient cursor sentinel ([`plan::TRANSIENT_CURSOR`]); overlay
/// batches are staged legacy, so each contributes exactly one per-batch draw and
/// needs one uniform slot. The transient sentinel is not a batch: it carries no
/// per-batch uniform from this list and is drawn from its own instance buffer.
pub(super) struct OverlayPlan {
    pub(super) opaque: Vec<usize>,
    pub(super) transparent: Vec<usize>,
    pub(super) invisible: usize,
    /// Whether the opaque list contains the transient cursor sentinel.
    pub(super) transient: bool,
    cull: ChunkCull,
}

impl OverlayPlan {
    /// Number of **resident overlay batches** that will be drawn (opaque +
    /// transparent), excluding the transient cursor sentinel.
    pub(super) fn drawn_batches(&self) -> usize {
        (self.opaque.len() + self.transparent.len()).saturating_sub(usize::from(self.transient))
    }
}

/// Counters produced by the overlay composite pass, merged into `FrameStats`.
pub(super) struct OverlayOutcome {
    pub(super) submission: wgpu::SubmissionIndex,
    pub(super) draw_calls: usize,
    pub(super) opaque_batches: usize,
    pub(super) transparent_batches: usize,
    pub(super) invisible_batches: usize,
    /// Renderer-owned transient cursor draws (0 or 1); folded into `draw_calls`
    /// only, never into the committed batch counters.
    pub(super) transient_draws: u64,
    pub(super) encode_ms: f64,
    pub(super) set_pipeline_calls: u64,
    pub(super) set_bind_group_calls: u64,
    pub(super) set_vertex_buffer_calls: u64,
    pub(super) set_index_buffer_calls: u64,
}

impl Renderer {
    /// The instanced-line slot-1 stride of the live pipeline: 56 B on the
    /// universal tier, 64 B on the accelerated tier (the same buffers double as
    /// pulled storage). The transient cursor's buffer/packing must match it.
    pub(super) fn transient_line_stride(&self) -> u32 {
        if self.render_tier == RenderTier::Accelerated {
            plan::PULLED_LINE_STRIDE
        } else {
            plan::LINE_INSTANCE_STRIDE
        }
    }

    /// Set (or clear) the renderer-owned transient cursor.
    ///
    /// Returns whether the stored state changed. An empty segment list is
    /// normalised to `None` (an explicit empty), so a producer that computes "no
    /// cursor" either way never leaves a stale sentinel behind. Packing is pure
    /// CPU work; the GPU buffer is written on the next render ([`Self::render`])
    /// only when this is dirty or the buffer had to be (re)created.
    ///
    /// This never touches `batches`, `draw_plan`, `BatchFacts`, pick keys,
    /// revisions or commits — it is the Phase 2a transient path (ADR 0028).
    pub fn set_cursor_transient(&mut self, cursor: Option<cad_scene::TransientCursor>) -> bool {
        let cursor = cursor.filter(|cursor| !cursor.segments.is_empty());
        if self.cursor_transient == cursor {
            return false;
        }
        self.repack_cursor_transient(&cursor);
        self.cursor_transient = cursor;
        self.cursor_transient_dirty = true;
        true
    }

    /// Pack `cursor`'s instances at the live pipeline stride.
    fn repack_cursor_transient(&mut self, cursor: &Option<cad_scene::TransientCursor>) {
        let stride = self.transient_line_stride();
        self.cursor_transient_packed = cursor
            .as_ref()
            .map(|cursor| {
                plan::transient_instance_bytes(&plan::pack_transient_cursor(cursor), stride)
            })
            .unwrap_or_default();
    }

    /// The transient cursor last set by [`Self::set_cursor_transient`].
    pub fn cursor_transient(&self) -> Option<&cad_scene::TransientCursor> {
        self.cursor_transient.as_ref()
    }

    /// Ensure the transient cursor's fixed-size instance buffer exists and
    /// contains the current packed bytes.
    ///
    /// Runs once per render before the frame borrows the device/queue. The buffer
    /// is created lazily at its fixed maximum size and then **reused** across
    /// moves (stable handle); only `write_buffer` runs while dirty. A device loss
    /// or rebuild drops the buffer but keeps the CPU cursor/packed bytes, so the
    /// next render recreates and re-uploads without a new producer input.
    pub(super) fn sync_cursor_transient_buffer(&mut self) {
        if self.cursor_transient.is_none() {
            self.cursor_transient_dirty = false;
            return;
        }
        let (Some(device), Some(queue)) = (self.device.clone(), self.queue.clone()) else {
            return;
        };
        let stride = self.transient_line_stride();
        // A device generation change keeps the CPU cursor but marks it dirty: the
        // live tier's pipeline stride is only known after attach, so repack here
        // (56 B universal / 64 B accelerated) before uploading.
        if self.cursor_transient_dirty {
            let cursor = self.cursor_transient.clone();
            self.repack_cursor_transient(&cursor);
        }
        let size = u64::from(stride) * cad_scene::MAX_TRANSIENT_CURSOR_SEGMENTS as u64;
        let created = match self.cursor_transient_buffer.as_ref() {
            Some(buffer) if buffer.size() >= size => false,
            _ => {
                self.cursor_transient_buffer =
                    Some(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("cad-cursor-transient"),
                        size,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
                true
            }
        };
        if created || self.cursor_transient_dirty {
            if let Some(buffer) = self.cursor_transient_buffer.as_ref() {
                if !self.cursor_transient_packed.is_empty() {
                    queue.write_buffer(buffer, 0, &self.cursor_transient_packed);
                }
            }
            self.cursor_transient_dirty = false;
        }
    }

    /// Build the overlay draw-order lists from the resident overlay range plus the
    /// optional renderer-owned transient cursor.
    ///
    /// Every batch after `base_end`, in the scene's visual order (opaque then
    /// transparent), with the identity cull (an overlay batch is always drawn;
    /// its geometry is never stream/chunk/conic-converted). `transient` appends
    /// the sentinel with its own `draw_order`, so the cursor sorts between the
    /// tool preview and the snap hints instead of being forced last.
    pub(super) fn plan_overlay(
        &self,
        camera_position: [f32; 3],
        transient: Option<&cad_scene::TransientCursor>,
    ) -> OverlayPlan {
        let mut overlay: Vec<usize> = (self.base_end..self.batches.len()).collect();
        // Every resident overlay batch was staged legacy and admitted by the
        // commit's overlay-run rejection; the base-only `batch_facts` (built over
        // `[0, base_end)`) therefore describes no overlay batch, and the overlay
        // encode pass safely looks a missing class up as `0`.
        debug_assert!(
            self.batches[self.base_end..].iter().all(|batch| {
                batch.instanced.is_empty()
                    && batch.mesh_runs.is_empty()
                    && batch.arc_runs.is_empty()
            }),
            "overlay batches must be run-free (staged legacy)"
        );
        let mut entries: Vec<BatchOrderEntry> = overlay
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
        let transient = transient.filter(|cursor| !cursor.segments.is_empty());
        if let Some(cursor) = transient {
            overlay.push(plan::TRANSIENT_CURSOR);
            entries.push(BatchOrderEntry {
                draw_order: cursor.draw_order,
                // Opaque: the transient is only ever in the opaque pass.
                alpha: 1.0,
                centroid: [0.0; 3],
            });
        }
        let passes = draw_passes(&overlay, &entries, Some(camera_position));
        OverlayPlan {
            opaque: passes.opaque,
            transparent: passes.transparent,
            invisible: passes.invisible,
            transient: transient.is_some(),
            cull: ChunkCull::all_visible(),
        }
    }

    /// Write one per-batch uniform slot for every drawn overlay batch, starting
    /// at `first_slot`; returns the next free slot and, when the transient cursor
    /// is present, its uniform slot.
    ///
    /// The shape is identical to the base legacy loop so the same shaders and
    /// flag tint apply. Overlay batches own no stream/chunk/arc slots. The
    /// transient sentinel is skipped here (it is not a batch) and gets exactly one
    /// extra uniform: its fixed-point origin transform, white tint, pick key `0`
    /// and the AA ramp width in `key_flags.z`. Takes the disjoint
    /// arena/slot/batch state (not `&mut self`) because `render` holds immutable
    /// view/pipeline borrows of other `Renderer` fields.
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
        transient: Option<&cad_scene::TransientCursor>,
    ) -> (usize, Option<u32>) {
        let mut next_slot = first_slot;
        for &index in plan.opaque.iter().chain(plan.transparent.iter()) {
            // The sentinel is not a resident batch; its uniform is written below.
            if index == plan::TRANSIENT_CURSOR {
                continue;
            }
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
        let mut transient_slot = None;
        if let Some(cursor) = transient.filter(|cursor| !cursor.segments.is_empty()) {
            debug_assert!(plan.transient, "a transient slot needs its sentinel");
            let origin = cursor.segments[0][0];
            // White tint: colour and the (zero) pick key travel per instance.
            let uniform = uniform_bytes(
                &projection.batch_matrix_f64([origin.x, origin.y, origin.z]),
                [1.0, 1.0, 1.0, 1.0],
                [0, 0, line_wpp_bits, 0],
                light,
            );
            uniforms.write_slot(next_slot, &uniform);
            transient_slot = Some(next_slot as u32);
            next_slot += 1;
        }
        (next_slot, transient_slot)
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
    /// `transient` is the renderer-owned cursor instance buffer (and its count and
    /// uniform slot); it is drawn in the opaque walk at the sentinel's sorted
    /// position and never in the id/pick pass.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode_overlay_pass(
        &self,
        queue: &wgpu::Queue,
        device: &wgpu::Device,
        overlay_view: &wgpu::TextureView,
        overlay_depth_view: &wgpu::TextureView,
        background: [f32; 4],
        plan: &OverlayPlan,
        transient: Option<(&wgpu::Buffer, u32, u32)>,
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
            transient,
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
            opaque_batches: plan.opaque.len() - usize::from(plan.transient),
            transparent_batches: plan.transparent.len(),
            invisible_batches: plan.invisible,
            transient_draws: outcome.transient_draws,
            encode_ms: outcome.encode_ms,
            set_pipeline_calls: outcome.encode_state.tally.set_pipeline_calls,
            set_bind_group_calls: outcome.encode_state.tally.set_bind_group_calls,
            set_vertex_buffer_calls: outcome.encode_state.tally.set_vertex_buffer_calls,
            set_index_buffer_calls: outcome.encode_state.tally.set_index_buffer_calls,
        }
    }
}
