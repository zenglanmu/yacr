//! Wave 2 read-only reports for the extended command batch.
//!
//! `SYSVDLG`, `VTOPTIONS`, `ALIASEDIT`, `WHOHAS`, `ACADINFO`, `PTYPE` and
//! `SCALELISTEDIT` open no transaction, bump no revision, record no
//! `cad-history` entry and publish no `ChangeSet`, so all seven are Viewer-legal.
//! (`SYSVARMONITOR` grew a real session watch set and lives in
//! `app_extended_sysvar`.)
//!
//! Each report reads only data this build really has:
//!
//! * the `SETVAR` closed set (`PDMODE`, `PDSIZE`, `SNAPUNIT`, `LTSCALE`, `GRID`,
//!   `SNAP`, `ORTHO`, `TEXTSIZE`) through the same reader `SETVAR` uses;
//! * the canonical `COMMAND_ALIASES` table (`ALIASEDIT`);
//! * product/version/build constants and the document revision/entity counts
//!   (`ACADINFO`);
//! * the drawing's point display style (`PTYPE`) and the default page of the
//!   first paper layout (`SCALELISTEDIT`).
//!
//! Everything else is a named `not_tracked`/`write_unsupported`/
//! `dialog_not_implemented`/`single_document` diagnostic. No value is invented.

use super::*;
use crate::app_extended_sysvar::{
    modelled_system_variable, ModelledValue, MODELLED_SYSTEM_VARIABLES,
};
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};

impl Application {
    /// Dispatch one Wave 2 read-only report.
    pub(crate) fn extended_report2(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::SysVarDialog => self.system_variable_dialog(session, command),
            ExtendedCommandId::ViewTransitionOptions => self.view_transition_report(command),
            ExtendedCommandId::AliasEdit => self.alias_report(command),
            ExtendedCommandId::WhoHas => self.who_has_report(command),
            ExtendedCommandId::AcadInfo => self.acad_info_report(command),
            ExtendedCommandId::PointType => self.point_type_report(command),
            // `SCALELISTEDIT` shares one handler with `-SCALELISTEDIT`: the bare
            // form is the report, a `ScaleList` payload is the write
            // (`app_extended_scale_list`).
            ExtendedCommandId::ScaleListEdit => self.scale_list(session, command, "SCALELISTEDIT"),
            other => Err(CadError::Invariant(format!(
                "not a Wave 2 report: {other:?}"
            ))),
        }
    }

    /// `SYSVDLG`: one row per modelled system variable plus the honest gap.
    ///
    /// The report reads the same closed set `SETVAR` does. Anything outside it is
    /// named `sysvdlg.not_tracked`; this build has no settings dialog, so nothing
    /// can be edited, saved or restored.
    fn system_variable_dialog(
        &self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "SYSVDLG")?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::with_capacity(MODELLED_SYSTEM_VARIABLES.len() + 3);
        for name in MODELLED_SYSTEM_VARIABLES {
            // The list is exactly the reader's closed set, so a miss is a build
            // bug; a modelled name that simply has no value right now is
            // reported as such, never filled with a default.
            let value = modelled_system_variable(&drawing, session, name)
                .expect("MODELLED_SYSTEM_VARIABLES is exactly the reader's closed set");
            match value {
                ModelledValue::Value(value, kind) => diagnostics.push(diagnostic(
                    "sysvdlg.value",
                    format!("SYSVDLG {name} = {value} ({kind})"),
                )),
                ModelledValue::Absent(reason) => {
                    diagnostics.push(diagnostic("sysvdlg.not_stored", reason))
                }
            }
        }
        diagnostics.push(diagnostic(
            "sysvdlg.summary",
            format!(
                "SYSVDLG: {} modelled system variable(s) reported with live values; variables \
                 outside the SETVAR closed set are not modeled and were not invented",
                MODELLED_SYSTEM_VARIABLES.len()
            ),
        ));
        diagnostics.push(diagnostic(
            "sysvdlg.not_tracked",
            "SYSVDLG: every other AutoCAD system variable is not tracked by this build — there is \
             no per-variable table to enumerate, and the write-only shell settings \
             (NAVVCUBEDISPLAY, DISPLAYVIEWCUBEIN2D, DISPLAYVIEWCUBEIN3D) have no readable value"
                .into(),
        ));
        diagnostics.push(diagnostic(
            "sysvdlg.dialog_not_implemented",
            "SYSVDLG: this build has no system-variable dialog, so there is no edit/save/restore \
             surface; use `SETVAR <name>` to read one modelled value"
                .into(),
        ));
        Ok(report_outcome(diagnostics))
    }

    /// `VTOPTIONS`: view-transition settings are not modeled.
    fn view_transition_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "VTOPTIONS")?;
        let _drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "vtoptions.not_tracked",
            "VTOPTIONS: view-transition settings are not modeled by this build; no smoothing, \
             duration or zoom/pan transition option exists to report, and none was invented"
                .into(),
        )]))
    }

    /// `ALIASEDIT`: read-only list of the canonical alias table.
    ///
    /// The table is `crate::command_aliases::COMMAND_ALIASES` — the same rows the
    /// router normalizes and the docs gate checks. Runtime alias editing is
    /// **session state** (`CadSession`'s `ALIASEDIT ADD|REMOVE|LIST` command-line
    /// forms); this payload path has no session view, so it stays a read of the
    /// build table and says so.
    fn alias_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "ALIASEDIT")?;
        let _drawing = document_drawing(self, command)?;
        let aliases = crate::command_aliases::COMMAND_ALIASES;
        let mut diagnostics = Vec::with_capacity(aliases.len() + 1);
        for (alias, target) in aliases {
            diagnostics.push(diagnostic(
                "aliasedit.alias",
                format!("ALIASEDIT: {alias} -> {target}"),
            ));
        }
        diagnostics.push(diagnostic(
            "aliasedit.summary",
            format!(
                "ALIASEDIT: {} alias row(s) from the build's canonical table; creating, modifying \
                 or deleting aliases is NOT IMPLEMENTED on this payload path — the live session's \
                 command line provides the session-only `ALIASEDIT ADD|REMOVE|LIST` forms and \
                 there is no acad.pgp file",
                aliases.len()
            ),
        ));
        Ok(report_outcome(diagnostics))
    }

    /// `WHOHAS`: this is a single-document build with no network lookup.
    fn who_has_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "WHOHAS")?;
        let _drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "whohas.single_document",
            "WHOHAS: this is a single-document build; it cannot see other users, other open \
             drawings or file locks, so no network lookup was attempted and no owner information \
             was invented"
                .into(),
        )]))
    }

    /// `ACADINFO`: product/version/build plus document revision and entity counts.
    ///
    /// Only data the crate actually carries is reported. Licence, hardware and
    /// installed-module lists do not exist here, so they are named as not
    /// collected rather than fabricated.
    fn acad_info_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "ACADINFO")?;
        let drawing = document_drawing(self, command)?;
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        Ok(report_outcome(vec![
            diagnostic(
                "acadinfo.report",
                format!(
                    "ACADINFO: product yacr (cad-app) version {} build {profile}; document {} \
                     revision {} entities {}",
                    env!("CARGO_PKG_VERSION"),
                    command.document.0,
                    drawing.revision().0,
                    drawing.entity_count()
                ),
            ),
            diagnostic(
                "acadinfo.not_collected",
                "ACADINFO: licence, hardware and installed-module data are not collected by this \
                 build and are not reported"
                    .into(),
            ),
        ]))
    }

    /// `PTYPE`: read-only `$PDMODE`/`$PDSIZE` report.
    ///
    /// The write path is the existing `PDMODE`/`PDSIZE` commands; a typed
    /// `PTYPE <code> [size]` line is routed to exactly those payloads by
    /// `cad-ui-core`, so PTYPE itself only reports.
    fn point_type_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "PTYPE")?;
        let drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "ptype.report",
            format!(
                "PTYPE: $PDMODE = {} (integer), $PDSIZE = {} (real); this bare form is read-only — \
                 set them with PDMODE/PDSIZE or `PTYPE <code> [size]`",
                drawing.point_display_mode(),
                drawing.point_display_size()
            ),
        )]))
    }
}
