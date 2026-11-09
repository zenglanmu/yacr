//! Wave 5 **attribute and external-reference** handlers for the extended batch.
//!
//! `ATTDISP` reads/flips one session token ([`SessionState::drafting_extras`])
//! and states plainly that this build has no block-attribute model, so no
//! attribute is rendered either way.
//!
//! `BATTMAN` and `BATTORDER` are read-only listings of the drawing's block
//! definitions, reusing the same `drawing.blocks()` access `XLIST`/`BLOCK?` read;
//! both add a `*.attributes_not_modeled` diagnostic because the block table has
//! no attribute rows to list. `XREF`/`-XREF`/`EXTERNALREFERENCES` report the
//! import-time external-reference provenance through
//! `drawing.external_references()` — the database **does** expose that
//! enumeration, so no `xref.not_modeled` fallback is reachable; a drawing with no
//! xrefs reports zero rows rather than inventing any. `EXTERNALREFERENCESCLOSE`
//! is a named no-op because this build has no palette to close.
//!
//! None of these opens a transaction, bumps the revision, records a
//! `cad-history` entry or publishes a `ChangeSet`; all seven are Viewer-legal.

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};

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
            ExtendedCommandId::BattMan => {
                self.block_attribute_report(command, BlockReport::BattMan)
            }
            ExtendedCommandId::BattOrder => {
                self.block_attribute_report(command, BlockReport::BattOrder)
            }
            ExtendedCommandId::XRef => self.xref_report(command, XrefReport::XRef),
            ExtendedCommandId::DashXRef => self.xref_report(command, XrefReport::DashXRef),
            ExtendedCommandId::ExternalReferences => {
                self.xref_report(command, XrefReport::ExternalReferences)
            }
            ExtendedCommandId::ExternalReferencesClose => self.externalreferences_close(command),
            other => Err(CadError::Invariant(format!(
                "not a Wave 5 attribute/xref command: {other:?}"
            ))),
        }
    }

    /// `ATTDISP`: flip the session attribute-display token and report it.
    ///
    /// The token is session state only. This build has no block-attribute model,
    /// so no attribute is rendered (or hidden) and no drawing row changes.
    fn attdisp(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, "ATTDISP")?;
        let enabled = session.drafting_extras.toggle_attdisp();
        Ok(report_outcome(vec![diagnostic(
            "attdisp.toggled",
            format!(
                "ATTDISP: session attribute-display token is now {} (session state only); this \
                 build has no block-attribute model, so no attribute is rendered and the drawing \
                 was not written",
                state(enabled)
            ),
        )]))
    }

    /// `BATTMAN` / `BATTORDER`: read-only block-definition listing.
    ///
    /// The block table is the authority; there is no attribute row to enumerate,
    /// which the trailing `*.attributes_not_modeled` diagnostic states. This is a
    /// listing, not the block-attribute manager/dialog.
    fn block_attribute_report(
        &self,
        command: &Command,
        report: BlockReport,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, report.selector())?;
        let drawing = document_drawing(self, command)?;
        let mut diagnostics = Vec::new();
        let mut count = 0usize;
        for block in drawing.blocks() {
            count += 1;
            let name = drawing
                .block_name(block.id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("block#{}", block.id.0));
            diagnostics.push(diagnostic(
                report.entry_code(),
                format!(
                    "{}: block '{name}' id {} members {}",
                    report.selector(),
                    block.id.0,
                    block.entities.len()
                ),
            ));
        }
        diagnostics.push(diagnostic(
            report.summary_code(),
            format!(
                "{}: {count} block definition(s) from the drawing's block table; this is a \
                 read-only listing, not the block-attribute manager",
                report.selector()
            ),
        ));
        diagnostics.push(diagnostic(
            report.not_modeled_code(),
            format!(
                "{}: block attributes are NOT MODELED in this build; no attribute row, value or \
                 display order exists to list, and nothing was invented",
                report.selector()
            ),
        ));
        Ok(report_outcome(diagnostics))
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

fn state(enabled: bool) -> &'static str {
    if enabled {
        "ON"
    } else {
        "OFF"
    }
}

/// Which block-attribute report is running (`BATTMAN` vs `BATTORDER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockReport {
    /// `BATTMAN`: `battman.entry` / `battman.summary`.
    BattMan,
    /// `BATTORDER`: `battorder.entry` / `battorder.summary`.
    BattOrder,
}

impl BlockReport {
    fn selector(self) -> &'static str {
        match self {
            Self::BattMan => "BATTMAN",
            Self::BattOrder => "BATTORDER",
        }
    }

    fn entry_code(self) -> &'static str {
        match self {
            Self::BattMan => "battman.entry",
            Self::BattOrder => "battorder.entry",
        }
    }

    fn summary_code(self) -> &'static str {
        match self {
            Self::BattMan => "battman.summary",
            Self::BattOrder => "battorder.summary",
        }
    }

    fn not_modeled_code(self) -> &'static str {
        match self {
            Self::BattMan => "battman.attributes_not_modeled",
            Self::BattOrder => "battorder.attributes_not_modeled",
        }
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
}

impl XrefReport {
    fn selector(self) -> &'static str {
        match self {
            Self::XRef => "XREF",
            Self::DashXRef => "-XREF",
            Self::ExternalReferences => "EXTERNALREFERENCES",
        }
    }

    fn entry_code(self) -> &'static str {
        match self {
            Self::XRef => "xref.entry",
            Self::DashXRef => "-xref.entry",
            Self::ExternalReferences => "externalreferences.entry",
        }
    }

    fn summary_code(self) -> &'static str {
        match self {
            Self::XRef => "xref.summary",
            Self::DashXRef => "-xref.summary",
            Self::ExternalReferences => "externalreferences.summary",
        }
    }
}
