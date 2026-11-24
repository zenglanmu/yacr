//! The action-recorder commands' application-layer entries (ADR-0056).
//!
//! The recorder's real implementation is the session command line
//! (`cad_ui_core::session::action_recorder`): it owns the typed name/path, the
//! recording buffer, the macro file and playback. These variants still exist in
//! the command table so the vocabulary is honest and the command line can reach
//! them; a direct `Command` with no typed text (an IPC caller) has no name, path
//! or message to act on, so it is refused by name instead of fabricating a
//! recorder action. This is the same split `TABLEEXPORT` uses for its typed
//! destination.

use super::*;
use crate::app_utility::require_none_payload;

impl Application {
    /// Refuse one action-recorder command that arrived as a typed payload (no
    /// command-line text).
    pub(crate) fn extended_action_recorder_refusal(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        require_none_payload(command, ext.selector())?;
        let reason = match ext {
            ExtendedCommandId::ActRecord => {
                "actrecord.no_argument: ACTRECORD needs a typed macro name or `.json` path; the \
                 IPC form carries none and this build has no file dialog"
            }
            ExtendedCommandId::ActStop | ExtendedCommandId::DashActStop => {
                "actstop.no_recording: ACTSTOP is a session command-line command; this IPC path \
                 holds no recorder state"
            }
            ExtendedCommandId::ActUserInput => {
                "actuserinput.not_recording: ACTUSERINPUT records a pause only on the session \
                 command line while ACTRECORD is active"
            }
            ExtendedCommandId::ActUserMessage | ExtendedCommandId::DashActUserMessage => {
                "actusermessage.message: ACTUSERMESSAGE needs a typed non-empty message; the IPC \
                 form carries none"
            }
            ExtendedCommandId::ActManager => {
                "actmanager.command_line: ACTMANAGER is the session command-line list/PLAY/DELETE; \
                 this IPC path holds no recorder state"
            }
            _ => {
                return Err(CadError::InvalidInput(format!(
                    "{}: not an action-recorder command",
                    ext.selector()
                )))
            }
        };
        Err(CadError::Unsupported(reason.to_string()))
    }
}
