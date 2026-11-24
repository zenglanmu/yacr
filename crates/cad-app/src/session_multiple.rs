//! The session-side `MULTIPLE` repeat request.
//!
//! Split out of `session_state.rs` (which is at the code-size ceiling) with the
//! behaviour unchanged. `MULTIPLE` is session state only: it arms which
//! interactive capture command repeats and never touches the drawing, the
//! revision or the history.
//!
//! # The repeat loop
//!
//! AutoCAD's `MULTIPLE` repeats the next command until it is canceled. This
//! build has no command prompt loop, so the repeat lives on the one funnel where
//! a command really ends: a **confirmed draw/edit capture**
//! ([`crate::host::HostController::confirm_draw`]). After a successful confirm
//! the host asks this state whether the completed capture should keep going:
//!
//! * `MULTIPLE` (bare) repeats the *next* capture command — the first successful
//!   confirm converts [`MultipleRepeat::NextCommand`] into
//!   [`MultipleRepeat::Command`] for that exact command;
//! * `MULTIPLE <command>` repeats only that command.
//!
//! The stop conditions are explicit and bounded:
//!
//! * **cancel** (`Esc` / `CANCEL`) clears the arm — the documented "until
//!   canceled";
//! * a repeat that cannot re-start the capture clears the arm and reports
//!   `multiple.repeat_failed`;
//! * [`MULTIPLE_MAX_REPEATS`] caps a client that never cancels; reaching it
//!   clears the arm and reports `multiple.repeat_limit`. Every repetition still
//!   needs a real user confirm, so the cap is a guard, not the loop condition.
//!
//! One-shot commands (and measurement tools) do not repeat in this build: they
//! have no capture to re-open, so `MULTIPLE <one-shot>` is refused by name when
//! the command is armed.

use crate::{CommandId, SessionState};

/// The hard cap on repetitions of one armed capture.
///
/// The only documented stop is `Esc`/`CANCEL`; this cap exists so an automation
/// client that confirms forever cannot spin the session indefinitely. It is
/// deliberately large enough that a real drafting session never reaches it.
pub const MULTIPLE_MAX_REPEATS: u32 = 512;

/// What a `MULTIPLE` invocation asked to repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultipleRepeat {
    /// Bare `MULTIPLE`: repeat the next interactive capture command.
    NextCommand,
    /// `MULTIPLE <command>`: repeat that exact capture command.
    Command(CommandId),
}

impl MultipleRepeat {
    /// Whether this arm repeats `command`.
    fn matches(&self, command: CommandId) -> bool {
        match self {
            // The next capture command becomes the target when it first
            // completes; until then every capture is a candidate.
            Self::NextCommand => true,
            Self::Command(armed) => *armed == command,
        }
    }
}

/// The resolved target of a typed `MULTIPLE <command>`.
///
/// The `cad-ui-core` command vocabulary resolves the typed name before the
/// payload is built, so the application never keeps a string it cannot map:
/// [`Self::command`] is the one admitted [`CommandId`] and [`Self::selector`] is
/// the canonical uppercase spelling for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipleTarget {
    /// The canonical uppercase selector the user typed (`LINE`, `CIRCLE`, …).
    pub selector: String,
    /// The admitted command the selector resolves to.
    pub command: CommandId,
}

impl SessionState {
    /// Arm the `MULTIPLE` repeat request.
    ///
    /// `None` arms the next capture command; `Some(command)` arms that exact
    /// command. Arming resets the repeat counter.
    pub fn arm_multiple(&mut self, target: Option<CommandId>) {
        self.multiple = Some(match target {
            Some(command) => MultipleRepeat::Command(command),
            None => MultipleRepeat::NextCommand,
        });
        self.multiple_repeats = 0;
    }

    /// The armed `MULTIPLE` request, if any. Session state only.
    pub fn multiple_target(&self) -> Option<&MultipleRepeat> {
        self.multiple.as_ref()
    }

    /// Whether the armed request repeats `command`.
    pub fn multiple_repeats_command(&self, command: CommandId) -> bool {
        self.multiple
            .as_ref()
            .is_some_and(|arm| arm.matches(command))
    }

    /// Replace a bare `NextCommand` arm with the concrete command that just
    /// completed, so only that command keeps repeating.
    pub(crate) fn pin_multiple_command(&mut self, command: CommandId) {
        if matches!(self.multiple, Some(MultipleRepeat::NextCommand)) {
            self.multiple = Some(MultipleRepeat::Command(command));
        }
    }

    /// Count one real repetition. `false` means [`MULTIPLE_MAX_REPEATS`] is
    /// reached and the caller must stop (and clear) the loop.
    pub(crate) fn note_multiple_repeat(&mut self) -> bool {
        if self.multiple_repeats >= MULTIPLE_MAX_REPEATS {
            return false;
        }
        self.multiple_repeats += 1;
        true
    }

    /// How many repetitions the current arm has run (test/status read).
    pub fn multiple_repeat_count(&self) -> u32 {
        self.multiple_repeats
    }

    /// Disarm the `MULTIPLE` request (cancel/stop).
    pub fn clear_multiple(&mut self) {
        self.multiple = None;
        self.multiple_repeats = 0;
    }
}
