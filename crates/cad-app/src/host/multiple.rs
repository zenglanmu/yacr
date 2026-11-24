//! The `MULTIPLE` repeat loop on the host's capture-confirm funnel.
//!
//! Split out of `host/mod.rs` for the file-size ceiling. The arm lives in
//! [`crate::SessionState`]; this module only advances it after a **successful**
//! capture confirm, so every repetition is one real user confirmation and the
//! loop can never spin on its own.

use super::*;

impl HostController {
    /// Cancel an active tool without executing a drawing command.
    ///
    /// This is also the documented `MULTIPLE` stop: `Esc`/`CANCEL` clears the
    /// repeat arm, so a canceled capture is never re-opened.
    pub fn cancel_tool(&mut self) -> CadResult<()> {
        self.session.cancel_tool()?;
        self.session.clear_multiple();
        Ok(())
    }

    /// Open the next pass of an armed `MULTIPLE` capture after a successful
    /// confirm, or stop the loop with a named reason.
    ///
    /// The stop conditions are explicit: a bare `MULTIPLE` is pinned to the
    /// command that just completed, a target mismatch is a no-op (some other
    /// command completed), `Esc`/`CANCEL` clears the arm before this runs, a
    /// failed re-start clears the arm (`multiple.repeat_failed`) and the
    /// [`crate::MULTIPLE_MAX_REPEATS`] guard clears it with
    /// `multiple.repeat_limit`. Each repetition is one real user confirm, so the
    /// loop can never spin on its own.
    pub(super) fn continue_multiple(&mut self, kind: DrawToolKind) -> Option<Diagnostic> {
        let command = kind.command();
        if !self.session.multiple_repeats_command(command) {
            return None;
        }
        self.session.pin_multiple_command(command);
        if !self.session.note_multiple_repeat() {
            self.session.clear_multiple();
            return Some(Diagnostic {
                object: None,
                code: "multiple.repeat_limit".into(),
                message: format!(
                    "MULTIPLE: stopped after {} repetition(s) (the guard against an automation \
                     client that never cancels); the repeat arm is cleared",
                    crate::MULTIPLE_MAX_REPEATS
                ),
            });
        }
        match self.session.begin_draw(kind) {
            Ok(()) => Some(Diagnostic {
                object: None,
                code: "multiple.repeated".into(),
                message: "MULTIPLE: the capture was re-opened for another pass; Esc/CANCEL stops \
                          the repeat"
                    .into(),
            }),
            Err(error) => {
                self.session.clear_multiple();
                Some(Diagnostic {
                    object: None,
                    code: "multiple.repeat_failed".into(),
                    message: format!(
                        "MULTIPLE: the repeat could not re-open the capture ({error}); the repeat \
                         arm is cleared and nothing was written"
                    ),
                })
            }
        }
    }
}
