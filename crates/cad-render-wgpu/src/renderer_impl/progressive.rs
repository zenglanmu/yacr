//! Progressive-frame restart: restart the base-only page walk when the frame
//! key changed or no plan exists. An opaque-only camera/target change reuses the
//! retained order/page index; a transparent or base-set change rebuilds it.

use super::*;

impl Renderer {
    /// The page plan for a paused navigation frame: keep the cursor and draw no
    /// base page (the retained colour is copied and the overlay recomposited).
    ///
    /// `None` when there is no retained page (`cursor == 0`) or the renderer is
    /// not paging, so the caller must draw a real page. A camera/target change
    /// has already restarted the walk to `cursor == 0`, so a paused frame still
    /// draws page 1 under the new camera.
    pub(super) fn retained_page_plan(&self) -> Option<(plan::GpuPlan, usize)> {
        let frame = self.progressive_frame.as_ref()?;
        if frame.cursor == 0 {
            return None;
        }
        Some((
            plan::GpuPlan {
                accepted: Vec::new(),
                usage: cad_scene::FrameUsage::default(),
                over_budget: None,
            },
            frame.invisible,
        ))
    }

    /// Restart the progressive walk when the camera/attachment key changed (or
    /// no plan exists), reusing the previous `ordered`/page-index allocations.
    ///
    /// Returns `1` when this frame restarted the walk, `0` otherwise. The
    /// restart is filtered to the **base** prefix: overlay batches are
    /// recomposited every frame and never enter the page plan, so a suffix
    /// commit cannot invalidate or reset the base walk.
    pub(super) fn restart_progressive_frame(
        &mut self,
        frame_key: FrameKey,
        camera_position: [f32; 3],
    ) -> u64 {
        if !self.progressive
            || self
                .progressive_frame
                .as_ref()
                .is_some_and(|frame| frame.key == frame_key)
        {
            return 0;
        }
        // A camera/attachment key change leaves the base draw order and page index
        // **identical** when there is no transparent batch to re-sort: opaque
        // order and the opaque/invisible partition are camera-independent, and a
        // suffix-only commit keeps `base_end`/the base prefix (a base change drops
        // this frame entirely, see `commit_upload_with_base`). Reuse the retained
        // plan and restart only the cursor, instead of re-collecting `ordered` and
        // rebuilding the O(total batches) page index on every pan/zoom frame.
        let opaque_only = self
            .draw_plan
            .as_ref()
            .is_some_and(|cache| cache.transparent().is_empty());
        let fact_len = self.batch_facts.upload_bytes.len();
        if let Some(frame) = self.progressive_frame.as_mut() {
            if frame.key != frame_key && opaque_only {
                debug_assert_eq!(
                    frame.opaque_count,
                    frame.ordered.len(),
                    "an opaque-only restart reuses an ordered list that is all opaque"
                );
                debug_assert_eq!(
                    frame.page_index.batch_pos.len(),
                    fact_len,
                    "the reused page index must cover the current batch facts"
                );
                debug_assert_eq!(
                    frame.page_index.bytes_prefix.len(),
                    frame.ordered.len() + 1,
                    "the reused byte prefix must match the reused order"
                );
                frame.key = frame_key;
                frame.cursor = 0;
                self.preserve_color_on_restart = false;
                self.progressive_order_reuses += 1;
                return 1;
            }
        }
        // A key mismatch (camera/target change) invalidates the retained colour.
        // Only a flag/tint restart — which leaves geometry and camera untouched —
        // may preserve it; a `None` frame already had the flag cleared by
        // whichever mutation dropped it (commit/clear/toggle).
        if self
            .progressive_frame
            .as_ref()
            .is_some_and(|frame| frame.key != frame_key)
        {
            self.preserve_color_on_restart = false;
        }
        // Reuse the previous frame's `ordered` and page-index allocations: the
        // restart replaces them, so the old Vecs are cleared and refilled instead
        // of freshly collected per camera move.
        let (mut ordered, mut page_index) = self
            .progressive_frame
            .take()
            .map(|frame| (frame.ordered, frame.page_index))
            .unwrap_or_default();
        let cache = self.draw_plan.as_mut().expect("draw plan cache");
        cache.sort_transparent(Some(camera_position));
        let base_end = self.base_end;
        let opaque_count = cache
            .opaque()
            .iter()
            .filter(|&&index| index < base_end)
            .count();
        let invisible = cache
            .invisible()
            .iter()
            .filter(|&&index| index < base_end)
            .count();
        ordered.clear();
        ordered.extend(
            cache
                .opaque()
                .iter()
                .chain(cache.transparent())
                .copied()
                .filter(|&index| index < base_end),
        );
        // Fuse the segmented planner's per-`ordered` index into the same sweep
        // that (re)builds `ordered`, so it is derived from that one truth and
        // rebuilt exactly when the order is.
        page_index.rebuild(&self.batch_facts.upload_bytes, &ordered);
        self.progressive_frame = Some(ProgressiveFrame {
            key: frame_key,
            ordered,
            opaque_count,
            invisible,
            cursor: 0,
            page_index,
        });
        self.progressive_order_rebuilds += 1;
        1
    }
}
