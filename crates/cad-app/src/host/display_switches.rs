//! Session display switches: idle hover preview, object snap, the display grid and
//! the `ORTHO` drafting constraint.
//!
//! Each of these is session/display state only: toggling it writes no database,
//! produces no history entry and schedules no ChangeSet, so all of them are
//! allowed in Viewer **and** Work. They are independent of one another and of
//! the persisted preferences with the same-sounding names — `view.overlays.*`
//! gates what is **drawn**, not what the session resolves. The display grid is
//! the one case where both gates compose: the host draws it only when the
//! session switch and `view.overlays.grid` are both on.
//!
//! `ORTHO` is the one switch here that changes the **value** a capture stores:
//! it constrains the next point entry (and the matching preview cursor) to a
//! horizontal or vertical direction from the tool's previous point. It still has
//! no database, history or preference side effect; see [`crate::ortho`].

use super::*;

impl HostController {
    /// The session's object-snap switch (session state only).
    pub fn snap_enabled(&self) -> bool {
        self.session.snap_enabled()
    }

    /// Enable or disable object snapping for this session.
    ///
    /// Session state only: no database write, no history entry, and no change
    /// to the persisted `view.overlays.snapHints` preference (ADR 0014 A.4).
    pub fn set_snap_enabled(&mut self, enabled: bool) {
        self.session.set_snap_enabled(enabled);
    }

    /// Flip object snapping and return the new state (F3 binding).
    pub fn toggle_snap(&mut self) -> bool {
        self.session.toggle_snap()
    }

    /// Whether the display grid overlay is enabled for this session (`GRID`,
    /// F7).
    ///
    /// The session switch only; the persisted `view.overlays.grid` preference is
    /// a separate gate, and the grid is drawn only when both are on.
    pub fn grid_enabled(&self) -> bool {
        self.session.grid_enabled()
    }

    /// Enable or disable the display grid for this session.
    ///
    /// Session state only: no database write, no history entry, and no change to
    /// the persisted `view.overlays.grid` preference. Display grid only — grid
    /// snap (AutoCAD's `SNAP`/F9) is a different, unimplemented feature.
    pub fn set_grid_enabled(&mut self, enabled: bool) {
        self.session.set_grid_enabled(enabled);
    }

    /// Flip the display grid and return the new state (F7 binding).
    pub fn toggle_grid(&mut self) -> bool {
        self.session.toggle_grid()
    }

    /// Whether the `ORTHO` drafting constraint is enabled for this session
    /// (`ORTHO`, F8).
    ///
    /// Session state only; the capture and preview paths read the same flag, so a
    /// rubber band and its commit can never disagree.
    pub fn ortho_enabled(&self) -> bool {
        self.session.ortho_enabled()
    }

    /// Enable or disable the `ORTHO` drafting constraint for this session.
    ///
    /// Session state only: no database write, no history entry, and no change to
    /// any persisted preference. See [`crate::ortho`] for the constraint itself.
    pub fn set_ortho_enabled(&mut self, enabled: bool) {
        self.session.set_ortho_enabled(enabled);
    }

    /// Flip the `ORTHO` drafting constraint and return the new state (F8 binding).
    pub fn toggle_ortho(&mut self) -> bool {
        self.session.toggle_ortho()
    }

    /// Whether idle pointer movement previews the entity under the cursor.
    ///
    /// See [`crate::SessionState::hover_preview`] for the default (`true`,
    /// matching AutoCAD's `SELECTIONPREVIEW` initial `3`).
    pub fn hover_preview_enabled(&self) -> bool {
        self.session.hover_preview()
    }

    /// Flip the idle hover entity preview and return the new state.
    ///
    /// Session/display state only: no database write, no history entry and no
    /// ChangeSet. Object-snap markers are unaffected.
    pub fn toggle_hover_preview(&mut self) -> bool {
        self.session.toggle_hover_preview()
    }
}
