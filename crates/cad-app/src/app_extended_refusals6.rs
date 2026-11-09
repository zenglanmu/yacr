//! Wave 6 **named refusals** over the file / plot / export surfaces.
//!
//! Twenty-four AutoCAD 2026 names describe features this build has no model or
//! API for: multi-document operations (`SAVEALL`, `QNEW`, `CLOSEALL`,
//! `CLOSEALLOTHER`), reopen-to-source (`REVERT`), host-owned application exit
//! (`QQUIT`, `QUIT`), property propagation (`PROPULATE`), plotter and plot-style
//! configuration (`PLOTTERMANAGER`, `STYLESMANAGER`, `PLOTSTAMP`,
//! `-PLOTSTAMP`), export settings/preview/layout (`EXPORTSETTINGS`, `PREVIEW`,
//! `EXPORTLAYOUT`), the command-line plot/export spellings (`-PLOT`, `-EXPORT`),
//! and packaging/publishing (`ETRANSMIT`, `-ETRANSMIT`, `ARCHIVE`, `-ARCHIVE`,
//! `PUBLISH`, `-PUBLISH`, `AUTOPUBLISH`).
//!
//! Each name is admitted by the vocabulary and then refused with
//! `CadError::Unsupported` carrying a **stable** reason string. A refusal writes
//! nothing, bumps no revision and records no `cad-history` entry, so every one of
//! these stays Viewer-legal. A caller payload is refused first with
//! `require_none_payload`, so a malformed request never reaches the
//! feature-specific reason.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 6 command with its stable reason string.
    pub(crate) fn extended_refusal6(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

/// The stable refusal reason for one Wave 6 command.
///
/// These strings are contract-tested and published in
/// `docs/autocad-2026-command-coverage.md` and `docs/compatibility.md`; changing
/// one is a behaviour change.
fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::SaveAll => {
            "saveall.single_document: this build has one document; use SAVE"
        }
        ExtendedCommandId::QNew => "qnew.single_document: use NEW",
        ExtendedCommandId::Revert => "revert.unsupported: no reopen-to-source path",
        ExtendedCommandId::CloseAll => "closeall.single_document: use CLOSE",
        ExtendedCommandId::CloseAllOther => {
            "closeallother.single_document: only one document exists"
        }
        ExtendedCommandId::QQuit => "qquit.unsupported: application exit is host-owned",
        ExtendedCommandId::Quit => "quit.unsupported: application exit is host-owned",
        ExtendedCommandId::Propulate => "propulate.unsupported: no property-propagation model",
        ExtendedCommandId::PlotterManager => {
            "plottermanager.unsupported: no plotter configuration model"
        }
        ExtendedCommandId::StylesManager => {
            "stylesmanager.unsupported: no CTB/STB plot-style model"
        }
        ExtendedCommandId::ExportSettings => "exportsettings.unsupported: no export-settings model",
        ExtendedCommandId::Preview => "preview.unsupported: no plot-preview window",
        ExtendedCommandId::ExportLayout => {
            "exportlayout.unsupported: use PLOT (one layout, SVG/PDF)"
        }
        ExtendedCommandId::PlotStamp => "plotstamp.unsupported: no plot-stamp model",
        ExtendedCommandId::DashPlotStamp => "-plotstamp.unsupported: no plot-stamp model",
        ExtendedCommandId::DashPlot => {
            "-plot.unsupported: use PLOT; this build has one vector plot path"
        }
        ExtendedCommandId::DashExport => {
            "-export.unsupported: use PLOT/EXPORT; this build has one vector export path"
        }
        ExtendedCommandId::ETransmit => "etransmit.unsupported: no eTransmit/zip packaging model",
        ExtendedCommandId::DashETransmit => {
            "-etransmit.unsupported: no eTransmit/zip packaging model"
        }
        ExtendedCommandId::Archive => "archive.unsupported: no sheet-set archive packaging model",
        ExtendedCommandId::DashArchive => {
            "-archive.unsupported: no sheet-set archive packaging model"
        }
        ExtendedCommandId::Publish => "publish.unsupported: no batch publish model",
        ExtendedCommandId::DashPublish => "-publish.unsupported: no batch publish model",
        ExtendedCommandId::AutoPublish => "autopublish.unsupported: no automatic publish model",
        other => {
            return format!(
                "{}: this command is not a Wave 6 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
