//! Dispatcher for the 2026-11-07 extras batch.
//!
//! The command arms live in the modules that own their authority:
//!
//! * `SKETCH` / `TRACE` / `MOCORO` / `MSTRETCH` / `LINETYPE` — `app_drawing`
//!   (`extras_batch`, `mstretch`), because they create or edit entity rows;
//! * `VIEWRES` / `VSCURRENT` — `app_view_extras`, session display tokens only.
//!
//! `-PAN` and `REGEN3` have **no** `CommandId` here: they are spellings that
//! normalize onto `PAN` and `REGEN` in `cad-ui-core`, so there is one camera path
//! and one regeneration path. `-LINETYPE` likewise normalizes onto `LINETYPE`.

use super::*;

/// Whether `id` is dispatched by this module.
pub(crate) fn is_extras(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::Sketch
            | CommandId::Trace
            | CommandId::Mocoro
            | CommandId::MStretch
            | CommandId::ViewRes
            | CommandId::VisualStyle
            | CommandId::Linetype
    )
}

impl Application {
    /// Dispatch one extras command.
    pub(crate) fn extras_command(
        &mut self,
        session: &mut SessionState,
        id: CommandId,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match id {
            CommandId::Sketch => self.sketch(session, command),
            CommandId::Trace => self.trace(session, command),
            CommandId::Mocoro => self.mocoro(session, command),
            CommandId::MStretch => self.mstretch_entities(session, command),
            CommandId::ViewRes => self.viewres(session, command),
            CommandId::VisualStyle => self.vscurrent(session, command),
            CommandId::Linetype => self.linetype(session, command),
            other => Err(CadError::Invariant(format!(
                "not an extras command: {other:?}"
            ))),
        }
    }
}
