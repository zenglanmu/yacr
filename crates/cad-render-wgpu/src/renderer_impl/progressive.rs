//! Progressive-frame restart: rebuild the base-only page walk when the frame
//! key changed or no plan exists.

use super::*;

impl Renderer {
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
        1
    }
}
