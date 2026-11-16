//! Wave 8 session tokens for the extended command batch: `ELEV` and `UCSICON`,
//! plus the `FSMODE` token `FS` reads.
//!
//! `ELEV` is the session **elevation token**: `ELEV <height>` sets it and every
//! entity the shared create funnel inserts is created at that elevation (the
//! funnel applies the z offset; see `app_drawing::elevation` for the exact
//! command list). The bare selector reports the stored value, and the same token
//! is readable/writable as `SETVAR ELEV`.
//!
//! `FSMODE ON|OFF` is the **only** setter of the session token `FS` reads, so the
//! chain mode is reachable from the command line. The bare selector reports the
//! stored mode; `SETVAR FSMODE` reads and writes the same token.
//!
//! `UCSICON` flips one session flag (default OFF) and no UCS icon is rendered.
//!
//! Every arm here is **session state only**: nothing opens a transaction, bumps a
//! revision, records a `cad-history` entry or produces a `ChangeSet`, and each
//! outcome says so. A bad payload is a stable named refusal that changes nothing
//! (`elev.invalid_payload`, `elev.invalid_value`, `fsmode.invalid_payload`).

use super::*;
use crate::app_drafting::toggle_outcome;
use crate::app_reports_ext::diagnostic;
use crate::app_utility::require_none_payload;

impl Application {
    /// Dispatch one Wave 8 session token.
    pub(crate) fn extended_session8(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        match ext {
            ExtendedCommandId::Elev => self.elev_token(session, command, name),
            ExtendedCommandId::FsMode => self.fsmode_token(session, command, name),
            _ => {
                require_none_payload(command, name)?;
                match ext {
                    ExtendedCommandId::UcsIcon => {
                        let enabled = session.drafting_extras.toggle_ucsicon();
                        Ok(toggle_outcome(
                            "ucsicon.toggled",
                            format!(
                                "{name}: session UCS-icon flag is now {} (session state only); this \
                                 build renders no UCS icon, so no icon visibility, placement or \
                                 appearance is claimed",
                                state(enabled)
                            ),
                        ))
                    }
                    other => Err(CadError::Invariant(format!(
                        "not a Wave 8 session token: {other:?}"
                    ))),
                }
            }
        }
    }

    /// `ELEV` (report) / `ELEV <height>` (set): the session elevation token.
    ///
    /// Session-only: the drawing is never written, no revision is bumped and no
    /// undo step is recorded. The value is validated by
    /// [`crate::session_state::DraftingExtras::set_elev`]
    /// (`elev.invalid_value`), never clamped, so the create funnel can rely on a
    /// finite token.
    fn elev_token(
        &self,
        session: &mut SessionState,
        command: &Command,
        name: &'static str,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::None => Ok(report(
                "elev.value",
                format!(
                    "{name}: session elevation token is {:?} drawing units (session state only); \
                     `{name} <height>` sets it and new objects are created at that elevation",
                    session.drafting_extras.elev()
                ),
            )),
            CommandPayload::Elevation(height) => {
                session.drafting_extras.set_elev(*height)?;
                Ok(report(
                    "elev.set",
                    format!(
                        "{name}: session elevation token is now {:?} drawing units (session state \
                         only — nothing was written to the drawing); new objects the create funnel \
                         inserts are created at this elevation",
                        session.drafting_extras.elev()
                    ),
                ))
            }
            _ => Err(CadError::InvalidInput(format!(
                "elev.invalid_payload: {name} takes no payload or `Elevation(<height>)` (the typed \
                 line is `{name} <height>`); nothing was changed"
            ))),
        }
    }

    /// `FSMODE` (report) / `FSMODE ON|OFF` (set): the session chain mode `FS`
    /// reads.
    ///
    /// Session-only: the selection and the drawing are untouched. A non-boolean
    /// payload is a named refusal, never a toggle.
    fn fsmode_token(
        &self,
        session: &mut SessionState,
        command: &Command,
        name: &'static str,
    ) -> CadResult<CommandOutcome> {
        let enabled = match &command.payload {
            CommandPayload::None => {
                let current = state(session.drafting_extras.fs_mode());
                return Ok(report(
                    "fsmode.value",
                    format!("{name}: the FS chain mode is {current} (session state only)"),
                ));
            }
            CommandPayload::FsMode(enabled) => *enabled,
            _ => {
                return Err(CadError::InvalidInput(format!(
                "fsmode.invalid_payload: {name} takes no payload or `FsMode(<bool>)`; the typed \
                     line is `{name} ON|OFF` and nothing was changed"
            )))
            }
        };
        if session.drafting_extras.fs_mode() == enabled {
            return Ok(report(
                "fsmode.unchanged",
                format!(
                    "{name}: the FS chain mode is already {} (session state only); nothing changed",
                    state(enabled)
                ),
            ));
        }
        session.drafting_extras.set_fs_mode(enabled);
        let effect = if enabled {
            "FS follows the transitive tangent chain"
        } else {
            "FS adds only the directly touching objects"
        };
        Ok(report(
            "fsmode.set",
            format!(
                "{name}: the FS chain mode is now {} (session state only — no drawing write, \
                 revision bump or undo step); {effect}",
                state(enabled)
            ),
        ))
    }
}

fn state(enabled: bool) -> &'static str {
    if enabled {
        "ON"
    } else {
        "OFF"
    }
}

fn report(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![diagnostic(code, message)],
        measurement: None,
        point_report: None,
    }
}
