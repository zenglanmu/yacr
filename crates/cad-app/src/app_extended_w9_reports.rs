//! Wave 9 **reports and named refusals** for the extended batch.
//!
//! `UNITS` is a read-only report of the session display-format token: this
//! build has no UNITS dialog and stores no per-drawing precision/format table,
//! so the report states the fixed display rule instead of inventing a value.
//!
//! The other six names describe features with no model or API here and are
//! refused with a stable `CadError::Unsupported` reason: dimension-style
//! export/import files (`DIMEX`, `DIMIM`), the model/paper distance converter
//! (`SPACETRANS`), text-to-polyline explosion (`TXTEXP`) and field updates
//! (`UPDATEFIELD`). A refusal writes nothing, bumps no revision and records no
//! `cad-history` entry.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::require_none_payload;

impl Application {
    /// Dispatch one Wave 9 report/refusal.
    pub(crate) fn extended_w9_reports(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        if matches!(ext, ExtendedCommandId::Units) {
            require_none_payload(command, "UNITS")?;
            return Ok(report_outcome(vec![
                diagnostic(
                    "units.report",
                    "UNITS: coordinates and distances are displayed in drawing units with 4 \
                     decimal places; angles are displayed in degrees; the drawing header stores \
                     no linear/angular unit-format table, so no per-drawing override exists"
                        .into(),
                ),
                diagnostic(
                    "units.dialog_not_implemented",
                    "UNITS: the AutoCAD Units dialog (precision, insertion scale, direction \
                     control) is NOT IMPLEMENTED; this report changes nothing"
                        .into(),
                ),
            ]));
        }
        require_none_payload(command, ext.selector())?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::DimExport => {
            "dimex.unsupported: dimension-style export files are not modeled"
        }
        ExtendedCommandId::DimImport => {
            "dimim.unsupported: dimension-style import files are not modeled"
        }
        ExtendedCommandId::SpaceTrans => {
            "spacetrans.unsupported: no model/paper distance-conversion API"
        }
        ExtendedCommandId::TextExplode => {
            "txtexp.unsupported: no font-outline-to-geometry path is implemented"
        }
        ExtendedCommandId::UpdateField => "updatefield.unsupported: fields are not modeled",
        other => {
            return format!(
                "{}: this command is not a Wave 9 report/refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
