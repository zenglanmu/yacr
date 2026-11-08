//! Session-only drafting toggles for the extended command batch.
//!
//! `QTEXT`, `FILL`, `TFRAMES`, `REGENAUTO` and `ISODRAFT` each flip one flag in
//! [`SessionState::drafting_extras`]. The commands take no payload, open no
//! transaction, bump no revision and record no `cad-history` entry. This build
//! does **not** consume the flags in a renderer, so each outcome says the effect
//! is NOT IMPLEMENTED and never claims a visual change.

use super::*;
use crate::app_utility::require_none_payload;

pub(crate) fn toggle_outcome(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}

impl Application {
    /// Toggle one drafting flag (`QTEXT`/`FILL`/`TFRAMES`/`REGENAUTO`/`ISODRAFT`).
    pub(crate) fn drafting_toggle(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        let (code, enabled, behaviour) = match ext {
            ExtendedCommandId::QText => (
                "qtext.toggled",
                session.drafting_extras.toggle_qtext(),
                "quick-text rectangle display",
            ),
            ExtendedCommandId::Fill => (
                "fill.toggled",
                session.drafting_extras.toggle_fill(),
                "solid-fill display",
            ),
            ExtendedCommandId::TFrames => (
                "tframes.toggled",
                session.drafting_extras.toggle_tframes(),
                "text/grid frame display",
            ),
            ExtendedCommandId::RegenAuto => (
                "regenauto.toggled",
                session.drafting_extras.toggle_regenauto(),
                "automatic regeneration",
            ),
            ExtendedCommandId::IsoDraft => (
                "isodraft.toggled",
                session.drafting_extras.toggle_isodraft(),
                "isometric-plane drafting",
            ),
            other => {
                return Err(CadError::Invariant(format!(
                    "not a drafting toggle: {other:?}"
                )))
            }
        };
        let state = if enabled { "ON" } else { "OFF" };
        Ok(toggle_outcome(
            code,
            format!(
                "{name}: session flag is now {state} (session state only; the drawing was not \
                 written and {behaviour} is NOT IMPLEMENTED in this build, so no visual effect is \
                 claimed or verified)"
            ),
        ))
    }
}
