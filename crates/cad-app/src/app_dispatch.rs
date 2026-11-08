//! Dispatch for the sustained extended-command wrapper.
//!
//! `crate::CommandId::Extended` is one enum arm; this module routes it to the
//! module that owns the authority: session toggles in `app_drafting`, read-only
//! reports in `app_reports_ext`. Keeping the routing here means `application.rs`
//! stays a one-line interception rather than growing ten more arms.

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
        };
        Ok(Some(outcome))
    }
}
