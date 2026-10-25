//! The typed `SNAPUNIT <value>` line: the session grid-snap spacing sysvar.
//!
//! This is the second (and last) argument-taking selector, sharing the grammar
//! machinery with `ZOOM` ([`super::typed_command_argument`],
//! [`super::parse_snap_unit_argument`]). It is **not** a second command layer: a
//! valid value is dispatched through the ordinary
//! [`cad_app::CommandId::SetSnapSpacing`] payload
//! ([`cad_app::CommandPayload::SnapSpacing`]) and the application/session layer
//! is the validation authority.
//!
//! Session/display state only: no database write, no revision bump, no undo entry
//! and no persisted preference. Allowed in Viewer and Work.
//!
//! Kept in its own module only to keep `session/mod.rs` within the file-size
//! budget.

use cad_app::{CommandId, CommandPayload};

use super::diagnostics::cad_error_text;
use super::CadSession;

impl CadSession {
    /// Apply one typed `SNAPUNIT <value>` line.
    ///
    /// | argument | behaviour |
    /// | --- | --- |
    /// | positive decimal (`25`, `0.5`, `2.5`) | set the session snap spacing, the value the `SNAP` (F9) quantiser uses |
    /// | anything else (`0`, `-5`, `abc`, `2 3`, `.5`, `1e3`, `ALL`) | the stable [`super::invalid_snap_unit_argument_reason`], recorded in [`CadSession`]'s error; nothing changes |
    ///
    /// A bare `SNAPUNIT` never reaches this method: it has no argument, so it
    /// keeps the name-only payload refusal of the application layer (an explicit
    /// "needs a positive spacing" error, never a fabricated default).
    pub(super) fn snap_unit_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(spacing) = super::parse_snap_unit_argument(argument) else {
            let reason = super::invalid_snap_unit_argument_reason(text);
            *self.error.borrow_mut() = Some(reason.clone());
            return Err(reason);
        };
        // Echo the admitted selector name (never the typed value), exactly like
        // the name-only path.
        self.core.record_command(super::SNAPUNIT_SELECTOR);
        match self.core.command(
            CommandId::SetSnapSpacing,
            CommandPayload::SnapSpacing(spacing),
        ) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}
