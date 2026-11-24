//! Wave 5 **attribute and external-reference** handlers for the extended batch.
//!
//! `ATTDISP` reads/sets the session attribute-display mode
//! ([`SessionState::drafting_extras`]). It is a **display setting** (ADR-0055
//! D6): the representation lane reads it when it expands a block reference's
//! values, and the command never writes the drawing, bumps a revision or records
//! an undo step. The bare line toggles `Normal`/`Off`; the typed line sets
//! `NORMAL`, `ON` or `OFF`. `BATTMAN`/`BATTORDER` moved to
//! [`crate::app_drawing::attribute_commands`] when the block-attribute model
//! landed (ADR-0055 M3).
//!
//! `XREF`/`-XREF`/`EXTERNALREFERENCES`/`CLASSICXREF`
//! report the import-time external-reference provenance through
//! `drawing.external_references()` — the database **does** expose that
//! enumeration, so no `xref.not_modeled` fallback is reachable; a drawing with no
//! xrefs reports zero rows rather than inventing any. `CLASSICXREF` additionally
//! names attach/detach/reload as unsupported; its panel is a chrome surface.
//! `EXTERNALREFERENCESCLOSE` is a named no-op because this build has no palette
//! to close.
//!
//! None of these opens a transaction, bumps the revision, records a
//! `cad-history` entry or publishes a `ChangeSet`; all eight are Viewer-legal.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};
use crate::command_payload::AttributeCommand;

impl Application {
    /// Dispatch one Wave 5 attribute/xref token or report.
    pub(crate) fn extended_attrs5(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::AttDisp => self.attdisp(session, command),
            ExtendedCommandId::XRef => self.xref_report(command, XrefReport::XRef),
            ExtendedCommandId::DashXRef => self.xref_report(command, XrefReport::DashXRef),
            ExtendedCommandId::ExternalReferences => {
                self.xref_report(command, XrefReport::ExternalReferences)
            }
            ExtendedCommandId::ExternalReferencesClose => self.externalreferences_close(command),
            ExtendedCommandId::ClassicXRef => self.xref_report(command, XrefReport::ClassicXRef),
            other => Err(CadError::Invariant(format!(
                "not a Wave 5 attribute/xref command: {other:?}"
            ))),
        }
    }

    /// `ATTDISP`: set or toggle the session attribute-display mode.
    ///
    /// Session state only. The model exists, so the message no longer claims the
    /// feature is missing; it states that the command is a display setting with
    /// no drawing write, revision or undo step. A typed `AttributeDisplay`
    /// payload sets one of the three modes; the payload-free form toggles
    /// `Normal`/`Off` (there is no in-command prompt).
    fn attdisp(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let mode = match &command.payload {
            CommandPayload::None => session.drafting_extras.toggle_attdisp(),
            CommandPayload::Attribute(AttributeCommand::Display(mode)) => {
                session.drafting_extras.set_attribute_display(*mode)
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "ATTDISP takes no payload, or one AttributeDisplay { mode }".into(),
                ))
            }
        };
        Ok(report_outcome(vec![diagnostic(
            "attdisp.mode",
            format!(
                "ATTDISP: block-attribute display is now {} (session display setting; the model \
                 exists, the representation lane reads this mode, and no drawing row, revision or \
                 history entry was written)",
                mode.as_str()
            ),
        )]))
    }

    /// `XREF` / `-XREF` / `EXTERNALREFERENCES`: read-only xref provenance report.
    ///
    /// `cad-db` stores import-time external-reference metadata and exposes it via
    /// `drawing.external_references()`. This report lists exactly those rows; it
    /// never claims attachment, unload, reload, bind or edit capability, and an
    /// empty map is reported as zero rows.
    fn xref_report(&self, command: &Command, report: XrefReport) -> CadResult<CommandOutcome> {
        require_none_payload(command, report.selector())?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::new();
        let mut count = 0usize;
        for reference in drawing.external_references() {
            count += 1;
            let block = match reference.block {
                Some(block) => format!("block#{}", block.0),
                None => "no-block".to_string(),
            };
            let resolved = reference.resolved_path.as_deref().unwrap_or("<unresolved>");
            let reason = reference
                .reason
                .as_deref()
                .map(|reason| format!(" reason '{reason}'"))
                .unwrap_or_default();
            diagnostics.push(diagnostic(
                report.entry_code(),
                format!(
                    "{}: xref '{}' status {} raw_path '{}' resolved '{}' depth {} overlay {} \
                     {}{}",
                    report.selector(),
                    reference.name,
                    reference.status.as_str(),
                    reference.raw_path,
                    resolved,
                    reference.depth,
                    reference.overlay,
                    block,
                    reason
                ),
            ));
        }
        diagnostics.push(diagnostic(
            report.summary_code(),
            format!(
                "{}: {count} external reference(s) from the drawing's import provenance; \
                 attaching, unloading, reloading, binding or editing xrefs is NOT IMPLEMENTED in \
                 this build",
                report.selector()
            ),
        ));
        if matches!(report, XrefReport::ExternalReferences) {
            diagnostics.push(diagnostic(
                "externalreferences.palette_not_implemented",
                "EXTERNALREFERENCES: this build has no External References palette; the report \
                 above is the whole surface and nothing was opened"
                    .into(),
            ));
        }
        if matches!(report, XrefReport::ClassicXRef) {
            for (code, action) in [
                ("classicxref.attach_unsupported", "attaching"),
                ("classicxref.detach_unsupported", "detaching"),
                ("classicxref.reload_unsupported", "reloading"),
            ] {
                diagnostics.push(diagnostic(
                    code,
                    format!(
                        "CLASSICXREF: {action} an external reference is NOT IMPLEMENTED in this \
                         build; the classic panel lists the existing rows and changes nothing"
                    ),
                ));
            }
        }
        Ok(report_outcome(diagnostics))
    }

    /// `EXTERNALREFERENCESCLOSE`: named no-op — there is no palette to close.
    fn externalreferences_close(&self, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "EXTERNALREFERENCESCLOSE")?;
        let _drawing = document_drawing(self, command)?;
        Ok(report_outcome(vec![diagnostic(
            "externalreferences.close_not_implemented",
            "EXTERNALREFERENCESCLOSE: this build has no External References palette, so there is \
             nothing to close; nothing changed"
                .into(),
        )]))
    }
}

/// Which external-reference report is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XrefReport {
    /// `XREF`: `xref.entry` / `xref.summary`.
    XRef,
    /// `-XREF`: `-xref.entry` / `-xref.summary`.
    DashXRef,
    /// `EXTERNALREFERENCES`: `externalreferences.entry` / `.summary`.
    ExternalReferences,
    /// `CLASSICXREF`: `classicxref.entry` / `.summary` plus explicit
    /// attach/detach/reload refusals; the UI panel is a chrome surface.
    ClassicXRef,
}

impl XrefReport {
    fn selector(self) -> &'static str {
        match self {
            Self::XRef => "XREF",
            Self::DashXRef => "-XREF",
            Self::ExternalReferences => "EXTERNALREFERENCES",
            Self::ClassicXRef => "CLASSICXREF",
        }
    }

    fn entry_code(self) -> &'static str {
        match self {
            Self::XRef => "xref.entry",
            Self::DashXRef => "-xref.entry",
            Self::ExternalReferences => "externalreferences.entry",
            Self::ClassicXRef => "classicxref.entry",
        }
    }

    fn summary_code(self) -> &'static str {
        match self {
            Self::XRef => "xref.summary",
            Self::DashXRef => "-xref.summary",
            Self::ExternalReferences => "externalreferences.summary",
            Self::ClassicXRef => "classicxref.summary",
        }
    }
}
