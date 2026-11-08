//! Wave 2 session tokens for the extended command batch.
//!
//! `DRAGMODE`, `APERTURE`, `ISOPLANE`, `HIGHLIGHTNEW`, `TRAYSETTINGS`, `TEXTSCR`,
//! `GRAPHSCR`, `FINDINCLUDE`, `FINDMATCH` and `REVCLOUDPROPERTIES` each read or
//! flip one token in [`SessionState::drafting_extras`]. The commands take no
//! payload, open no transaction, bump no revision and record no `cad-history`
//! entry.
//!
//! This build wires **no** renderer/behaviour consumer for these tokens: it has
//! no drag preview, no object-snap aperture box, no isometric cursor, no
//! new-feature highlighting, no status-bar tray, no text-screen window, no
//! cross-block find and no revision-cloud arc-length setter. Every outcome says
//! exactly that, and the two read-only forms (`APERTURE`,
//! `REVCLOUDPROPERTIES`) state that their setter form is NOT IMPLEMENTED.
//!
//! `ISOPLANE` and `APERTURE`/`REVCLOUDPROPERTIES` are **tokens, not toggles**:
//! `ISOPLANE` cycles `LEFT → TOP → RIGHT`, and the other two only report the
//! session value.

use super::*;
use crate::app_drafting::toggle_outcome;
use crate::app_reports_ext::diagnostic;
use crate::app_utility::require_none_payload;
use crate::session_state::IsoPlane;

impl Application {
    /// Dispatch one Wave 2 session token.
    pub(crate) fn extended_session2_toggle(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        match ext {
            ExtendedCommandId::DragMode => {
                let enabled = session.drafting_extras.toggle_dragmode();
                Ok(toggle_outcome(
                    "dragmode.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only; the drawing was not \
                         written and drag-preview behaviour is NOT IMPLEMENTED in this build, so \
                         no dragged-image effect is claimed or verified)",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::Aperture => Ok(report(
                "aperture.value",
                format!(
                    "APERTURE: session aperture token is {} (session state only); the setter form \
                     `APERTURE <n>` is NOT IMPLEMENTED, and no object-snap target box is drawn",
                    session.drafting_extras.aperture()
                ),
            )),
            ExtendedCommandId::IsoPlane => {
                let plane: IsoPlane = session.drafting_extras.cycle_isoplane();
                Ok(toggle_outcome(
                    "isoplane.toggled",
                    format!(
                        "{name}: session isometric plane is now {} (session state only); this build \
                         has no isometric cursor or snap change, so no drafting-plane effect is \
                         claimed or verified",
                        plane.label()
                    ),
                ))
            }
            ExtendedCommandId::HighlightNew => {
                let enabled = session.drafting_extras.toggle_highlightnew();
                Ok(toggle_outcome(
                    "highlightnew.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only); this build renders \
                         no new-feature highlight, so no orange-dot effect is claimed",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::TraySettings => {
                let enabled = session.drafting_extras.toggle_traysettings();
                Ok(toggle_outcome(
                    "traysettings.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only); this build has no \
                         status-bar tray UI, so no icon or notification change is claimed",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::TextScreen => {
                session.drafting_extras.set_text_screen(true);
                Ok(toggle_outcome(
                    "textscr.toggled",
                    format!(
                        "{name}: session text-screen token is now ON (session state only); this \
                         build has no text-screen window, so no window is shown and no prompt \
                         history is displayed"
                    ),
                ))
            }
            ExtendedCommandId::GraphScreen => {
                session.drafting_extras.set_text_screen(false);
                Ok(toggle_outcome(
                    "graphscr.toggled",
                    format!(
                        "{name}: session text-screen token is now OFF (session state only); this \
                         build has no text-screen window to hide, so no window change is claimed"
                    ),
                ))
            }
            ExtendedCommandId::FindInclude => {
                let enabled = session.drafting_extras.toggle_find_include();
                Ok(toggle_outcome(
                    "findinclude.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only); cross-block/xref \
                         text search is NOT IMPLEMENTED in this build, so no search scope change \
                         is claimed",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::FindMatch => {
                let enabled = session.drafting_extras.toggle_find_match_case();
                Ok(toggle_outcome(
                    "findmatch.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only); the implemented \
                         `-FIND`/`FIND` reports always compare ASCII case-insensitively, so this \
                         token does not change their matching",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::RevCloudProperties => Ok(report(
                "revcloud.properties",
                format!(
                    "REVCLOUDPROPERTIES: session arc-length token is {} (session state only); the \
                     setter form `REVCLOUDPROPERTIES <length>` is NOT IMPLEMENTED, and REVCLOUD \
                     does not read this token",
                    session.drafting_extras.revcloud_arc_length()
                ),
            )),
            other => Err(CadError::Invariant(format!(
                "not a Wave 2 session token: {other:?}"
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
