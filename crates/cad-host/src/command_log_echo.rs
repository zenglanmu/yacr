//! Host-facing accessors for the typed command echo (`commandLog`).
//!
//! Split out of the crate root's `impl HostCore` to keep that file inside the
//! code-size limit; the methods and their contracts are unchanged.

use cad_domain::CadError;

use crate::command_log::{error_code, CommandLogEntry};
use crate::HostCore;

impl HostCore {
    /// Record an admitted selector in the typed command echo.
    ///
    /// The UI layer owns the selector vocabulary (and `DIST`/`AREA`/zoom steps
    /// carry no `CommandId`), so it records the stable name at admission. This
    /// is observational only: it never admits, refuses or reorders anything.
    pub fn record_command(&self, name: &str) {
        self.command_log.borrow_mut().push_command(name);
    }

    /// The retained command echo, oldest first (at most
    /// [`COMMAND_LOG_CAPACITY`] entries).
    pub fn command_log_entries(&self) -> Vec<CommandLogEntry> {
        self.command_log.borrow().entries().cloned().collect()
    }

    pub(crate) fn record_command_result(&self, record: &cad_db::MeasurementRecord) {
        self.command_log.borrow_mut().push_result(record);
    }

    /// Echo a real `ID` point report as a typed `Result` entry.
    ///
    /// Observational only, exactly like [`Self::record_command_result`]: the
    /// coordinate, its space, its snap provenance and the precision token travel;
    /// the localized `Diagnostic::message` never does.
    pub(crate) fn record_point_result(&self, report: &cad_app::PointReport) {
        self.command_log.borrow_mut().push_point_result(report);
    }

    pub(crate) fn record_command_error(&self, error: &CadError) {
        self.command_log.borrow_mut().push_error(error_code(error));
    }
}
