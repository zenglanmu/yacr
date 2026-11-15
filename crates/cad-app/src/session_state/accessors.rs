//! Small `SessionState` accessors split out of `session_state.rs` to keep the
//! parent file inside the code-size gate.
//!
//! These are pure reads/sets of session-only values: the current style names, the
//! `VPMAX` maximized-viewport marker and the active draw capture's pending-step
//! routing flags (which step accepts an entity pick). Nothing here writes the
//! drawing or records history.

use super::*;

impl SessionState {
    /// The session's current dimension-style name (`Standard` until set).
    pub fn current_dim_style_name(&self) -> &str {
        &self.current_dim_style
    }

    /// The session's current multileader-style name (`Standard` until set).
    pub fn current_mleader_style_name(&self) -> &str {
        &self.current_mleader_style
    }

    /// The session-only maximized layout viewport (`VPMAX`), if any.
    pub fn maximized_viewport(&self) -> Option<LayoutViewportRef> {
        self.maximized_viewport
    }

    /// Set the maximized viewport marker. Session state only.
    pub fn set_maximized_viewport(&mut self, viewport: LayoutViewportRef) {
        self.maximized_viewport = Some(viewport);
    }

    /// Clear the maximized viewport marker (`VPMIN`). Session state only.
    pub fn clear_maximized_viewport(&mut self) {
        self.maximized_viewport = None;
    }
    /// Whether the active drawing tool has a pending entity-pick step.
    ///
    /// True only for TRIM/EXTEND while a step is still un-captured; the host uses
    /// it to route a click to the pick path and to know whether to resolve a
    /// cursor-time candidate.
    pub fn draw_requires_entity_pick(&self) -> bool {
        matches!(
            &self.tool,
            ToolState::Drawing(capture) if capture.tool.requires_entity_pick()
        )
    }

    /// Whether the active drawing tool's pending step **accepts** an entity pick.
    ///
    /// The same authority as [`Self::draw_requires_entity_pick`], except that it is
    /// also true for the mixed step (DIMANGULAR's "select a line **or** specify a
    /// vertex"): the host resolves the entity under the click and, when nothing is
    /// there, falls back to a free point instead of refusing. That fallback is only
    /// legal because `draw_requires_entity_pick` is `false` for a mixed step.
    pub fn draw_accepts_entity_pick(&self) -> bool {
        matches!(
            &self.tool,
            ToolState::Drawing(capture) if capture.tool.accepts_entity_pick()
        )
    }
}
