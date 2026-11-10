//! Wave 13 **named refusals** over plot styles, tables,
//! fields, remote text, spelling, super-hatching, viewport alignment and layer
//! translation.
//!
//! `PSETUPIN`/`-PSETUPIN` are no longer refusals: they are page-setup writes in
//! the sibling report module and are therefore Work-only.
//!
//! Each name here is refused with `CadError::Unsupported` carrying a stable
//! reason; a refusal writes nothing, bumps no revision and records no history.
//! All are Viewer-legal.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 13 command with its stable reason string.
    pub(crate) fn extended_w13_refusals(
        &mut self,
        _session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, ext.selector())?;
        Err(CadError::Unsupported(reason(ext)))
    }
}

fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::PlotStyleList => {
            "plotstyle.unsupported: named plot styles are not implemented (no CTB/STB table)"
        }
        ExtendedCommandId::DashPlotStyleList => {
            "-plotstyle.unsupported: named plot styles are not implemented (no CTB/STB table)"
        }
        ExtendedCommandId::ConvertPlotStyles => {
            "convertpstyles.unsupported: plot-style conversion is not implemented (no CTB/STB model)"
        }
        ExtendedCommandId::ConvertCtb => {
            "convertctb.unsupported: CTB conversion is not implemented (no CTB/STB model)"
        }
        ExtendedCommandId::ConvertGeneric => "convert.unsupported: generic object conversion is not implemented",
        ExtendedCommandId::TableExport => {
            "tableexport.unsupported: no CSV/file writer is implemented"
        }
        ExtendedCommandId::TableStyleList => {
            "tablestyle.unsupported: no table-style table is implemented"
        }
        ExtendedCommandId::TableInsert => {
            "tinsert.unsupported: no block-in-cell creation is implemented"
        }
        ExtendedCommandId::MatchCell => {
            "matchcell.unsupported: no cell-style copy surface is implemented"
        }
        ExtendedCommandId::Field => "field.unsupported: fields are not implemented (no field model)",
        ExtendedCommandId::RText => "rtext.unsupported: remote text is not implemented (no RTEXT model)",
        ExtendedCommandId::RTextEdit => {
            "rtedit.unsupported: remote text is not implemented (no RTEXT model)"
        }
        ExtendedCommandId::Spell => "spell.unsupported: no spell-check dictionary or model is implemented",
        ExtendedCommandId::SuperHatch => {
            "superhatch.unsupported: image/block-based super-hatching is not implemented"
        }
        ExtendedCommandId::AlignSpace => {
            "alignspace.unsupported: viewport alignment is not implemented (no viewport alignment API)"
        }
        ExtendedCommandId::VpSync => {
            "vpsync.unsupported: viewport synchronization is not implemented (no viewport group model)"
        }
        ExtendedCommandId::LayTrans => {
            "laytrans.unsupported: layer translation standards are not implemented"
        }
        other => {
            return format!(
                "{}: this command is not a Wave 13 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
