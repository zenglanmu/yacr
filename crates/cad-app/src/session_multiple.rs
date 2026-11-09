//! The session-side `MULTIPLE` repeat request.
//!
//! Split out of `session_state.rs` (which is at the code-size ceiling) with the
//! behaviour unchanged. `MULTIPLE` is session state only: it arms which command
//! the next invocation repeats and never touches the drawing, the revision or
//! the history.

use crate::SessionState;

/// What a `MULTIPLE` invocation asked to repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultipleRepeat {
    /// Bare `MULTIPLE`: repeat the next command.
    NextCommand,
    /// `MULTIPLE <command>`: repeat that canonical selector.
    Command(String),
}

impl SessionState {
    /// Arm the `MULTIPLE` repeat request (`None` target = the next command).
    pub fn arm_multiple(&mut self, target: Option<String>) {
        self.multiple = Some(match target {
            Some(name) => MultipleRepeat::Command(name),
            None => MultipleRepeat::NextCommand,
        });
    }

    /// The armed `MULTIPLE` request, if any. Session state only.
    pub fn multiple_target(&self) -> Option<&MultipleRepeat> {
        self.multiple.as_ref()
    }

    /// Disarm the `MULTIPLE` request.
    pub fn clear_multiple(&mut self) {
        self.multiple = None;
    }
}
