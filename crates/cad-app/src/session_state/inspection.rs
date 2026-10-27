//! Pure read/write accessors over retained session measurement + clipboard state.
//!
//! Split out of `session_state.rs` to keep that module inside the code-size limit;
//! the `impl` block applies to the same type, so the public paths
//! (`SessionState::clipboard_len`, `SessionState::last_measurement`, …) are
//! unchanged. Nothing here dispatches a command or touches a database.

use super::*;

impl SessionState {
    /// Number of entities currently held in the in-process clipboard.
    ///
    /// Pure getter; reading it never captures or mutates anything.
    pub fn clipboard_len(&self) -> usize {
        self.clipboard.len()
    }

    /// The last structured measurement result, if one has been confirmed.
    ///
    /// Pure getter: reading it never advances a tool or writes a database.
    pub fn last_measurement(&self) -> Option<&MeasurementRecord> {
        self.last_measurement.as_ref()
    }

    /// Whether a confirmed measurement result is currently retained.
    pub fn has_last_measurement(&self) -> bool {
        self.last_measurement.is_some()
    }

    /// Record the structured result of a completed measurement (F06).
    pub fn set_last_measurement(&mut self, record: MeasurementRecord) {
        self.last_measurement = Some(record);
    }

    /// Snapshot of the active measurement for preview rendering, if any.
    pub fn measurement_preview(&self) -> Option<MeasurementPreview> {
        match &self.tool {
            ToolState::Measuring(tool) => Some(self.annotate_measurement_preview(tool.preview())),
            _ => None,
        }
    }

    /// Move the measurement preview cursor without capturing a point.
    ///
    /// This is a pure state update: it emits no command and touches no database.
    pub fn set_measurement_cursor(&mut self, cursor: Option<Point3>) -> CadResult<()> {
        match &mut self.tool {
            ToolState::Measuring(tool) => {
                tool.set_cursor(cursor);
                Ok(())
            }
            _ => Err(CadError::InvalidInput(
                "no measurement tool is active".into(),
            )),
        }
    }
}
