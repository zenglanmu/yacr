//! Session drafting switches (object snap, display grid, grid snap, SNAPUNIT,
//! ORTHO) as their own `impl SessionState` block.
//!
//! Split out of `session_state.rs` so the struct and its command-facing methods
//! stay inside the file-size gate; nothing here is a second state authority.

use super::*;

impl SessionState {
    pub fn mode(&self) -> AppMode {
        self.mode
    }

    /// Whether object snapping is active in this session.
    pub fn snap_enabled(&self) -> bool {
        self.snap_enabled
    }

    /// Enable or disable object snapping for this session.
    ///
    /// Session state only: no database write, no history entry. When disabled,
    /// [`crate::host::HostController::resolve_snap`] returns `Ok(None)` (the raw
    /// point is used) and the snap-hint overlay is empty.
    pub fn set_snap_enabled(&mut self, enabled: bool) {
        self.snap_enabled = enabled;
    }

    /// Flip object snapping and return the new state (F3 binding).
    pub fn toggle_snap(&mut self) -> bool {
        self.snap_enabled = !self.snap_enabled;
        self.snap_enabled
    }

    /// Whether the display grid overlay is enabled in this session.
    ///
    /// Session/display state only, and **off** when a session is created (the
    /// grid is opt-in). `true` here does not by itself mean the grid is drawn:
    /// the host also requires the persisted `view.overlays.grid` preference, so
    /// the two gates compose with AND.
    pub fn grid_enabled(&self) -> bool {
        self.grid_enabled
    }

    /// Enable or disable the display grid for this session.
    ///
    /// Session state only: no database write, no history entry. This is the
    /// **display** grid only; grid snap has its own switch
    /// ([`SessionState::set_snap_grid_enabled`]).
    pub fn set_grid_enabled(&mut self, enabled: bool) {
        self.grid_enabled = enabled;
    }

    /// Flip the display grid and return the new state (F7 binding).
    pub fn toggle_grid(&mut self) -> bool {
        self.grid_enabled = !self.grid_enabled;
        self.grid_enabled
    }

    /// Whether **grid snap** is active in this session (`SNAP`, F9).
    ///
    /// Session/display state only; the default is `false` (AutoCAD's initial
    /// `SNAP` off).
    pub fn snap_grid_enabled(&self) -> bool {
        self.snap_grid_enabled
    }

    /// Enable or disable grid snap for this session.
    ///
    /// Session state only: no database write, no history entry, no ChangeSet and
    /// no persisted preference. See [`crate::snap`] for the quantisation rule the
    /// capture and preview paths apply while this is on.
    pub fn set_snap_grid_enabled(&mut self, enabled: bool) {
        self.snap_grid_enabled = enabled;
    }

    /// Flip grid snap and return the new state (F9 binding).
    pub fn toggle_snap_grid(&mut self) -> bool {
        self.snap_grid_enabled = !self.snap_grid_enabled;
        self.snap_grid_enabled
    }

    /// The session grid-snap spacing in drawing units (`SNAPUNIT`).
    ///
    /// Session sysvar state only; see [`SessionState::set_snap_spacing`] for the
    /// accepted values and the default.
    pub fn snap_spacing(&self) -> f64 {
        self.snap_spacing
    }

    /// Set the session grid-snap spacing (`SNAPUNIT <value>`).
    ///
    /// Only a finite, strictly positive spacing is stored
    /// ([`crate::snap::snap_spacing_is_valid`]); `0`, a negative value, `NaN` and
    /// the infinities are explicit `InvalidInput` refusals, never a silent clamp
    /// and never a division by zero. Session/display state only: no database
    /// write, no history entry, no ChangeSet and no persisted preference, so it is
    /// allowed in Viewer and Work.
    pub fn set_snap_spacing(&mut self, spacing: f64) -> CadResult<()> {
        if !crate::snap::snap_spacing_is_valid(spacing) {
            return Err(CadError::InvalidInput(
                "snap spacing must be finite and positive".into(),
            ));
        }
        self.snap_spacing = spacing;
        Ok(())
    }

    /// Whether the `ORTHO` drafting constraint is active in this session.
    ///
    /// Session/display state only; the default is `false` (AutoCAD's initial
    /// `ORTHO` off).
    pub fn ortho_enabled(&self) -> bool {
        self.ortho_enabled
    }

    /// Enable or disable the `ORTHO` drafting constraint for this session.
    ///
    /// Session state only: no database write, no history entry, no ChangeSet and
    /// no persisted preference. See [`crate::ortho`] for the rule the capture and
    /// preview paths apply while this is on.
    pub fn set_ortho_enabled(&mut self, enabled: bool) {
        self.ortho_enabled = enabled;
    }

    /// Flip the `ORTHO` drafting constraint and return the new state (F8 binding).
    pub fn toggle_ortho(&mut self) -> bool {
        self.ortho_enabled = !self.ortho_enabled;
        self.ortho_enabled
    }
}
