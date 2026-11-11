//! Wave 13 command group: two real viewport-alignment writes and the named
//! refusals over plot styles, tables, fields, remote text, spelling,
//! super-hatching and layer translation.
//!
//! `PSETUPIN`/`-PSETUPIN` are no longer refusals: they are page-setup writes in
//! the sibling report module and are therefore Work-only.
//!
//! `ALIGNSPACE` and `VPSYNC` graduated from refusals to real
//! `set_layout_viewports` writes (one layout patch, one revision, one undo
//! step); they live in `crate::app_layout::viewport_align` and are Work-only.
//! Every remaining name here is refused with `CadError::Unsupported` carrying a
//! stable reason; a refusal writes nothing, bumps no revision and records no
//! history. All of them are Viewer-legal.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Handle one Wave 13 command: a real alignment write or its stable refusal.
    pub(crate) fn extended_w13_refusals(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::AlignSpace => self.align_space(session, command),
            ExtendedCommandId::VpSync => self.vp_sync(session, command),
            _ => {
                require_none_payload(command, ext.selector())?;
                Err(CadError::Unsupported(reason(ext)))
            }
        }
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
        // `ALIGNSPACE` and `VPSYNC` are dispatched above and never reach this
        // table; their old refusal strings are gone with the refusal.
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
