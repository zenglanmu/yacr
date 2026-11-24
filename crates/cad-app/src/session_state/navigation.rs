//! Continuous pan/orbit runs and their one-step view-history rule.
//!
//! Pointer move events dispatch one `PAN`/`ORBIT` command each. The first
//! command of a run records the pre-change view; the rest merge into that same
//! step. A pointer release (`cancel_tool`), a switch between pan and orbit, or
//! any other command closes the run. This module holds only the session marker
//! and its two operations; [`SessionState`](super::SessionState) owns the field
//! and the command layer decides when to call them.

use super::SessionState;

/// The continuous navigation kind currently recorded as one `ZOOM PREVIOUS`
/// step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigationStepKind {
    Pan,
    Orbit,
}

impl SessionState {
    /// Open (or continue) one continuous pan/orbit run.
    ///
    /// Returns `true` when this command is the **first** of its kind in the run
    /// (the caller records the pre-change view); `false` when an earlier command
    /// already recorded it. Switching pan ↔ orbit starts a new step.
    pub(crate) fn begin_navigation_step(&mut self, kind: NavigationStepKind) -> bool {
        let first = self.navigation_step != Some(kind);
        self.navigation_step = Some(kind);
        first
    }

    /// Close the continuous pan/orbit run (a discrete command or a gesture end).
    pub(crate) fn end_navigation_step(&mut self) {
        self.navigation_step = None;
    }
}
