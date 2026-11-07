//! Navigation-time display LOD policy on the render runtime.
//!
//! The renderer owns the mechanism (hide text-glyph draws); this module owns the
//! invalidation contract: a mode change must redraw the retained frame so the
//! settled image can never keep text-free colour.

use super::*;

impl CadRenderRuntime {
    /// Enable/disable navigation-time text hiding on the attached renderer.
    ///
    /// Returns whether the renderer mode changed. A change invalidates the frame
    /// so the settled text-free colour is redrawn (see
    /// [`Renderer::set_navigation_lod`]); no renderer attached is a no-op.
    pub fn set_navigation_lod(&mut self, hide_text: bool) -> bool {
        let changed = self
            .renderer
            .as_mut()
            .is_some_and(|renderer| renderer.set_navigation_lod(hide_text));
        if changed {
            self.dirty.invalidate();
        }
        changed
    }
}
