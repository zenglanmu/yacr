//! Wave 6 **file / plot / export** handlers for the extended batch.
//!
//! `DWGLOG`, `LOGFILEON` and `LOGFILEOFF` read or write one session token
//! ([`SessionState::drafting_extras`]). The token is session state only: this
//! build writes **no** log file, so the outcome says that plainly instead of
//! implying that logging started.
//!
//! `DWGPROPS` is a read-only document report: it reads the document id, the
//! drawing revision and the entity count that `cad-db` really stores, and names
//! the absent title/author/custom-properties model. `DWFFORMAT` and `EXPORTPDF`
//! are **read-only capability reports**, not errors: they state that DWF is not
//! an export format in this build and that PDF export is host-owned through
//! `PLOT`. No host file I/O is re-implemented in `cad-app`.
//!
//! None of these opens a transaction, bumps the revision, records a
//! `cad-history` entry or publishes a `ChangeSet`; all six are Viewer-legal.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};

impl Application {
    /// Dispatch one Wave 6 file/plot/export token or report.
    pub(crate) fn extended_file6(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::DwgLog => self.dwglog(session, command),
            ExtendedCommandId::LogFileOn => self.logfile(session, command, true),
            ExtendedCommandId::LogFileOff => self.logfile(session, command, false),
            ExtendedCommandId::DwgProps => self.dwgprops(command),
            ExtendedCommandId::DwfFormat => self.dwfformat(command),
            ExtendedCommandId::ExportPdf => self.exportpdf(command),
            other => Err(CadError::Invariant(format!(
                "not a Wave 6 file/plot command: {other:?}"
            ))),
        }
    }

    /// `DWGLOG`: flip the session drawing-log token and report it.
    ///
    /// The token is session state only. This build creates no per-drawing log
    /// file, so nothing is written and no drawing row changes.
    fn dwglog(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "DWGLOG")?;
        let enabled = session.drafting_extras.toggle_dwglog();
        Ok(report_outcome(vec![diagnostic(
            "dwglog.toggled",
            format!(
                "DWGLOG: session drawing-log token is now {} (session state only); this build \
                 writes no log file and the drawing was not written",
                state(enabled)
            ),
        )]))
    }

    /// `LOGFILEON` / `LOGFILEOFF`: set the same session token explicitly.
    fn logfile(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        enabled: bool,
    ) -> CadResult<CommandOutcome> {
        let name = if enabled { "LOGFILEON" } else { "LOGFILEOFF" };
        require_none_payload(command, name)?;
        session.drafting_extras.set_dwglog(enabled);
        let (code, verb) = if enabled {
            ("logfileon.toggled", "ON")
        } else {
            ("logfileoff.toggled", "OFF")
        };
        Ok(report_outcome(vec![diagnostic(
            code,
            format!(
                "{name}: session drawing-log token is now {verb} (session state only); this build \
                 writes no log file"
            ),
        )]))
    }

    /// `DWGPROPS`: read-only document id/revision/entity-count report.
    ///
    /// `cad-db` stores no title, author, subject or custom properties, so the
    /// trailing `dwgprops.metadata_not_tracked` diagnostic names that gap instead
    /// of inventing metadata.
    fn dwgprops(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "DWGPROPS")?;
        let drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![
            diagnostic(
                "dwgprops.report",
                format!(
                    "DWGPROPS: document {} revision {} entities {}",
                    command.document.0,
                    drawing.revision().0,
                    drawing.entity_count()
                ),
            ),
            diagnostic(
                "dwgprops.metadata_not_tracked",
                "DWGPROPS: drawing title, author, subject, keywords and custom properties are \
                 NOT MODELED by this database; nothing was invented"
                    .into(),
            ),
        ]))
    }

    /// `DWFFORMAT`: read-only report — DWF/DWFx is not an export format here.
    fn dwfformat(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "DWFFORMAT")?;
        let _drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "dwfformat.not_supported",
            "DWFFORMAT: this build does not export DWF or DWFx and there is no default-format \
             setting; use SVG/PDF via PLOT; nothing was changed"
                .into(),
        )]))
    }

    /// `EXPORTPDF`: read-only report — PDF export is host-owned through `PLOT`.
    fn exportpdf(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "EXPORTPDF")?;
        let _drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "exportpdf.use_plot",
            "EXPORTPDF: PDF export is host-owned through PLOT (the platform save picker); this \
             command writes nothing itself; nothing was changed"
                .into(),
        )]))
    }
}

fn state(enabled: bool) -> &'static str {
    if enabled {
        "ON"
    } else {
        "OFF"
    }
}
