//! Small `SessionState` accessors split out of `session_state.rs` to keep the
//! parent file inside the code-size gate.
//!
//! These are pure reads/sets of session-only values: the current style names and
//! the `VPMAX` maximized-viewport marker. Nothing here writes the drawing or
//! records history.

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
}
