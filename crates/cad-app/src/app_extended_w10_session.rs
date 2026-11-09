//! Wave 10 **session-token reports** for the extended batch.
//!
//! `-UNITS`, `LWEIGHT`/`-LWEIGHT` and `COLOR`/`-COLOR` are read-only reports of
//! session tokens. The drawing header stores no unit-format table, the renderer
//! does not consume the lineweight-display flag, and no creation path consumes
//! the current-colour token, so every report states exactly that instead of
//! claiming an effect. None of these opens a transaction, bumps the revision or
//! records history; all are Viewer-legal.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::require_none_payload;

impl Application {
    /// Dispatch one Wave 10 session-token report.
    pub(crate) fn extended_w10_session(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::DashUnits => {
                require_none_payload(command, "-UNITS")?;
                Ok(report_outcome(vec![
                    diagnostic(
                        "units.report",
                        "-UNITS: coordinates and distances are displayed in drawing units with 4 \
                         decimal places; angles are displayed in degrees; no per-drawing unit \
                         table exists"
                            .into(),
                    ),
                    diagnostic(
                        "units.dialog_not_implemented",
                        "-UNITS: the Units dialog and its precision/insertion-scale/direction \
                         controls are NOT IMPLEMENTED"
                            .into(),
                    ),
                ]))
            }
            ExtendedCommandId::LineWeight | ExtendedCommandId::DashLineWeight => {
                let name = ext.selector();
                require_none_payload(command, name)?;
                let state = if session.drafting_extras.lweight_display() {
                    "ON"
                } else {
                    "OFF"
                };
                Ok(report_outcome(vec![diagnostic(
                    "lweight.report",
                    format!(
                        "{name}: session lineweight display is {state}; the renderer does not \
                         consume this flag in this build, so no lineweight width or display \
                         change is claimed, and the LWEIGHT dialog is NOT IMPLEMENTED"
                    ),
                )]))
            }
            ExtendedCommandId::Color | ExtendedCommandId::DashColor => {
                let name = ext.selector();
                require_none_payload(command, name)?;
                let aci = session.drafting_extras.current_color();
                let label = if aci == 256 {
                    "ByLayer".to_string()
                } else {
                    format!("ACI {aci}")
                };
                Ok(report_outcome(vec![diagnostic(
                    "color.report",
                    format!(
                        "{name}: session current colour is {label} (token {aci}); no creation \
                         path consumes this token yet and the Select Color dialog is NOT \
                         IMPLEMENTED, so nothing is changed"
                    ),
                )]))
            }
            other => Err(CadError::Invariant(format!(
                "not a Wave 10 session report: {}",
                other.selector()
            ))),
        }
    }
}
