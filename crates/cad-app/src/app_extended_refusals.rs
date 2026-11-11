//! Wave 2 **named refusals** for the extended command batch.
//!
//! Seven AutoCAD 2026 names describe features this build has no model or API
//! for: hyperlinks, input-search settings, digital signatures, a drawing
//! insertion base point, drawing limits and plot-job details.
//!
//! Each name is admitted by the vocabulary and then refused with
//! `CadError::Unsupported` carrying a **stable** reason string (the same shape
//! `LAYOUTMERGE`/`DIMBREAK` use). That is a legitimate partial implementation:
//! the command path exists and answers honestly, but it must never be documented
//! as an implemented feature. A refusal writes nothing, bumps no revision and
//! records no `cad-history` entry, so every one of these is Viewer-legal.
//!
//! A caller payload is refused first with `require_none_payload`, so a malformed
//! request never reaches the feature-specific reason.
//!
//! # Graduated writes (2026-11-11)
//!
//! `VPLAYER`, `MVSETUP`, `PSBSCALE` and `PSTSCALE` were Wave 2 refusals and are
//! now real 2D paper-space features (per-viewport layer visibility, a scaled
//! viewport grid, and the two paper-space scale writes). They are dispatched
//! through [`Application::extended_paper_write`] to
//! [`crate::app_layout::paper_writes`]; the seven names left here keep their
//! exact published reason strings.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one Wave 2 command with its stable reason string.
    pub(crate) fn extended_refusal(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        Err(CadError::Unsupported(reason(ext)))
    }

    /// Dispatch one graduated Wave 2 **paper-space write**.
    ///
    /// Each arm is a real write (one transaction / one layout patch) except the
    /// bare `VPLAYER` report, which is read-only. The module that owns the
    /// implementation documents every `*.` refusal code.
    pub(crate) fn extended_paper_write(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::VPlayer => self.vplayer(session, command),
            ExtendedCommandId::MvSetup => self.mv_setup(session, command),
            ExtendedCommandId::PsbScale => self.psb_scale(session, command),
            ExtendedCommandId::PstScale => self.pst_scale(session, command),
            other => Err(CadError::Unsupported(reason(other))),
        }
    }
}

/// The stable refusal reason for one Wave 2 command.
///
/// These strings are contract-tested and published in
/// `docs/autocad-2026-command-coverage.md`; changing one is a behaviour change.
fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::HyperlinkOptions => {
            "hyperlinkoptions.unsupported: this build has no hyperlink storage model"
        }
        ExtendedCommandId::InputSearchOptions => {
            "inputsearchoptions.unsupported: input-search settings are not modeled"
        }
        ExtendedCommandId::DashInputSearchOptions => {
            "-inputsearchoptions.unsupported: input-search settings are not modeled"
        }
        ExtendedCommandId::SigValidate => {
            "sigvalidate.unsupported: this build has no digital-signature model"
        }
        ExtendedCommandId::BasePoint => {
            "base.unsupported: the drawing stores no insertion base point"
        }
        ExtendedCommandId::DrawingLimits => {
            "limits.unsupported: this build has no drawing-limits model"
        }
        ExtendedCommandId::ViewPlotDetails => "viewplotdetails.unsupported: no plot-details model",
        other => {
            return format!(
                "{}: this command is not a Wave 2 refusal",
                other.selector().to_ascii_lowercase()
            )
        }
    };
    text.to_string()
}
