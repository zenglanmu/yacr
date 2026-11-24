//! Wave 2 read-only reports for the extended command batch.
//!
//! `VTOPTIONS`, `ALIASEDIT`, `WHOHAS`, `ACADINFO`, `PTYPE` and `SCALELISTEDIT`
//! open no transaction, bump no revision, record no `cad-history` entry and
//! publish no `ChangeSet`, so their bare forms are Viewer-legal. `SYSVDLG`'s
//! bare report is Viewer-legal too, but its typed edit/restore forms write
//! through `app_extended_sysvar_dialog`; `SYSVARMONITOR` grew a real session
//! watch set and lives in `app_extended_sysvar`.
//!
//! Each report reads only data this build really has:
//!
//! * the `SETVAR` closed set (`PDMODE`, `PDSIZE`, `SNAPUNIT`, `LTSCALE`, `GRID`,
//!   `SNAP`, `ORTHO`, `TEXTSIZE`) through the same reader `SETVAR` uses
//!   (`SYSVDLG`);
//! * the canonical `COMMAND_ALIASES` table (`ALIASEDIT`);
//! * product/version/build constants and the document revision/entity counts
//!   (`ACADINFO`);
//! * the drawing's point display style (`PTYPE`) and the default page of the
//!   first paper layout (`SCALELISTEDIT`).
//!
//! Everything else is a named `not_tracked`/`write_unsupported`/
//! `dialog_not_implemented`/`single_document` diagnostic. No value is invented.

use super::*;
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
    /// router normalizes and the docs gate checks. Runtime rows are the live
    /// session's validated set (`CadSession`'s `ALIASEDIT ADD|REMOVE|LIST`
    /// command-line forms, persisted through the host's root `commandAliases`
    /// preference); this payload path has no session view, so it stays a read of
    /// the build table and says so.
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
                "ALIASEDIT: {} alias row(s) from the build's canonical table; runtime rows added \
                 with the command-line `ALIASEDIT ADD|REMOVE` are persisted through the host's \
                 root commandAliases preference and are not part of this build table (there is no \
                 acad.pgp file)",
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
    /// collected rather than fabricated. The information **file** is written by
    /// the typed `ACADINFO <file.json>` form in `cad-ui-core` through the host
    /// named-file bridge; the bare command reports the same facts and how to
    /// write them.
    fn acad_info_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "ACADINFO")?;
        let snapshot = self
            .acad_info_snapshot(command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        Ok(report_outcome(vec![
            diagnostic("acadinfo.report", snapshot.report_line()),
            diagnostic(
                "acadinfo.not_collected",
                "ACADINFO: licence, hardware and installed-module data are not collected by this \
                 build and are not reported (they stay global-scope omissions, also listed in \
                 the information file's notCollected array)"
                    .into(),
            ),
            diagnostic(
                "acadinfo.file_available",
                "ACADINFO: `ACADINFO <file.json>` writes the same facts as a versioned \
                 (yacr.acadinfo v1) JSON information file through the host named-file bridge; \
                 the file carries no licence, hardware, module or timestamp rows, and a host \
                 without a named-file sink refuses the write by name instead of claiming one"
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
