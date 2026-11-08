//! Dispatch for the sustained extended-command wrapper.
//!
//! `crate::CommandId::Extended` is one enum arm; this module routes it to the
//! module that owns the authority: the pilot session toggles in `app_drafting`,
//! the pilot read-only reports in `app_reports_ext`, and the Wave 2 handlers in
//! `app_extended_session2` (tokens), `app_extended_reports2` (reports) and
//! `app_extended_refusals` (named refusals). Keeping the routing here means
//! `application.rs` stays a one-line interception rather than growing forty more
//! arms.

use super::*;

impl Application {
    /// Dispatch an [`ExtendedCommandId`], or `None` when the command is not one.
    ///
    /// The caller has already authorized the command and checked the stale
    /// document, so every handler can assume a live session.
    pub(crate) fn dispatch_extended(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<Option<CommandOutcome>> {
        let CommandId::Extended(ext) = command.id else {
            return Ok(None);
        };
        let outcome = match ext {
            ExtendedCommandId::QText
            | ExtendedCommandId::Fill
            | ExtendedCommandId::TFrames
            | ExtendedCommandId::RegenAuto
            | ExtendedCommandId::IsoDraft => self.drafting_toggle(session, command, ext)?,
            ExtendedCommandId::BlockCount
            | ExtendedCommandId::CountObjects
            | ExtendedCommandId::EditTime
            | ExtendedCommandId::JulianDate
            | ExtendedCommandId::XList => self.extended_report(session, command, ext)?,
            // Wave 2 session tokens: each flips or reports one session token.
            ExtendedCommandId::DragMode
            | ExtendedCommandId::Aperture
            | ExtendedCommandId::IsoPlane
            | ExtendedCommandId::HighlightNew
            | ExtendedCommandId::TraySettings
            | ExtendedCommandId::TextScreen
            | ExtendedCommandId::GraphScreen
            | ExtendedCommandId::FindInclude
            | ExtendedCommandId::FindMatch
            | ExtendedCommandId::RevCloudProperties => {
                self.extended_session2_toggle(session, command, ext)?
            }
            // Wave 2 read-only reports: no transaction, no revision, no history.
            ExtendedCommandId::SysVarMonitor
            | ExtendedCommandId::SysVarDialog
            | ExtendedCommandId::ViewTransitionOptions
            | ExtendedCommandId::AliasEdit
            | ExtendedCommandId::WhoHas
            | ExtendedCommandId::AcadInfo
            | ExtendedCommandId::PointType
            | ExtendedCommandId::ScaleListEdit => self.extended_report2(session, command, ext)?,
            // Wave 2 named refusals: a stable `Unsupported` reason per name.
            ExtendedCommandId::HyperlinkOptions
            | ExtendedCommandId::InputSearchOptions
            | ExtendedCommandId::DashInputSearchOptions
            | ExtendedCommandId::SigValidate
            | ExtendedCommandId::BasePoint
            | ExtendedCommandId::DrawingLimits
            | ExtendedCommandId::ViewPlotDetails
            | ExtendedCommandId::VPlayer
            | ExtendedCommandId::MView
            | ExtendedCommandId::PsbScale
            | ExtendedCommandId::PstScale
            | ExtendedCommandId::MvSetup => self.extended_refusal(command, ext)?,
        };
        Ok(Some(outcome))
    }
}
