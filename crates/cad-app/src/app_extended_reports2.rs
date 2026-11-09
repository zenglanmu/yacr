//! Wave 2 read-only reports for the extended command batch.
//!
//! `SYSVARMONITOR`, `SYSVDLG`, `VTOPTIONS`, `ALIASEDIT`, `WHOHAS`, `ACADINFO`,
//! `PTYPE` and `SCALELISTEDIT` open no transaction, bump no revision, record no
//! `cad-history` entry and publish no `ChangeSet`, so all eight are Viewer-legal.
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
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{
    document_drawing, modelled_system_variable, require_none_payload, MODELLED_SYSTEM_VARIABLES,
};

impl Application {
    /// Dispatch one Wave 2 read-only report.
    pub(crate) fn extended_report2(
        &self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::SysVarMonitor => {
                self.system_variable_report(session, command, SystemVariableReport::Monitor)
            }
            ExtendedCommandId::SysVarDialog => {
                self.system_variable_report(session, command, SystemVariableReport::Dialog)
            }
            ExtendedCommandId::ViewTransitionOptions => self.view_transition_report(command),
            ExtendedCommandId::AliasEdit => self.alias_report(command),
            ExtendedCommandId::WhoHas => self.who_has_report(command),
            ExtendedCommandId::AcadInfo => self.acad_info_report(command),
            ExtendedCommandId::PointType => self.point_type_report(command),
            ExtendedCommandId::ScaleListEdit => self.scale_list_report(command),
            other => Err(CadError::Invariant(format!(
                "not a Wave 2 report: {other:?}"
            ))),
        }
    }

    /// `SYSVARMONITOR` / `SYSVDLG`: one row per modelled system variable.
    ///
    /// Both read the same closed set; `SYSVDLG` also states that this build has
    /// no settings dialog. Nothing is watched or written.
    fn system_variable_report(
        &self,
        session: &SessionState,
        command: &Command,
        report: SystemVariableReport,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, report.selector())?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::with_capacity(MODELLED_SYSTEM_VARIABLES.len() + 2);
        for name in MODELLED_SYSTEM_VARIABLES {
            let (value, kind) = modelled_system_variable(&drawing, session, name)
                .expect("MODELLED_SYSTEM_VARIABLES is exactly the reader's closed set");
            diagnostics.push(diagnostic(
                report.value_code(),
                format!("{} {name} = {value} ({kind})", report.selector()),
            ));
        }
        diagnostics.push(diagnostic(
            report.summary_code(),
            format!(
                "{}: {} modelled system variable(s) reported; variables outside the SETVAR \
                 closed set are not modeled and were not invented",
                report.selector(),
                MODELLED_SYSTEM_VARIABLES.len()
            ),
        ));
        if report == SystemVariableReport::Dialog {
            diagnostics.push(diagnostic(
                "sysvdlg.dialog_not_implemented",
                "SYSVDLG: this build has no system-variable dialog, so there is no edit/save/\
                 restore surface; use `SETVAR <name>` to read one modelled value"
                    .into(),
            ));
        }
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
    /// router normalizes and the docs gate checks. Editing is NOT IMPLEMENTED.
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
                 or deleting aliases is NOT IMPLEMENTED (there is no acad.pgp file and no \
                 user-defined alias storage)",
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
    /// The write path is the existing `PDMODE`/`PDSIZE` commands; PTYPE itself
    /// only reports.
    fn point_type_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "PTYPE")?;
        let drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "ptype.report",
            format!(
                "PTYPE: $PDMODE = {} (integer), $PDSIZE = {} (real); this command is read-only — \
                 set them with PDMODE/PDSIZE (the typed PTYPE dialog form is NOT IMPLEMENTED)",
                drawing.point_display_mode(),
                drawing.point_display_size()
            ),
        )]))
    }

    /// `SCALELISTEDIT`: report the current paper size; the write is unsupported.
    ///
    /// The drawing stores a page per layout, not a scale list, so only the first
    /// paper layout's page is read. Nothing is written.
    fn scale_list_report(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "SCALELISTEDIT")?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::with_capacity(2);
        match drawing.layouts().next() {
            Some(layout) => {
                let page = drawing.plot_settings_for(layout.id);
                diagnostics.push(diagnostic(
                    "scalelistedit.summary",
                    format!(
                        "SCALELISTEDIT: layout '{}' page {} ({} x {}), scale {}:{}; read-only",
                        layout.name,
                        page.paper_size_name,
                        page.paper_width,
                        page.paper_height,
                        page.scale_numerator,
                        page.scale_denominator
                    ),
                ));
            }
            None => diagnostics.push(diagnostic(
                "scalelistedit.summary",
                "SCALELISTEDIT: this drawing has no paper layout, so there is no page to report; \
                 the model-space scale list is not modeled"
                    .into(),
            )),
        }
        diagnostics.push(diagnostic(
            "scalelistedit.write_unsupported",
            "SCALELISTEDIT: editing the scale list is NOT IMPLEMENTED — this build has no scale-list \
             storage, so no scale was added, removed or reset"
                .into(),
        ));
        Ok(report_outcome(diagnostics))
    }
}

/// Which system-variable report is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemVariableReport {
    /// `SYSVARMONITOR`: the closed set only.
    Monitor,
    /// `SYSVDLG`: the closed set plus `sysvdlg.dialog_not_implemented`.
    Dialog,
}

impl SystemVariableReport {
    fn selector(self) -> &'static str {
        match self {
            Self::Monitor => "SYSVARMONITOR",
            Self::Dialog => "SYSVDLG",
        }
    }

    fn value_code(self) -> &'static str {
        match self {
            Self::Monitor => "sysvarmonitor.value",
            Self::Dialog => "sysvdlg.value",
        }
    }

    fn summary_code(self) -> &'static str {
        match self {
            Self::Monitor => "sysvarmonitor.summary",
            Self::Dialog => "sysvdlg.summary",
        }
    }
}
