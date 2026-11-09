//! Wave 8 session tokens for the extended command batch.
//!
//! `ELEV` is a **read-only report** of the session elevation token (default
//! 0.0): the typed setter form `ELEV <height>` is NOT IMPLEMENTED and no object
//! is created at an elevation. `UCSICON` flips one session flag (default OFF)
//! and no UCS icon is rendered.
//!
//! Both commands take no payload, open no transaction, bump no revision and
//! record no `cad-history` entry; neither is a drawing write.

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
        require_none_payload(command, name)?;
        match ext {
            ExtendedCommandId::Elev => Ok(report(
                "elev.value",
                format!(
                    "{name}: session elevation token is {:?} (session state only); the setter \
                     form `ELEV <height>` is NOT IMPLEMENTED and this build creates no object at \
                     an elevation",
                    session.drafting_extras.elev()
                ),
            )),
            ExtendedCommandId::UcsIcon => {
                let enabled = session.drafting_extras.toggle_ucsicon();
                Ok(toggle_outcome(
                    "ucsicon.toggled",
                    format!(
                        "{name}: session UCS-icon flag is now {} (session state only); this build \
                         renders no UCS icon, so no icon visibility, placement or appearance is \
                         claimed",
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
