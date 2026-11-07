//! The typed `PEDIT <option>` line (2026-11-07): AutoCAD 2026's `PEDIT`
//! declared v1 subset.
//!
//! `PEDIT` is an argument form of its base selector, not a command of its own:
//! there is no second `CommandId` and no second IPC. The current selection is
//! still the input (exactly one model-space LWPOLYLINE), so this module only
//! parses the typed option and dispatches [`cad_app::CommandPayload::PeditOption`].
//!
//! * `PEDIT CLOSE|C` closes the one selected open polyline (at least three
//!   vertices); an already-closed polyline is the `pedit.already_closed` no-op.
//! * `PEDIT OPEN|O` opens the one selected closed polyline; an already-open
//!   polyline is the `pedit.already_open` no-op.
//! * `PEDIT REVERSE|R` reuses the `REVERSE` geometry rule (bulges reversed and
//!   negated), for open and closed polylines.
//!
//! A bare `PEDIT` never reaches here (it keeps the name-only funnel and the
//! application refuses it with `pedit.needs_option`). An argument outside the
//! grammar is refused with the stable
//! [`super::invalid_pedit_argument_reason`], recorded for `cad_status.error`,
//! and changes nothing.

use cad_app::{CommandId, CommandPayload};

use super::diagnostics::cad_error_text;
use super::{parse_pedit_argument, CadSession, PEDIT_SELECTOR};

impl CadSession {
    /// Apply one typed `PEDIT <option>` line.
    pub(super) fn pedit_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(option) = parse_pedit_argument(argument) else {
            return self.refuse_argument(super::invalid_pedit_argument_reason(text));
        };
        // Echo the admitted selector (never the typed argument).
        self.core.record_command(PEDIT_SELECTOR);
        match self
            .core
            .command(CommandId::Pedit, CommandPayload::PeditOption(option))
        {
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
